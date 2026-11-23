mod cancel;
mod commands;
mod git;
mod github;
mod messages;
mod registry;
mod sync;

pub(crate) use commands::{command, description};

use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow, bail, ensure};
use clap::ArgMatches;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    localization::{self, message, set_language},
    workspace::{Repo, Workspace, canonical},
};

use self::{
    git::{Checkout, ReadAction, Target, WriteAction},
    github::{Catalog, Remote},
    messages as msg,
    registry::{Record, Registry, Store, now, same_path},
    sync::{SyncAction, SyncProgress, SyncState},
};

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Row {
    pub selector: String,
    pub name: String,
    pub alias: Option<String>,
    pub path: Option<PathBuf>,
    pub branch: Option<String>,
    pub state: String,
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
    pub revision: Option<String>,
    pub groups: Vec<String>,
    pub remote: Option<String>,
    pub shell: Option<String>,
    pub active: bool,
    pub registered: bool,
    pub last_verified: Option<u64>,
    pub last_sync: Option<u64>,
    pub private: Option<bool>,
    pub archived: Option<bool>,
}

impl Row {
    fn new(selector: String, name: String, state: &str) -> Self {
        Self {
            selector,
            name,
            state: state.into(),
            ..Self::default()
        }
    }

    fn apply_checkout(&mut self, checkout: Checkout) {
        self.path = Some(checkout.path);
        self.branch = checkout.branch;
        self.state = checkout.status.to_string();
        self.ahead = checkout.ahead;
        self.behind = checkout.behind;
        self.revision = checkout.last_commit;
        self.last_verified = Some(now());
    }
}

fn store(workspace: &Workspace, args: &ArgMatches) -> Result<Store> {
    Store::resolve(
        workspace,
        args.get_one::<PathBuf>("state-dir").map(PathBuf::as_path),
    )
}

pub(crate) fn wants_dashboard(args: &ArgMatches) -> Option<bool> {
    match args.subcommand() {
        None => Some(false),
        Some(("update", options))
            if names(options).is_empty()
                && !options.get_flag("all")
                && !options.get_flag("dry-run") =>
        {
            Some(true)
        }
        _ => None,
    }
}

pub(crate) fn interactive_required(update: bool) -> &'static str {
    msg::get(if update {
        msg::SCOPE_REQUIRED
    } else {
        msg::INTERACTIVE
    })
}

pub(crate) fn operation_failed() -> &'static str {
    msg::get(msg::SYNC_FAILED)
}
pub(crate) fn context_refreshing() -> &'static str {
    msg::get(msg::CONTEXT_REFRESHING)
}

pub(crate) type ActiveContext = Option<(PathBuf, Option<String>)>;

pub(crate) fn active_context(workspace: &Workspace, flags: &ArgMatches) -> Result<ActiveContext> {
    let registry = store(workspace, flags)?.load()?;
    let Some(entry) = registry
        .active
        .as_ref()
        .and_then(|active| registry.records.iter().find(|entry| entry.id == *active))
    else {
        return Ok(None);
    };
    let path = entry
        .path
        .as_ref()
        .ok_or_else(|| anyhow!("{}", msg::get(msg::LOCAL_REQUIRED)))?;
    let actual = checked_checkout(path, &entry.remote, entry.slug.as_deref())?;
    Ok(Some((actual.path, entry.shell.clone())))
}

pub(crate) fn completion_values(workspace: &Workspace, flags: &ArgMatches) -> Result<Vec<String>> {
    let store = store(workspace, flags)?;
    let registry = store.load()?;
    let mut values: BTreeSet<_> = Repo::KEYS.into_iter().map(str::to_owned).collect();
    for entry in registry.records {
        values.insert(entry.id);
        values.extend(entry.alias);
        values.extend(entry.slug);
        values.extend(
            entry
                .path
                .and_then(|path| path.into_os_string().into_string().ok()),
        );
    }
    if let Some(catalog) = read_catalog(&store)? {
        values.extend(catalog.repositories.into_iter().map(|item| item.slug));
    }
    Ok(values.into_iter().collect())
}

pub(crate) fn registry_directory(workspace: &Workspace, args: &ArgMatches) -> Result<PathBuf> {
    Ok(store(workspace, args)?.directory)
}

pub(crate) fn group_names(workspace: &Workspace, args: &ArgMatches) -> Result<Vec<String>> {
    Ok(store(workspace, args)?.load()?.groups.into_iter().collect())
}

