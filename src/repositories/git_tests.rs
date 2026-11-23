use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub(in crate::repositories) struct TempRepo(pub PathBuf);
impl TempRepo {
    pub(in crate::repositories) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "cnl-git-tests-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub(in crate::repositories) fn init(&self, path: &Path) {
        fs::create_dir_all(path).unwrap();
        fixture_git(path, &["init", "--initial-branch=main"]);
        fixture_git(path, &["config", "user.name", "CIRNETLAB Fixture"]);
        fixture_git(path, &["config", "user.email", "fixture@example.invalid"]);
        fixture_git(path, &["commit", "--allow-empty", "-m", "initial"]);
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(in crate::repositories) fn fixture_git(path: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    isolate_git(&mut command);
    let output = command
        .args(["-C"])
        .arg(path)
        .args(args)
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git fixture failed: {args:?} — {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

#[test]
fn porcelain_reports_worktree_changes_and_detached_head() {
    let fixture = TempRepo::new();
    fixture.init(&fixture.0);
    assert_eq!(
        inspect(&fixture.0, "origin").unwrap().status,
        GitStatus::Clean
    );
    fs::write(fixture.0.join("untracked.txt"), "").unwrap();
    assert_eq!(
        inspect(&fixture.0, "origin").unwrap().status,
        GitStatus::Untracked
    );
    fixture_git(&fixture.0, &["add", "--", "untracked.txt"]);
    assert_eq!(
        inspect(&fixture.0, "origin").unwrap().status,
        GitStatus::Modified
    );
    fixture_git(&fixture.0, &["switch", "--detach", "HEAD"]);
    let checkout = inspect(&fixture.0, "origin").unwrap();
    assert_eq!(checkout.status, GitStatus::Detached);
    assert!(checkout.branch.is_none());
    assert!(checkout.commit.unwrap().message.contains("initial"));
}

#[test]
fn scans_over_one_hundred_repositories_skipping_unrelated_and_deep_paths() {
    let fixture = TempRepo::new();
    for i in 0..=100 {
        let path = fixture.0.join(format!("repo-{i:03}"));
        fs::create_dir(&path).unwrap();
        fixture_git(&path, &["init", "--initial-branch=main"]);
    }
    fixture.init(&fixture.0.join("target").join("ignored"));
    fixture.init(&fixture.0.join("folder").join("too-deep"));
    fs::create_dir_all(fixture.0.join("not-git").join(".git")).unwrap();
    let found = discover(&fixture.0, 1).unwrap();
    assert_eq!(found.len(), 101);
    assert!(found[0].ends_with("repo-000"));
    assert!(
        discover(&fixture.0, 2)
            .unwrap()
            .iter()
            .any(|path| path.ends_with("too-deep"))
    );
}

#[test]
fn worktree_resolves_the_same_common_storage_and_refuses_nested_directories() {
    let fixture = TempRepo::new();
    let main = fixture.0.join("primary");
    let linked = fixture.0.join("linked with spaces");
    fixture.init(&main);
    fixture_git(
        &main,
        &["worktree", "add", "-b", "feature", linked.to_str().unwrap()],
    );
    let primary = inspect(&main, "origin").unwrap();
    let linked_checkout = inspect(&linked, "origin").unwrap();
    assert_eq!(linked_checkout.common_dir, primary.common_dir);
    assert_eq!(linked_checkout.branch.as_deref(), Some("feature"));
    assert_ne!(linked_checkout.path, primary.path);
    let nested = linked.join("nested");
    fs::create_dir(&nested).unwrap();
    assert!(inspect(&nested, "origin").is_err());
}

#[test]
fn displays_canonical_github_identity_without_embedded_credentials() {
    let fixture = TempRepo::new();
    fixture.init(&fixture.0);
    fixture_git(
        &fixture.0,
        &[
            "remote",
            "add",
            "origin",
            "https://username:token@github.com/CIRNETLAB/Example.git?access_token=secret",
        ],
    );
    let checkout = inspect(&fixture.0, "origin").unwrap();
    assert_eq!(checkout.slug.as_deref(), Some("cirnetlab/example"));
    assert_eq!(
        checkout.remote_url.as_deref(),
        Some("https://github.com/CIRNETLAB/Example.git")
    );
    let target = Target {
        path: checkout.path,
        remote: "origin".into(),
        slug: checkout.slug,
    };
    assert!(preflight(&target, SyncAction::Fetch).is_err());
    let remotes = read(&fixture.0, "origin", ReadAction::Remotes).unwrap();
    assert!(!remotes.contains("token"));
    assert!(!remotes.contains("secret"));
}

#[test]
fn clone_refuses_mismatched_remote_and_existing_destination_without_touching_files() {
    let fixture = TempRepo::new();
    let existing = fixture.0.join("existing");
    fs::create_dir(&existing).unwrap();
    let data = existing.join("existing.txt");
    fs::write(&data, "preserve").unwrap();
    assert!(
        clone_repository(
            &existing,
            "https://github.com/cirnetlab/example.git",
            "cirnetlab/example"
        )
        .is_err()
    );
    assert_eq!(fs::read_to_string(data).unwrap(), "preserve");
    let absent = fixture.0.join("absent");
    assert!(
        clone_repository(
            &absent,
            "https://github.com/different/example.git",
            "cirnetlab/example"
        )
        .is_err()
    );
    assert!(!absent.exists());
}

#[test]
fn clone_service_reserves_exact_directory_and_verifies_the_created_checkout() {
    let fixture = TempRepo::new();
    let upstream = fixture.0.join("fixture-upstream");
    fixture.init(&upstream);
    let destination = missing_destination(&fixture.0.join("destination with spaces")).unwrap();
    let checkout = clone_at(&destination, "fixture-upstream", |checkout| {
        assert_eq!(checkout.path, destination);
        assert_eq!(checkout.branch.as_deref(), Some("main"));
        assert_eq!(checkout.ahead.zip(checkout.behind), Some((0, 0)),);
        Ok(())
    })
    .unwrap();
    assert!(checkout.commit.unwrap().message.contains("initial"));
    assert!(!fixture.0.join("fixture-upstream.git").exists());
}

#[test]
fn common_directory_lock_serializes_mutations_across_linked_worktrees() {
    let fixture = TempRepo::new();
    let primary = fixture.0.join("primary");
    let linked = fixture.0.join("linked");
    fixture.init(&primary);
    fixture_git(
        &primary,
        &["worktree", "add", "-b", "feature", linked.to_str().unwrap()],
    );
    let first = inspect(&primary, "origin").unwrap();
    let second = inspect(&linked, "origin").unwrap();
    let target = Target {
        path: second.path,
        remote: second.remote,
        slug: None,
    };
    let first_lock = lock(&first.common_dir).unwrap();
    assert!(write(&target, WriteAction::CreateBranch("shared".into())).is_err());
    drop(first_lock);
    write(&target, WriteAction::CreateBranch("shared".into())).unwrap();
    assert_eq!(
        fixture_git(&primary, &["branch", "--list", "shared"]),
        "shared"
    );
}

#[test]
fn explicit_git_writes_keep_branches_worktrees_and_staging_isolated() {
    let fixture = TempRepo::new();
    let main = fixture.0.join("primary");
    fixture.init(&main);
    let checkout = inspect(&main, "origin").unwrap();
    let target = Target {
        path: checkout.path,
        remote: "origin".into(),
        slug: None,
    };
    write(&target, WriteAction::CreateBranch("feature".into())).unwrap();
    let linked = fixture.0.join("linked");
    write(
        &target,
        WriteAction::AddWorktree {
            path: linked.clone(),
            branch: "feature".into(),
        },
    )
    .unwrap();
    assert!(write(&target, WriteAction::SwitchBranch("feature".into())).is_err());
    assert!(
        write(
            &target,
            WriteAction::Stage(vec![PathBuf::from("../outside")])
        )
        .is_err()
    );
    fs::write(main.join("notes.txt"), "work").unwrap();
    write(
        &target,
        WriteAction::Stage(vec![PathBuf::from("notes.txt")]),
    )
    .unwrap();
    write(&target, WriteAction::Commit("explicit change".into())).unwrap();
    assert_eq!(
        fixture_git(&linked, &["log", "-1", "--format=%s"]),
        "initial"
    );
    assert_eq!(
        fixture_git(&main, &["log", "-1", "--format=%s"]),
        "explicit change"
    );
}
