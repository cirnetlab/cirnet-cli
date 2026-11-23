use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::Path,
};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use super::{completion, editor::Input};
use crate::{environment::write_json, workspace::canonical};

const LIMIT: usize = 100;
const LINE_LIMIT: usize = 256;
const FILE_LIMIT: u64 = 64 * 1024;
const FILE: &str = "tui-history.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Origin {
    Cnl,
    Native,
}

struct Entry {
    line: String,
    origin: Origin,
    pending: bool,
}

struct Browse {
    draft: Input,
    index: usize,
    prefix: bool,
}

#[derive(Default)]
pub(super) struct History {
    entries: VecDeque<Entry>,
    browse: Option<Browse>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u8,
    commands: Vec<String>,
}

impl History {
    pub(super) fn entries(&self) -> impl DoubleEndedIterator<Item = (&str, Origin)> {
        self.entries
            .iter()
            .map(|entry| (entry.line.as_str(), entry.origin))
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn record_cnl(&mut self, line: &str, args: &[String]) {
        let line = line.trim();
        if safe_line(line) && eligible_cnl(args) {
            self.push(line.to_owned(), Origin::Cnl, true);
        }
    }

    pub(super) fn record_native(&mut self, line: &str) {
        let line = line.trim();
        // A known query with no user-supplied values can be safely recalled.
        if line.len() <= LINE_LIMIT
            && matches!(
                line.to_ascii_lowercase().as_str(),
                "git --version"
                    | "git status"
                    | "git status --short"
                    | "git status -sb"
                    | "git log --oneline"
                    | "git branch"
                    | "git branch --list"
                    | "get-location"
                    | "get-childitem"
                    | "get-childitem -force"
                    | "dir"
                    | "dir /b"
                    | "dir /w"
                    | "pwd"
                    | "ls"
                    | "ls -a"
                    | "ls -la"
                    | "ls -l"
            )
        {
            self.push(line.to_owned(), Origin::Native, false);
        }
    }

    fn push(&mut self, line: String, origin: Origin, pending: bool) {
        if self
            .entries
            .back()
            .is_some_and(|last| last.line == line && last.origin == origin)
        {
            return;
        }
        if self.entries.len() == LIMIT {
            self.entries.pop_front();
        }
        self.entries.push_back(Entry {
            line,
            origin,
            pending,
        });
    }

    pub(super) fn reset_navigation(&mut self) {
        self.browse = None;
    }

    pub(super) fn restore_draft(&mut self) -> Option<Input> {
        self.browse.take().map(|browse| browse.draft)
    }

    pub(super) fn navigate(&mut self, current: &Input, older: bool, prefix: bool) -> Option<Input> {
        let next = match (older, self.browse.as_ref()) {
            (true, browse) => {
                let end = browse.map_or(self.entries.len(), |browse| browse.index);
                let query = browse.map_or(current, |browse| &browse.draft);
                let prefix = prefix || browse.is_some_and(|browse| browse.prefix);
                let index = (0..end)
                    .rev()
                    .find(|&index| !prefix || self.entries[index].line.starts_with(&query.value))?;
                let draft = self
                    .browse
                    .take()
                    .map_or_else(|| current.clone(), |browse| browse.draft);
                Browse {
                    draft,
                    index,
                    prefix,
                }
            }
            (false, Some(browse)) => {
                let index = (browse.index + 1..self.entries.len()).find(|&index| {
                    !browse.prefix || self.entries[index].line.starts_with(&browse.draft.value)
                });
                let Some(index) = index else {
                    return self.restore_draft();
                };
                let mut browse = self.browse.take().expect("history navigation is active");
                browse.index = index;
                browse
            }
            (false, None) => return None,
        };
        let value = self.entries[next.index].line.clone();
        self.browse = Some(next);
        Some(Input::at_end(value))
    }

    pub(super) fn load(directory: &Path) -> Result<Self> {
        let path = directory.join(FILE);
        let mut history = Self::default();
        if !ordinary_file(&path)? {
            return Ok(history);
        }
        let mut bytes = Vec::new();
        File::open(&path)?
            .take(FILE_LIMIT + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() as u64 <= FILE_LIMIT, "HISTORY_SIZE_LIMIT");
        let stored: Stored = serde_json::from_slice(&bytes)?;
        ensure!(stored.version == 1, "HISTORY_SCHEMA_VERSION");
        let first = stored.commands.len().saturating_sub(LIMIT);
        for line in stored
            .commands
            .into_iter()
            .skip(first)
            .filter(|line| valid_cnl_line(line))
        {
            history.push(line, Origin::Cnl, false);
        }
        Ok(history)
    }

    pub(super) fn save(&self, directory: &Path) -> Result<()> {
        if !self.entries.iter().any(|entry| entry.pending) {
            return Ok(());
        }
        fs::create_dir_all(directory)?;
        ensure!(canonical(directory)? == directory, "HISTORY_DIRECTORY");
        let lock_path = directory.join("tui-history.lock");
        ordinary_file(&lock_path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.lock()?;
        // Merge with the newest file under a dedicated lock so other terminal
        // sessions and runtime operations can share the same profile.
        let mut stored = Self::load(directory)?;
        for entry in self.entries.iter().filter(|entry| entry.pending) {
            stored.push(entry.line.clone(), Origin::Cnl, false);
        }
        write_json(
            &directory.join(FILE),
            &Stored {
                version: 1,
                commands: stored.entries.into_iter().map(|entry| entry.line).collect(),
            },
        )
    }
}

fn ordinary_file(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(file) => {
            ensure!(
                file.is_file() && canonical(path)? == path,
                "HISTORY_FILE_TYPE"
            );
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn valid_cnl_line(line: &str) -> bool {
    if !safe_line(line) {
        return false;
    }
    let Ok(args) = completion::arguments(line) else {
        return false;
    };
    let args = if args
        .first()
        .is_some_and(|arg| matches!(arg.as_str(), "cnl" | "cnl.exe"))
    {
        &args[1..]
    } else {
        &args
    };
    eligible_cnl(args)
}

fn safe_line(line: &str) -> bool {
    line.len() <= LINE_LIMIT && !line.chars().any(char::is_control)
}

fn eligible_cnl(args: &[String]) -> bool {
    if !safe_cnl(args) {
        return false;
    }
    match completion::grammar()
        .clone()
        .try_get_matches_from(std::iter::once("cnl").chain(args.iter().map(String::as_str)))
    {
        Ok(_) => true,
        Err(error) => matches!(
            error.kind(),
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
        ),
    }
}

pub(super) fn safe_cnl(args: &[String]) -> bool {
    const WORDS: &[&str] = &[
        "doctor",
        "env",
        "show",
        "check",
        "set",
        "setup",
        "repos",
        "discover",
        "status",
        "clone",
        "update",
        "build",
        "run",
        "dev",
        "stop",
        "verify",
        "help",
        "logs",
        "completion",
        "shell",
        "development",
        "packaged",
        "native",
        "vite",
        "powershell",
        "cmd",
        "bash",
        "wsl",
        "en",
        "vi",
    ];
    const FLAGS: &[&str] = &[
        "--all",
        "--repo",
        "--offline",
        "--dry-run",
        "--initialize-db",
        "--open",
        "--desktop",
        "--help",
        "-h",
        "--version",
        "-V",
        "--json",
        "--lang",
        "--browser",
        "--mode",
        "--service",
    ];
    !args.is_empty()
        && args.iter().all(|arg| {
            WORDS.contains(&arg.as_str())
                || crate::workspace::Repo::KEYS.contains(&arg.as_str())
                || FLAGS.contains(&arg.as_str())
                || matches!(
                    arg.as_str(),
                    "--lang=en"
                        | "--lang=vi"
                        | "--desktop=interface"
                        | "--mode=development"
                        | "--mode=packaged"
                        | "--service=native"
                        | "--service=vite"
                )
                || arg
                    .strip_prefix("--repo=")
                    .is_some_and(|repo| crate::workspace::Repo::KEYS.contains(&repo))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(history: &mut History, line: &str) {
        history.record_cnl(line, &completion::arguments(line).unwrap());
    }

    #[test]
    fn bounds_and_filters_cnl_and_native_entries() {
        let mut history = History::default();
        record(&mut history, "run --mode=packaged");
        record(&mut history, "run --mode=packaged");
        record(&mut history, "run --receipt sensitive-path");
        history.record_native("git status --short");
        history.record_native("git clone https://user:secret@example.invalid/project");
        assert_eq!(
            history.entries().collect::<Vec<_>>(),
            [
                ("run --mode=packaged", Origin::Cnl),
                ("git status --short", Origin::Native)
            ]
        );
        for _ in 0..100 {
            record(&mut history, "stop --dry-run");
            record(&mut history, "status");
        }
        assert_eq!(history.entries().count(), LIMIT);
    }

    #[test]
    fn prefix_navigation_restores_exact_draft_and_selection() {
        let mut history = History::default();
        for line in ["status --json", "env show", "status"] {
            record(&mut history, line);
        }
        let mut draft = Input::at_end("st".into());
        draft.move_to(1, true);
        let newest = history.navigate(&draft, true, true).unwrap();
        assert_eq!(newest.value, "status");
        let older = history.navigate(&newest, true, true).unwrap();
        assert_eq!(older.value, "status --json");
        assert_eq!(
            history.navigate(&older, false, false).unwrap().value,
            "status"
        );
        let restored = history.navigate(&newest, false, false).unwrap();
        assert_eq!(restored.value, draft.value);
        assert_eq!(restored.selection(), draft.selection());
    }

    #[test]
    fn persistence_merges_sessions_and_excludes_native_and_sensitive_entries() {
        let dir = canonical(&std::env::temp_dir()).unwrap().join(format!(
            "cnl-history-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut a = History::load(&dir).unwrap();
        let mut b = History::load(&dir).unwrap();
        record(&mut a, "status");
        a.record_native("git status");
        record(&mut b, "env show");
        record(&mut b, "setup --receipt arbitrary");
        a.save(&dir).unwrap();
        b.save(&dir).unwrap();
        let restored = History::load(&dir).unwrap();
        assert_eq!(
            restored.entries().collect::<Vec<_>>(),
            [("status", Origin::Cnl), ("env show", Origin::Cnl)]
        );
        assert!(
            !fs::read_to_string(dir.join(FILE))
                .unwrap()
                .contains("arbitrary")
        );
        fs::remove_file(dir.join(FILE)).unwrap();
        fs::remove_file(dir.join("tui-history.lock")).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
