use super::{
    Entry, MAX_REF_BYTES, MAX_SCANNED, bounded_text, directory, entry_metadata, local_metadata,
    named_entries, redirected, safe_text,
};
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::tui::completion::native) enum GitData {
    Branches,
    Refs,
    Tags,
    Remotes,
    Stashes,
    Worktrees,
    Config,
}

struct Repository {
    git: PathBuf,
    common: PathBuf,
}

impl Repository {
    fn at(cwd: &Path) -> Option<Self> {
        let (git, metadata) = cwd
            .ancestors()
            .take(32)
            .map(|path| path.join(".git"))
            .find_map(|path| Some((path.clone(), local_metadata(&path)?)))
            .or_else(|| {
                let cwd = local_directory(cwd)?;
                bounded_text(&cwd.join("HEAD"), 4096)?;
                local_directory(&cwd.join("objects"))?;
                local_directory(&cwd.join("refs"))?;
                let metadata = local_metadata(&cwd)?;
                Some((cwd, metadata))
            })?;
        let git = if metadata.is_dir() {
            local_directory(&git)?
        } else {
            let pointer = bounded_text(&git, 4096)?;
            let target = pointer.trim().strip_prefix("gitdir: ")?;
            local_directory(&git.parent()?.join(target))?
        };
        let common = if let Some(pointer) = bounded_text(&git.join("commondir"), 4096) {
            local_directory(&git.join(pointer.trim()))?
        } else {
            git.clone()
        };
        Some(Self { git, common })
    }

    fn refs(&self) -> BTreeSet<String> {
        let mut refs = BTreeSet::new();
        if let Some(packed) = bounded_text(&self.common.join("packed-refs"), MAX_REF_BYTES) {
            for name in packed.lines().filter_map(|line| {
                let (hash, reference) = line.split_once(' ')?;
                (!hash.is_empty()
                    && hash.bytes().all(|ch| ch.is_ascii_hexdigit())
                    && reference.starts_with("refs/")
                    && safe_ref(reference))
                .then_some(reference)
            }) {
                refs.insert(name.to_owned());
                if refs.len() == MAX_SCANNED {
                    return refs;
                }
            }
        }

        let mut dirs = vec![(self.common.join("refs"), "refs/".to_owned(), 0)];
        let mut scanned = 0;
        while let Some((path, prefix, depth)) = dirs.pop() {
            for entry in directory(&path).into_iter().flatten() {
                if scanned == MAX_SCANNED || refs.len() == MAX_SCANNED {
                    return refs;
                }
                scanned += 1;
                let Ok(entry) = entry else { continue };
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                let name = format!("{prefix}{name}");
                if !safe_ref(&name) {
                    continue;
                }
                let Some(metadata) = entry_metadata(&entry.path()) else {
                    continue;
                };
                if redirected(&metadata) {
                    continue;
                }
                if metadata.is_dir() && depth < 12 {
                    dirs.push((entry.path(), format!("{name}/"), depth + 1));
                } else if metadata.is_file() {
                    refs.insert(name);
                }
            }
        }
        refs
    }

    fn stashes(&self) -> impl Iterator<Item = String> {
        let count = bounded_text(&self.common.join("logs/refs/stash"), MAX_REF_BYTES)
            .map_or(0, |text| text.lines().take(128).count());
        (0..count).map(|index| format!("stash@{{{index}}}"))
    }

    fn worktrees(&self) -> Vec<String> {
        let main = self
            .common
            .parent()
            .filter(|_| self.common.file_name().is_some_and(|name| name == ".git"))
            .and_then(|path| path.to_str().map(str::to_owned));
        let known = directory(&self.common.join("worktrees"))
            .into_iter()
            .flatten()
            .take(MAX_SCANNED)
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let metadata = entry.path().join("gitdir");
                let target = bounded_text(&metadata, 4096)?;
                let target = entry.path().join(target.trim());
                if target.file_name()? != ".git" {
                    return None;
                }
                local_metadata(&target)?;
                local_directory(target.parent()?)?
                    .into_os_string()
                    .into_string()
                    .ok()
            });
        main.into_iter().chain(known).collect()
    }

    fn configs(&self) -> impl Iterator<Item = String> {
        let shared = bounded_text(&self.common.join("config"), MAX_REF_BYTES);
        let worktree = bounded_text(&self.git.join("config.worktree"), MAX_REF_BYTES);
        shared.into_iter().chain(worktree)
    }
}

