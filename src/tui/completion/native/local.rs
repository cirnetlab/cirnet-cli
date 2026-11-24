use std::{
    collections::{BTreeSet, VecDeque},
    ffi::OsString,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    rc::Rc,
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::{Duration, Instant},
};

mod git;
mod tools;

pub(super) use git::GitData;

const MAX_SCANNED: usize = 1024;
const MAX_CACHE: usize = 24;
const MAX_WANTED: usize = 8;
const MAX_REF_BYTES: u64 = 512 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Key {
    Paths(PathBuf),
    Executables { path: OsString, cwd: PathBuf },
    GitCommands,
    GitOptions(String),
    GitRepository { cwd: PathBuf, kind: GitData },
    GitConfigKeys,
    PowerShellCommands,
    PowerShellParameters(String),
    PowerShellValues { command: String, parameter: String },
    BashCommands,
    CmdExecutables { path: OsString, cwd: PathBuf },
    WslDistributions,
}

impl Key {
    fn lifetime(&self) -> Duration {
        match self {
            Self::Paths(_) | Self::GitRepository { .. } => Duration::from_secs(3),
            Self::WslDistributions => Duration::from_secs(10),
            Self::Executables { .. } | Self::CmdExecutables { .. } => Duration::from_secs(30),
            _ => Duration::from_secs(60),
        }
    }

    fn scan(&self) -> Vec<Entry> {
        match self {
            Self::Paths(path) => paths(path),
            Self::Executables { path, cwd } => executables(path, cwd),
            Self::GitCommands => tools::git_commands(),
            Self::GitOptions(command) => tools::git_options(command),
            Self::GitRepository { cwd, kind } => git::repository(cwd, *kind),
            Self::GitConfigKeys => tools::git_config_keys(),
            Self::PowerShellCommands => tools::powershell(None, None),
            Self::PowerShellParameters(command) => tools::powershell(Some(command), None),
            Self::PowerShellValues { command, parameter } => {
                tools::powershell(Some(command), Some(parameter))
            }
            Self::BashCommands => tools::bash_commands(),
            Self::CmdExecutables { path, cwd } => cmd_executables(path, cwd),
            Self::WslDistributions => tools::wsl_distributions(),
        }
    }
}

pub(super) struct Entry {
    pub(super) name: String,
    pub(super) directory: bool,
}

impl Entry {
    fn named(name: String) -> Self {
        Self {
            name,
            directory: false,
        }
    }
}

fn named_entries(names: impl IntoIterator<Item = String>) -> Vec<Entry> {
    names
        .into_iter()
        .filter(|name| name.len() <= 4096 && safe_text(name))
        .take(MAX_SCANNED)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(Entry::named)
        .collect()
}

struct Snapshot {
    key: Key,
    entries: Rc<[Entry]>,
    scanned: Instant,
}

struct Worker {
    requests: SyncSender<Key>,
    results: Receiver<(Key, Vec<Entry>)>,
}

