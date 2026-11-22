mod ownership;

use std::{
    fs::{self, File},
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use clap::ArgMatches;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    build::{self, Candidate, sha},
    environment::{Profile, State, write_json},
    process::{capture, hidden, isolate_stdio, powershell, run},
    workspace::{Repo, Workspace},
};
use ownership::Process;

#[derive(Debug, Serialize, Deserialize)]
struct Instance {
    profile: Profile,
    bundle: PathBuf,
    digest: String,
    launcher_hash: String,
    remote_hash: String,
    fingerprint: String,
    log_dir: PathBuf,
    supervisor: Process,
    browser_server: Option<Process>,
    dev: bool,
    native_stopped: bool,
    native_run_id: Option<String>,
}

fn read_instance(state: &State) -> Result<Option<Instance>> {
    match fs::read(state.dir.join("instance.json")) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn verify_ports(ports: &[u16]) -> Result<()> {
    for port in ports {
        if std::net::TcpStream::connect_timeout(
            &std::net::SocketAddrV4::new(Ipv4Addr::LOCALHOST, *port).into(),
            Duration::from_millis(300),
        )
        .is_ok()
        {
            bail!("127.0.0.1:{port} is already in use. Select another port or stop its owner.");
        }
        TcpListener::bind((Ipv4Addr::LOCALHOST, *port))
            .with_context(|| format!("Cannot bind 127.0.0.1:{port}; check for another listener or select a different port"))?;
    }
    Ok(())
}

pub fn start(workspace: &Workspace, state: &State, args: &ArgMatches, dev: bool) -> Result<Value> {
    let profile = &state.profile;
    if args.get_flag("dry-run") {
        return Ok(
            json!({"dryRun":true, "mode":if dev {"dev"} else {"native"},"ports":profile,
            "dataDirectory":state.dir.join("data"),"initializeDb":args.get_flag("initialize-db"),
            "receipt":args.get_one::<PathBuf>("receipt")}),
        );
    }
    if !cfg!(all(windows, target_arch = "x86_64")) {
        bail!("CIRNETLAB native runtime requires Windows x64");
    }
    let _lock = state.lock()?;
    if let Some(prior) = read_instance(state)? {
        if prior.supervisor.active()?
            || prior
                .browser_server
                .as_ref()
                .is_some_and(|process| process.active().unwrap_or(true))
        {
            bail!(
                "This profile already owns a running process. Inspect cnl status before starting again."
            );
        }
        bail!(
            "The previous instance exited without a confirmed cnl stop. Inspect {} and use a different profile until shutdown is resolved.",
            prior.log_dir.display()
        );
    }
    let mut ports = vec![profile.port, profile.hardware_port];
    if dev {
        ports.push(profile.dev_port);
    }
    verify_ports(&ports)?;
    let candidate = build::candidate(workspace, state, args.get_one("receipt"))?;
    let data = state.dir.join("data");
    if !data.join("cirnet.db").is_file() && !args.get_flag("initialize-db") {
        bail!(
            "No initialized SQLite database in {}. Use --initialize-db to create a fresh database.",
            data.display()
        );
    }
    let web = if dev {
        let path = workspace.require(Repo::Interface)?;
        build::prepare_web(&path, &candidate, false)?;
        Some(path)
    } else {
        None
    };
    let log_dir = build::output_dir(&state.dir.join("logs"), "runtime")?;
    fs::write(
        log_dir.join("probe.mjs"),
        include_str!("../scripts/probe.mjs"),
    )?;
    let (fingerprint, launcher_hash, remote_hash) = runtime_identity(&candidate)?;
    let mut native = launcher(
        &candidate.directory,
        &candidate.manifest_sha256,
        &data,
        profile,
        "Start",
    );
    if dev {
        native.args(["-DevPort", &profile.dev_port.to_string()]);
    }
    if args.get_flag("initialize-db") {
        native.arg("-InitializeDatabase");
    }
    let mut supervisor = background(&mut native, &log_dir.join("native.log"))?;
    let mut instance = Instance {
        profile: profile.clone(),
        bundle: candidate.directory,
        digest: candidate.manifest_sha256,
        launcher_hash,
        remote_hash,
        fingerprint,
        log_dir,
        supervisor: Process::identify(&supervisor)?,
        browser_server: None,
        dev,
        native_stopped: false,
        native_run_id: None,
    };
    let record = state.dir.join("instance.json");
    write_json(&record, &instance)?;
    let startup = (|| -> Result<Value> {
        wait_for(
            &mut supervisor,
            &instance.log_dir.join("native.log"),
            &format!("CIRNET_NATIVE_READY=http://127.0.0.1:{}", profile.port),
        )?;
        instance.native_run_id = Some(native_run_id(state)?);
        write_json(&record, &instance)?;
        let backend = probe(&instance, false)?;
        if backend["healthy"] != true {
            bail!("Native application failed HTTP, Interface or Hardware RPC readiness: {backend}");
        }
        if let Some(web) = web {
            let script = instance.log_dir.join("vite.mjs");
            fs::write(&script, include_str!("../scripts/vite.mjs"))?;
            let stop = instance.log_dir.join("vite.stop");
            let mut command = Command::new("node");
            command
                .arg(script)
                .arg(&web)
                .arg(profile.dev_port.to_string())
                .arg(stop)
                .current_dir(web)
                .env(
                    "CIRNET_DEV_API_TARGET",
                    format!("http://127.0.0.1:{}", profile.port),
                );
            let log = instance.log_dir.join("vite.log");
            let mut child = background(&mut command, &log)?;
            instance.browser_server = Some(Process::identify(&child)?);
            write_json(&record, &instance)?;
            wait_for(
                &mut child,
                &log,
                &format!("CIRNET_VITE_READY=http://127.0.0.1:{}", profile.dev_port),
            )?;
            if probe(&instance, true)?["healthy"] != true {
                bail!("Vite frontend or proxied API is unhealthy");
            }
        }
        Ok(backend)
    })();
    let backend = match startup {
        Ok(status) => status,
        Err(error) => {
            let acted = if instance.supervisor.active()? && data.join("supervisor.json").is_file() {
                shutdown(state, &mut instance).is_ok()
            } else {
                supervisor.try_wait()?.is_some() && !data.join("supervisor.json").exists()
            };
            if acted {
                fs::remove_file(&record)?;
            }
            return Err(error)
                .with_context(|| format!("Startup failed. See {}", instance.log_dir.display()));
        }
    };
    let url = url(&instance);
    if args.get_flag("open") {
        let mut command = Command::new("rundll32.exe");
        hidden(
            command
                .arg("url.dll,FileProtocolHandler")
                .arg(&url)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        );
        if let Err(error) = command.spawn() {
            eprintln!("Open {url} in your browser ({error}).");
        }
    }
    Ok(
        json!({"state":"ready","url":url,"native":backend,"ports":profile,"logDirectory":instance.log_dir}),
    )
}

pub fn status(state: &State) -> Result<Value> {
    let Some(instance) = read_instance(state)? else {
        return Ok(json!({"state":"stopped","stateDirectory":state.dir}));
    };
    let active = instance.supervisor.active()?;
    let dev_active = instance
        .browser_server
        .as_ref()
        .map(Process::active)
        .transpose()?
        .unwrap_or(false);
    let native = if active {
        probe(&instance, false)
            .unwrap_or_else(|error| json!({"healthy":false,"detail":error.to_string()}))
    } else {
        json!({"healthy":false})
    };
    let dev = if dev_active {
        probe(&instance, true)
            .unwrap_or_else(|error| json!({"healthy":false,"detail":error.to_string()}))
    } else {
        Value::Null
    };
    let healthy = active && native["healthy"] == true && (!instance.dev || dev["healthy"] == true);
    let services = if active {
        fs::read(state.dir.join("data/supervisor.json")).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .map(|metadata| json!({"remotePid":metadata["remote"],"hardwarePid":metadata["hardware"]}))
    } else {
        None
    };
    Ok(
        json!({"state":if healthy {"ready"} else if active {"unhealthy"} else {"exited-without-confirmed-stop"},
        "ports":instance.profile,"url":url(&instance),"supervisorPid":instance.supervisor.pid, "services":services,
        "vitePid":instance.browser_server.as_ref().map(|process| process.pid),
        "native":native,"vite":dev,"logDirectory":instance.log_dir}),
    )
}

pub fn stop(state: &State, dry: bool) -> Result<Value> {
    if dry {
        return Ok(
            json!({"dryRun":true,"ownedInstance":read_instance(state)?.is_some(),"stateDirectory":state.dir}),
        );
    }
    if !state.dir.exists() {
        return Ok(json!({"state":"stopped"}));
    }
    let _lock = state.lock()?;
    let Some(mut instance) = read_instance(state)? else {
        return Ok(json!({"state":"stopped"}));
    };
    shutdown(state, &mut instance)?;
    fs::remove_file(state.dir.join("instance.json"))?;
    Ok(json!({"state":"stopped","logDirectory":instance.log_dir}))
}

fn shutdown(state: &State, instance: &mut Instance) -> Result<()> {
    // Try native Stop even if Vite cannot exit; it owns service and worker cleanup.
    let vite = (|| -> Result<()> {
        if let Some(server) = &instance.browser_server
            && server.active()?
        {
            fs::write(instance.log_dir.join("vite.stop"), "")?;
            wait_for_exit(server, Duration::from_secs(15))?;
            let outcome = fs::read_to_string(instance.log_dir.join("vite.result"))
                .context("Vite exited without a cleanup receipt")?;
            if outcome.trim() != "clean" {
                bail!(
                    "Vite cleanup {}. See {}",
                    outcome.trim(),
                    instance.log_dir.display()
                );
            }
        }
        Ok(())
    })();
    if instance.native_stopped {
        return vite;
    }
    if !instance.supervisor.active()? {
        bail!("The owned supervisor already exited. Clean native shutdown cannot be confirmed.");
    }
    if sha(&instance.bundle.join("cirnet-native.ps1"))? != instance.launcher_hash {
        bail!("The selected Stop launcher has changed. Restore the verified file before stopping.");
    }
    if let Some(expected) = &instance.native_run_id
        && &native_run_id(state)? != expected
    {
        bail!("The launcher run identity no longer matches the cnl-owned instance.");
    }
    run(&mut launcher(
        &instance.bundle,
        &instance.digest,
        &state.dir.join("data"),
        &instance.profile,
        "Stop",
    ))?;
    wait_for_exit(&instance.supervisor, Duration::from_secs(10))?;
    instance.native_stopped = true;
    write_json(&state.dir.join("instance.json"), instance)?;
    vite
}

fn native_run_id(state: &State) -> Result<String> {
    let control: Value =
        serde_json::from_slice(&fs::read(state.dir.join("data/supervisor.json"))?)?;
    let id = control["id"]
        .as_str()
        .context("Missing launcher run identity")?;
    if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("Invalid launcher run identity");
    }
    Ok(id.to_owned())
}