fn record_row(record: &Record, active: Option<&str>, live: bool) -> Row {
    let state = if record.path.is_none() {
        "uncloned"
    } else {
        record.last_state.as_deref().unwrap_or("missing")
    };
    let mut row = Row {
        selector: record.id.clone(),
        name: record.name.clone(),
        alias: record.alias.clone(),
        path: record.path.clone(),
        branch: record.last_branch.clone(),
        state: state.into(),
        groups: record.groups.iter().cloned().collect(),
        remote: record.slug.clone().or_else(|| record.remote_url.clone()),
        shell: record.shell.clone(),
        active: active == Some(record.id.as_str()),
        registered: true,
        last_verified: record.last_verified,
        last_sync: record.last_sync,
        ..Row::default()
    };
    if live && let Some(path) = &record.path {
        match checked_checkout(path, &record.remote, record.slug.as_deref()) {
            Ok(checkout) => row.apply_checkout(checkout),
            Err(_) => row.state = if path.exists() { "invalid" } else { "missing" }.into(),
        }
    }
    row
}

fn remote_row(repo: &Remote) -> Row {
    Row {
        selector: repo.slug.clone(),
        name: repo.name().into(),
        branch: Some(repo.default_branch.clone()),
        remote: Some(repo.slug.clone()),
        private: Some(repo.private),
        archived: Some(repo.archived),
        ..Row::new(String::new(), String::new(), "available")
    }
}

fn checked_checkout(path: &Path, remote: &str, slug: Option<&str>) -> Result<Checkout> {
    let result = git::inspect(path, remote)?;
    ensure!(
        slug.is_none_or(|slug| result
            .slug
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case(slug))),
        "{}",
        msg::get(msg::IDENTITY_CHANGED)
    );
    Ok(result)
}

fn read_catalog(store: &Store) -> Result<Option<Catalog>> {
    let cached: Option<Catalog> = store.read(github::CACHE)?;
    ensure!(
        cached.as_ref().is_none_or(Catalog::valid),
        "{}",
        msg::get(msg::REGISTRY_INVALID)
    );
    Ok(cached)
}

fn catalog(
    store: &Store,
    offline: bool,
    refresh: bool,
) -> Result<(Option<Catalog>, Option<github::Failure>)> {
    let cached = read_catalog(store)?;
    if offline || (!refresh && cached.is_some()) {
        return Ok((cached, None));
    }
    match github::fetch() {
        Ok(catalog) => {
            let _lock = store.lock()?;
            store.write(github::CACHE, &catalog)?;
            Ok((Some(catalog), None))
        }
        Err(problem) => Ok((cached, Some(problem))),
    }
}

fn local_path(workspace: &Workspace, value: &str) -> PathBuf {
    let native = Repo::ALL.into_iter().find(|repo| repo.key() == value);
    let name = native.map_or(value, |repo| repo.name());
    let direct = workspace.root.join(name);
    let nested = workspace.root.join("Branch").join(name);
    if !direct.exists() && nested.is_dir() {
        nested
    } else {
        direct
    }
}

fn target_for(workspace: &Workspace, registry: &Registry, value: &str) -> Result<Target> {
    if let Some(index) = registry.index(value, &workspace.root)? {
        let record = &registry.records[index];
        return Ok(Target {
            path: record
                .path
                .clone()
                .ok_or_else(|| anyhow!("{} {value}", msg::get(msg::LOCAL_REQUIRED)))?,
            remote: record.remote.clone(),
            slug: record.slug.clone(),
        });
    }
    let path = local_path(workspace, value);
    let expected = Repo::ALL
        .into_iter()
        .find(|entry| entry.key() == value || entry.name() == value)
        .map(|entry| format!("cirnetlab/{}", entry.name()));
    let checkout = checked_checkout(&path, "origin", expected.as_deref())?;
    Ok(Target {
        path: checkout.path,
        remote: checkout.remote,
        slug: checkout.slug,
    })
}

pub(crate) fn terminal_context(
    workspace: &Workspace,
    args: &ArgMatches,
    value: &str,
) -> Result<(String, PathBuf, Option<String>)> {
    let registry = store(workspace, args)?.load()?;
    let index = registry.index(value, &workspace.root)?;
    let target = target_for(workspace, &registry, value)?;
    let checkout = checked_checkout(&target.path, &target.remote, target.slug.as_deref())?;
    // The canonical directory is part of the key. Relinking a record cannot silently move a live terminal.
    let key = index.map_or_else(|| value.into(), |index| registry.records[index].id.clone());
    let key = format!("{key} · {}", checkout.path.display());
    Ok((
        key,
        checkout.path,
        index.and_then(|index| registry.records[index].shell.clone()),
    ))
}