impl Worker {
    fn start() -> Option<Self> {
        let (requests, work) = mpsc::sync_channel::<Key>(2);
        let (responses, results) = mpsc::sync_channel(3);
        thread::Builder::new()
            .name("cnl-completion".into())
            .spawn(move || {
                while let Ok(key) = work.recv() {
                    let entries = key.scan();
                    if responses.send((key, entries)).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self { requests, results })
    }
}

#[derive(Default)]
pub(super) struct Cache {
    worker: Option<Worker>,
    entries: VecDeque<Snapshot>,
    pending: Vec<Key>,
    wanted: Vec<Key>,
    waiting_for_slot: bool,
}

impl Cache {
    pub(super) fn reset_request(&mut self) {
        self.wanted.clear();
        self.waiting_for_slot = false;
    }

    pub(super) fn metadata_ready(&mut self) -> bool {
        let mut ready = false;
        if let Some(worker) = &self.worker {
            while let Ok((key, entries)) = worker.results.try_recv() {
                self.pending.retain(|request| request != &key);
                ready |= self.wanted.contains(&key);
                self.entries.retain(|entry| entry.key != key);
                self.entries.push_back(Snapshot {
                    key,
                    entries: entries.into(),
                    scanned: Instant::now(),
                });
                if self.entries.len() > MAX_CACHE {
                    self.entries.pop_front();
                }
            }
        }
        if self.waiting_for_slot {
            self.schedule_wanted();
        }
        ready
    }

    pub(super) fn metadata_pending(&self) -> bool {
        self.waiting_for_slot || self.wanted.iter().any(|key| self.pending.contains(key))
    }

    pub(super) fn get(&mut self, key: Key) -> Rc<[Entry]> {
        if self.wanted.len() < MAX_WANTED && !self.wanted.contains(&key) {
            self.wanted.push(key.clone());
            self.schedule_wanted();
        }
        self.entries
            .iter()
            .find(|entry| entry.key == key)
            .map_or_else(Rc::default, |entry| Rc::clone(&entry.entries))
    }

    fn schedule_wanted(&mut self) {
        self.waiting_for_slot = false;
        for key in &self.wanted {
            if self.pending.contains(key)
                || self
                    .entries
                    .iter()
                    .any(|entry| &entry.key == key && entry.scanned.elapsed() < key.lifetime())
            {
                continue;
            }
            if self.worker.is_none() {
                self.worker = Worker::start();
            }
            let Some(worker) = &self.worker else {
                return;
            };
            match worker.requests.try_send(key.clone()) {
                Ok(()) => self.pending.push(key.clone()),
                Err(mpsc::TrySendError::Full(_)) => {
                    self.waiting_for_slot = true;
                    return;
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.worker = None;
                    self.pending.clear();
                    self.waiting_for_slot = true;
                    return;
                }
            }
        }
    }
}

fn paths(path: &Path) -> Vec<Entry> {
    let mut entries = directory(path)
        .into_iter()
        .flatten()
        .take(MAX_SCANNED)
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let metadata = entry_metadata(&entry.path())?;
            (!redirected(&metadata)).then_some(Entry {
                name,
                directory: metadata.is_dir(),
            })
        })
        .filter(|entry| safe_text(&entry.name))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries
}

fn executables(path: &OsString, cwd: &Path) -> Vec<Entry> {
    #[cfg(unix)]
    fn executable(metadata: &fs::Metadata) -> bool {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    fn executable(_: &fs::Metadata) -> bool {
        false
    }

    let mut names = BTreeSet::new();
    // Budget per directory so one large directory cannot hide every later PATH entry.
    for directory_path in std::env::split_paths(path).take(24) {
        for entry in directory(&cwd.join(directory_path))
            .into_iter()
            .flatten()
            .take(768)
            .filter_map(Result::ok)
        {
            if let Ok(name) = entry.file_name().into_string()
                && safe_text(&name)
                && entry_metadata(&entry.path()).is_some_and(|metadata| {
                    executable(&metadata)
                        || metadata.file_type().is_symlink()
                            && fs::read_link(entry.path())
                                .ok()
                                .and_then(|target| {
                                    local_metadata(&entry.path().parent()?.join(target))
                                })
                                .is_some_and(|target| executable(&target))
                })
            {
                names.insert(name);
                if names.len() >= MAX_SCANNED {
                    return names.into_iter().map(Entry::named).collect();
                }
            }
        }
    }
    names.into_iter().map(Entry::named).collect()
}

#[cfg(windows)]
fn cmd_executables(path: &OsString, cwd: &Path) -> Vec<Entry> {
    let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let extensions = extensions
        .split(';')
        .take(24)
        .filter_map(|extension| extension.strip_prefix('.'))
        .collect::<Vec<_>>();
    let mut names = BTreeSet::new();
    for folder in std::iter::once(cwd.to_owned()).chain(std::env::split_paths(path).take(24)) {
        for entry in directory(&cwd.join(folder))
            .into_iter()
            .flatten()
            .take(768)
            .filter_map(Result::ok)
        {
            let path = entry.path();
            let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some(name) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            if extensions
                .iter()
                .any(|value| value.eq_ignore_ascii_case(extension))
                && safe_text(name)
                && entry_metadata(&path)
                    .is_some_and(|metadata| metadata.is_file() && !redirected(&metadata))
            {
                names.insert(name.to_owned());
                if names.len() == MAX_SCANNED {
                    return named_entries(names);
                }
            }
        }
    }
    named_entries(names)
}

#[cfg(not(windows))]
fn cmd_executables(_: &OsString, _: &Path) -> Vec<Entry> {
    Vec::new()
}

pub(super) fn safe_text(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(char::is_control)
}

fn bounded_text(path: &Path, limit: u64) -> Option<String> {
    let metadata = local_metadata(path)?;
    if !metadata.is_file() || metadata.len() > limit {
        return None;
    }
    let mut text = String::new();
    fs::File::open(path)
        .ok()?
        .take(limit + 1)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() as u64 <= limit).then_some(text)
}

fn directory(path: &Path) -> Option<fs::ReadDir> {
    local_metadata(path)?
        .is_dir()
        .then(|| fs::read_dir(path).ok())?
}

