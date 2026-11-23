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
        self.git(&path, &["init", "-q", "-b", "main"]);
        path
    }

    fn git(&self, path: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args([
                "-c",
                "user.name=Fixture Author",
                "-c",
                "user.email=test@example.org",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
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

    fn set_catalog(&self, repositories: Value) {
        fs::write(
            self.directory().join("github-catalog.json"),
            serde_json::to_vec(
                &json!({"version": 1, "refreshed": 1, "repositories": repositories}),
            )
            .unwrap(),
        )
        .unwrap();
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
fn discovery_inspects_live_git_metadata() {
    let f = Fixture::new();
    let checkout = f.init("alpha");
    f.git(&checkout, &["config", "core.abbrev", "no"]);
    f.git(&checkout, &["config", "status.aheadBehind", "false"]);
    f.git(
        &checkout,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/cirnetlab/alpha.git",
        ],
    );
    f.git(&checkout, &["commit", "-q", "--allow-empty", "-m", "first"]);
    f.git(
        &checkout,
        &["update-ref", "refs/remotes/origin/main", "HEAD"],
    );
    f.git(&checkout, &["branch", "--set-upstream-to=origin/main"]);
    f.git(
        &checkout,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "second",
            "-m",
            "Second paragraph",
        ],
    );
    let revision = f.git(&checkout, &["rev-parse", "HEAD"]);
    fs::write(checkout.join("untracked.txt"), "new").unwrap();

    let discovery = f.json(&["discover", "--offline"]);
    let row = &discovery["repositories"][0];
    assert_eq!(row["registered"], false);
    assert!(same_path(&checkout, &row["path"]));
    assert_eq!(row["branch"], "main");
    assert_eq!(row["state"], "untracked");
    assert_eq!(row["upstream"], "origin/main");
    assert_eq!(row["ahead"], 1);
    assert_eq!(row["behind"], 0);
    assert_eq!(row["remote"], "cirnetlab/alpha");
    assert_eq!(row["revision"], revision);
    let short = row["shortRevision"].as_str().unwrap();
    assert!(short.len() < revision.len() && revision.starts_with(short));
    assert_eq!(row["commitMessage"], "second\n\nSecond paragraph");
    assert_eq!(row["commitAuthor"], "Fixture Author");
    assert!(row["commitTimestamp"].is_string());
    assert!(row["lastVerified"].is_number());
}

#[test]
fn discovery_keeps_unborn_branch_and_detached_commit_distinct() {
    let f = Fixture::new();
    f.init("new-checkout");
    let detached = f.init("detached-checkout");
    f.git(&detached, &["commit", "-q", "--allow-empty", "-m", "first"]);
    f.git(&detached, &["checkout", "--detach", "-q"]);

    let discovery = f.json(&["discover", "--offline"]);
    let rows = discovery["repositories"].as_array().unwrap();
    let new = rows
        .iter()
        .find(|row| row["name"] == "new-checkout")
        .unwrap();
    assert_eq!(new["branch"], "main");
    assert_eq!(new["unborn"], true);
    assert!(new["revision"].is_null());
    let detached = rows
        .iter()
        .find(|row| row["name"] == "detached-checkout")
        .unwrap();
    assert!(detached["branch"].is_null());
    assert_eq!(detached["state"], "detached");
    assert_eq!(detached["detached"], true);
    assert!(detached["revision"].is_string());
}

#[test]
fn discovery_merges_registry_and_catalog_identity_with_unique_checkout() {
    let f = Fixture::new();
    f.json(&["group", "create", "fixture"]);
    f.set_catalog(json!([
        {"id": 3, "full_name": "cirnetlab/original", "private": true, "archived": false, "default_branch": "release"}
    ]));
    let registration = f.json(&["register", "original", "--offline"]);
    let id = registration["repository"]["id"].as_str().unwrap();
    f.json(&[
        "config",
        id,
        "--alias",
        "test-alias",
        "--preferred-branch",
        "stable",
        "--shell",
        "bash",
    ]);
    f.json(&["group", "add", "fixture", id]);
    f.set_catalog(json!([
        {"id": 3, "full_name": "cirnetlab/renamed", "private": true, "archived": false, "default_branch": "release"},
        {"id": 4, "full_name": "cirnetlab/remote-only", "private": false, "archived": true, "default_branch": "remote-main"}
    ]));
    let checkout = f.init("different-folder");
    f.git(
        &checkout,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/CIRNETLAB/Renamed.git",
        ],
    );

    let discovery = f.json(&["discover", "--offline"]);
    let rows = discovery["repositories"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let local = rows.iter().find(|row| row["selector"] == id).unwrap();
    assert_eq!(local["alias"], "test-alias");
    assert_eq!(local["preferredBranch"], "stable");
    assert_eq!(local["groups"], json!(["fixture"]));
    assert_eq!(local["shell"], "bash");
    assert_eq!(local["branch"], "main");
    assert_eq!(local["private"], true);
    assert!(same_path(&checkout, &local["path"]));
    let remote = rows
        .iter()
        .find(|row| row["name"] == "remote-only")
        .unwrap();
    assert!(remote["path"].is_null());
    assert_eq!(remote["state"], "available");
    assert_eq!(remote["branch"], "remote-main");
    assert_eq!(remote["archived"], true);
    assert_eq!(f.json(&["status", id])["repositories"][0]["branch"], "main");
    assert!(f.json(&["list"])["repositories"][0]["path"].is_null());
}

#[test]
fn discovery_keeps_distinct_linked_worktrees_for_one_catalog_repository() {
    let f = Fixture::new();
    let main = f.init("main-checkout");
    f.git(
        &main,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/cirnetlab/shared.git",
        ],
    );
    f.git(&main, &["commit", "-q", "--allow-empty", "-m", "first"]);
    f.git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            "../feature-checkout",
        ],
    );
    f.json(&["add", "main-checkout"]);
    f.set_catalog(json!([
        {"id": 1, "full_name": "cirnetlab/shared", "private": false, "archived": false, "default_branch": "release"}
    ]));

    let discovery = f.json(&["discover", "--offline"]);
    let rows = discovery["repositories"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let main = rows.iter().find(|row| row["branch"] == "main").unwrap();
    let feature = rows.iter().find(|row| row["branch"] == "feature").unwrap();
    assert_eq!(main["registered"], true);
    assert_eq!(feature["registered"], false);
    assert_eq!(feature["remote"], "cirnetlab/shared");
    assert_ne!(main["path"], feature["path"]);
}