pub(crate) fn snapshot(workspace: &Workspace, args: &ArgMatches) -> Result<Vec<Row>> {
    let store = store(workspace, args)?;
    let registry = store.load()?;
    let cached = read_catalog(&store)?;
    rows(workspace, &registry, cached.as_ref(), &workspace.root)
}

fn rows(
    workspace: &Workspace,
    registry: &Registry,
    catalog: Option<&Catalog>,
    root: &Path,
) -> Result<Vec<Row>> {
    let mut rows: Vec<_> = registry
        .records
        .iter()
        .map(|record| record_row(record, registry.active.as_deref(), true))
        .collect();
    let candidates = git::discover(root, 2)?;
    for candidate in candidates {
        if rows.iter().any(|row| {
            row.path
                .as_ref()
                .is_some_and(|path| same_path(path, &candidate))
        }) {
            continue;
        }
        let name = candidate
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut row = Row::new(candidate.to_string_lossy().into_owned(), name, "local");
        row.path = Some(candidate);
        rows.push(row);
    }
    if let Some(catalog) = catalog {
        for repo in &catalog.repositories {
            if !registry.records.iter().any(|record| {
                record.github_id == Some(repo.id)
                    || record
                        .slug
                        .as_deref()
                        .is_some_and(|slug| slug.eq_ignore_ascii_case(&repo.slug))
            }) {
                rows.push(remote_row(repo));
            }
        }
    }
    // The workspace is not itself registered merely by viewing it.
    rows.retain(|row| {
        row.path
            .as_ref()
            .is_none_or(|path| !same_path(path, &workspace.root))
            || row.registered
    });
    Ok(rows)
}

fn names(args: &ArgMatches) -> Vec<String> {
    ["targets", "select"]
        .into_iter()
        .flat_map(|id| {
            args.try_get_many::<String>(id)
                .ok()
                .flatten()
                .into_iter()
                .flatten()
                .cloned()
        })
        .collect()
}

fn named(args: &ArgMatches) -> &str {
    args.get_one::<String>("target").expect("required target")
}

fn registered(registry: &Registry, workspace: &Workspace, value: &str) -> Result<usize> {
    registry
        .index(value, &workspace.root)?
        .ok_or_else(|| anyhow!("{} {value}", msg::get(msg::NOT_FOUND)))
}

fn configured_root(workspace: &Workspace, args: &ArgMatches) -> Result<PathBuf> {
    let path = args
        .get_one::<PathBuf>("root")
        .map_or_else(|| workspace.root.clone(), |path| workspace.root.join(path));
    ensure!(
        path.is_dir() && !std::fs::symlink_metadata(&path)?.file_type().is_symlink(),
        "{}",
        msg::get(msg::INVALID_ROOT)
    );
    let actual = canonical(&path)?;
    let mut lexical = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                lexical.pop();
            }
            std::path::Component::CurDir => {}
            other => lexical.push(other.as_os_str()),
        }
    }
    ensure!(
        same_path(&lexical, &actual),
        "{}",
        msg::get(msg::INVALID_ROOT)
    );
    Ok(actual)
}

fn apply_checkout(record: &mut Record, checkout: &Checkout) {
    record.path = Some(checkout.path.clone());
    record.remote = checkout.remote.clone();
    record.slug = checkout.slug.clone();
    record.remote_url = checkout
        .slug
        .as_ref()
        .map(|slug| format!("https://github.com/{slug}.git"));
    record.last_verified = Some(now());
    record.last_branch = checkout.branch.clone();
    record.last_state = Some(checkout.status.to_string());
}

fn save_checkout(
    registry: &mut Registry,
    checkout: &Checkout,
    remote: Option<&Remote>,
) -> Result<String> {
    if let Some(record) = registry.records.iter_mut().find(|record| {
        record
            .path
            .as_ref()
            .is_some_and(|path| same_path(path, &checkout.path))
            || (record.path.as_ref().is_none_or(|path| !path.exists())
                && (remote.is_some_and(|item| record.github_id == Some(item.id))
                    || checkout.slug.as_ref().is_some_and(|slug| {
                        record
                            .slug
                            .as_deref()
                            .is_some_and(|saved| saved.eq_ignore_ascii_case(slug))
                    })))
    }) {
        apply_checkout(record, checkout);
        record.github_id = remote.map(|item| item.id).or(record.github_id);
        return Ok(record.id.clone());
    }
    let fallback = checkout
        .path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let name = checkout
        .slug
        .as_ref()
        .and_then(|slug| slug.split_once('/').map(|(_, name)| name.to_owned()))
        .unwrap_or(fallback);
    let mut record = Record::new(name);
    record.github_id = remote.map(|item| item.id);
    apply_checkout(&mut record, checkout);
    registry.insert(record)
}

