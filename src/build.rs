use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use clap::ArgMatches;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    environment::{State, write_json},
    process::{capture, powershell, run},
    workspace::{Repo, Workspace, canonical},
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub directory: PathBuf,
    pub manifest_sha256: String,
    #[serde(skip)]
    pub receipt: PathBuf,
}

impl Candidate {
    pub fn session(&self) -> &Path {
        self.receipt.parent().expect("absolute receipt parent")
    }
}

pub fn candidate(
    workspace: &Workspace,
    state: &State,
    receipt: Option<&PathBuf>,
) -> Result<Candidate> {
    let path = match receipt {
        Some(path) => path.clone(),
        None => {
            let latest: PathBuf = serde_json::from_slice(
                &fs::read(state.dir.join("candidate.json"))
                    .context("Run cnl build --all or select an existing --receipt")?,
            )?;
            latest
        }
    };
    let path = canonical(&path)?;
    let mut candidate: Candidate = serde_json::from_slice(&fs::read(&path)?)?;
    candidate.receipt = path;
    if candidate.directory.parent() != Some(candidate.session())
        || !valid_sha(&candidate.manifest_sha256)
    {
        bail!("Candidate receipt must pin a build in its own output session");
    }
    let remote = workspace.require(Repo::Remote)?;
    run(Command::new("node")
        .arg(remote.join("scripts/verify-native-windows.mjs"))
        .arg(&candidate.directory)
        .arg(&candidate.manifest_sha256))?;
    Ok(candidate)
}

pub fn sha(path: &Path) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
}

fn valid_sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn output_dir(parent: &Path, label: &str) -> Result<PathBuf> {
    fs::create_dir_all(parent)?;
    let moment = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let dir = parent.join(format!("{label}-{moment}-{}", std::process::id()));
    fs::create_dir(&dir)?;
    Ok(dir)
}

fn selected(arguments: &ArgMatches) -> Vec<Repo> {
    arguments
        .get_many::<String>("repo")
        .into_iter()
        .flatten()
        .map(|name| Repo::parse(name))
        .collect()
}

pub fn build(workspace: &Workspace, state: &State, args: &ArgMatches) -> Result<Value> {
    let all = args.get_flag("all");
    let selection = if all {
        Repo::ALL[1..].to_vec()
    } else {
        Repo::ordered(&selected(args))
    };
    let names = selection.iter().map(|repo| repo.name()).collect::<Vec<_>>();
    if args.get_flag("dry-run") {
        return Ok(json!({"dryRun":true,"buildOrder":names,"stateDirectory":state.dir}));
    }
    for &repo in &selection {
        workspace.require(repo)?;
    }
    let _lock = state.lock()?;
    let output = output_dir(&state.dir.join("builds"), "build")?;
    if all {
        let remote = workspace.require(Repo::Remote)?;
        let selection = output.join("selection.json");
        run(Command::new("node")
            .arg(remote.join("scripts/native-source-selection.mjs"))
            .arg("lock")
            .arg(&selection))?;
        let driver = output.join("build.ps1");
        fs::write(&driver, include_str!("../scripts/windows-build.ps1"))?;
        let mut command = powershell();
        command
            .arg(driver)
            .arg("-Remote")
            .arg(remote)
            .arg("-Selection")
            .arg(selection)
            .arg("-OutputParent")
            .arg(&output);
        if args.get_flag("offline") {
            command.arg("-Offline");
        }
        run(&mut command)?;
        let mut receipts = Vec::new();
        for entry in fs::read_dir(&output)? {
            let path = entry?.path().join("assembly-result.json");
            if path.is_file() {
                receipts.push(path);
            }
        }
        if receipts.len() != 1 {
            bail!("Remote did not return exactly one verified assembly receipt");
        }
        let receipt = receipts.pop().unwrap();
        let build = candidate(workspace, state, Some(&receipt))?;
        write_json(&state.dir.join("candidate.json"), &build.receipt)?;
        return Ok(
            json!({"state":"built","buildOrder":names,"receipt":build.receipt,"bundle":build.directory,
            "manifestSha256":build.manifest_sha256}),
        );
    }

    let chosen = if selection
        .iter()
        .any(|repo| matches!(repo, Repo::Interface | Repo::Embedded))
    {
        Some(candidate(workspace, state, args.get_one("receipt"))?)
    } else {
        None
    };
    for repo in selection {
        let dir = workspace.require(repo)?;
        match repo {
            Repo::Simulator => {
                simulator(&dir, &output)?;
            }
            Repo::Interface => {
                let build = chosen.as_ref().unwrap();
                prepare_web(&dir, build, args.get_flag("offline"))?;
                run(Command::new("npm.cmd")
                    .args(["run", "build"])
                    .current_dir(&dir))?;
            }
            Repo::Embedded => rtl(&dir, chosen.as_ref().unwrap(), args, &output)?,
            _ => cargo(&dir, &output, "build", args.get_flag("offline"))?,
        }
    }
    Ok(json!({"state":"built","buildOrder":names,"output":output}))
}

