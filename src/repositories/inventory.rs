use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    thread,
};

use anyhow::{Result, ensure};

use super::{
    Catalog, Checkout, Record, Registry, Remote, Row, git, msg, record_row, registry::path_key,
    remote_row,
};
use crate::workspace::Workspace;

struct CatalogIndex<'a> {
    ids: HashMap<u64, &'a Remote>,
    slugs: HashMap<String, &'a Remote>,
}

impl<'a> CatalogIndex<'a> {
    fn new(catalog: Option<&'a Catalog>) -> Self {
        let repositories = catalog
            .into_iter()
            .flat_map(|catalog| &catalog.repositories);
        Self {
            ids: repositories.clone().map(|repo| (repo.id, repo)).collect(),
            slugs: repositories
                .map(|repo| (repo.slug.to_ascii_lowercase(), repo))
                .collect(),
        }
    }

    fn for_record(&self, record: &Record) -> Option<&'a Remote> {
        match record.github_id {
            Some(id) => self.ids.get(&id).copied(),
            None => self.for_slug(record.slug.as_deref()),
        }
    }

    fn for_slug(&self, slug: Option<&str>) -> Option<&'a Remote> {
        self.slugs.get(&slug?.to_ascii_lowercase()).copied()
    }
}

fn represents_slug(record: &Record, catalog: Option<&Remote>, slug: Option<&str>) -> bool {
    slug.is_some_and(|actual| {
        record
            .slug
            .as_deref()
            .is_some_and(|saved| saved.eq_ignore_ascii_case(actual))
            || catalog.is_some_and(|remote| remote.slug.eq_ignore_ascii_case(actual))
    })
}

pub(super) fn verify_record(
    checkout: &Checkout,
    record: &Record,
    catalog: Option<&Remote>,
    actual: Option<&Remote>,
) -> Result<()> {
    ensure!(
        record
            .github_id
            .zip(actual.map(|remote| remote.id))
            .is_none_or(|(saved, actual)| saved == actual)
            && ((record.slug.is_none() && catalog.is_none())
                || represents_slug(record, catalog, checkout.slug.as_deref())),
        "{}",
        msg::get(msg::IDENTITY_CHANGED)
    );
    Ok(())
}

pub(super) fn record_remote<'a>(
    catalog: Option<&'a Catalog>,
    record: &Record,
) -> Option<&'a Remote> {
    let repositories = &catalog?.repositories;
    match record.github_id {
        Some(id) => repositories.iter().find(|repo| repo.id == id),
        None => repositories.iter().find(|repo| {
            record
                .slug
                .as_deref()
                .is_some_and(|slug| repo.slug.eq_ignore_ascii_case(slug))
        }),
    }
}

pub(super) fn checkout_remote<'a>(
    catalog: Option<&'a Catalog>,
    checkout: &Checkout,
) -> Option<&'a Remote> {
    let slug = checkout.slug.as_deref()?;
    catalog?
        .repositories
        .iter()
        .find(|remote| remote.slug.eq_ignore_ascii_case(slug))
}

enum Source<'a> {
    Record(&'a Record, usize),
    Discovery,
}

struct Candidate<'a> {
    path: PathBuf,
    source: Source<'a>,
}

