use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    fmt, fs, io,
    path::{Component, Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{LazyLock, Mutex},
};

use anyhow::{Context, Result, ensure};
use serde::Serialize;

use crate::{
    localization::Diagnostic,
    process::{isolate_git, sanitize_console_output},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GitStatus {
    Clean,
    Modified,
    Untracked,
    Conflicted,
    Detached,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetails {
    pub short_revision: String,
    pub author: String,
    pub timestamp: String,
    pub message: String,
}

impl fmt::Display for GitStatus {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(match self {
            Self::Clean => "clean",
            Self::Modified => "modified",
            Self::Untracked => "untracked",
            Self::Conflicted => "conflicted",
            Self::Detached => "detached",
        })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkout {
    pub path: PathBuf,
    pub common_dir: PathBuf,
    pub remote: String,
    pub remote_url: Option<String>,
    pub slug: Option<String>,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
    pub status: GitStatus,
    pub commit: Option<CommitDetails>,
    pub modified: bool,
    pub untracked: bool,
    pub conflicted: bool,
    #[serde(skip)]
    git_dir: PathBuf,
    #[serde(skip)]
    pub(super) head: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Target {
    pub path: PathBuf,
    pub remote: String,
    pub slug: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub enum ReadAction {
    Diff,
    StagedDiff,
    Log,
    Branches,
    Worktrees,
    Remotes,
}

#[derive(Clone, Debug)]
pub enum WriteAction {
    CreateBranch(String),
    SwitchBranch(String),
    AddWorktree { path: PathBuf, branch: String },
    Stage(Vec<PathBuf>),
    Commit(String),
    Push,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncAction {
    Fetch,
    Update,
}

fn diagnostic(en: &str, vi: &str) -> anyhow::Error {
    anyhow::Error::msg(Diagnostic::new(|vietnamese| {
        if vietnamese { vi } else { en }.to_owned()
    }))
}

fn git(path: &Path) -> Command {
    let mut command = Command::new("git");
    isolate_git(&mut command);
    command
        .args([
            "--no-optional-locks",
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "-c",
            "core.longpaths=true",
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
            "-c",
            "protocol.ssh.allow=always",
            "-c",
            "protocol.file.allow=always",
            "-C",
        ])
        .arg(git_path(path).as_ref())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env_remove("CNL_TUI_REPORT")
        .stdin(Stdio::null());
    crate::process::hidden(&mut command);
    command
}

fn git_path(path: &Path) -> Cow<'_, Path> {
    // Git for Windows accepts normal DOS/UNC syntax, which differs from the
    // verbatim path returned by fs::canonicalize on this platform.
    #[cfg(windows)]
    if let Some(text) = path.to_str() {
        let visible = crate::workspace::user_path(path);
        if visible != text {
            return Cow::Owned(PathBuf::from(visible.as_ref()));
        }
    }
    Cow::Borrowed(path)
}

fn capture(command: &mut Command) -> Result<Output> {
    command.output().context(diagnostic(
        "Could not start Git. Check that Git is installed and available on PATH.",
        "Không chạy được Git. Hãy kiểm tra Git đã được cài đặt và có trong PATH.",
    ))
}

#[derive(Debug)]
struct InaccessibleGit;

impl fmt::Display for InaccessibleGit {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(crate::localization::message([
            "Git could not access the checkout",
            "Git không thể truy cập checkout",
        ]))
    }
}

impl std::error::Error for InaccessibleGit {}

pub(super) fn failure_state(path: &Path, error: &anyhow::Error) -> &'static str {
    match fs::metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => "missing",
        Err(_) => "inaccessible",
        Ok(_) if error.downcast_ref::<InaccessibleGit>().is_some() => "inaccessible",
        Ok(_)
            if error.chain().any(|cause| {
                cause.downcast_ref::<io::Error>().is_some_and(|e| {
                    matches!(
                        e.kind(),
                        io::ErrorKind::PermissionDenied | io::ErrorKind::NotFound
                    )
                })
            }) =>
        {
            "inaccessible"
        }
        Ok(_) => "invalid",
    }
}

fn checked(result: Output) -> Result<Vec<u8>> {
    if !result.status.success() {
        let detail = sanitize_console_output(&result.stderr);
        let error = anyhow::Error::msg(Diagnostic::new(|vi| {
            let operation = if vi {
                "Lệnh Git thất bại"
            } else {
                "Git command failed"
            };
            format!("{operation} ({}) {}", result.status, detail.trim())
        }));
        let text = detail.to_ascii_lowercase();
        return Err(
            if [
                "dubious ownership",
                "permission denied",
                "access is denied",
                "operation not permitted",
            ]
            .iter()
            .any(|message| text.contains(message))
            {
                error.context(InaccessibleGit)
            } else {
                error
            },
        );
    }
    Ok(result.stdout)
}

fn perform(command: &mut Command) -> Result<String> {
    Ok(sanitize_console_output(&checked(capture(command)?)?)
        .trim_end()
        .to_owned())
}

fn raw_text(command: &mut Command) -> Result<String> {
    Ok(String::from_utf8(checked(capture(command)?)?)?
        .trim_end_matches(['\n', '\r'])
        .to_owned())
}

fn optional(command: &mut Command, missing_code: i32) -> Result<Option<String>> {
    let output = capture(command)?;
    if output.status.code() == Some(missing_code) {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8(checked(output)?)?
            .trim_end_matches(['\n', '\r'])
            .to_owned(),
    ))
}

fn worktree_root(path: &Path) -> Result<PathBuf> {
    let path = fs::canonicalize(path).with_context(|| {
        diagnostic(
            "The checkout path is missing or inaccessible.",
            "Đường dẫn checkout bị thiếu hoặc không thể truy cập.",
        )
    })?;
    let directory =
        raw_text(git(&path).args(["rev-parse", "--path-format=absolute", "--show-toplevel"]))?;
    ensure!(
        fs::canonicalize(directory)? == path,
        diagnostic(
            "Use the root of an independent Git worktree.",
            "Hãy dùng thư mục gốc của một cây làm việc Git độc lập.",
        ),
    );
    Ok(path)
}

fn locations(path: &Path) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let path = fs::canonicalize(path)?;
    let paths = raw_text(git(&path).args([
        "rev-parse",
        "--path-format=absolute",
        "--show-toplevel",
        "--absolute-git-dir",
        "--git-common-dir",
    ]))?;
    let lines: Vec<_> = paths.lines().collect();
    if let [root, git_dir, common_dir] = lines.as_slice() {
        ensure!(
            fs::canonicalize(root)? == path,
            diagnostic(
                "Use the root of an independent Git worktree.",
                "Hãy dùng thư mục gốc của một cây làm việc Git độc lập.",
            )
        );
        return Ok((
            path,
            fs::canonicalize(git_dir)?,
            fs::canonicalize(common_dir)?,
        ));
    }
    // Paths with embedded newlines require individual Git queries.
    let path = worktree_root(&path)?;
    let git_dir = raw_text(git(&path).args(["rev-parse", "--absolute-git-dir"]))?;
    let common_dir =
        raw_text(git(&path).args(["rev-parse", "--path-format=absolute", "--git-common-dir"]))?;
    Ok((
        path,
        fs::canonicalize(git_dir)?,
        fs::canonicalize(common_dir)?,
    ))
}