pub fn prepare_web(web: &Path, candidate: &Candidate, offline: bool) -> Result<()> {
    let inputs = candidate.session().join("interface-input.json");
    let selected = trusted_web_inputs(candidate, &inputs)?;
    preserve_old_web_inputs(web, &selected)?;
    let ownership: Value = serde_json::from_slice(&fs::read(
        candidate.session().join(".cirnet-artifact-session.json"),
    )?)?;
    run(Command::new("node")
        .arg(web.join("scripts/bootstrap-contracts.mjs"))
        .arg("--manifest")
        .arg(&inputs)
        .env("CIRNET_ARTIFACTS_CONTEXT", ownership["context"].to_string())
        .env("CIRNET_ARTIFACTS_SESSION", candidate.session())
        .current_dir(web))?;
    let mut npm = Command::new("npm.cmd");
    npm.args(["ci", "--ignore-scripts", "--no-audit", "--no-fund"])
        .current_dir(web);
    if offline {
        npm.env("npm_config_offline", "true");
    }
    run(&mut npm)
}

const WEB_ROLES: [&str; 4] = ["npm", "dev_npm", "native_crate", "remote_openapi"];

fn trusted_web_inputs(candidate: &Candidate, inputs: &Path) -> Result<Value> {
    let selected: Value = serde_json::from_slice(&fs::read(inputs)?)?;
    let manifest: Value =
        serde_json::from_slice(&fs::read(candidate.directory.join("native-manifest.json"))?)?;
    for (role, packaged) in [
        ("npm", "extras/cirnet-contracts"),
        ("dev_npm", "extras/cirnet-artifact-paths"),
        ("native_crate", "extras/contracts-source.crate"),
        ("remote_openapi", "receipts/remote-openapi.json"),
    ] {
        let declared = &selected[role];
        let records = manifest["files"]
            .as_array()
            .context("Missing candidate inventory")?;
        let expected = records
            .iter()
            .find(|record| {
                record["path"].as_str().is_some_and(|path| {
                    path == packaged
                        || matches!(role, "npm" | "dev_npm")
                            && path.starts_with(&format!("{packaged}-"))
                            && path.ends_with(".tgz")
                })
            })
            .with_context(|| format!("Missing candidate input {role}"))?;
        if declared["sha256"].is_null() || declared["sha256"] != expected["sha256"] {
            bail!("The external Interface {role} input is not the selected candidate artifact");
        }
    }
    Ok(selected)
}