fn inspect(candidates: &[Candidate<'_>]) -> Vec<Result<Checkout>> {
    const MAX_WORKERS: usize = 8;
    let count = thread::available_parallelism().map_or(1, usize::from);
    let workers = count.min(MAX_WORKERS).min(candidates.len());
    let next = AtomicUsize::new(0);
    let mut results: Vec<_> = thread::scope(|scope| {
        (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut results = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(candidate) = candidates.get(index) else {
                            break;
                        };
                        let result = match candidate.source {
                            Source::Record(record, _) => {
                                git::inspect(&candidate.path, &record.remote)
                            }
                            Source::Discovery => git::inspect_discovered(&candidate.path),
                        };
                        results.push((index, result));
                    }
                    results
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flat_map(|worker| worker.join().expect("Git inspection worker"))
            .collect()
    });
    results.sort_unstable_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

fn decorate(row: &mut Row, remote: &Remote) {
    row.private = Some(remote.private);
    row.archived = Some(remote.archived);
    if row.path.is_none() {
        row.branch = (!remote.default_branch.is_empty()).then(|| remote.default_branch.clone());
        row.remote = Some(remote.slug.clone());
        row.remote_url = Some(remote.url());
    }
}

pub(super) fn rows(
    workspace: &Workspace,
    registry: &Registry,
    catalog: Option<&Catalog>,
    root: &Path,
) -> Result<Vec<Row>> {
    let remotes = CatalogIndex::new(catalog);
    let mut represented = HashSet::new();
    let mut records = Vec::with_capacity(registry.records.len());
    let mut candidates = Vec::new();
    let mut paths = HashSet::new();
    let active = registry.active.as_deref();
    // If legacy path spellings resolve to the same checkout, keep the active selector.
    let order = registry
        .records
        .iter()
        .filter(|record| active == Some(record.id.as_str()))
        .chain(
            registry
                .records
                .iter()
                .filter(|record| active != Some(record.id.as_str())),
        );
    for record in order {
        if let Some(path) = &record.path {
            let actual = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !paths.insert(path_key(&actual)) {
                continue;
            }
            candidates.push(Candidate {
                path: path.clone(),
                source: Source::Record(record, records.len()),
            });
        }
        let mut row = record_row(record, active);
        if let Some(remote) = remotes.for_record(record) {
            decorate(&mut row, remote);
            represented.insert(remote.id);
        }
        records.push((record, row));
    }

    // The workspace is not itself registered merely by viewing it.
    let workspace_key = path_key(&workspace.root);
    for path in git::discovery_candidates(root, 2)? {
        let key = path_key(&path);
        if key != workspace_key && paths.insert(key) {
            candidates.push(Candidate {
                path,
                source: Source::Discovery,
            });
        }
    }

    let mut discovered = Vec::new();
    let inspections = inspect(&candidates);
    for (candidate, result) in candidates.into_iter().zip(inspections) {
        let Candidate { path, source } = candidate;
        let failure = |error: &anyhow::Error| git::failure_state(&path, error).to_owned();
        match source {
            Source::Record(record, index) => {
                let known = remotes.for_record(record);
                let row = &mut records[index].1;
                match result.and_then(|checkout| {
                    let actual = remotes.for_slug(checkout.slug.as_deref());
                    verify_record(&checkout, record, known, actual)?;
                    Ok((checkout, actual))
                }) {
                    Ok((checkout, actual)) => {
                        row.apply_checkout(checkout);
                        if let Some(remote) = known.or(actual) {
                            decorate(row, remote);
                            represented.insert(remote.id);
                        }
                    }
                    Err(error) => row.state = failure(&error),
                }
            }
            Source::Discovery => {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let mut row = Row::new(path.to_string_lossy().into_owned(), name, "local");
                match result {
                    Ok(checkout) => {
                        let catalog = remotes.for_slug(checkout.slug.as_deref());
                        row.apply_checkout(checkout);
                        if let Some(remote) = catalog {
                            decorate(&mut row, remote);
                            represented.insert(remote.id);
                        }
                    }
                    Err(error) => {
                        row.state = failure(&error);
                        row.path = Some(path);
                    }
                }
                discovered.push(Some(row));
            }
        }
    }

    // Link a pathless registry entry to a checkout for this view only when
    // the remote identifies one unambiguous worktree. A stored path always wins.
    let mut local_matches: HashMap<(&str, String), Vec<usize>> = HashMap::new();
    for (index, row) in discovered
        .iter()
        .enumerate()
        .filter_map(|(i, row)| Some((i, row.as_ref()?)))
    {
        if let (Some(remote), Some(slug)) = (&row.remote_name, &row.remote) {
            local_matches
                .entry((remote, slug.to_ascii_lowercase()))
                .or_default()
                .push(index);
        }
    }
    let matches: Vec<_> = records
        .iter()
        .map(|(record, row)| {
            if row.path.is_some() {
                return None;
            }
            let saved = record.slug.as_deref();
            let current = remotes.for_record(record).map(|repo| repo.slug.as_str());
            let mut indices = saved
                .into_iter()
                .chain(
                    current
                        .filter(|slug| saved.is_none_or(|saved| !saved.eq_ignore_ascii_case(slug))),
                )
                .filter(|slug| {
                    record
                        .github_id
                        .zip(remotes.for_slug(Some(slug)).map(|remote| remote.id))
                        .is_none_or(|(saved, actual)| saved == actual)
                })
                .filter_map(|slug| {
                    local_matches.get(&(record.remote.as_str(), slug.to_ascii_lowercase()))
                })
                .flatten();
            let first = *indices.next()?;
            indices.next().is_none().then_some(first)
        })
        .collect();
    drop(local_matches);

    let mut counts = HashMap::new();
    for index in matches.iter().flatten() {
        *counts.entry(*index).or_insert(0) += 1;
    }
    let mut rows: Vec<_> = records
        .into_iter()
        .zip(matches)
        .map(|((_, record), index)| {
            let Some(index) = index.filter(|index| counts[index] == 1) else {
                return record;
            };
            let mut local = discovered[index]
                .take()
                .expect("unique discovered checkout");
            local.selector = record.selector;
            local.name = record.name;
            local.alias = record.alias;
            local.groups = record.groups;
            local.preferred_branch = record.preferred_branch;
            local.shell = record.shell;
            local.active = record.active;
            local.registered = true;
            local.last_sync = record.last_sync;
            local.private = local.private.or(record.private);
            local.archived = local.archived.or(record.archived);
            local
        })
        .collect();
    rows.extend(discovered.into_iter().flatten());
    rows.extend(
        remotes
            .ids
            .into_values()
            .filter(|repo| !represented.contains(&repo.id))
            .map(remote_row),
    );
    rows.sort_by_cached_key(|row| (row.name.to_lowercase(), row.selector.clone()));
    Ok(rows)
}