fn entry_metadata(path: &Path) -> Option<fs::Metadata> {
    local_mount(path).then(|| fs::symlink_metadata(path).ok())?
}

fn local_metadata(path: &Path) -> Option<fs::Metadata> {
    if !path.is_absolute() || !local_drive(path) {
        return None;
    }
    let mut checked = PathBuf::new();
    let mut result = None;
    for (index, component) in path.components().enumerate() {
        if index == 128 {
            return None;
        }
        if component == Component::ParentDir {
            checked.pop();
        } else {
            checked.push(component);
        }
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        if !local_mount(&checked) {
            return None;
        }
        let metadata = fs::symlink_metadata(&checked).ok()?;
        if redirected(&metadata) {
            return None;
        }
        result = Some(metadata);
    }
    result
}

fn redirected(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

#[cfg(windows)]
fn local_drive(path: &Path) -> bool {
    use std::path::Prefix;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDriveTypeW(root: *const u16) -> u32;
    }

    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return false;
    };
    let (Prefix::Disk(drive) | Prefix::VerbatimDisk(drive)) = prefix.kind() else {
        return false;
    };
    let root = [u16::from(drive), u16::from(b':'), u16::from(b'\\'), 0];
    // GetDriveTypeW reads a NUL-terminated local drive root.
    let drive_type = unsafe { GetDriveTypeW(root.as_ptr()) };
    matches!(drive_type, 2 | 3 | 5 | 6)
}

#[cfg(not(windows))]
fn local_drive(_: &Path) -> bool {
    true
}

#[cfg(target_os = "linux")]
fn local_mount(path: &Path) -> bool {
    use std::cell::RefCell;

    thread_local! {
        static MOUNTS: RefCell<Option<(Instant, Option<Vec<PathBuf>>)>> = const { RefCell::new(None) };
    }
    let mut absolute = PathBuf::new();
    for component in path.components() {
        if component == Component::ParentDir {
            absolute.pop();
        } else {
            absolute.push(component);
        }
    }
    MOUNTS.with_borrow_mut(|cache| {
        if cache
            .as_ref()
            .is_none_or(|(checked, _)| checked.elapsed() >= Duration::from_secs(3))
        {
            *cache = Some((Instant::now(), remote_mounts()));
        }
        cache
            .as_ref()
            .and_then(|(_, mounts)| mounts.as_ref())
            .is_some_and(|mounts| !mounts.iter().any(|mount| absolute.starts_with(mount)))
    })
}

#[cfg(target_os = "linux")]
fn remote_mounts() -> Option<Vec<PathBuf>> {
    // mountinfo is kernel-generated and reading it does not traverse mounts.
    let mut text = String::new();
    fs::File::open("/proc/self/mountinfo")
        .ok()?
        .take(MAX_REF_BYTES + 1)
        .read_to_string(&mut text)
        .ok()?;
    if text.is_empty() || text.len() as u64 > MAX_REF_BYTES {
        return None;
    }
    let mut remote = Vec::new();
    for line in text.lines() {
        let (mount, filesystem) = line.split_once(" - ")?;
        let filesystem = filesystem.split_ascii_whitespace().next()?;
        if filesystem.starts_with("fuse")
            || matches!(
                filesystem,
                "9p" | "afs"
                    | "autofs"
                    | "ceph"
                    | "cifs"
                    | "davfs"
                    | "glusterfs"
                    | "lustre"
                    | "ncpfs"
                    | "nfs"
                    | "nfs4"
                    | "smb3"
                    | "smbfs"
                    | "virtiofs"
            )
        {
            remote.push(PathBuf::from(
                mount
                    .split_ascii_whitespace()
                    .nth(4)?
                    .replace("\\040", " ")
                    .replace("\\011", "\t")
                    .replace("\\012", "\n")
                    .replace("\\134", "\\"),
            ));
        }
    }
    Some(remote)
}