#[test]
fn discovery_does_not_treat_a_reused_github_slug_as_the_registered_identity() {
    let f = Fixture::new();
    f.json(&["group", "create", "fixture"]);
    f.set_catalog(json!([
        {"id": 3, "full_name": "cirnetlab/original", "private": true, "archived": false, "default_branch": "main"}
    ]));
    let registration = f.json(&["register", "original", "--offline"]);
    let id = registration["repository"]["id"].as_str().unwrap();
    let checkout = f.init("local");
    f.git(
        &checkout,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/cirnetlab/original.git",
        ],
    );
    f.json(&["add", "local"]);
    f.set_catalog(json!([
        {"id": 3, "full_name": "cirnetlab/renamed", "private": true, "archived": false, "default_branch": "main"},
        {"id": 4, "full_name": "cirnetlab/original", "private": false, "archived": false, "default_branch": "main"}
    ]));
    let discovery = f.json(&["discover", "--offline"]);
    let rows = discovery["repositories"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let saved = rows.iter().find(|row| row["selector"] == id).unwrap();
    assert_eq!(saved["state"], "invalid");
    let current = rows
        .iter()
        .find(|row| row["selector"] == "cirnetlab/original")
        .unwrap();
    assert_eq!(current["state"], "available");
    assert!(!f.run(&["switch", id]).status.success());
    f.git(
        &checkout,
        &[
            "remote",
            "set-url",
            "origin",
            "https://github.com/cirnetlab/renamed.git",
        ],
    );
    assert_eq!(f.json(&["status", id])["repositories"][0]["branch"], "main");
}

#[test]
fn discovery_reports_missing_and_invalid_checkout_separately() {
    let f = Fixture::new();
    let missing = f.init("missing-checkout");
    f.json(&["add", "missing-checkout"]);
    fs::rename(missing, f.root.join("old-checkout")).unwrap();
    let invalid = f.workspace.join("invalid-checkout");
    fs::create_dir(&invalid).unwrap();
    fs::write(invalid.join(".git"), "invalid gitdir pointer").unwrap();

    let discovery = f.json(&["discover", "--offline"]);
    let rows = discovery["repositories"].as_array().unwrap();
    let missing = rows
        .iter()
        .find(|row| row["name"] == "missing-checkout")
        .unwrap();
    let invalid = rows
        .iter()
        .find(|row| row["name"] == "invalid-checkout")
        .unwrap();
    assert_eq!(missing["state"], "missing");
    assert_eq!(invalid["state"], "invalid");
    assert!(missing["branch"].is_null());
    assert!(invalid["lastVerified"].is_null());
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
