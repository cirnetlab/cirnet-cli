use std::{fs, process::Command};

use super::*;

struct Fixture {
    root: PathBuf,
    workspace: Workspace,
    store: Store,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = crate::build::output_dir(&std::env::temp_dir(), label).unwrap();
        let workspace = Workspace {
            root: root.join("workspace"),
        };
        fs::create_dir(&workspace.root).unwrap();
        let store = Store::resolve(&workspace, Some(&root.join("state"))).unwrap();
        Self {
            root,
            workspace,
            store,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn entries(count: usize, refreshed: u64) -> Catalog {
    let repositories: Vec<_> = (1..=count)
        .map(|id| {
            json!({
                "id": id, "full_name": format!("cirnetlab/repo-{id:02}"), "default_branch": "main",
                "private": id == count, "archived": false,
            })
        })
        .collect();
    serde_json::from_value(
        json!({"version": 1, "repositories": repositories, "refreshed": refreshed}),
    )
    .unwrap()
}

#[test]
fn catalog_refreshes_missing_and_stale_data_and_reuses_recent_or_offline_cache() {
    let fixture = Fixture::new("cnl-catalog-refresh");
    let store = &fixture.store;
    let first = catalog_with(store, CatalogMode::Automatic, || Ok(entries(33, 1))).unwrap();
    assert!(first.refreshed);
    let offline = catalog_with(store, CatalogMode::Cached, || panic!("offline fetch")).unwrap();
    assert!(!offline.refreshed);
    assert_eq!(offline.catalog.unwrap().repositories.len(), 33);

    let refreshed = catalog_with(store, CatalogMode::Automatic, || Ok(entries(34, now()))).unwrap();
    assert!(refreshed.refreshed);
    let cached = catalog_with(store, CatalogMode::Automatic, || {
        panic!("recent cache fetch")
    })
    .unwrap();
    assert!(!cached.refreshed);
    assert_eq!(cached.catalog.unwrap().repositories.len(), 34);

    let manual = catalog_with(store, CatalogMode::Refresh, || Ok(entries(35, now()))).unwrap();
    assert!(manual.refreshed);
    assert_eq!(read_catalog(store).unwrap().unwrap().repositories.len(), 35);
}

#[test]
fn unavailable_discovery_preserves_cache_and_does_not_invent_a_total() {
    let fixture = Fixture::new("cnl-catalog-unavailable");
    let store = &fixture.store;
    let no_cache =
        catalog_with(store, CatalogMode::Automatic, || Err(github::Failure::Auth)).unwrap();
    assert!(no_cache.catalog.is_none());
    assert_eq!(no_cache.error, Some(github::Failure::Auth));

    catalog_with(store, CatalogMode::Refresh, || Ok(entries(34, 1))).unwrap();
    let failed = catalog_with(store, CatalogMode::Automatic, || {
        Err(github::Failure::Network)
    })
    .unwrap();
    assert_eq!(failed.error, Some(github::Failure::Network));
    assert!(!failed.refreshed);
    assert_eq!(failed.catalog.unwrap().repositories.len(), 34);
    assert_eq!(read_catalog(store).unwrap().unwrap().refreshed, 1);
}

fn git(directory: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(directory)
        .args([
            "-c",
            "user.email=fixture@example.org",
            "-c",
            "user.name=Fixture",
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
}

#[test]
fn inventory_counts_33_github_repositories_out_of_34_despite_a_duplicate_worktree() {
    let fixture = Fixture::new("cnl-catalog-inventory");
    let root = &fixture.workspace.root;
    for id in 1..=33 {
        let name = format!("repo-{id:02}");
        let origin = if id == 33 {
            "repo-before-rename"
        } else {
            &name
        };
        git(root, &["init", "--quiet", "--initial-branch=main", &name]);
        git(
            &root.join(&name),
            &[
                "remote",
                "add",
                "origin",
                &format!("https://github.com/cirnetlab/{origin}.git"),
            ],
        );
    }
    let first = root.join("repo-01");
    git(&first, &["commit", "-q", "--allow-empty", "-m", "fixture"]);
    git(
        &first,
        &["worktree", "add", "-q", "--detach", "../duplicate"],
    );
    git(
        root,
        &["init", "--quiet", "--initial-branch=main", "unrelated"],
    );

    catalog_with(&fixture.store, CatalogMode::Refresh, || {
        let mut catalog = entries(34, now());
        catalog
            .aliases
            .insert("cirnetlab/repo-before-rename".into(), Some(33));
        Ok(catalog)
    })
    .unwrap();
    let args = crate::cli::app()
        .try_get_matches_from([
            "cnl",
            "--state-dir",
            fixture.root.join("state").to_str().unwrap(),
            "repos",
        ])
        .unwrap();
    let inventory = dashboard_inventory(&fixture.workspace, &args, CatalogMode::Cached).unwrap();
    let counts = inventory.counts.unwrap();
    assert_eq!((counts.local, counts.remote), (33, 34));
    assert!(!counts.fresh);
    assert_eq!(
        inventory
            .rows
            .iter()
            .filter(|row| row.remote.as_deref() == Some("cirnetlab/repo-01"))
            .count(),
        2
    );
    let remote_only = inventory
        .rows
        .iter()
        .find(|row| row.name == "repo-34")
        .unwrap();
    assert_eq!(remote_only.state, "available");
    assert!(remote_only.path.is_none());
    assert!(
        inventory
            .rows
            .iter()
            .all(|row| row.name != "repo-33" || row.path.is_some())
    );
}
