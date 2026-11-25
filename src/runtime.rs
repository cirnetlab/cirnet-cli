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
    localization::{self, MISSING_INVENTORY, START_FAILED, message},
    process::{capture, hidden, isolate_stdio, powershell, run, sanitize_console_output},
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
    #[serde(default)]
    target: Target,
    native_stopped: bool,
    native_run_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Target {
    #[default]
    Interface,
}

impl Target {
    pub(crate) const VALUES: [&'static str; 1] = ["interface"];

    pub(crate) fn requested(args: &ArgMatches) -> Option<Self> {
        args.get_one::<String>("desktop").map(|_| Self::Interface)
    }

    fn verify(self, requested: Option<Self>) -> Result<()> {
        if requested.is_some_and(|target| target != self) {
            bail!("{}", message(localization::RUNTIME_TARGET_MISMATCH));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Development,
    Packaged,
}

impl Mode {
    fn from_dev(dev: bool) -> Self {
        if dev {
            Self::Development
        } else {
            Self::Packaged
        }
    }
}

fn read_instance(state: &State) -> Result<Option<Instance>> {
    match fs::read(record_path(state)) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn record_path(state: &State) -> PathBuf {
    state.dir.join("instance.json")
}

pub fn verify_ports(ports: &[u16]) -> Result<()> {
    for port in ports {
        if std::net::TcpStream::connect_timeout(
            &std::net::SocketAddrV4::new(Ipv4Addr::LOCALHOST, *port).into(),
            Duration::from_millis(300),
        )
        .is_ok()
        {
            bail!("127.0.0.1:{port} {}", message(localization::PORT_IN_USE));
        }
        TcpListener::bind((Ipv4Addr::LOCALHOST, *port)).with_context(|| {
            format!(
                "{} 127.0.0.1:{port}. {}",
                message(localization::PORT_BIND_FAILED),
                message(localization::PORT_BIND_GUIDANCE)
            )
        })?;
    }
    Ok(())
}

pub(crate) fn start(workspace: &Workspace, state: &State, args: &ArgMatches) -> Result<Value> {
    let profile = &state.profile;
    let requested = Target::requested(args);
    let target = requested.unwrap_or_default();
    let dev = args
        .get_one::<String>("mode")
        .is_none_or(|mode| mode == "development");
    let mode = Mode::from_dev(dev);
    let open_browser = args.get_flag("open") || requested.is_some();
    if args.get_flag("dry-run") {
        return Ok(
            json!({"dryRun":true, "mode":mode,"target":target,"ports":profile,
            "dataDirectory":state.dir.join("data"),"initializeDb":args.get_flag("initialize-db"),
            "receipt":args.get_one::<PathBuf>("receipt"),"openBrowser":open_browser}),
        );
    }
    if !cfg!(all(windows, target_arch = "x86_64")) {
        bail!("{}", message(localization::RUNTIME_PLATFORM));
    }
    let _lock = state.lock()?;
    if let Some(prior) = read_instance(state)? {
        if prior.supervisor.active()? || browser_active(&prior)? {
            bail!("{}", message(localization::RUNTIME_ALREADY_OWNED));
        }
        reconcile_exited(state, &prior, ownership::windows_boot_time)?;
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
            "{} {}. {}",
            message(localization::SQLITE_MISSING),
            data.display(),
            message(localization::SQLITE_INITIALIZE)
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
    let receipt = candidate.receipt.clone();
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
        target,
        native_stopped: false,
        native_run_id: None,
    };
    let record = record_path(state);
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
            bail!(
                "{} · {backend}",
                message(localization::NATIVE_READINESS_FAILED)
            );
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
                bail!("{}", message(localization::VITE_READINESS_FAILED));
            }
        }
        Ok(backend)
    })();
    let backend = match startup {
        Ok(status) => status,
        Err(error) => {
            if instance.native_run_id.is_some() {
                let _ = shutdown(state, &mut instance);
            } else if supervisor.try_wait()?.is_some()
                && !data.join("supervisor.json").try_exists()?
            {
                fs::remove_file(&record)?;
            }
            return Err(error).with_context(|| {
                format!(
                    "{} {}",
                    message(localization::RUNTIME_START_FAILED),
                    instance.log_dir.display()
                )
            });
        }
    };
    let url = url(&instance);
    if open_browser {
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
            eprintln!(
                "{} {url} ({error}).",
                message(localization::BROWSER_OPEN_GUIDANCE)
            );
        }
    }
    Ok(
        json!({"state":"ready","mode":mode,"target":target,"url":url,"native":backend,"ports":profile,"logDirectory":instance.log_dir,
            "receipt":receipt,"manifestSha256":instance.digest}),
    )
}