fn valid_remote(remote: &str) -> bool {
    !remote.starts_with('-')
        && !remote.contains("..")
        && remote.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && !part.ends_with('.')
                && !part.ends_with(".lock")
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}

fn remote_urls(path: &Path, remote: &str, push: bool) -> Result<Vec<String>> {
    ensure!(
        valid_remote(remote),
        diagnostic("Invalid Git remote name.", "Tên remote Git không hợp lệ."),
    );
    let mut command = git(path);
    command.args(["remote", "get-url", "--all"]);
    if push {
        command.arg("--push");
    }
    Ok(optional(command.args(["--", remote]), 2)?
        .map(|output| output.lines().map(str::to_owned).collect())
        .unwrap_or_default())
}

fn sanitized_url(url: &str) -> Option<String> {
    let url = url.split(['?', '#']).next()?;
    if url.is_empty() || url.chars().any(|ch| ch.is_control()) {
        return None;
    }
    if let Some((scheme, rest)) = url.split_once("://") {
        if !matches!(scheme, "https" | "http" | "ssh" | "git" | "file") {
            return None;
        }
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?;
        let user = if scheme == "ssh" && authority.starts_with("git@") {
            "git@"
        } else {
            ""
        };
        return Some(format!("{scheme}://{user}{host}/{path}"));
    }
    if Path::new(url).is_absolute() {
        return Some(url.to_owned());
    }
    let Some((host, path)) = url.split_once(':') else {
        return Some(url.to_owned());
    };
    let host_name = host.rsplit('@').next()?;
    let user = if host.starts_with("git@") { "git@" } else { "" };
    if host_name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ".-_".contains(ch))
        && !host_name.is_empty()
        && !path.starts_with(':')
    {
        return Some(format!("{user}{host_name}:{path}"));
    }
    None
}