fn preserve_old_web_inputs(web: &Path, selected: &Value) -> Result<()> {
    let cache = web.join(".artifacts");
    if !cache.exists() {
        return Ok(());
    }
    let marker = cache.join("selection.json");
    let current: Value = serde_json::from_slice(&fs::read(&marker).context(
        "Interface .artifacts has no selection receipt; move or inspect it before refreshing dependencies",
    )?)?;
    if WEB_ROLES
        .iter()
        .all(|role| current[role]["sha256"] == selected[role]["sha256"])
    {
        return Ok(());
    }
    let ignored = capture(Command::new("git").current_dir(web).args([
        "check-ignore",
        "--",
        ".artifacts/",
        ".runtime/",
    ]))?;
    if !ignored.lines().any(|line| line == ".artifacts/")
        || !ignored.lines().any(|line| line == ".runtime/")
        || canonical(&cache)? != cache
    {
        bail!("Refusing to move an unowned Interface cache");
    }
    let mut expected = vec![marker];
    for role in WEB_ROLES {
        let file = current[role]["path"]
            .as_str()
            .context("Missing cached artifact path")?;
        let file = PathBuf::from(file);
        if file.parent() != Some(cache.as_path())
            || canonical(&file)? != file
            || current[role]["sha256"] != sha(&file)?
        {
            bail!(
                "Interface cached {role} differs from its recorded selection, inspect {}",
                cache.display()
            );
        }
        expected.push(file);
    }
    let inventory = fs::read_dir(&cache)?.collect::<std::io::Result<Vec<_>>>()?;
    if inventory.len() != expected.len()
        || !inventory.iter().all(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file()) && expected.contains(&entry.path())
        })
    {
        bail!(
            "Interface cache contains unrecognized files, inspect {}",
            cache.display()
        );
    }
    let runtime = web.join(".runtime");
    if runtime.exists() && canonical(&runtime)? != runtime {
        bail!("Interface .runtime directory must be an ordinary directory");
    }
    let backup = output_dir(&runtime, "cnl-previous-inputs")?.join(".artifacts");
    fs::rename(&cache, &backup)?;
    eprintln!(
        "Preserved previous Interface inputs at {}",
        backup.display()
    );
    Ok(())
}

pub fn verify(workspace: &Workspace, state: &State, args: &ArgMatches) -> Result<Value> {
    let targets = selected(args);
    if args.get_flag("dry-run") {
        return Ok(
            json!({"dryRun":true,"targets":targets.iter().map(|repo| repo.name()).collect::<Vec<_>>(),
            "integration":targets.is_empty(), "stateDirectory":state.dir}),
        );
    }
    let _lock = state.lock()?;
    let output = output_dir(&state.dir.join("verification"), "verify")?;
    if targets.is_empty() {
        let candidate = candidate(workspace, state, args.get_one("receipt"))?;
        let runner = candidate
            .session()
            .join("src/cirnet-remote-next/scripts/test-native-windows.mjs");
        if !runner.is_file() {
            bail!("The selected candidate has no staged native smoke runner");
        }
        let default_browser = state.dir.join("chromium");
        let browser = args
            .get_one::<PathBuf>("browser-cache")
            .cloned()
            .or_else(|| default_browser.is_dir().then_some(default_browser));
        let mut command = Command::new("node");
        command
            .arg(runner)
            .arg(&candidate.directory)
            .arg(&candidate.manifest_sha256);
        if let Some(cache) = &browser {
            command.arg(canonical(cache)?);
        }
        run(&mut command)?;
        return Ok(
            json!({"state":"verified","receipt":candidate.receipt,"browser":browser.is_some()}),
        );
    }
    for repo in &targets {
        let dir = workspace.require(*repo)?;
        match repo {
            Repo::Simulator => {
                let worker = simulator(&dir, &output)?;
                run(Command::new(python())
                    .current_dir(&dir)
                    .args([
                        "-B",
                        "-m",
                        "pytest",
                        "-q",
                        "-p",
                        "no:cacheprovider",
                        "--basetemp",
                    ])
                    .arg(output.join("sim-pytest"))
                    .arg("tests")
                    .env("CIRNET_TEST_CPP_WORKER", worker))?;
            }
            Repo::Embedded => rtl(
                &dir,
                &candidate(workspace, state, args.get_one("receipt"))?,
                args,
                &output,
            )?,
            Repo::Interface => {
                let build = candidate(workspace, state, args.get_one("receipt"))?;
                let web = build.session().join("src/cirnet-interface-next");
                let ownership: Value = serde_json::from_slice(&fs::read(
                    build.session().join(".cirnet-artifact-session.json"),
                )?)?;
                let mut npm = Command::new("npm.cmd");
                run(npm
                    .args(["test", "--", "--all-script-tests"])
                    .current_dir(web)
                    .env(
                        "CIRNET_CONTRACTS_ARTIFACT_MANIFEST",
                        build.session().join("interface-input.json"),
                    )
                    .env("CIRNET_ARTIFACTS_CONTEXT", ownership["context"].to_string())
                    .env("CIRNET_ARTIFACTS_SESSION", build.session()))?;
            }
            _ => cargo(&dir, &output, "test", false)?,
        }
    }
    Ok(
        json!({"state":"verified","targets":targets.iter().map(|repo| repo.name()).collect::<Vec<_>>(),"output":output}),
    )
}