pub fn status(state: &State) -> Result<Value> {
    let Some(instance) = read_instance(state)? else {
        return Ok(json!({"state":"stopped","stateDirectory":state.dir}));
    };
    let active = instance.supervisor.active()?;
    let dev_active = browser_active(&instance)?;
    let native = if active {
        probe(&instance, false).unwrap_or_else(
            |error| json!({"healthy":false,"detail":localization::report_diagnostic(&error)}),
        )
    } else {
        json!({"healthy":false})
    };
    let dev = if dev_active {
        probe(&instance, true).unwrap_or_else(
            |error| json!({"healthy":false,"detail":localization::report_diagnostic(&error)}),
        )
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
        "mode":Mode::from_dev(instance.dev),"target":instance.target,
        "ports":instance.profile,"url":url(&instance),"supervisorPid":instance.supervisor.pid, "services":services,
        "vitePid":instance.browser_server.as_ref().map(|process| process.pid),
        "native":native,"vite":dev,"logDirectory":instance.log_dir}),
    )
}

pub(crate) fn stop(state: &State, target: Option<Target>, dry: bool) -> Result<Value> {
    if dry {
        let instance = read_instance(state)?;
        if let Some(instance) = &instance {
            instance.target.verify(target)?;
        }
        return Ok(
            json!({"dryRun":true,"ownedInstance":instance.is_some(),"target":instance.map(|i| i.target).or(target),"stateDirectory":state.dir}),
        );
    }
    if !state.dir.exists() {
        return Ok(json!({"state":"stopped"}));
    }
    let _lock = state.lock()?;
    let Some(mut instance) = read_instance(state)? else {
        return Ok(json!({"state":"stopped"}));
    };
    instance.target.verify(target)?;
    shutdown(state, &mut instance)?;
    Ok(json!({"state":"stopped","logDirectory":instance.log_dir}))
}

fn browser_active(instance: &Instance) -> Result<bool> {
    Ok(instance
        .browser_server
        .as_ref()
        .map(Process::active)
        .transpose()?
        .unwrap_or(false))
}

fn stop_browser(instance: &Instance) -> Result<()> {
    if let Some(server) = &instance.browser_server
        && server.active()?
    {
        fs::write(instance.log_dir.join("vite.stop"), "")?;
        wait_for_exit(server, Duration::from_secs(15))?;
        let outcome = fs::read_to_string(instance.log_dir.join("vite.result"))
            .context(message(localization::VITE_CLEANUP_MISSING))?;
        if outcome.trim() != "clean" {
            bail!(
                "{} {} · {}",
                message(localization::VITE_CLEANUP),
                outcome.trim(),
                instance.log_dir.display()
            );
        }
    }
    Ok(())
}