pub(super) fn repository(cwd: &Path, kind: GitData) -> Vec<Entry> {
    let Some(repo) = Repository::at(cwd) else {
        return Vec::new();
    };
    match kind {
        GitData::Branches => named_entries(
            repo.refs()
                .iter()
                .filter_map(|reference| branch_name(reference)),
        ),
        GitData::Refs => {
            let refs = repo.refs();
            let names = refs.iter().filter_map(|reference| {
                branch_name(reference)
                    .or_else(|| reference.strip_prefix("refs/tags/").map(str::to_owned))
            });
            named_entries(names.chain(refs.iter().cloned()))
        }
        GitData::Tags => named_entries(
            repo.refs()
                .iter()
                .filter_map(|name| name.strip_prefix("refs/tags/").map(str::to_owned)),
        ),
        GitData::Remotes => {
            let mut configured = BTreeSet::new();
            for config in repo.configs() {
                for line in config.lines() {
                    if let Some((section, Some(rem))) = config_section(line.trim())
                        && section == "remote"
                        && safe_ref(&rem)
                    {
                        configured.insert(rem);
                    }
                    if configured.len() == MAX_SCANNED {
                        return named_entries(configured);
                    }
                }
            }
            configured.extend(repo.refs().iter().filter_map(|name| {
                let (remote, _) = name.strip_prefix("refs/remotes/")?.split_once('/')?;
                Some(remote.to_owned())
            }));
            named_entries(configured)
        }
        GitData::Stashes => repo.stashes().map(Entry::named).collect(),
        GitData::Worktrees => named_entries(repo.worktrees()),
        GitData::Config => named_entries(repo.configs().flat_map(|text| config_keys(&text))),
    }
}

fn local_directory(path: &Path) -> Option<PathBuf> {
    if !local_metadata(path)?.is_dir() {
        return None;
    }
    let mut normalized = PathBuf::new();
    for part in path.components() {
        if part == Component::ParentDir {
            normalized.pop();
        } else {
            normalized.push(part);
        }
    }
    Some(normalized)
}

fn branch_name(reference: &str) -> Option<String> {
    if let Some(local) = reference.strip_prefix("refs/heads/") {
        return safe_ref(local).then(|| local.to_owned());
    }
    let tracking = reference.strip_prefix("refs/remotes/")?;
    let (_, branch) = tracking.split_once('/')?;
    (branch != "HEAD" && safe_ref(tracking)).then(|| tracking.to_owned())
}

fn safe_ref(value: &str) -> bool {
    value.len() <= 4096
        && safe_text(value)
        && value != "@"
        && !value.starts_with('-')
        && !value.ends_with('.')
        && !value.contains([' ', '~', '^', ':', '?', '*', '[', '\\'])
        && !value.contains("..")
        && !value.contains("@{")
        && value
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && !part.ends_with(".lock"))
}

fn config_word(word: &str) -> bool {
    !word.is_empty()
        && word
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-')
}

fn config_section(line: &str) -> Option<(String, Option<String>)> {
    let head = line.strip_prefix('[')?;
    let word_end = head.find([' ', '\t', ']', '.'])?;
    let (name, tail) = head.split_at(word_end);
    if !config_word(name) {
        return None;
    }
    let name = name.to_ascii_lowercase();
    let tail = tail.trim_start();
    if tail.starts_with(']') {
        return Some((name, None));
    }
    if let Some(legacy) = tail.strip_prefix('.') {
        let (subsection, _) = legacy.split_once(']')?;
        return (!subsection.is_empty() && safe_text(subsection))
            .then(|| (name, Some(subsection.to_ascii_lowercase())));
    }

    let mut quoted = tail.strip_prefix('"')?.chars();
    let mut subsection = String::new();
    while let Some(ch) = quoted.next() {
        match ch {
            '"' => {
                return (quoted.as_str().trim_start().starts_with(']') && safe_text(&subsection))
                    .then_some((name, Some(subsection)));
            }
            '\\' => subsection.push(quoted.next()?),
            _ => subsection.push(ch),
        }
    }
    None
}

fn config_keys(text: &str) -> Vec<String> {
    let mut section = None;
    let mut keys = Vec::new();
    let mut continued = false;
    for line in text.lines() {
        let line = line.trim();
        let part_of_value = continued;
        let trailing_slashes = line.bytes().rev().take_while(|ch| *ch == b'\\').count();
        continued = trailing_slashes % 2 == 1;
        if part_of_value || line.starts_with(['#', ';']) {
            continue;
        }
        if line.starts_with('[') {
            section = config_section(line)
                .filter(|(name, _)| name != "include" && name != "includeif")
                .map(|(name, subsection)| {
                    subsection
                        .map_or_else(|| name.clone(), |subsection| format!("{name}.{subsection}"))
                });
            continue;
        }
        if let Some(section) = &section {
            let key = line.split([' ', '\t', '=']).next().unwrap_or_default();
            if config_word(key) && key.starts_with(|ch: char| ch.is_ascii_alphabetic()) {
                keys.push(format!("{section}.{}", key.to_ascii_lowercase()));
                if keys.len() == MAX_SCANNED {
                    break;
                }
            }
        }
    }
    keys
}
