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

const MAX_SCANNED: usize = 1024;
const MAX_CACHE: usize = 12;
const MAX_REF_BYTES: u64 = 512 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Key {
    Paths(PathBuf),
    Branches(PathBuf),
    Executables { path: OsString, cwd: PathBuf },
}

impl Key {
    fn lifetime(&self) -> Duration {
        match self {
            Self::Executables { .. } => Duration::from_secs(30),
            Self::Paths(_) | Self::Branches(_) => Duration::from_secs(3),
        }
    }

    fn scan(&self) -> Vec<Entry> {
        match self {
            Self::Paths(path) => paths(path),
            Self::Branches(cwd) => branches(cwd).into_iter().map(Entry::named).collect(),
            Self::Executables { path, cwd } => executables(path, cwd),
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
        let (requests, work) = mpsc::sync_channel::<Key>(1);
        let (responses, results) = mpsc::channel();
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
    wanted: Option<Key>,
    waiting_for_slot: bool,
}

impl Cache {
    pub(super) fn reset_request(&mut self) {
        self.wanted = None;
        self.waiting_for_slot = false;
    }

    pub(super) fn metadata_ready(&mut self) -> bool {
        let mut ready = false;
        if let Some(worker) = &self.worker {
            while let Ok((key, entries)) = worker.results.try_recv() {
                self.pending.retain(|request| request != &key);
                ready |= self.wanted.as_ref() == Some(&key);
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
        self.waiting_for_slot
            || self
                .wanted
                .as_ref()
                .is_some_and(|key| self.pending.contains(key))
    }

    pub(super) fn get(&mut self, key: Key) -> Rc<[Entry]> {
        self.wanted = Some(key.clone());
        self.metadata_ready();
        self.schedule_wanted();
        self.entries
            .iter()
            .find(|entry| entry.key == key)
            .map_or_else(Rc::default, |entry| Rc::clone(&entry.entries))
    }

    fn schedule_wanted(&mut self) {
        self.waiting_for_slot = false;
        let Some(key) = self.wanted.as_ref() else {
            return;
        };
        if self.pending.contains(key)
            || self
                .entries
                .iter()
                .any(|entry| &entry.key == key && entry.scanned.elapsed() < key.lifetime())
        {
            return;
        }
        if self.worker.is_none() {
            self.worker = Worker::start();
        }
        if let Some(worker) = &self.worker {
            match worker.requests.try_send(key.clone()) {
                Ok(()) => self.pending.push(key.clone()),
                Err(mpsc::TrySendError::Full(_)) => self.waiting_for_slot = true,
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.worker = None;
                    self.pending.clear();
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

fn branches(cwd: &Path) -> BTreeSet<String> {
    let Some(git) = cwd
        .ancestors()
        .take(32)
        .map(|path| path.join(".git"))
        .find(|path| local_metadata(path).is_some())
    else {
        return BTreeSet::new();
    };
    let git = if let Some(marker) = bounded_text(&git, 4096) {
        let Some(destination) = marker.trim().strip_prefix("gitdir: ") else {
            return BTreeSet::new();
        };
        git.parent().unwrap_or(cwd).join(destination)
    } else {
        git
    };
    let common_dir = bounded_text(&git.join("commondir"), 4096)
        .filter(|text| !text.trim().is_empty())
        .map_or_else(|| git.clone(), |text| git.join(text.trim()));
    let mut branches = BTreeSet::new();
    if let Some(packed) = bounded_text(&common_dir.join("packed-refs"), MAX_REF_BYTES) {
        for name in packed.lines().filter_map(|line| {
            let (_, reference) = line.split_once(' ')?;
            reference.strip_prefix("refs/heads/")
        }) {
            if safe_ref(name) {
                branches.insert(name.to_owned());
            }
            if branches.len() == MAX_SCANNED {
                return branches;
            }
        }
    }
    let mut dirs = vec![(common_dir.join("refs/heads"), String::new(), 0)];
    let mut scanned = 0;
    while let Some((path, prefix, depth)) = dirs.pop() {
        for entry in directory(&path)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
        {
            if scanned == MAX_SCANNED || branches.len() == MAX_SCANNED {
                return branches;
            }
            scanned += 1;
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
                branches.insert(name);
            }
        }
    }
    branches
}

pub(super) fn safe_text(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(char::is_control)
}

fn safe_ref(value: &str) -> bool {
    safe_text(value)
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