pub fn github_slug(url: &str) -> Option<String> {
    let url = sanitized_url(url)?;
    let (host, path) = if let Some((scheme, rest)) = url.split_once("://") {
        if !matches!(scheme, "https" | "ssh") {
            return None;
        }
        rest.split_once('/')?
    } else {
        url.split_once(':')?
    };
    let host = host.strip_prefix("git@").unwrap_or(host);
    if !host.eq_ignore_ascii_case("github.com") {
        return None;
    }
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    let valid = |part: &str| {
        !matches!(part, "" | "." | "..")
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    (valid(owner) && valid(name)).then(|| path.to_ascii_lowercase())
}

pub fn inspect(path: &Path, remote: &str) -> Result<Checkout> {
    inspect_checkout(path, Some(remote))
}

pub(super) fn inspect_discovered(path: &Path) -> Result<Checkout> {
    inspect_checkout(path, None)
}

fn inspect_checkout(path: &Path, remote: Option<&str>) -> Result<Checkout> {
    let (path, git_dir, common_dir) = locations(path)?;
    let mut checkout = Checkout {
        path,
        common_dir,
        remote: remote.unwrap_or("origin").to_owned(),
        remote_url: None,
        slug: None,
        branch: None,
        upstream: None,
        ahead: None,
        behind: None,
        status: GitStatus::Clean,
        commit: None,
        modified: false,
        untracked: false,
        conflicted: false,
        git_dir,
        head: None,
    };
    let records = checked(capture(git(&checkout.path).args([
        "status",
        "--porcelain=v2",
        "--branch",
        "--ahead-behind",
        "-z",
        "--untracked-files=normal",
        "--ignore-submodules=none",
    ]))?)?;
    parse_status(&mut checkout, &records)?;
    let mut url = remote_urls(&checkout.path, &checkout.remote, false)?
        .into_iter()
        .next();
    if url.is_none() && remote.is_none() {
        let names = raw_text(git(&checkout.path).arg("remote"))?;
        let mut names = names.lines().filter(|name| valid_remote(name));
        let first = names.next();
        let tracked = checkout.upstream.as_deref().and_then(|upstream| {
            first
                .into_iter()
                .chain(names.clone())
                .filter(|name| {
                    upstream
                        .strip_prefix(name)
                        .is_some_and(|rest| rest.starts_with('/'))
                })
                .max_by_key(|name| name.len())
        });
        if let Some(name) = tracked.or_else(|| first.filter(|_| names.next().is_none())) {
            checkout.remote = name.to_owned();
            url = remote_urls(&checkout.path, name, false)?.into_iter().next();
        }
    }
    checkout.remote_url = url.as_deref().and_then(sanitized_url);
    checkout.slug = url.as_deref().and_then(github_slug);
    checkout.commit = checkout
        .head
        .as_deref()
        .and_then(|head| commit_details(&checkout, head).ok());
    Ok(checkout)
}

const COMMIT_CACHE_LIMIT: usize = 1024;

#[derive(Default)]
struct CommitCache {
    entries: HashMap<(PathBuf, String), CommitDetails>,
    order: VecDeque<(PathBuf, String)>,
}

// Commit objects are immutable. Reuse their details across refreshes while
// running status and reading the configured remote again for each worktree.
static COMMITS: LazyLock<Mutex<CommitCache>> = LazyLock::new(Mutex::default);

fn commit_details(checkout: &Checkout, revision: &str) -> Result<CommitDetails> {
    let key = (checkout.common_dir.clone(), revision.to_owned());
    if let Some(commit) = COMMITS.lock().expect("commit cache lock").entries.get(&key) {
        return Ok(commit.clone());
    }
    let raw = checked(capture(
        git(&checkout.path)
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .args([
                "log",
                "-1",
                "--abbrev=12",
                "--no-show-signature",
                "--encoding=UTF-8",
                "--format=%h%x00%an%x00%aI%x00%B",
                revision,
                "--",
            ]),
    )?)?;
    let mut fields = raw.splitn(4, |&byte| byte == 0);
    let [Some(short), Some(author), Some(timestamp), Some(message)] =
        std::array::from_fn(|_| fields.next())
    else {
        anyhow::bail!("Unexpected Git commit metadata");
    };
    let commit = CommitDetails {
        short_revision: sanitize_console_output(short),
        author: sanitize_console_output(author),
        timestamp: sanitize_console_output(timestamp),
        message: sanitize_console_output(message)
            .trim_end_matches(['\r', '\n'])
            .to_owned(),
    };
    if raw.len() <= 16 * 1024 {
        let mut cache = COMMITS.lock().expect("commit cache lock");
        if !cache.entries.contains_key(&key) {
            if cache.entries.len() >= COMMIT_CACHE_LIMIT
                && let Some(oldest) = cache.order.pop_front()
            {
                cache.entries.remove(&oldest);
            }
            cache.entries.insert(key.clone(), commit.clone());
            cache.order.push_back(key);
        }
    }
    Ok(commit)
}

fn parse_status(checkout: &mut Checkout, records: &[u8]) -> Result<()> {
    let mut records = records.split(|&byte| byte == 0);
    while let Some(record) = records.next() {
        match record.first() {
            Some(b'#') => {
                let text = std::str::from_utf8(record)?;
                if let Some(head) = text.strip_prefix("# branch.oid ") {
                    checkout.head = (head != "(initial)").then(|| head.to_owned());
                } else if let Some(branch) = text.strip_prefix("# branch.head ") {
                    checkout.branch = (branch != "(detached)").then(|| branch.to_owned());
                } else if let Some(upstream) = text.strip_prefix("# branch.upstream ") {
                    checkout.upstream = Some(upstream.to_owned());
                } else if let Some(counts) = text.strip_prefix("# branch.ab ")
                    && let Some((ahead, behind)) = counts.split_once(' ')
                {
                    checkout.ahead = ahead.strip_prefix('+').and_then(|value| value.parse().ok());
                    checkout.behind = behind
                        .strip_prefix('-')
                        .and_then(|value| value.parse().ok());
                }
            }
            Some(b'1') => checkout.modified = true,
            Some(b'2') => {
                checkout.modified = true;
                records.next(); // A rename has a separate, NUL-delimited source pathname.
            }
            Some(b'u') => checkout.conflicted = true,
            Some(b'?') => checkout.untracked = true,
            _ => {}
        }
    }
    checkout.status = if checkout.conflicted {
        GitStatus::Conflicted
    } else if checkout.branch.is_none() {
        GitStatus::Detached
    } else if checkout.modified {
        GitStatus::Modified
    } else if checkout.untracked {
        GitStatus::Untracked
    } else {
        GitStatus::Clean
    };
    Ok(())
}

const MAX_DISCOVERY_DEPTH: usize = 12;

pub fn discover(root: &Path, depth: usize) -> Result<Vec<PathBuf>> {
    Ok(discovery_candidates(root, depth)?
        .into_iter()
        .filter(|path| worktree_root(path).is_ok())
        .collect())
}

pub(super) fn discovery_candidates(root: &Path, depth: usize) -> Result<Vec<PathBuf>> {
    ensure!(
        depth <= MAX_DISCOVERY_DEPTH,
        diagnostic(
            "Local discovery depth must be between 0 and 12.",
            "Độ sâu tìm checkout phải từ 0 đến 12.",
        )
    );
    ensure!(
        fs::symlink_metadata(root).is_ok_and(|metadata| metadata.is_dir() && !is_link(&metadata)),
        diagnostic(
            "Choose a directory to scan for Git worktrees.",
            "Hãy chọn một thư mục để tìm cây làm việc Git.",
        )
    );
    let root = fs::canonicalize(root)?;
    let mut found = Vec::new();
    let mut remaining = vec![(root.clone(), 0)];
    while let Some((path, level)) = remaining.pop() {
        if !fs::symlink_metadata(&path)
            .is_ok_and(|metadata| metadata.is_dir() && !is_link(&metadata))
        {
            continue;
        }
        let Ok(path) = fs::canonicalize(path) else {
            continue;
        };
        if !path.starts_with(&root) {
            continue;
        }
        if fs::symlink_metadata(path.join(".git"))
            .is_ok_and(|metadata| !is_link(&metadata) && (metadata.is_file() || metadata.is_dir()))
        {
            found.push(path.clone());
        }
        if level >= depth {
            continue;
        }
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if level == 0 => return Err(error.into()),
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if matches!(
                name.as_str(),
                ".git"
                    | ".cirnet-build-output"
                    | ".cache"
                    | ".venv"
                    | "__pycache__"
                    | "target"
                    | "node_modules"
                    | "build"
                    | "dist"
                    | "venv"
            ) {
                continue;
            }
            remaining.push((entry.path(), level + 1));
        }
    }
    found.sort_unstable();
    found.dedup();
    Ok(found)
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    metadata.file_type().is_symlink()
}

fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn missing_destination(destination: &Path) -> Result<PathBuf> {
    let destination = std::path::absolute(destination)?;
    let (Some(parent), Some(name)) = (destination.parent(), destination.file_name()) else {
        return Err(diagnostic(
            "Choose a new directory with an existing parent.",
            "Hãy chọn một thư mục mới trong thư mục cha đã tồn tại.",
        ));
    };
    let name_text = name.to_string_lossy();
    ensure!(
        !name_text.eq_ignore_ascii_case(".git")
            && !name_text.contains(':')
            && !name_text.ends_with(['.', ' ']),
        diagnostic(
            "Invalid destination directory.",
            "Thư mục đích không hợp lệ."
        ),
    );
    let destination = fs::canonicalize(parent)?.join(name);
    ensure!(
        !exists(&destination)?,
        diagnostic(
            "The destination already exists.",
            "Đường dẫn đích đã tồn tại.",
        )
    );
    Ok(destination)
}

fn validate_remote_url(path: &Path, url: &str, expected_slug: Option<&str>) -> Result<()> {
    let accepted = if let Some(slug) = expected_slug {
        github_slug(url).is_some_and(|actual| actual.eq_ignore_ascii_case(slug))
            && sanitized_url(url).as_deref() == Some(url)
            && (url.starts_with("https://")
                || url.starts_with("ssh://git@")
                || url.starts_with("git@"))
    } else if url.starts_with("https://") {
        sanitized_url(url).as_deref() == Some(url)
    } else if !url.contains("://") && !url.starts_with('-') {
        let remote_path = path.join(url);
        remote_path.is_dir() && fs::canonicalize(remote_path).is_ok()
    } else {
        false
    };
    ensure!(
        accepted,
        diagnostic(
            "The effective Git remote does not match the expected identity or uses an unsupported URL.",
            "Remote Git thực tế sai định danh dự kiến hoặc dùng URL không được hỗ trợ.",
        )
    );
    Ok(())
}

