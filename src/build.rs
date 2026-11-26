use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use clap::ArgMatches;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    environment::{State, write_json},
    localization::{self, MISSING_INVENTORY, message},
    process::{capture, isolate_git, powershell, run},
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

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SavedCandidate {
    Scoped(CandidateSelection),
    Legacy(PathBuf),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CandidateSelection {
    schema_version: u8,
    workspace: PathBuf,
    state_directory: PathBuf,
    profile: String,
    receipt: PathBuf,
    manifest_sha256: String,
}

impl CandidateSelection {
    fn new(workspace: &Workspace, state: &State, candidate: &Candidate) -> Self {
        Self {
            schema_version: 1,
            workspace: workspace.root.clone(),
            state_directory: state.dir.clone(),
            profile: state.name.clone(),
            receipt: candidate.receipt.clone(),
            manifest_sha256: candidate.manifest_sha256.clone(),
        }
    }

    fn validate_scope(&self, workspace: &Workspace, state: &State) -> Result<()> {
        if self.schema_version != 1 {
            bail!(
                "{} {}",
                message(localization::CANDIDATE_SCHEMA),
                self.schema_version
            );
        }
        if self.workspace != workspace.root
            || self.state_directory != state.dir
            || self.profile != state.name
        {
            bail!(
                "{} {} · {} · {}. {}",
                message(localization::CANDIDATE_SCOPE),
                self.workspace.display(),
                self.profile,
                self.state_directory.display(),
                message(localization::CANDIDATE_SCOPE_GUIDANCE)
            );
        }
        Ok(())
    }
}

pub fn candidate(
    workspace: &Workspace,
    state: &State,
    receipt: Option<&PathBuf>,
) -> Result<Candidate> {
    let pointer = state.dir.join("candidate.json");
    let (path, digest) = if let Some(receipt) = receipt {
        (receipt.clone(), None)
    } else {
        let saved = (|| -> Result<_> {
            match serde_json::from_slice(&fs::read(&pointer)?)? {
                SavedCandidate::Scoped(record) => {
                    record.validate_scope(workspace, state)?;
                    if !record.receipt.is_absolute() || !valid_sha(&record.manifest_sha256) {
                        bail!("{}", message(localization::CANDIDATE_RECORDED_RECEIPT));
                    }
                    Ok((record.receipt, Some(record.manifest_sha256)))
                }
                SavedCandidate::Legacy(path) if path.is_absolute() => Ok((path, None)),
                SavedCandidate::Legacy(_) => {
                    bail!("{}", message(localization::LEGACY_RECEIPT_PATH))
                }
            }
        })();
        saved.with_context(|| {
            format!(
                "{} {} · {} · {}. {} {}. {}",
                message(localization::CANDIDATE_SELECT),
                workspace.root.display(),
                state.name,
                state.dir.display(),
                message(localization::CANDIDATE_SELECTION_FILE),
                pointer.display(),
                message(localization::CANDIDATE_SELECT_GUIDANCE),
            )
        })?
    };
    read_candidate(workspace, &path, digest.as_deref()).with_context(|| {
        format!(
            "{} {} · {} · {} · {}. {}",
            message(localization::RECEIPT_UNUSABLE),
            path.display(),
            workspace.root.display(),
            state.name,
            state.dir.display(),
            message(localization::RECEIPT_GUIDANCE),
        )
    })
}

fn read_candidate(workspace: &Workspace, path: &Path, digest: Option<&str>) -> Result<Candidate> {
    let path = canonical(path)?;
    let mut candidate: Candidate = serde_json::from_slice(&fs::read(&path)?)?;
    candidate.receipt = path;
    if candidate.directory.parent() != Some(candidate.session())
        || canonical(&candidate.directory)? != candidate.directory
        || !candidate.directory.is_dir()
        || !valid_sha(&candidate.manifest_sha256)
    {
        bail!("{}", message(localization::RECEIPT_SESSION));
    }
    if digest.is_some_and(|expected| expected != candidate.manifest_sha256) {
        bail!("{}", message(localization::RECEIPT_DIGEST));
    }
    let remote = workspace.require(Repo::Remote)?;
    let mut verifier = Command::new("node");
    verifier
        .arg(remote.join("scripts/verify-native-windows.mjs"))
        .arg(&candidate.directory)
        .arg(&candidate.manifest_sha256);
    capture(in_build_directory(&mut verifier, &remote))?;
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

fn storage_allocator(workspace: &Workspace) -> Result<PathBuf> {
    Ok(workspace
        .require(Repo::Contracts)?
        .join("tools/artifact-paths/cirnet_artifact_paths/artifact-paths.mjs"))
}

fn development_output(workspace: &Workspace, root: &Path) -> Result<PathBuf> {
    let mut allocator = Command::new("node");
    isolate_git(&mut allocator);
    let output = capture(
        allocator
            .arg(storage_allocator(workspace)?)
            .current_dir(root),
    )?;
    Ok(PathBuf::from(output.trim()))
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
    let pointer = state.dir.join("candidate.json");
    if all
        && let Ok(bytes) = fs::read(&pointer)
        && let Ok(SavedCandidate::Scoped(record)) = serde_json::from_slice(&bytes)
    {
        record.validate_scope(workspace, state)?;
    }
    let centralized = env::var_os("CIRNET_LOCAL_STORAGE_ROOT").is_some();
    if all {
        let remote = workspace.require(Repo::Remote)?;
        let mut parent = state.dir.join("builds");
        if centralized {
            let mut allocator = Command::new("node");
            in_build_directory(&mut allocator, &remote);
            let resolved = capture(
                allocator
                    .arg(storage_allocator(workspace)?)
                    .arg("--parent-info"),
            )?;
            parent = PathBuf::from(resolved.trim());
        }
        let output = output_dir(&parent, "build")?;
        let selection = output.join("selection.json");
        let mut select = Command::new("node");
        select
            .arg(remote.join("scripts/native-source-selection.mjs"))
            .arg("lock")
            .arg(&selection);
        run(in_build_directory(&mut select, &remote))?;
        let driver = output.join("build.ps1");
        fs::write(&driver, include_str!("../scripts/windows-build.ps1"))?;
        let mut command = powershell();
        command
            .arg(driver)
            .arg("-Remote")
            .arg(&remote)
            .arg("-Selection")
            .arg(selection)
            .arg("-OutputParent")
            .arg(&output);
        if args.get_flag("offline") {
            command.arg("-Offline");
        }
        in_build_directory(&mut command, &remote);
        if centralized {
            command.env_remove("CIRNET_ARTIFACTS_DIR");
            command.env_remove("NETCIRCUIT_ARTIFACTS_DIR");
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
            bail!("{}", message(localization::ASSEMBLY_RECEIPT_COUNT));
        }
        let receipt = receipts.pop().unwrap();
        let build = candidate(workspace, state, Some(&receipt))?;
        write_json(&pointer, &CandidateSelection::new(workspace, state, &build))?;
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
    let legacy_output = (!centralized)
        .then(|| output_dir(&state.dir.join("builds"), "build"))
        .transpose()?;
    let mut outputs = Vec::new();
    for repo in selection {
        let dir = workspace.require(repo)?;
        let output = match &legacy_output {
            Some(path) => path.clone(),
            None => development_output(workspace, &dir)?,
        };
        match repo {
            Repo::Simulator => {
                simulator(&dir, &output)?;
            }
            Repo::Interface => {
                let build = chosen.as_ref().unwrap();
                prepare_web(&dir, build, args.get_flag("offline"))?;
                let mut npm = Command::new("npm.cmd");
                npm.args(["run", "build"]).current_dir(&dir);
                if centralized {
                    npm.env(
                        "CIRNET_ARTIFACT_PATHS_MODULE",
                        storage_allocator(workspace)?,
                    )
                    .env("CIRNET_ARTIFACTS_SESSION", &output)
                    .env("NETCIRCUIT_ARTIFACTS_SESSION", &output);
                }
                run(&mut npm)?;
            }
            Repo::Embedded => rtl(workspace, &dir, chosen.as_ref().unwrap(), args, &output)?,
            _ => cargo(workspace, &dir, &output, "build", args.get_flag("offline"))?,
        }
        outputs.push(output);
    }
    Ok(json!({"state":"built","buildOrder":names,"output":outputs.last(),"outputs":outputs}))
}

pub fn prepare_web(web: &Path, candidate: &Candidate, offline: bool) -> Result<()> {
    let inputs = candidate.session().join("interface-input.json");
    let selected = trusted_web_inputs(candidate, &inputs)?;
    preserve_old_web_inputs(web, &selected)?;
    let mut bootstrap = Command::new("node");
    in_build_directory(&mut bootstrap, web)
        .arg(web.join("scripts/bootstrap-contracts.mjs"))
        .arg("--manifest")
        .arg(&inputs);
    capture(with_candidate_artifacts(&mut bootstrap, candidate)?)?;
    let mut npm = Command::new("npm.cmd");
    npm.args(["ci", "--ignore-scripts", "--no-audit", "--no-fund"])
        .current_dir(web);
    if offline {
        npm.env("npm_config_offline", "true");
    }
    run(&mut npm)
}

fn with_candidate_artifacts<'a>(
    command: &'a mut Command,
    candidate: &Candidate,
) -> Result<&'a mut Command> {
    let session = candidate.session();
    let ownership: Value =
        serde_json::from_slice(&fs::read(session.join(".cirnet-artifact-session.json"))?)?;
    let context = ownership["context"].to_string();
    let parent = session.parent().context("Missing artifact parent")?;
    for prefix in ["CIRNET", "NETCIRCUIT"] {
        command
            .env(format!("{prefix}_ARTIFACTS_CONTEXT"), &context)
            .env(format!("{prefix}_ARTIFACTS_SESSION"), session)
            .env(format!("{prefix}_ARTIFACTS_DIR"), parent);
    }
    Ok(command)
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
            .context(message(MISSING_INVENTORY))?;
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
            .with_context(|| {
                format!("{} {role}", message(localization::CANDIDATE_INPUT_MISSING))
            })?;
        if declared["sha256"].is_null() || declared["sha256"] != expected["sha256"] {
            bail!(
                "Interface {role} · {}",
                message(localization::CANDIDATE_INPUT_MISMATCH)
            );
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
    let current: Value = serde_json::from_slice(
        &fs::read(&marker).context(message(localization::INTERFACE_SELECTION_MISSING))?,
    )?;
    if WEB_ROLES
        .iter()
        .all(|role| current[role]["sha256"] == selected[role]["sha256"])
    {
        return Ok(());
    }
    let mut git = Command::new("git");
    isolate_git(&mut git);
    let ignored =
        capture(
            git.current_dir(web)
                .args(["check-ignore", "--", ".artifacts/", ".runtime/"]),
        )?;
    if !ignored.lines().any(|line| line == ".artifacts/")
        || !ignored.lines().any(|line| line == ".runtime/")
        || canonical(&cache)? != cache
    {
        bail!("{}", message(localization::INTERFACE_CACHE_UNOWNED));
    }
    let mut expected = vec![marker];
    for role in WEB_ROLES {
        let file = current[role]["path"]
            .as_str()
            .context(message(localization::INTERFACE_CACHED_ARTIFACT_MISSING))?;
        let file = PathBuf::from(file);
        if file.parent() != Some(cache.as_path())
            || canonical(&file)? != file
            || current[role]["sha256"] != sha(&file)?
        {
            bail!(
                "Interface {role} · {} {}",
                message(localization::INTERFACE_CACHED_ARTIFACT_MISMATCH),
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
            "{} {}",
            message(localization::INTERFACE_CACHE_UNKNOWN_FILES),
            cache.display()
        );
    }
    let runtime = web.join(".runtime");
    if runtime.exists() && canonical(&runtime)? != runtime {
        bail!("{}", message(localization::INTERFACE_RUNTIME_DIRECTORY));
    }
    let backup = output_dir(&runtime, "cnl-previous-inputs")?.join(".artifacts");
    fs::rename(&cache, &backup)?;
    eprintln!(
        "{} {}",
        message(localization::INTERFACE_INPUTS_PRESERVED),
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
    if targets.is_empty() {
        let candidate = candidate(workspace, state, args.get_one("receipt"))?;
        let runner = candidate
            .session()
            .join("src/cirnet-remote/scripts/test-native-windows.mjs");
        if !runner.is_file() {
            bail!("{}", message(localization::VERIFY_RUNNER_MISSING));
        }
        let default_browser = state.dir.join("chromium");
        let browser = args
            .get_one::<PathBuf>("browser-cache")
            .cloned()
            .or_else(|| default_browser.is_dir().then_some(default_browser));
        let mut command = Command::new("node");
        in_build_directory(&mut command, candidate.session())
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
    let centralized = env::var_os("CIRNET_LOCAL_STORAGE_ROOT").is_some();
    let legacy_output = (!centralized)
        .then(|| output_dir(&state.dir.join("verification"), "verify"))
        .transpose()?;
    let mut outputs = Vec::new();
    for repo in &targets {
        let dir = workspace.require(*repo)?;
        let output = match &legacy_output {
            Some(path) => path.clone(),
            None if *repo == Repo::Interface => {
                candidate(workspace, state, args.get_one("receipt"))?
                    .session()
                    .to_path_buf()
            }
            None => development_output(workspace, &dir)?,
        };
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
                workspace,
                &dir,
                &candidate(workspace, state, args.get_one("receipt"))?,
                args,
                &output,
            )?,
            Repo::Interface => {
                let build = candidate(workspace, state, args.get_one("receipt"))?;
                let web = build.session().join("src/cirnet-interface");
                let mut npm = Command::new("npm.cmd");
                npm.args(["test", "--", "--all-script-tests"])
                    .current_dir(web)
                    .env(
                        "CIRNET_CONTRACTS_ARTIFACT_MANIFEST",
                        build.session().join("interface-input.json"),
                    );
                run(with_candidate_artifacts(&mut npm, &build)?)?;
            }
            _ => cargo(workspace, &dir, &output, "test", false)?,
        }
        outputs.push(output);
    }
    Ok(
        json!({"state":"verified","targets":targets.iter().map(|repo| repo.name()).collect::<Vec<_>>(),"output":outputs.last(),"outputs":outputs}),
    )
}

fn cargo(
    workspace: &Workspace,
    root: &Path,
    output: &Path,
    task: &str,
    offline: bool,
) -> Result<()> {
    let driver = output.join("cargo.ps1");
    fs::write(&driver, include_str!("../scripts/windows-build.ps1"))?;
    let mut command = powershell();
    in_build_directory(&mut command, root)
        .arg(driver)
        .arg("-Component")
        .arg(root)
        .args(["-CargoOperation", task])
        .env("CARGO_TARGET_DIR", output.join("cargo"));
    if env::var_os("CIRNET_LOCAL_STORAGE_ROOT").is_some() {
        command
            .arg("-ArtifactAllocator")
            .arg(storage_allocator(workspace)?)
            .env("CIRNET_ARTIFACTS_SESSION", output)
            .env("NETCIRCUIT_ARTIFACTS_SESSION", output);
        for name in [
            "CIRNET_ARTIFACTS_CONTEXT",
            "NETCIRCUIT_ARTIFACTS_CONTEXT",
            "CARGO_TARGET_DIR",
        ] {
            if let Some(value) = env::var_os(name) {
                command.env(name, value);
            }
        }
    }
    if offline {
        command.arg("-Offline");
    }
    run(&mut command)
}

fn in_build_directory<'a>(command: &'a mut Command, directory: &Path) -> &'a mut Command {
    isolate_git(command);
    for name in [
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_TARGET_DIR",
        "CARGO_BUILD_BUILD_DIR",
        "CARGO_BUILD_DIR",
        "CARGO_BUILD_TARGET",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTFLAGS",
        "RUSTUP_TOOLCHAIN",
        "CIRNET_ARTIFACTS_CONTEXT",
        "CIRNET_ARTIFACTS_SESSION",
        "NETCIRCUIT_ARTIFACTS_CONTEXT",
        "NETCIRCUIT_ARTIFACTS_SESSION",
    ] {
        command.env_remove(name);
    }
    command.current_dir(directory)
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

fn rtl(
    workspace: &Workspace,
    root: &Path,
    candidate: &Candidate,
    args: &ArgMatches,
    output: &Path,
) -> Result<()> {
    let toolchain = args
        .get_one::<PathBuf>("fpga-toolchain")
        .cloned()
        .or_else(|| env::var_os("CIRNET_FPGA_TOOLCHAIN_ROOT").map(PathBuf::from))
        .context(message(localization::RTL_TOOLCHAIN_REQUIRED))?;
    let toolchain = canonical(&toolchain)?;
    let receipt = candidate
        .session()
        .join("contracts/contracts-native-manifest.json");
    let centralized = env::var_os("CIRNET_LOCAL_STORAGE_ROOT").is_some();
    let allocator = if centralized {
        storage_allocator(workspace)?
    } else {
        candidate.session().join(
            "src/cirnet-contracts/tools/artifact-paths/cirnet_artifact_paths/artifact-paths.mjs",
        )
    };
    let mut command = Command::new(python());
    let parent = if centralized {
        output.parent().context("Missing artifact parent")?
    } else {
        output
    };
    for prefix in ["CIRNET", "NETCIRCUIT"] {
        command.env(format!("{prefix}_ARTIFACTS_DIR"), parent);
        if centralized {
            command.env(format!("{prefix}_ARTIFACTS_SESSION"), output);
        } else {
            command
                .env_remove(format!("{prefix}_ARTIFACTS_SESSION"))
                .env_remove(format!("{prefix}_ARTIFACTS_CONTEXT"));
        }
    }
    run(command
        .current_dir(root)
        .args(["-B", "scripts/check_rtl.py", "--toolchain-root"])
        .arg(toolchain)
        .arg("--contracts-receipt")
        .arg(&receipt)
        .arg("--contracts-sha256")
        .arg(sha(&receipt)?)
        .env("CIRNET_ARTIFACTS_CLI_SHA256", sha(&allocator)?)
        .env("CIRNET_ARTIFACTS_CLI", allocator))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_selection_pins_scope_and_digest_and_preserves_legacy_receipts() {
        let root = output_dir(&env::temp_dir(), "cnl-candidate-test").unwrap();
        let workspace = Workspace {
            root: canonical(&root).unwrap(),
        };
        let state = State::resolve(&workspace, Some(&root.join("state")), "fixture").unwrap();
        state.prepare().unwrap();
        let pointer = state.dir.join("candidate.json");
        let candidate = Candidate {
            directory: root.join("bundle"),
            manifest_sha256: "a".repeat(64),
            receipt: root.join("assembly-result.json"),
        };
        let record = CandidateSelection::new(&workspace, &state, &candidate);
        write_json(&pointer, &candidate.receipt).unwrap();
        let legacy: SavedCandidate = serde_json::from_slice(&fs::read(&pointer).unwrap()).unwrap();
        assert!(matches!(legacy, SavedCandidate::Legacy(path) if path == candidate.receipt));

        write_json(&pointer, &record).unwrap();
        let bytes = fs::read(&pointer).unwrap();
        let SavedCandidate::Scoped(roundtrip) = serde_json::from_slice(&bytes).unwrap() else {
            panic!("expected an atomically replaced scoped candidate");
        };
        assert_eq!(roundtrip.receipt, candidate.receipt);
        assert_eq!(roundtrip.manifest_sha256, candidate.manifest_sha256);
        roundtrip.validate_scope(&workspace, &state).unwrap();
        for (wrong_workspace, wrong_state) in [
            (
                Workspace {
                    root: workspace.root.join("other"),
                },
                State::resolve(&workspace, Some(&root.join("state")), "fixture").unwrap(),
            ),
            (
                Workspace {
                    root: workspace.root.clone(),
                },
                State::resolve(&workspace, Some(&root.join("state")), "other").unwrap(),
            ),
            (
                Workspace {
                    root: workspace.root.clone(),
                },
                State::resolve(&workspace, Some(&root.join("other-state")), "fixture").unwrap(),
            ),
        ] {
            assert!(
                roundtrip
                    .validate_scope(&wrong_workspace, &wrong_state)
                    .is_err()
            );
        }

        let failed = candidate_for_test(&workspace, &state, None);
        assert!(
            failed.contains("assembly-result.json")
                && failed.contains("--receipt")
                && failed.contains("fixture"),
            "{failed}"
        );
        let alternate = candidate_for_test(&workspace, &state, Some(&root.join("alternate.json")));
        assert!(alternate.contains("alternate.json"), "{alternate}");
        assert_eq!(fs::read(&pointer).unwrap(), bytes);
    }

    fn candidate_for_test(workspace: &Workspace, state: &State, path: Option<&PathBuf>) -> String {
        format!("{:#}", candidate(workspace, state, path).unwrap_err())
    }

    #[test]
    fn subprocess_directory_and_artifact_overrides_are_independent_of_the_caller() {
        let root = env::current_dir().unwrap();
        let mut command = Command::new("node");
        let names = [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "CARGO_TARGET_DIR",
            "CIRNET_ARTIFACTS_CONTEXT",
        ];
        for name in names {
            command.env(name, "ambient override");
        }
        in_build_directory(&mut command, &root);
        assert_eq!(command.get_current_dir(), Some(root.as_path()));
        for name in names {
            assert!(
                command
                    .get_envs()
                    .any(|(key, value)| key == name && value.is_none()),
                "{name}"
            );
        }
    }
}