#[cfg(not(target_os = "linux"))]
fn local_mount(_: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_ignores_stale_results_and_keeps_multiple_current_keys() {
        let (requests, _work) = mpsc::sync_channel(3);
        let (responses, results) = mpsc::sync_channel(3);
        let mut cache = Cache {
            worker: Some(Worker { requests, results }),
            ..Cache::default()
        };
        let stale = Key::Paths(PathBuf::from("stale"));
        let current = Key::Paths(PathBuf::from("current"));
        let second = Key::GitCommands;

        assert!(cache.get(stale.clone()).is_empty());
        cache.reset_request();
        assert!(cache.get(current.clone()).is_empty());
        assert!(cache.get(second.clone()).is_empty());

        responses
            .try_send((stale, vec![Entry::named("wrong".into())]))
            .unwrap_or_else(|_| panic!("mock worker result queue is available"));
        assert!(!cache.metadata_ready());
        assert!(cache.metadata_pending());

        responses
            .try_send((current.clone(), vec![Entry::named("right".into())]))
            .unwrap_or_else(|_| panic!("mock worker result queue is available"));
        assert!(cache.metadata_ready());
        assert_eq!(cache.get(current)[0].name, "right");
        assert!(cache.metadata_pending());

        responses
            .try_send((second, Vec::new()))
            .unwrap_or_else(|_| panic!("mock worker result queue is available"));
        assert!(cache.metadata_ready());
        assert!(!cache.metadata_pending());
    }

    #[test]
    fn cache_keeps_requests_bounded_when_discovery_cannot_make_progress() {
        let (requests, work) = mpsc::sync_channel(2);
        let (responses, results) = mpsc::sync_channel(2);
        let mut cache = Cache {
            worker: Some(Worker { requests, results }),
            ..Cache::default()
        };

        for index in 0..MAX_WANTED + 3 {
            assert!(
                cache
                    .get(Key::Paths(PathBuf::from(index.to_string())))
                    .is_empty()
            );
        }
        assert_eq!(cache.wanted.len(), MAX_WANTED);
        assert_eq!(cache.pending.len(), 2);
        assert!(cache.waiting_for_slot);

        cache.reset_request();
        let current = Key::Paths(PathBuf::from("latest"));
        assert!(cache.get(current.clone()).is_empty());
        let stale = work.try_recv().unwrap();
        responses
            .try_send((stale, Vec::new()))
            .unwrap_or_else(|_| panic!("mock worker result queue is available"));
        assert!(!cache.metadata_ready());
        assert!(cache.metadata_pending());
        assert!(cache.pending.contains(&current));
    }

    #[test]
    fn repository_metadata_reads_linked_refs_and_config_without_exposing_values() {
        let fixture =
            std::env::temp_dir().join(format!("cnl-metadata-test-{}", std::process::id()));
        let main = fixture.join("main");
        let linked = fixture.join("other");
        let common = main.join(".git");
        let worktree = common.join("worktrees/other");
        fs::create_dir_all(&worktree).unwrap();
        fs::create_dir_all(common.join("refs/heads")).unwrap();
        fs::create_dir_all(common.join("logs/refs")).unwrap();
        fs::create_dir_all(&linked).unwrap();
        fs::write(
            linked.join(".git"),
            "gitdir: ../main/.git/worktrees/other\n",
        )
        .unwrap();
        fs::write(worktree.join("commondir"), "../..\n").unwrap();
        fs::write(
            worktree.join("gitdir"),
            linked.join(".git").to_str().unwrap(),
        )
        .unwrap();
        fs::write(common.join("refs/heads/local"), "abc\n").unwrap();
        fs::write(common.join("logs/refs/stash"), "one\ntwo\n").unwrap();
        fs::write(
            common.join("packed-refs"),
            "abc refs/remotes/origin/main\nabc refs/remotes/origin/HEAD\nabc refs/tags/v1.0\n",
        )
        .unwrap();
        fs::write(
            common.join("config"),
            "[remote \"origin\"]\nurl = https://user:secret@invalid.invalid\n[include]\npath = //invalid.invalid/config\n",
        ).unwrap();

        let names = |kind| {
            Key::GitRepository {
                cwd: linked.clone(),
                kind,
            }
            .scan()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>()
        };
        let branches = names(GitData::Branches);
        let refs = names(GitData::Refs);
        let tags = names(GitData::Tags);
        let remotes = names(GitData::Remotes);
        let stashes = names(GitData::Stashes);
        let worktrees = names(GitData::Worktrees);
        let config = names(GitData::Config);
        fs::remove_dir_all(&fixture).unwrap();

        assert_eq!(branches, ["local", "origin/main"]);
        assert!(refs.contains(&"refs/remotes/origin/main".into()));
        assert!(refs.contains(&"refs/tags/v1.0".into()));
        assert_eq!(tags, ["v1.0"]);
        assert_eq!(remotes, ["origin"]);
        assert_eq!(stashes, ["stash@{0}", "stash@{1}"]);
        assert!(worktrees.contains(&linked.to_string_lossy().into_owned()));
        assert_eq!(config, ["remote.origin.url"]);
    }
}
