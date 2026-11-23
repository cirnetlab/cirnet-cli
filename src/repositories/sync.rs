use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};

use serde::Serialize;

use super::git::{self, Preflight, Target};

pub use super::git::SyncAction;

pub const DEFAULT_JOBS: usize = 3;
const MAX_JOBS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncState {
    Preview,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutcome {
    pub path: PathBuf,
    pub state: SyncState,
    pub detail: String,
}

#[derive(Clone, Debug)]
pub enum SyncProgress {
    Started(PathBuf),
    Finished(SyncOutcome),
}

pub fn run(
    targets: &[Target],
    action: SyncAction,
    jobs: usize,
    dry_run: bool,
    cancel: &AtomicBool,
    progress: impl Fn(SyncProgress) + Sync,
) -> Vec<SyncOutcome> {
    let cancelled = || cancel.load(Ordering::Acquire);
    let finish = |index: usize, state, detail: String| {
        let result = SyncOutcome {
            path: targets[index].path.clone(),
            state,
            detail,
        };
        progress(SyncProgress::Finished(result.clone()));
        result
    };
    let cancel_result = |index| finish(index, SyncState::Cancelled, String::new());
    let mut outcomes: Vec<_> = (0..targets.len()).map(|_| None).collect();
    let mut work: BTreeMap<_, Vec<_>> = BTreeMap::new();

    // All selected repositories are checked before starting a fetch or merge.
    let plans = bounded_map(targets, jobs, |_, target| {
        (!cancelled()).then(|| git::preflight(target, action))
    });
    for (index, plan) in plans.into_iter().enumerate() {
        match plan {
            None => outcomes[index] = Some(cancel_result(index)),
            Some(Err(error)) => {
                outcomes[index] = Some(finish(index, SyncState::Failed, error.to_string()));
            }
            Some(Ok(_)) if cancelled() => outcomes[index] = Some(cancel_result(index)),
            Some(Ok(plan)) if dry_run => {
                outcomes[index] = Some(finish(
                    index,
                    SyncState::Preview,
                    plan.checkout.remote_url.unwrap_or_default(),
                ));
            }
            Some(Ok(plan)) => {
                work.entry(plan.checkout.common_dir.clone())
                    .or_default()
                    .push((index, plan));
            }
        }
    }
    let groups: Vec<_> = work.into_values().collect();
    for (index, result) in bounded_map(&groups, jobs, |_, group: &Vec<(usize, Preflight)>| {
        group
            .iter()
            .map(|(index, plan)| {
                let index = *index;
                let result = if cancelled() {
                    cancel_result(index)
                } else {
                    progress(SyncProgress::Started(targets[index].path.clone()));
                    match git::apply(&targets[index], action, plan, cancelled) {
                        Ok(Some(detail)) => finish(index, SyncState::Succeeded, detail),
                        Ok(None) => cancel_result(index),
                        Err(error) => finish(index, SyncState::Failed, error.to_string()),
                    }
                };
                (index, result)
            })
            .collect::<Vec<_>>()
    })
    .into_iter()
    .flatten()
    {
        outcomes[index] = Some(result);
    }
    outcomes
        .into_iter()
        .map(|outcome| outcome.expect("one result per repository"))
        .collect()
}

fn bounded_map<T: Sync, U: Send>(
    items: &[T],
    jobs: usize,
    apply: impl Fn(usize, &T) -> U + Sync,
) -> Vec<U> {
    let next = AtomicUsize::new(0);
    let results = Mutex::new((0..items.len()).map(|_| None).collect::<Vec<_>>());
    let vietnamese = crate::localization::message(["en", "vi"]) == "vi";
    let jobs = if jobs == 0 {
        DEFAULT_JOBS
    } else {
        jobs.min(MAX_JOBS)
    };
    thread::scope(|scope| {
        for _ in 0..jobs.min(items.len()) {
            scope.spawn(|| {
                crate::localization::set_language(vietnamese);
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else {
                        break;
                    };
                    let result = apply(index, item);
                    results.lock().expect("result mutex")[index] = Some(result);
                }
            });
        }
    });
    results
        .into_inner()
        .expect("result mutex")
        .into_iter()
        .map(|result| result.expect("worker returned one result per item"))
        .collect()
}

#[cfg(test)]
#[path = "sync_tests.rs"]
mod tests;