pub fn clone_repository(destination: &Path, url: &str, expected_slug: &str) -> Result<Checkout> {
    let destination = missing_destination(destination)?;
    let parent = destination.parent().expect("validated destination parent");
    validate_remote_url(parent, url, Some(expected_slug))?;
    let rewritten = raw_text(git(parent).args(["ls-remote", "--get-url", "--", url]))?;
    validate_remote_url(parent, &rewritten, Some(expected_slug))?;
    clone_at(&destination, url, |checkout| {
        ensure!(
            checkout
                .slug
                .as_deref()
                .is_some_and(|slug| slug.eq_ignore_ascii_case(expected_slug)),
            diagnostic(
                "The cloned checkout does not have the expected GitHub identity.",
                "Checkout vừa clone sai định danh GitHub dự kiến.",
            )
        );
        Ok(())
    })
}

fn clone_at(
    destination: &Path,
    url: &str,
    verify: impl FnOnce(&Checkout) -> Result<()>,
) -> Result<Checkout> {
    let parent = destination.parent().expect("validated destination parent");
    reserve_destination(destination, || {
        perform(
            git(parent)
                .args([
                    "clone",
                    "--no-recurse-submodules",
                    "--origin",
                    "origin",
                    "--",
                    url,
                ])
                .arg(git_path(destination).as_ref()),
        )?;
        let checkout = inspect(destination, "origin")?;
        verify(&checkout)?;
        Ok(checkout)
    })
}

fn reserve_destination<T>(destination: &Path, action: impl FnOnce() -> Result<T>) -> Result<T> {
    fs::create_dir(destination)?;
    let result = action();
    if result.is_err() {
        // Remove only an empty reservation created by this operation.
        let _ = fs::remove_dir(destination);
    }
    result
}

