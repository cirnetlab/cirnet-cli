use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("cnl-manager-{}-{id}", std::process::id()));
        let workspace = root.join("workspace");
        let state = root.join("state");
        fs::create_dir_all(&workspace).unwrap();
        Self {
            root,
            workspace,
            state,
        }
    }

    fn init(&self, path: &str) -> PathBuf {
        let path = self.workspace.join(path);
        fs::create_dir_all(&path).unwrap();
        let status = Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(["init", "-q"])
            .status()
            .unwrap();
        assert!(status.success());
        path
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_cnl"))
            .arg("--workspace")
            .arg(&self.workspace)
            .arg("--state-dir")
            .arg(&self.state)
            .args(["--json", "repos"])
            .args(args)
            .env_remove("CNL_TUI_REPORT")
            .output()
            .unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn directory(&self) -> PathBuf {
        fs::read_dir(&self.state)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.root.parent() == Some(std::env::temp_dir().as_path())
            && self
                .root
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("cnl-manager-"))
        {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn same_path(path: &Path, value: &Value) -> bool {
    fs::canonicalize(path).unwrap() == fs::canonicalize(value.as_str().unwrap()).unwrap()
}

#[test]
fn registration_preview_groups_context_and_removal_preserve_git_files() {
    let f = Fixture::new();
    let alpha = f.init("Branch/extra alpha");
    let beta = f.init("extra-beta");
    fs::write(alpha.join("unchanged.txt"), "keep this").unwrap();
    let root = ".";
    let preview = f.json(&["add", "--all", "--root", root]);
    assert_eq!(preview["confirmationRequired"], true);
    assert_eq!(preview["operations"].as_array().unwrap().len(), 2);
    assert!(!f.state.exists());
    f.json(&["add", "--all", "--root", root, "--yes"]);
    let list = f.json(&["list"]);
    let records = list["repositories"].as_array().unwrap();
    assert_eq!(records.len(), 2);
    let entry = records
        .iter()
        .find(|record| same_path(&alpha, &record["path"]))
        .unwrap();
    let id = entry["id"].as_str().unwrap();
    f.json(&[
        "config",
        id,
        "--alias",
        "alpha",
        "--preferred-branch",
        "main",
    ]);
    f.json(&["group", "create", "prototype"]);
    f.json(&["group", "add", "prototype", "alpha"]);
    let active = f.json(&["switch", "alpha"]);
    assert_eq!(active["active"], id);
    assert!(same_path(&alpha, &active["path"]));
    assert_eq!(
        f.json(&["config", "alpha"])["repository"]["groups"],
        json!(["prototype"])
    );
    assert_eq!(
        f.json(&["status", "alpha"])["repositories"][0]["state"],
        "untracked"
    );
    let removed = f.json(&["remove", "--select", "alpha"]);
    assert_eq!(removed["operations"][0]["id"], id);
    assert!(f.json(&["list"])["active"].is_null());
    assert!(alpha.join(".git").is_dir() && beta.join(".git").is_dir());
    assert_eq!(
        fs::read_to_string(alpha.join("unchanged.txt")).unwrap(),
        "keep this"
    );
    let result = f.run(&["update"]);
    assert!(!result.status.success());
}

#[test]
fn cached_private_catalog_can_register_an_uncloned_repo_without_network() {
    let f = Fixture::new();
    f.init("unrelated");
    f.json(&["group", "create", "placeholder"]);
    fs::write(f.directory().join("github-catalog.json"),serde_json::to_vec(&json!({
        "version":1, "refreshed":1, "repositories":[
            {"id":3,"full_name":"cirnetlab/repo-from-cache","private":true,"archived":false,"default_branch":"main"}
        ]
    })).unwrap()).unwrap();
    let discovery = f.json(&["discover", "--offline", "--query", "repo-from-cache"]);
    assert_eq!(discovery["discoveryStatus"], "offline");
    assert_eq!(discovery["catalogStale"], true);
    assert_eq!(discovery["repositories"][0]["state"], "available");
    assert_eq!(discovery["repositories"][0]["private"], true);
    let registration = f.json(&["register", "repo-from-cache", "--offline"]);
    let id = registration["repository"]["id"].as_str().unwrap();
    assert_eq!(
        f.json(&["status", id])["repositories"][0]["state"],
        "uncloned"
    );
    let preview = f.json(&["clone", id, "--offline", "--dry-run"]);
    assert_eq!(preview["operations"][0]["dryRun"], true);
    assert!(!f.workspace.join("repo-from-cache").exists());
    let path = f.directory().join("github-catalog.json");
    let mut catalog: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    catalog["repositories"][0]["full_name"] = json!("cirnetlab/renamed-repo");
    fs::write(path, serde_json::to_vec(&catalog).unwrap()).unwrap();
    let renamed = f.json(&["clone", id, "--offline", "--dry-run"]);
    assert_eq!(renamed["operations"][0]["repo"], "cirnetlab/renamed-repo");
    assert_eq!(
        f.json(&["register", "renamed-repo", "--offline"])["repository"]["id"],
        id
    );
}

#[test]
fn damaged_registry_is_reported_and_never_replaced() {
    let f = Fixture::new();
    f.json(&["group", "create", "fixture"]);
    let registry = f.directory().join("registry.json");
    fs::write(&registry, br#"{"version":999}"#).unwrap();
    assert!(!f.run(&["group", "create", "new"]).status.success());
    assert_eq!(fs::read(&registry).unwrap(), br#"{"version":999}"#);
}