fn url(instance: &Instance) -> String {
    let port = if instance.dev {
        instance.profile.dev_port
    } else {
        instance.profile.port
    };
    format!("http://127.0.0.1:{port}")
}

fn probe(instance: &Instance, dev: bool) -> Result<Value> {
    // The probe runs only after a confirmed process identity and never emits session credentials.
    let port = if dev {
        instance.profile.dev_port
    } else {
        instance.profile.port
    };
    let check = capture(
        Command::new("node")
            .arg(instance.log_dir.join("probe.mjs"))
            .arg(port.to_string())
            .arg(if dev { "vite" } else { &instance.fingerprint }),
    )?;
    let mut status: Value = serde_json::from_str(&check)?;
    if !dev {
        let executable = instance.bundle.join("bin/cirnet-remote.exe");
        if sha(&executable)? != instance.remote_hash {
            bail!("The selected Remote executable changed since startup");
        }
        let mut command = Command::new(executable);
        for (name, _) in std::env::vars_os() {
            let upper = name.to_string_lossy().to_ascii_uppercase();
            if upper.starts_with("CIRNET_") || upper.starts_with("NETCIRCUIT_") {
                command.env_remove(name);
            }
        }
        let hardware = capture(command.arg("--check-hardware").env(
            "CIRNET_HARDWARE_GRPC_TARGET",
            format!("127.0.0.1:{}", instance.profile.hardware_port),
        ));
        let rpc = match hardware {
            Ok(receipt) => {
                let receipt: Value = serde_json::from_str(&receipt)?;
                receipt["status"] == "ok" && receipt["station_id"] == "virtual-station-01"
            }
            Err(error) => {
                status["hardwareDetail"] = json!(error.to_string());
                false
            }
        };
        status["hardwareRpc"] = json!(rpc);
        status["healthy"] = json!(status["healthy"] == true && rpc);
    }
    Ok(status)
}