fn cargo(root: &Path, output: &Path, task: &str, offline: bool) -> Result<()> {
    let driver = output.join("cargo.ps1");
    fs::write(&driver, include_str!("../scripts/windows-build.ps1"))?;
    let mut command = powershell();
    command
        .arg(driver)
        .arg("-Component")
        .arg(root)
        .args(["-CargoOperation", task])
        .env("CARGO_TARGET_DIR", output.join("cargo"))
        .current_dir(root);
    if offline {
        command.arg("-Offline");
    }
    run(&mut command)
}

fn simulator(root: &Path, output: &Path) -> Result<PathBuf> {
    let build = output.join("simulator-build");
    let install = output.join("simulator");
    run(Command::new("cmake")
        .arg("-S")
        .arg(root)
        .arg("-B")
        .arg(&build)
        .args(["-G", "Visual Studio 17 2022", "-A", "x64"]))?;
    run(Command::new("cmake")
        .arg("--build")
        .arg(&build)
        .args(["--config", "Release"]))?;
    run(Command::new("cmake")
        .arg("--install")
        .arg(&build)
        .args(["--config", "Release", "--prefix"])
        .arg(&install))?;
    Ok(install.join("bin/cirnet-simulator-worker.exe"))
}

fn python() -> std::ffi::OsString {
    env::var_os("CNL_VERIFICATION_PYTHON").unwrap_or_else(|| "python".into())
}

fn rtl(root: &Path, candidate: &Candidate, args: &ArgMatches, output: &Path) -> Result<()> {
    let toolchain = args
        .get_one::<PathBuf>("fpga-toolchain")
        .cloned()
        .or_else(|| env::var_os("CIRNET_FPGA_TOOLCHAIN_ROOT").map(PathBuf::from))
        .context("Set --fpga-toolchain to the pinned Embedded Hardware toolchain root")?;
    let toolchain = canonical(&toolchain)?;
    let receipt = candidate
        .session()
        .join("contracts/contracts-native-manifest.json");
    let allocator = candidate.session().join(
        "src/cirnet-contracts-next/tools/artifact-paths/cirnet_artifact_paths/artifact-paths.mjs",
    );
    let mut command = Command::new(python());
    run(command
        .current_dir(root)
        .args(["-B", "scripts/check_rtl.py", "--toolchain-root"])
        .arg(toolchain)
        .arg("--contracts-receipt")
        .arg(&receipt)
        .arg("--contracts-sha256")
        .arg(sha(&receipt)?)
        .env("CIRNET_ARTIFACTS_DIR", output)
        .env_remove("CIRNET_ARTIFACTS_SESSION")
        .env_remove("NETCIRCUIT_ARTIFACTS_SESSION")
        .env_remove("CIRNET_ARTIFACTS_CONTEXT")
        .env_remove("NETCIRCUIT_ARTIFACTS_CONTEXT")
        .env("CIRNET_ARTIFACTS_CLI_SHA256", sha(&allocator)?)
        .env("CIRNET_ARTIFACTS_CLI", allocator))
}