pub fn read(path: &Path, remote: &str, action: ReadAction) -> Result<String> {
    let (path, _, _) = locations(path)?;
    let mut command = git(&path);
    let args: &[&str] = match action {
        ReadAction::Diff => &["diff", "--no-color", "--no-ext-diff", "--no-textconv"],
        ReadAction::StagedDiff => &[
            "diff",
            "--cached",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
        ],
        ReadAction::Log => &[
            "log",
            "--no-show-signature",
            "--oneline",
            "--decorate=short",
            "-50",
        ],
        ReadAction::Branches => &[
            "for-each-ref",
            "--format=%(refname:short)%09%(upstream:short)",
            "refs/heads",
        ],
        ReadAction::Worktrees => &["worktree", "list", "--porcelain"],
        ReadAction::Remotes => {
            ensure!(
                valid_remote(remote),
                diagnostic("Invalid Git remote name.", "Tên remote Git không hợp lệ.")
            );
            let mut lines = Vec::new();
            for name in raw_text(command.arg("remote"))?.lines() {
                for (push, direction) in [(false, "fetch"), (true, "push")] {
                    for url in remote_urls(&path, name, push)? {
                        if let Some(url) = sanitized_url(&url) {
                            lines.push(format!("{name}\t{url} ({direction})"));
                        }
                    }
                }
            }
            return Ok(sanitize_console_output(lines.join("\n").as_bytes()));
        }
    };
    perform(command.args(args))
}

fn verify_target(target: &Target) -> Result<Checkout> {
    let checkout = inspect(&target.path, &target.remote)?;
    if let Some(expected) = &target.slug {
        ensure!(
            checkout
                .slug
                .as_deref()
                .is_some_and(|slug| slug.eq_ignore_ascii_case(expected)),
            diagnostic(
                "The checkout remote does not match the registered repository identity.",
                "Remote của checkout sai định danh kho mã đã đăng ký.",
            )
        );
    }
    Ok(checkout)
}

fn lock(common_dir: &Path) -> Result<fs::File> {
    let path = common_dir.join("cnl-repository-operation");
    ensure!(
        !fs::symlink_metadata(&path).is_ok_and(|metadata| is_link(&metadata)),
        diagnostic(
            "Unsafe Git operation lock path.",
            "Đường dẫn khóa Git không an toàn."
        ),
    );
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock().context(diagnostic(
        "Another operation holds the lock for this Git repository and its worktrees.",
        "Một thao tác khác đang giữ khóa của kho mã Git và các cây làm việc liên quan.",
    ))?;
    Ok(file)
}

fn safe_state(checkout: &Checkout) -> Result<()> {
    let underway = [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
        "BISECT_START",
    ];
    for marker in underway {
        if exists(&checkout.git_dir.join(marker))? {
            return Err(diagnostic(
                "Complete the Git operation already in progress before using the manager.",
                "Hãy hoàn tất thao tác Git đang diễn ra trước khi dùng trình quản lý.",
            ));
        }
    }
    for (path, marker) in [
        (&checkout.git_dir, "index.lock"),
        (&checkout.git_dir, "HEAD.lock"),
        (&checkout.common_dir, "config.lock"),
        (&checkout.common_dir, "packed-refs.lock"),
    ] {
        ensure!(
            !exists(&path.join(marker))?,
            diagnostic(
                "Git holds a lock for this checkout.",
                "Git đang giữ khóa cho checkout này.",
            )
        );
    }
    ensure!(
        !checkout.conflicted,
        diagnostic(
            "Resolve conflicts in Git before using the manager.",
            "Hãy xử lý xung đột trong Git trước khi dùng trình quản lý.",
        )
    );
    Ok(())
}

fn ensure_clean(checkout: &Checkout) -> Result<()> {
    ensure!(
        !checkout.modified && !checkout.untracked,
        diagnostic(
            "The working tree or index has local changes.",
            "Cây làm việc hoặc chỉ mục đang có thay đổi cục bộ.",
        )
    );
    Ok(())
}

fn branch(checkout: &Checkout) -> Result<&str> {
    checkout.branch.as_deref().ok_or_else(|| {
        diagnostic(
            "Check out a branch before running this operation.",
            "Hãy checkout một nhánh trước khi chạy thao tác này.",
        )
    })
}