fn shutdown(state: &State, instance: &mut Instance) -> Result<()> {
    // Try native Stop even if Vite cannot exit; it owns service and worker cleanup.
    let vite = stop_browser(instance);
    if !instance.supervisor.active()? {
        vite?;
        return reconcile_exited(state, instance, ownership::windows_boot_time);
    }
    if instance.native_stopped {
        bail!("{}", message(localization::RUNTIME_ALREADY_OWNED));
    }
    if sha(&instance.bundle.join("cirnet-native.ps1"))? != instance.launcher_hash {
        bail!("{}", message(localization::STOP_LAUNCHER_CHANGED));
    }
    let expected = instance
        .native_run_id
        .as_deref()
        .context(message(localization::LAUNCHER_ID_MISSING))?;
    if native_run_id(state)? != expected {
        bail!("{}", message(localization::RUNTIME_RUN_MISMATCH));
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
    write_json(&record_path(state), instance)?;
    vite?;
    Ok(fs::remove_file(record_path(state))?)
}

fn reconcile_exited(
    state: &State,
    instance: &Instance,
    boot_time: impl FnOnce() -> Option<u64>,
) -> Result<()> {
    (|| {
        let data = state.dir.join("data");
        // Hold the launcher's exclusive lock through reconciliation. A native
        // launcher invoked outside the CLI uses this lock for the same data directory.
        let _native_lock = ownership::open_native_run_lock(&data.join("run.lock"))
            .context(message(localization::RUNTIME_NATIVE_LOCK))?;
        if instance.supervisor.active()? || browser_active(instance)? {
            bail!("{}", message(localization::RUNTIME_ALREADY_OWNED));
        }
        let control = data.join("supervisor.json").try_exists()?;
        let run_id = instance
            .native_run_id
            .as_deref()
            .map(valid_run_id)
            .transpose()?;
        if let Some(id) = run_id
            && data.join(format!("stop-{id}")).try_exists()?
        {
            bail!("{}", message(localization::RUNTIME_STOP_PENDING));
        }
        if instance.native_stopped {
            if control {
                bail!("{}", message(localization::RUNTIME_RUN_MISMATCH));
            }
        } else {
            let id = run_id.context(message(localization::LAUNCHER_ID_MISSING))?;
            match fs::read(data.join(format!("result-{id}"))) {
                Ok(result) if result == b"clean" && !control => {}
                Ok(result) if result == b"clean" => {
                    bail!("{}", message(localization::RUNTIME_RUN_MISMATCH));
                }
                Ok(_) => bail!("{}", message(localization::RUNTIME_RESULT_UNCLEAN)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if !control || native_run_id(state)? != id {
                        bail!("{}", message(localization::RUNTIME_RUN_MISMATCH));
                    }
                    // CLI Start uses the verified, simulation-only native launcher.
                    // A later full Windows boot proves that its old process tree is gone.
                    if sha(&instance.bundle.join("cirnet-native.ps1"))? != instance.launcher_hash
                        || sha(&instance.bundle.join("native-manifest.json"))? != instance.digest
                    {
                        bail!("{}", message(localization::STOP_LAUNCHER_CHANGED));
                    }
                    if !boot_time().is_some_and(|boot| {
                        instance.supervisor.created_before(boot)
                            && instance
                                .browser_server
                                .as_ref()
                                .is_none_or(|p| p.created_before(boot))
                    }) {
                        bail!("{}", message(localization::RUNTIME_REBOOT_UNCONFIRMED));
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        instance.profile.validate()?;
        let mut ports = vec![instance.profile.port, instance.profile.hardware_port];
        if instance.dev {
            ports.push(instance.profile.dev_port);
        }
        verify_ports(&ports)?;
        // The launcher owns its control file and will replace it on the next Start.
        Ok(fs::remove_file(record_path(state))?)
    })()
    .with_context(|| {
        format!(
            "{} {}. {}",
            message(localization::RUNTIME_EXITED_UNCONFIRMED),
            instance.log_dir.display(),
            message(localization::RUNTIME_EXITED_GUIDANCE)
        )
    })
}

fn native_run_id(state: &State) -> Result<String> {
    let control: Value =
        serde_json::from_slice(&fs::read(state.dir.join("data/supervisor.json"))?)?;
    let id = control["id"]
        .as_str()
        .context(message(localization::LAUNCHER_ID_MISSING))?;
    Ok(valid_run_id(id)?.to_owned())
}

fn valid_run_id(id: &str) -> Result<&str> {
    if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{}", message(localization::LAUNCHER_ID_INVALID));
    }
    Ok(id)
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
            bail!(localization::Diagnostic::new(|vi| {
                localization::text(localization::REMOTE_EXECUTABLE_CHANGED, vi).to_owned()
            }));
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
                status["hardwareDetail"] = localization::report_diagnostic(&error);
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
        .context(message(MISSING_INVENTORY))?;
    let digest = |path: &str| {
        files
            .iter()
            .find(|record| record["path"] == path)
            .and_then(|file| file["sha256"].as_str())
            .with_context(|| {
                format!(
                    "{} {path}",
                    message(localization::CANDIDATE_VERIFIED_FILE_MISSING)
                )
            })
    };
    let metadata: Value = serde_json::from_slice(&fs::read(
        candidate.directory.join("web/interface-artifact.json"),
    )?)?;
    let fingerprint = metadata["sourceFingerprint"]
        .as_str()
        .context(message(localization::INTERFACE_FINGERPRINT_MISSING))?;
    Ok((
        fingerprint.into(),
        digest("cirnet-native.ps1")?.into(),
        digest("bin/cirnet-remote.exe")?.into(),
    ))
}

fn launcher(bundle: &Path, digest: &str, data: &Path, profile: &Profile, action: &str) -> Command {
    let mut command = powershell();
    command.current_dir(bundle);
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
            "{} {}",
            message(START_FAILED),
            command.get_program().to_string_lossy()
        )
    })
}

fn wait_for(child: &mut Child, log: &Path, marker: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            bail!(
                "{} {status} · {} {}",
                message(localization::SERVICE_EXITED),
                message(localization::READINESS_BEFORE),
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
    bail!("{} {}", message(localization::READY_TIMEOUT), log.display())
}

fn wait_for_exit(process: &Process, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while process.active()? {
        if Instant::now() >= deadline {
            bail!(
                "{} {} · {}",
                message(localization::OWNED_PROCESS),
                process.pid,
                message(localization::SHUTDOWN_UNCONFIRMED)
            );
        }
        thread::sleep(Duration::from_millis(150));
    }
    Ok(())
}

pub(crate) fn logs(state: &State, args: &ArgMatches) -> Result<Value> {
    let directory = state.dir.join("logs");
    let session = if let Some(instance) = read_instance(state)? {
        instance.log_dir
    } else {
        let latest = match fs::read_dir(&directory) {
            Ok(entries) => entries
                .collect::<std::io::Result<Vec<_>>>()?
                .into_iter()
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("runtime-"))
                .max_by_key(|entry| entry.file_name()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        latest.context(message(localization::LOGS_MISSING))?.path()
    };
    if session.parent() != Some(directory.as_path())
        || crate::workspace::canonical(&session)? != session
        || !fs::symlink_metadata(&session)?.is_dir()
    {
        bail!("{}", message(localization::LOGS_SESSION_OUTSIDE_PROFILE));
    }
    let service = args.get_one::<String>("service").expect("default service");
    let path = session.join(format!("{service}.log"));
    if !fs::symlink_metadata(&path)
        .with_context(|| {
            format!(
                "{} {service} · {}",
                message(localization::LOG_MISSING),
                session.display()
            )
        })?
        .is_file()
        || crate::workspace::canonical(&path)? != path
    {
        bail!("{}", message(localization::LOG_INVALID));
    }
    let lines = usize::from(*args.get_one::<u16>("lines").expect("default line count"));
    Ok(json!({"service":service,"path":path,"content":log_tail(File::open(&path)?, lines)?}))
}

fn log_tail(
    mut reader: impl std::io::Read + std::io::Seek,
    lines: usize,
) -> std::io::Result<String> {
    use std::io::{Read, SeekFrom};
    const LIMIT: u64 = 64 * 1024;
    let end = reader.seek(SeekFrom::End(0))?;
    let start = end.saturating_sub(LIMIT);
    reader.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    reader.take(LIMIT).read_to_end(&mut bytes)?;
    let bytes = if start == 0 {
        bytes.as_slice()
    } else {
        bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(&[][..], |offset| &bytes[offset + 1..])
    };
    let text = sanitize_console_output(bytes);
    let mut tail = text.lines().rev().take(lines).collect::<Vec<_>>();
    tail.reverse();
    Ok(tail.join("\n"))
}

#[cfg(all(test, windows))]
mod recovery_tests;

#[cfg(test)]
mod log_tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn log_tail_caps_bytes_and_rows_and_strips_secrets() {
        let input = format!(
            "{}\nignored\n\x1b[31mready\x1b[0m\nAuthorization: Bearer sample\n",
            "x".repeat(65536)
        );
        assert_eq!(
            log_tail(Cursor::new(input), 2).unwrap(),
            "ready\n[redacted]"
        );
        assert_eq!(log_tail(Cursor::new("x".repeat(65537)), 2).unwrap(), "");
        let key =
            "\n-----BEGIN RSA PRIVATE KEY-----\nprivate body\n-----END RSA PRIVATE KEY-----\nready";
        assert_eq!(log_tail(Cursor::new(key), 2).unwrap(), "[redacted]\nready");
    }

    #[test]
    fn legacy_instance_records_resolve_to_the_interface_target() {
        let record = json!({
            "profile": {},
            "bundle": "fixture", "digest": "", "launcher_hash": "",
            "remote_hash": "", "fingerprint": "", "log_dir": "fixture",
            "supervisor": { "pid": 1, "created": 0 },
            "browser_server": null, "dev": true, "native_stopped": false,
            "native_run_id": null
        });
        let saved: Instance = serde_json::from_value(record.clone()).unwrap();
        assert_eq!(saved.target, Target::Interface);
        assert!(saved.target.verify(Some(Target::Interface)).is_ok());
        assert_eq!(serde_json::to_value(&saved).unwrap()["target"], "interface");
        let mut unsupported = record;
        unsupported["target"] = json!("unknown");
        assert!(serde_json::from_value::<Instance>(unsupported).is_err());
    }
}