fn bulk_preview(args: &ArgMatches) -> (bool, bool) {
    let confirm = args.get_flag("all") && !args.get_flag("yes") && !args.get_flag("dry-run");
    (args.get_flag("dry-run") || confirm, confirm)
}

fn report(operations: Vec<Value>, confirmation: bool) -> Value {
    let ok = operations
        .iter()
        .all(|op| op["state"] != "failed" && op["state"] != "cancelled");
    let mut result = json!({"operations": operations, "ok":ok});
    if confirmation {
        result["confirmationRequired"] = json!(true);
        result["detail"] = localization::report_message(msg::CONFIRM_ALL);
    }
    result
}

fn status(
    workspace: &Workspace,
    store: &Store,
    registry: &Registry,
    values: Vec<String>,
) -> Result<Value> {
    if values.is_empty() && registry.records.is_empty() {
        return Ok(json!({"workspace":workspace.root,"repositories":workspace.inventory(true)}));
    }
    let active = registry.active.as_deref();
    let rows: Vec<_> = if values.is_empty() {
        registry
            .records
            .iter()
            .map(|record| record_row(record, active, true))
            .collect()
    } else {
        values
            .iter()
            .map(|value| {
                if let Some(index) = registry.index(value, &workspace.root)? {
                    Ok(record_row(&registry.records[index], active, true))
                } else {
                    let target = target_for(workspace, registry, value)?;
                    let checkout =
                        checked_checkout(&target.path, &target.remote, target.slug.as_deref())?;
                    let mut row = Row::new(value.clone(), value.clone(), "local");
                    row.remote.clone_from(&checkout.slug);
                    row.apply_checkout(checkout);
                    Ok(row)
                }
            })
            .collect::<Result<_>>()?
    };
    if rows
        .iter()
        .any(|row| row.registered && row.last_verified.is_some())
    {
        store.edit(|saved| {
            for row in &rows {
                if let Some(record) = saved
                    .records
                    .iter_mut()
                    .find(|record| row.selector == record.id && row.path == record.path)
                    && !matches!(row.state.as_str(), "missing" | "invalid" | "uncloned")
                {
                    record.last_verified = row.last_verified;
                    record.last_branch.clone_from(&row.branch);
                    record.last_state = Some(row.state.clone());
                }
            }
            Ok(())
        })?;
    }
    Ok(json!({"workspace":workspace.root,"repositories":rows}))
}

fn add(workspace: &Workspace, store: &Store, args: &ArgMatches) -> Result<Value> {
    let base = configured_root(workspace, args)?;
    let (dry, confirm) = bulk_preview(args);
    let paths = if args.get_flag("all") {
        ensure!(
            args.get_one::<PathBuf>("root").is_some(),
            "{}",
            msg::get(msg::ADD_SCOPE)
        );
        git::discover(&base, 2)?
    } else {
        names(args)
            .into_iter()
            .map(|name| {
                let path = base.join(&name);
                if path.exists() {
                    path
                } else {
                    local_path(workspace, &name)
                }
            })
            .collect()
    };
    let checkouts = paths
        .iter()
        .map(|path| git::inspect(path, "origin"))
        .collect::<Result<Vec<_>>>()?;
    let mut seen = BTreeSet::new();
    let checkouts: Vec<_> = checkouts
        .into_iter()
        .filter(|item| seen.insert(item.path.clone()))
        .collect();
    if !dry {
        store.edit(|registry| {
            for checkout in &checkouts {
                save_checkout(registry, checkout, None)?;
            }
            Ok(())
        })?;
    }
    Ok(report(
        checkouts
            .into_iter()
            .map(|repo| json!({"operation":"add","path":repo.path,"dryRun":dry}))
            .collect(),
        confirm,
    ))
}

fn cloned_record(store: &Store, remote: &Remote, checkout: &Checkout) -> Result<String> {
    store.edit(|registry| save_checkout(registry, checkout, Some(remote)))
}