fn configured_upstream(checkout: &Checkout, branch: &str) -> Result<String> {
    let key = format!("branch.{branch}.remote");
    let remote = optional(git(&checkout.path).args(["config", "--get", &key]), 1)?;
    let key = format!("branch.{branch}.merge");
    let merge = optional(git(&checkout.path).args(["config", "--get-all", &key]), 1)?;
    ensure!(
        remote.as_deref() == Some(&checkout.remote)
            && merge.as_deref().is_some_and(
                |reference| reference.starts_with("refs/heads/") && !reference.contains('\n')
            ),
        diagnostic(
            "The current branch must track a branch on the selected remote.",
            "Nhánh hiện tại phải theo dõi một nhánh trên remote đã chọn.",
        ),
    );
    let merge = merge.expect("validated merge ref");
    checked(capture(
        git(&checkout.path).args(["check-ref-format", &merge]),
    )?)?;
    Ok(merge)
}

fn checked_remote(target: &Target, checkout: &Checkout, push: bool) -> Result<String> {
    let urls = remote_urls(&checkout.path, &target.remote, push)?;
    ensure!(
        urls.len() == 1,
        diagnostic(
            "Configure exactly one URL for the selected Git remote and operation.",
            "Hãy cấu hình đúng một URL cho remote Git và thao tác đã chọn.",
        )
    );
    let url = urls.into_iter().next().expect("one Git remote");
    validate_remote_url(&checkout.path, &url, target.slug.as_deref())?;
    Ok(url)
}