fn runtime_identity(candidate: &Candidate) -> Result<(String, String, String)> {
    let manifest: Value =
        serde_json::from_slice(&fs::read(candidate.directory.join("native-manifest.json"))?)?;
    let files = manifest["files"]
        .as_array()
        .context("Missing candidate inventory")?;
    let digest = |path: &str| {
        files
            .iter()
            .find(|record| record["path"] == path)
            .and_then(|file| file["sha256"].as_str())
            .with_context(|| format!("Candidate has no verified {path}"))
    };
    let metadata: Value = serde_json::from_slice(&fs::read(
        candidate.directory.join("web/interface-artifact.json"),
    )?)?;
    let fingerprint = metadata["sourceFingerprint"]
        .as_str()
        .context("Candidate has no Interface source fingerprint")?;
    Ok((
        fingerprint.into(),
        digest("cirnet-native.ps1")?.into(),
        digest("bin/cirnet-remote.exe")?.into(),
    ))
}

fn launcher(bundle: &Path, digest: &str, data: &Path, profile: &Profile, action: &str) -> Command {
    let mut command = powershell();
    command
        .arg(bundle.join("cirnet-native.ps1"))
        .args(["-Action", action, "-ManifestSha256", digest])
        .arg("-DataDirectory")
        .arg(data)
        .args([
            "-Port",
            &profile.port.to_string(),
            "-HardwarePort",
            &profile.hardware_port.to_string(),
        ]);
    command
}

fn background(command: &mut Command, log: &Path) -> Result<Child> {
    isolate_stdio()?;
    let output = File::options().create_new(true).write(true).open(log)?;
    hidden(
        command
            .stdin(Stdio::null())
            .stdout(output.try_clone()?)
            .stderr(output),
    );
    command.spawn().with_context(|| {
        format!(
            "Could not start {}",
            command.get_program().to_string_lossy()
        )
    })
}

fn wait_for(child: &mut Child, log: &Path, marker: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            bail!(
                "Service exited with {status} before reporting readiness in {}",
                log.display()
            );
        }
        if fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .any(|line| line.trim() == marker)
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(150));
    }
    bail!("Timed out waiting for readiness in {}", log.display())
}

fn wait_for_exit(process: &Process, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while process.active()? {
        if Instant::now() >= deadline {
            bail!("Owned process {} did not confirm shutdown", process.pid);
        }
        thread::sleep(Duration::from_millis(150));
    }
    Ok(())
}