fn clone(
    workspace: &Workspace,
    store: &Store,
    registry: &Registry,
    args: &ArgMatches,
) -> Result<Value> {
    let base = configured_root(workspace, args)?;
    let (dry, confirm) = bulk_preview(args);
    let mut known: Vec<_> = Vec::new();
    for value in names(args) {
        if let Some(index) = registry.index(&value, &workspace.root)? {
            let record = &registry.records[index];
            ensure!(
                record.path.as_ref().is_none_or(|path| !path.exists()),
                "{} {value}",
                msg::get(msg::ALREADY_LOCAL)
            );
            known.push((
                record
                    .slug
                    .clone()
                    .ok_or_else(|| anyhow!("{}", msg::get(msg::REMOTE_REQUIRED)))?,
                record.github_id,
            ));
        } else {
            known.push((value, None));
        }
    }
    let native_only = !args.get_flag("all")
        && known
            .iter()
            .all(|(name, id)| id.is_none() && Repo::KEYS.contains(&name.as_str()));
    let (catalog, remote_error) = catalog(
        store,
        args.get_flag("offline") || native_only,
        args.get_flag("refresh"),
    )?;
    if args.get_flag("all") && catalog.is_none() {
        if dry {
            // Legacy preview works offline for the native checkouts.
            let missing = workspace.missing_checkouts()?;
            let operations = workspace.mutate("clone", &missing, true)?;
            return Ok(report(operations, confirm));
        }
        return Err(anyhow!(
            "{}",
            remote_error.map_or_else(
                || msg::get(msg::REMOTE_REQUIRED),
                |failure| msg::get(failure.message())
            )
        ));
    }
    let mut remotes = Vec::new();
    if args.get_flag("all") {
        remotes.clone_from(&catalog.as_ref().expect("catalog checked").repositories);
    } else {
        for (name, id) in known {
            if let Some(native) = Repo::ALL.into_iter().find(|repo| repo.key() == name) {
                remotes.push(Remote {
                    id: 0,
                    slug: format!("cirnetlab/{}", native.name()),
                    default_branch: String::new(),
                    private: false,
                    archived: false,
                });
            } else {
                let item = catalog
                    .as_ref()
                    .and_then(|catalog| {
                        id.and_then(|id| catalog.repositories.iter().find(|entry| entry.id == id))
                            .or_else(|| catalog.get(&name))
                    })
                    .ok_or_else(|| {
                        anyhow!(
                            "{} {name}",
                            msg::get(
                                remote_error.map_or(msg::REMOTE_REQUIRED, github::Failure::message)
                            )
                        )
                    })?;
                remotes.push(item.clone());
            }
        }
    }
    let mut seen = HashSet::new();
    let mut plans = Vec::new();
    for remote in remotes {
        if !seen.insert(remote.slug.to_ascii_lowercase()) {
            continue;
        }
        let destination = base.join(remote.name());
        if destination.exists() && args.get_flag("all") {
            checked_checkout(&destination, "origin", Some(&remote.slug))?;
            continue;
        }
        ensure!(
            std::fs::symlink_metadata(&destination)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
            "{} {}",
            msg::get(msg::DEST_OCCUPIED),
            destination.display()
        );
        if args.get_flag("all") {
            let existing = registry.records.iter().find_map(|record| {
                let matches = record.github_id == Some(remote.id)
                    || record
                        .slug
                        .as_deref()
                        .is_some_and(|slug| slug.eq_ignore_ascii_case(&remote.slug));
                matches
                    .then_some(record.path.as_ref())
                    .flatten()
                    .filter(|path| path.exists())
                    .map(|path| (record, path))
            });
            if let Some((record, path)) = existing {
                checked_checkout(path, &record.remote, record.slug.as_deref())?;
                continue;
            }
        }
        plans.push((remote, destination));
    }
    let mut operations = Vec::with_capacity(plans.len());
    for (remote, destination) in plans {
        let mut entry = json!({"repo":if remote.id == 0 { remote.name() } else { &remote.slug }, "operation":"clone", "path":destination, "dryRun":dry});
        if !dry {
            let result = git::clone_repository(&destination, &remote.url(), &remote.slug).and_then(
                |checkout| {
                    if remote.id == 0 {
                        store.edit(|registry| save_checkout(registry, &checkout, None))
                    } else {
                        cloned_record(store, &remote, &checkout)
                    }
                },
            );
            match result {
                Ok(id) => {
                    entry["id"] = json!(id);
                    entry["state"] = json!("succeeded");
                }
                Err(error) => {
                    entry["state"] = json!("failed");
                    entry["detail"] = localization::report_diagnostic(&error);
                }
            }
        }
        operations.push(entry);
    }
    Ok(report(operations, confirm))
}