fn check_fetch_refspec(checkout: &Checkout) -> Result<()> {
    let key = format!("remote.{}.fetch", checkout.remote);
    let prefix = format!("refs/remotes/{}/", checkout.remote);
    if let Some(refspecs) = optional(git(&checkout.path).args(["config", "--get-all", &key]), 1)? {
        for spec in refspecs.lines() {
            let spec = spec.strip_prefix('+').unwrap_or(spec);
            let valid = if let Some((source, destination)) = spec.split_once(':') {
                source.starts_with("refs/heads/") && destination.starts_with(&prefix)
            } else {
                spec.starts_with("^refs/heads/") || spec.starts_with("refs/heads/")
            };
            ensure!(
                valid,
                diagnostic(
                    "The selected remote has a fetch refspec outside its remote-tracking branches.",
                    "Remote đã chọn có refspec fetch nằm ngoài các nhánh theo dõi của remote.",
                )
            );
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(super) struct Preflight {
    pub(super) checkout: Checkout,
    url: String,
    upstream: Option<String>,
}

pub(super) fn preflight(target: &Target, action: SyncAction) -> Result<Preflight> {
    let checkout = verify_target(target)?;
    safe_state(&checkout)?;
    let url = checked_remote(target, &checkout, false)?;
    check_fetch_refspec(&checkout)?;
    let upstream = if action == SyncAction::Update {
        ensure_clean(&checkout)?;
        let branch = branch(&checkout)?;
        ensure!(
            checkout.head.is_some(),
            diagnostic(
                "The branch has no local commit yet.",
                "Nhánh chưa có commit cục bộ.",
            )
        );
        Some(configured_upstream(&checkout, branch)?)
    } else {
        None
    };
    Ok(Preflight {
        checkout,
        url,
        upstream,
    })
}

fn same_plan(expected: &Preflight, actual: &Preflight) -> Result<()> {
    ensure!(
        expected.checkout.path == actual.checkout.path
            && expected.checkout.common_dir == actual.checkout.common_dir
            && expected.url == actual.url
            && expected.upstream == actual.upstream
            && (expected.upstream.is_none()
                || (expected.checkout.branch == actual.checkout.branch
                    && expected.checkout.head == actual.checkout.head)),
        diagnostic(
            "The checkout or its tracking configuration changed after preflight. Run the operation again.",
            "Checkout hoặc cấu hình theo dõi đã thay đổi sau kiểm tra. Hãy chạy lại thao tác.",
        ),
    );
    Ok(())
}

pub(super) fn apply(
    target: &Target,
    action: SyncAction,
    expected: &Preflight,
    cancelled: impl Fn() -> bool,
) -> Result<Option<String>> {
    let _lock = lock(&expected.checkout.common_dir)?;
    if cancelled() {
        return Ok(None);
    }
    let actual = preflight(target, action)?;
    same_plan(expected, &actual)?;
    if cancelled() {
        return Ok(None);
    }
    let path = &actual.checkout.path;
    let mut fetch = git(path);
    fetch.args([
        "fetch",
        "--no-recurse-submodules",
        "--no-auto-maintenance",
        "--no-prune",
        "--no-prune-tags",
        "--no-tags",
        "--",
        &target.remote,
    ]);
    if let Some(upstream) = &actual.upstream {
        fetch.arg(upstream);
    }
    perform(&mut fetch)?;
    if action == SyncAction::Fetch {
        return Ok(Some(String::new()));
    }
    if cancelled() {
        return Ok(None);
    }
    let after_fetch = preflight(target, action)?;
    same_plan(expected, &after_fetch)?;
    let fetched = raw_text(git(path).args(["rev-parse", "--verify", "FETCH_HEAD^{commit}"]))?;
    if cancelled() {
        return Ok(None);
    }
    Ok(Some(perform(git(path).args([
        "merge",
        "--ff-only",
        "--no-autostash",
        "--no-edit",
        "--",
        &fetched,
    ]))?))
}

fn validate_branch(path: &Path, name: &str) -> Result<()> {
    ensure!(
        !name.starts_with('-') && !matches!(name, "HEAD" | "@") && !name.contains("@{"),
        diagnostic("Invalid Git branch name.", "Tên nhánh Git không hợp lệ."),
    );
    checked(capture(git(path).args([
        "check-ref-format",
        "--branch",
        name,
    ]))?)?;
    Ok(())
}

pub fn write(target: &Target, action: WriteAction) -> Result<String> {
    let before = verify_target(target)?;
    safe_state(&before)?;
    let _lock = lock(&before.common_dir)?;
    let checkout = verify_target(target)?;
    ensure!(
        checkout.path == before.path && checkout.common_dir == before.common_dir,
        diagnostic(
            "The checkout or its common Git directory changed.",
            "Checkout hoặc thư mục Git dùng chung đã thay đổi.",
        ),
    );
    safe_state(&checkout)?;
    let path = &checkout.path;
    let mut command = git(path);
    match action {
        WriteAction::CreateBranch(name) => {
            validate_branch(path, &name)?;
            command.args(["branch", "--", &name]);
        }
        WriteAction::SwitchBranch(name) => {
            ensure_clean(&checkout)?;
            validate_branch(path, &name)?;
            command.args(["switch", "--no-guess", "--", &name]);
        }
        WriteAction::AddWorktree {
            path: directory,
            branch,
        } => {
            validate_branch(path, &branch)?;
            checked(capture(git(path).args([
                "show-ref",
                "--verify",
                "--",
                &format!("refs/heads/{branch}"),
            ]))?)?;
            let directory = missing_destination(&directory)?;
            return reserve_destination(&directory, || {
                perform(
                    command
                        .args(["worktree", "add", "--"])
                        .arg(git_path(&directory).as_ref())
                        .arg(branch),
                )
            });
        }
        WriteAction::Stage(paths) => {
            ensure!(
                !paths.is_empty()
                    && paths.iter().all(|path| {
                        !path.as_os_str().is_empty()
                            && path.components().all(|part| {
                                matches!(part, Component::Normal(_) | Component::CurDir)
                            })
                    }),
                diagnostic(
                    "Specify one or more paths inside the worktree to stage.",
                    "Hãy chỉ rõ một hoặc nhiều đường dẫn trong cây làm việc để stage.",
                )
            );
            command
                .args(["--literal-pathspecs", "add", "--"])
                .args(paths);
        }
        WriteAction::Commit(message) => {
            branch(&checkout)?;
            ensure!(
                !message.trim().is_empty(),
                diagnostic(
                    "Enter an explicit commit message.",
                    "Hãy nhập nội dung commit cụ thể.",
                )
            );
            command.args(["commit", "-m", &message]);
        }
        WriteAction::Push => {
            let branch = branch(&checkout)?;
            let upstream = configured_upstream(&checkout, branch)?;
            let push_url = checked_remote(target, &checkout, true)?;
            if target.slug.is_none() {
                let fetch_url = checked_remote(target, &checkout, false)?;
                ensure!(
                    fetch_url == push_url,
                    diagnostic(
                        "The push URL differs from the known fetch URL.",
                        "URL push khác với URL fetch đã biết.",
                    )
                );
            }
            command.args([
                "-c",
                "push.followTags=false",
                "-c",
                "push.recurseSubmodules=no",
                "-c",
                &format!("remote.{}.mirror=false", target.remote),
                "push",
                "--porcelain",
                "--no-force",
                "--",
                &target.remote,
                &format!("refs/heads/{branch}:{upstream}"),
            ]);
        }
    }
    perform(&mut command)
}

#[cfg(test)]
#[path = "git_tests.rs"]
pub(super) mod tests;
