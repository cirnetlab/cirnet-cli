use super::*;
use crate::repositories::git::tests::{TempRepo, fixture_git};
use std::{fs, path::Path, time::Duration};

fn clone_at(fixture: &TempRepo, upstream: &Path, name: &str) -> Target {
    let upstream = upstream.to_str().unwrap().replace('\\', "/");
    fixture_git(&fixture.0, &["clone", "--local", "--", &upstream, name]);
    let path = fixture.0.join(name);
    fixture_git(&path, &["config", "user.name", "CIRNETLAB Fixture"]);
    fixture_git(&path, &["config", "user.email", "fixture@example.invalid"]);
    Target {
        path,
        remote: "origin".to_owned(),
        slug: None,
    }
}

fn commit(path: &Path, message: &str) -> String {
    fixture_git(path, &["commit", "--allow-empty", "-m", message]);
    fixture_git(path, &["rev-parse", "HEAD"])
}

#[test]
fn updates_independent_clean_checkout_after_dry_run_and_reports_dirty_checkout() {
    let fixture = TempRepo::new();
    let upstream = fixture.0.join("upstream");
    fixture.init(&upstream);
    let ready = clone_at(&fixture, &upstream, "ready");
    let dirty = clone_at(&fixture, &upstream, "dirty");
    fs::write(dirty.path.join("notes.txt"), "local notes").unwrap();
    let previous = fixture_git(&ready.path, &["rev-parse", "HEAD"]);
    let after = commit(&upstream, "upstream work");
    let cancel = AtomicBool::new(false);
    let targets = [ready, dirty];
    let preview = run(&targets, SyncAction::Update, 2, true, &cancel, |_| {});
    assert_eq!(preview[0].state, SyncState::Preview);
    assert_eq!(preview[1].state, SyncState::Failed);
    assert_eq!(
        fixture_git(&targets[0].path, &["rev-parse", "HEAD"]),
        previous
    );
    let results = run(&targets, SyncAction::Update, 2, false, &cancel, |_| {});
    assert_eq!(
        results[0].state,
        SyncState::Succeeded,
        "{}",
        results[0].detail
    );
    assert_eq!(results[1].state, SyncState::Failed);
    assert_eq!(fixture_git(&targets[0].path, &["rev-parse", "HEAD"]), after);
    assert_eq!(
        fixture_git(&targets[1].path, &["rev-parse", "HEAD"]),
        previous
    );
    assert_eq!(
        fs::read_to_string(targets[1].path.join("notes.txt")).unwrap(),
        "local notes"
    );
}

#[test]
fn rejects_detached_missing_upstream_and_remote_changes_in_preflight() {
    let fixture = TempRepo::new();
    let upstream = fixture.0.join("upstream");
    fixture.init(&upstream);
    let detached = clone_at(&fixture, &upstream, "detached");
    fixture_git(&detached.path, &["switch", "--detach", "HEAD"]);
    let untracked_branch = clone_at(&fixture, &upstream, "untracked");
    fixture_git(&untracked_branch.path, &["branch", "--unset-upstream"]);
    let changed = clone_at(&fixture, &upstream, "changed");
    fixture_git(
        &changed.path,
        &[
            "remote",
            "set-url",
            "origin",
            "https://github.com/other/project",
        ],
    );
    let changed = Target {
        slug: Some("cirnetlab/project".into()),
        ..changed
    };
    let outcomes = run(
        &[detached, untracked_branch, changed],
        SyncAction::Update,
        3,
        false,
        &AtomicBool::new(false),
        |_| {},
    );
    assert!(
        outcomes
            .iter()
            .all(|outcome| outcome.state == SyncState::Failed)
    );
}

#[test]
fn diverged_update_preserves_local_head_and_worktree() {
    let fixture = TempRepo::new();
    let upstream = fixture.0.join("upstream");
    fixture.init(&upstream);
    let target = clone_at(&fixture, &upstream, "local");
    let local_head = commit(&target.path, "local work");
    let remote_head = commit(&upstream, "remote work");
    let results = run(
        std::slice::from_ref(&target),
        SyncAction::Update,
        1,
        false,
        &AtomicBool::new(false),
        |_| {},
    );
    assert_eq!(results[0].state, SyncState::Failed);
    assert_eq!(
        fixture_git(&target.path, &["rev-parse", "HEAD"]),
        local_head
    );
    assert_eq!(
        fixture_git(&target.path, &["rev-parse", "refs/remotes/origin/main"]),
        remote_head
    );
    assert_eq!(
        git::inspect(&target.path, "origin").unwrap().status,
        git::GitStatus::Clean
    );
}

#[test]
fn cancellation_leaves_queued_targets_and_worktrees_unchanged() {
    let fixture = TempRepo::new();
    let upstream = fixture.0.join("upstream");
    fixture.init(&upstream);
    let first = clone_at(&fixture, &upstream, "first");
    let second = clone_at(&fixture, &upstream, "second");
    let previous = fixture_git(&first.path, &["rev-parse", "HEAD"]);
    commit(&upstream, "upstream");
    let cancel = AtomicBool::new(false);
    let targets = [first, second];
    let results = run(
        &targets,
        SyncAction::Update,
        1,
        false,
        &cancel,
        |progress| {
            if matches!(progress, SyncProgress::Started(_)) {
                cancel.store(true, Ordering::Release);
            }
        },
    );
    assert!(
        results
            .iter()
            .all(|result| result.state == SyncState::Cancelled)
    );
    for target in targets {
        assert_eq!(fixture_git(&target.path, &["rev-parse", "HEAD"]), previous);
        assert_eq!(
            fixture_git(&target.path, &["rev-parse", "refs/remotes/origin/main"]),
            previous
        );
    }
}

#[test]
fn worker_pool_bounds_inflight_work_and_keeps_input_order() {
    let active = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    let input: Vec<_> = (0..10).collect();
    let doubled = bounded_map(&input, 2, |_, item| {
        let count = active.fetch_add(1, Ordering::Relaxed) + 1;
        peak.fetch_max(count, Ordering::Relaxed);
        thread::sleep(Duration::from_millis(20));
        active.fetch_sub(1, Ordering::Relaxed);
        item * 2
    });
    assert_eq!(peak.load(Ordering::Relaxed), 2);
    assert_eq!(doubled, (0..20).step_by(2).collect::<Vec<_>>());
}