fn register(store: &Store, args: &ArgMatches) -> Result<Value> {
    let (catalog, error) = catalog(store, args.get_flag("offline"), args.get_flag("refresh"))?;
    let value = named(args);
    let remote = catalog
        .as_ref()
        .and_then(|catalog| catalog.get(value))
        .ok_or_else(|| {
            anyhow!(
                "{} {value}",
                msg::get(error.map_or(msg::REMOTE_REQUIRED, github::Failure::message))
            )
        })?;
    let entry = store.edit(|registry| {
        if let Some(existing) = registry.records.iter_mut().find(|record| {
            record.github_id == Some(remote.id)
                || record
                    .slug
                    .as_deref()
                    .is_some_and(|slug| slug.eq_ignore_ascii_case(&remote.slug))
        }) {
            existing.github_id = Some(remote.id);
            if existing.path.is_none() {
                existing.slug = Some(remote.slug.clone());
                existing.remote_url = Some(remote.url());
            }
            return Ok(existing.clone());
        }
        let mut item = Record::new(remote.name().into());
        item.github_id = Some(remote.id);
        item.slug = Some(remote.slug.clone());
        item.remote_url = Some(remote.url());
        item.preferred_branch = Some(remote.default_branch.clone());
        let id = registry.insert(item)?;
        Ok(registry
            .records
            .iter()
            .find(|entry| entry.id == id)
            .expect("record inserted")
            .clone())
    })?;
    Ok(json!({"repository":entry}))
}

fn remove(
    workspace: &Workspace,
    store: &Store,
    original: &Registry,
    args: &ArgMatches,
) -> Result<Value> {
    let (dry, confirm) = bulk_preview(args);
    let values = if args.get_flag("all") {
        original
            .records
            .iter()
            .map(|record| record.id.clone())
            .collect()
    } else {
        names(args)
    };
    let ids = values
        .iter()
        .map(|value| {
            registered(original, workspace, value).map(|index| original.records[index].id.clone())
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if !dry {
        store.edit(|registry| {
            registry.records.retain(|record| !ids.contains(&record.id));
            if registry
                .active
                .as_ref()
                .is_some_and(|active| ids.contains(active))
            {
                registry.active = None;
            }
            Ok(())
        })?;
    }
    Ok(report(
        ids.into_iter()
            .map(|id| json!({"operation":"remove","id":id,"dryRun":dry}))
            .collect(),
        confirm,
    ))
}

fn group(workspace: &Workspace, store: &Store, args: &ArgMatches) -> Result<Value> {
    let (action, options) = args.subcommand().expect("group operation");
    if action == "list" {
        return Ok(json!({"groups":store.load()?.groups}));
    }
    let name = options.get_one::<String>("group").expect("group name");
    ensure!(registry::token(name), "{}", msg::get(msg::NAME_INVALID));
    store.edit(|registry| {
        if action == "create" {
            registry.groups.insert(name.clone());
        } else {
            ensure!(
                registry.groups.contains(name),
                "{} {name}",
                msg::get(msg::GROUP_MISSING)
            );
            let targets = names(options);
            if targets.is_empty() && action == "remove" {
                registry.groups.remove(name);
                registry.records.iter_mut().for_each(|record| {
                    record.groups.remove(name);
                });
            } else {
                for target in targets {
                    let index = registered(registry, workspace, &target)?;
                    if action == "add" {
                        registry.records[index].groups.insert(name.clone());
                    } else {
                        registry.records[index].groups.remove(name);
                    }
                }
            }
        }
        Ok(json!({"groups":registry.groups}))
    })
}

fn config(
    workspace: &Workspace,
    store: &Store,
    registry: &Registry,
    args: &ArgMatches,
) -> Result<Value> {
    let value = named(args);
    let current = &registry.records[registered(registry, workspace, value)?];
    let option = |id| args.get_one::<String>(id).cloned();
    let path = args.get_one::<PathBuf>("path");
    let (alias, name, remote, branch, shell) = (
        option("alias"),
        option("name"),
        option("remote"),
        option("preferred-branch"),
        option("shell"),
    );
    let clear = args.get_flag("clear-alias");
    if path.is_none()
        && !clear
        && [&alias, &name, &remote, &branch, &shell]
            .into_iter()
            .all(Option::is_none)
    {
        return Ok(json!({"repository":current}));
    }
    ensure!(
        alias.as_deref().is_none_or(registry::token)
            && remote.as_deref().is_none_or(registry::token)
            && name.as_deref().is_none_or(registry::valid_text)
            && branch.as_deref().is_none_or(registry::valid_text),
        "{}",
        msg::get(msg::INVALID_CONFIG)
    );
    let checkout = if let Some(new) = path {
        Some(checked_checkout(
            &workspace.root.join(new),
            remote.as_deref().unwrap_or(&current.remote),
            current.slug.as_deref(),
        )?)
    } else if let (Some(remote), Some(path)) = (&remote, &current.path) {
        Some(checked_checkout(path, remote, current.slug.as_deref())?)
    } else {
        None
    };
    store.edit(|saved| {
        let record = saved
            .records
            .iter_mut()
            .find(|record| record.id == current.id)
            .ok_or_else(|| anyhow!("{}", msg::get(msg::NOT_FOUND)))?;
        if let Some(checkout) = &checkout {
            apply_checkout(record, checkout);
        }
        if let Some(alias) = alias {
            record.alias = Some(alias);
        } else if clear {
            record.alias = None;
        }
        if let Some(name) = name {
            record.name = name;
        }
        if let Some(remote) = remote {
            record.remote = remote;
        }
        if let Some(branch) = branch {
            record.preferred_branch = Some(branch);
        }
        if let Some(shell) = shell {
            record.shell = Some(shell);
        }
        Ok(json!({"repository":record}))
    })
}

fn sync_repositories(
    workspace: &Workspace,
    store: &Store,
    registry: &Registry,
    args: &ArgMatches,
    action: SyncAction,
    quiet: bool,
) -> Result<Value> {
    let dry = args.get_flag("dry-run");
    let operation = if action == SyncAction::Update {
        "update"
    } else {
        "fetch"
    };
    let explicit = names(args);
    let values = if args.get_flag("all") {
        if registry.records.is_empty() {
            Repo::KEYS.into_iter().map(str::to_owned).collect()
        } else {
            registry
                .records
                .iter()
                .filter(|record| record.path.is_some())
                .map(|record| record.id.clone())
                .collect()
        }
    } else {
        explicit
    };
    ensure!(!values.is_empty(), "{}", msg::get(msg::SCOPE_REQUIRED));
    let mut requests = Vec::new();
    let mut operations = Vec::new();
    let mut paths = BTreeSet::new();
    for value in values {
        match target_for(workspace,registry,&value) {
            Ok(target) if paths.insert(target.path.clone()) => requests.push(target),
            Ok(_) => (),
            Err(error) => operations.push(json!({"repo":value,"operation":operation,"dryRun":dry,"state":"failed","detail":localization::report_diagnostic(&error)})),
        }
    }
    let cancel = cancel::Cancellation::listen()?;
    let worker_count = usize::from(*args.get_one::<u8>("jobs").expect("default jobs"));
    let started = msg::get(msg::SYNC_STARTED);
    let finished = msg::get(msg::SYNC_FINISHED);
    let outcomes = sync::run(
        &requests,
        action,
        worker_count,
        dry,
        cancel.flag(),
        |progress| {
            if quiet {
                return;
            }
            match progress {
                SyncProgress::Started(path) => eprintln!("{started} · {}", path.display()),
                SyncProgress::Finished(result) => {
                    eprintln!("{finished} · {}", result.path.display())
                }
            }
        },
    );
    if !dry
        && outcomes
            .iter()
            .any(|outcome| matches!(outcome.state, SyncState::Succeeded))
        && !registry.records.is_empty()
    {
        store.edit(|saved| {
            for record in &mut saved.records {
                if outcomes.iter().any(|outcome| {
                    matches!(outcome.state, SyncState::Succeeded)
                        && record
                            .path
                            .as_ref()
                            .is_some_and(|path| same_path(path, &outcome.path))
                }) {
                    record.last_sync = Some(now());
                }
            }
            Ok(())
        })?;
    }
    operations.extend(outcomes.into_iter().zip(requests).map(|(outcome, target)| {
        let name = target
            .slug
            .as_deref()
            .and_then(|slug| slug.rsplit_once('/').map(|(_, name)| name))
            .or_else(|| target.path.file_name().and_then(std::ffi::OsStr::to_str));
        let mut value = serde_json::to_value(outcome).expect("serializable sync outcome");
        value["repo"] = json!(name);
        value["operation"] = json!(operation);
        value["dryRun"] = json!(dry);
        value
    }));
    Ok(report(operations, false))
}

pub(crate) fn execute(
    workspace: &Workspace,
    flags: &ArgMatches,
    args: &ArgMatches,
) -> Result<Value> {
    let (action, options) = args.subcommand().expect("repository subcommand");
    let vi = flags
        .get_one::<String>("lang")
        .is_some_and(|language| language == "vi");
    set_language(vi);
    let store = store(workspace, flags)?;
    let registry = store.load()?;
    match action {
        "list" => Ok(
            json!({"workspace":workspace.root,"repositories":registry.records,"active":registry.active}),
        ),
        "status" => status(workspace, &store, &registry, names(options)),
        "discover" => {
            let root = configured_root(workspace, options)?;
            let offline = options.get_flag("offline");
            let (catalog, error) = catalog(&store, offline, options.get_flag("refresh"))?;
            let mut rows = rows(workspace, &registry, catalog.as_ref(), &root)?;
            if let Some(query) = options.get_one::<String>("query") {
                let query = query.to_lowercase();
                rows.retain(|row| {
                    row.name.to_lowercase().contains(&query)
                        || row.selector.to_lowercase().contains(&query)
                });
            }
            let source = error.map_or(
                if offline { "offline" } else { "available" },
                github::Failure::code,
            );
            let mut result = json!({"workspace":workspace.root,"repositories":rows,"discoveryStatus":source,
                "catalogRefreshed":catalog.as_ref().map(|catalog|catalog.refreshed),
                "catalogStale":catalog.as_ref().is_none_or(Catalog::stale)});
            if let Some(error) = error {
                result["detail"] = localization::report_message(error.message());
            }
            Ok(result)
        }
        "clone" => clone(workspace, &store, &registry, options),
        "add" => add(workspace, &store, options),
        "remove" => remove(workspace, &store, &registry, options),
        "register" => register(&store, options),
        "group" => group(workspace, &store, options),
        "config" => config(workspace, &store, &registry, options),
        "switch" => store.edit(|saved| {
            let index = registered(saved, workspace, named(options))?;
            let entry = &saved.records[index];
            let path = entry
                .path
                .as_ref()
                .ok_or_else(|| anyhow!("{}", msg::get(msg::LOCAL_REQUIRED)))?;
            checked_checkout(path, &entry.remote, entry.slug.as_deref())?;
            saved.active = Some(entry.id.clone());
            Ok(json!({"active":saved.active,"path":path}))
        }),
        "fetch" | "update" => sync_repositories(
            workspace,
            &store,
            &registry,
            options,
            if action == "fetch" {
                SyncAction::Fetch
            } else {
                SyncAction::Update
            },
            flags.get_flag("json"),
        ),
        "diff" | "log" | "branches" | "worktrees" | "remotes" => {
            let target = target_for(workspace, &registry, named(options))?;
            checked_checkout(&target.path, &target.remote, target.slug.as_deref())?;
            let read = match action {
                "diff" if options.get_flag("staged") => ReadAction::StagedDiff,
                "diff" => ReadAction::Diff,
                "log" => ReadAction::Log,
                "branches" => ReadAction::Branches,
                "worktrees" => ReadAction::Worktrees,
                _ => ReadAction::Remotes,
            };
            Ok(json!({"path":target.path,"content":git::read(&target.path,&target.remote,read)?}))
        }
        "branch" | "worktree" | "stage" | "commit" | "push" => {
            let target = target_for(workspace, &registry, named(options))?;
            let branch = || {
                options
                    .get_one::<String>("branch")
                    .expect("branch name")
                    .clone()
            };
            let write = match action {
                "branch" if options.get_flag("switch") => WriteAction::SwitchBranch(branch()),
                "branch" => WriteAction::CreateBranch(branch()),
                "worktree" => WriteAction::AddWorktree {
                    path: workspace
                        .root
                        .join(options.get_one::<PathBuf>("path").expect("worktree path")),
                    branch: branch(),
                },
                "stage" => WriteAction::Stage(
                    options
                        .get_many::<PathBuf>("paths")
                        .expect("stage paths")
                        .cloned()
                        .collect(),
                ),
                "commit" => WriteAction::Commit(
                    options
                        .get_one::<String>("message")
                        .expect("commit message")
                        .clone(),
                ),
                _ => WriteAction::Push,
            };
            Ok(json!({"path":target.path,"content":git::write(&target,write)?}))
        }
        _ => bail!("{}", message(msg::NOT_FOUND)),
    }
}
