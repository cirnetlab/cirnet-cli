use super::{Entry, local_metadata, named_entries};
use crate::process::metadata_output;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const METADATA_TIMEOUT: Duration = Duration::from_secs(2);
const POWERSHELL_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_OUTPUT: usize = 256 * 1024;

pub(super) fn git_commands() -> Vec<Entry> {
    let Some(output) = git_query(&["--list-cmds=builtins"], None) else {
        return Vec::new();
    };
    named_entries(
        output
            .split_whitespace()
            .filter(|name| git_word(name))
            .map(str::to_owned),
    )
}

pub(super) fn git_config_keys() -> Vec<Entry> {
    let Some(output) = git_query(&["help", "--config"], None) else {
        return Vec::new();
    };
    named_entries(
        output
            .lines()
            .map(str::trim)
            .filter(|name| {
                name.split_once('.').is_some()
                    && name
                        .bytes()
                        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'.' | b'-'))
            })
            .map(str::to_owned),
    )
}

pub(super) fn git_options(command: &str) -> Vec<Entry> {
    let Some((primary, nested)) = git_option_query(command) else {
        return Vec::new();
    };
    if !git_commands().iter().any(|entry| entry.name == primary) {
        return Vec::new();
    }
    let Some(scratch) = ScratchGit::new() else {
        return Vec::new();
    };
    let mut args = vec![primary];
    args.extend(nested);
    args.push("--git-completion-helper");
    git_query(&args, Some(&scratch.0)).map_or_else(Vec::new, |output| git_option_entries(&output))
}

fn git_option_entries(output: &str) -> Vec<Entry> {
    named_entries(
        output
            .split_whitespace()
            .filter(|name| {
                if let Some(flag) = name.strip_prefix("--") {
                    git_word(flag.trim_end_matches('='))
                } else {
                    git_word(name)
                }
            })
            .map(str::to_owned),
    )
}

fn git_word(word: &str) -> bool {
    !word.is_empty()
        && word.len() < 100
        && word.as_bytes()[0].is_ascii_alphabetic()
        && word
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-')
}

fn git_option_query(command: &str) -> Option<(&str, Option<&str>)> {
    let (main, subcommand) = command
        .split_once(' ')
        .map_or((command, None), |(main, sub)| (main, Some(sub)));
    // These built-ins provide the parse-options completion endpoint. In
    // particular, an unrecognized form never reaches an alias or git-* helper.
    let supported = matches!(
        (main, subcommand),
        (
            "add"
                | "am"
                | "apply"
                | "archive"
                | "bisect"
                | "blame"
                | "branch"
                | "bundle"
                | "cat-file"
                | "checkout"
                | "cherry-pick"
                | "clean"
                | "clone"
                | "commit"
                | "config"
                | "describe"
                | "diff"
                | "fetch"
                | "for-each-ref"
                | "grep"
                | "init"
                | "log"
                | "ls-files"
                | "ls-tree"
                | "maintenance"
                | "merge"
                | "mv"
                | "pull"
                | "push"
                | "rebase"
                | "reflog"
                | "remote"
                | "reset"
                | "restore"
                | "revert"
                | "rm"
                | "show"
                | "stash"
                | "status"
                | "switch"
                | "tag"
                | "update-index"
                | "update-ref"
                | "worktree",
            None,
        ) | (
            "worktree",
            Some("add" | "list" | "lock" | "move" | "prune" | "remove" | "repair" | "unlock"),
        ) | (
            "remote",
            Some(
                "add"
                    | "get-url"
                    | "prune"
                    | "remove"
                    | "rename"
                    | "set-branches"
                    | "set-head"
                    | "set-url"
                    | "show"
                    | "update",
            ),
        ) | (
            "stash",
            Some("apply" | "branch" | "drop" | "list" | "pop" | "push" | "show")
        ) | ("config", Some("get" | "list" | "set" | "unset"))
            | (
                "bundle",
                Some("create" | "verify" | "list-heads" | "unbundle")
            )
            | (
                "reflog",
                Some("delete" | "exists" | "expire" | "list" | "show")
            )
    );
    supported.then_some((main, subcommand))
}

fn git_query(args: &[&str], cwd: Option<&Path>) -> Option<String> {
    let executable = git_executable()?;
    let mut command = isolated_command(&executable)?;
    let installation = executable.parent()?;
    command
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_COUNT", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_ALLOW_PROTOCOL", "")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_CEILING_DIRECTORIES", installation.parent()?);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
        command.env("GIT_DIR", cwd.join(".git"));
        command.env("GIT_WORK_TREE", cwd);
    }
    output(&mut command, METADATA_TIMEOUT)
}

struct ScratchGit(PathBuf);

impl ScratchGit {
    fn new() -> Option<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temp = std::env::temp_dir();
        if !local_metadata(&temp)?.is_dir() {
            return None;
        }
        let path = temp.join(format!(
            "cnl-git-completion-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let directory = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut directory = directory;
            directory.mode(0o700);
            directory.create(&path).ok()?;
        }
        #[cfg(not(unix))]
        directory.create(&path).ok()?;
        let scratch = Self(path);
        let git = scratch.0.join(".git");
        fs::create_dir(&git).ok()?;
        fs::create_dir(git.join("objects")).ok()?;
        fs::create_dir(git.join("refs")).ok()?;
        fs::write(git.join("HEAD"), "ref: refs/heads/cnl-completion\n").ok()?;
        Some(scratch)
    }
}

impl Drop for ScratchGit {
    fn drop(&mut self) {
        let git = self.0.join(".git");
        let _ = fs::remove_file(git.join("HEAD"));
        let _ = fs::remove_dir(git.join("objects"));
        let _ = fs::remove_dir(git.join("refs"));
        let _ = fs::remove_dir(git);
        let _ = fs::remove_dir(&self.0);
    }
}

fn installed_file(paths: &[&str]) -> Option<PathBuf> {
    paths
        .iter()
        .map(Path::new)
        .find(|path| local_metadata(path).is_some_and(|metadata| metadata.is_file()))
        .map(Path::to_owned)
}

fn git_executable() -> Option<PathBuf> {
    #[cfg(windows)]
    return installed_file(&[
        r"C:\Program Files\Git\mingw64\bin\git.exe",
        r"C:\Program Files\Git\mingw32\bin\git.exe",
        r"C:\Program Files (x86)\Git\mingw32\bin\git.exe",
    ]);

    #[cfg(not(windows))]
    installed_file(&["/usr/bin/git", "/usr/local/bin/git", "/bin/git"])
}

fn powershell_executable() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        installed_file(&[
            r"C:\Program Files\PowerShell\7\pwsh.exe",
            r"C:\Program Files\PowerShell\7-preview\pwsh.exe",
            r"C:\Program Files (x86)\PowerShell\7\pwsh.exe",
        ])
        .or_else(|| {
            // The desktop runtime packages PowerShell with its other native tools.
            // The executable must be within the OS-controlled package directory.
            let path = std::env::var_os("PATH")?;
            let root = Path::new(r"C:\Program Files\WindowsApps");
            std::env::split_paths(&path)
                .take(32)
                .filter(|path| {
                    path.strip_prefix(root).ok().is_some_and(|relative| {
                        relative.components().next().is_some_and(|part| {
                            part.as_os_str()
                                .to_string_lossy()
                                .starts_with("OpenAI.CodexPrimaryRuntime.")
                        }) && relative.components().count() == 4
                            && relative.ends_with(r"dependencies\native\powershell")
                    })
                })
                .map(|path| path.join("pwsh.exe"))
                .find(|path| local_metadata(path).is_some_and(|metadata| metadata.is_file()))
        })
    }
    #[cfg(not(windows))]
    installed_file(&[
        "/opt/microsoft/powershell/7/pwsh",
        "/usr/bin/pwsh",
        "/usr/local/bin/pwsh",
    ])
}

fn isolated_command(executable: &Path) -> Option<Command> {
    let mut command = Command::new(executable);
    command
        .current_dir(executable.parent()?)
        .env_clear()
        .env("LANG", "C.UTF-8");

    // Windows process initialization and DLL resolution use the system directory.
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", system_root);
    }

    Some(command)
}

fn output(command: &mut Command, timeout: Duration) -> Option<String> {
    let (status, bytes) = metadata_output(command, timeout, MAX_OUTPUT)?;
    status.success().then(|| String::from_utf8(bytes).ok())?
}

const POWERSHELL_METADATA: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
$PSModuleAutoLoadingPreference = 'None'
$env:PSModulePath = [IO.Path]::Combine($PSHOME, 'Modules')
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
foreach ($module in @('Microsoft.PowerShell.Management', 'Microsoft.PowerShell.Utility', 'Microsoft.PowerShell.Security', 'Microsoft.PowerShell.Diagnostics')) {
    $manifest = [IO.Path]::Combine($PSHOME, 'Modules', $module, "$module.psd1")
    if ([IO.File]::Exists($manifest)) {
        Import-Module -Name $manifest -ErrorAction SilentlyContinue
    }
}
if (!$env:CNL_METADATA_COMMAND) {
    Get-Command -ListImported -CommandType Cmdlet,Alias,Function | ForEach-Object { $_.Name }
    return
}
$item = Get-Command -ListImported -Name $env:CNL_METADATA_COMMAND -CommandType Cmdlet,Alias,Function
if ($item -is [Management.Automation.AliasInfo]) {
    $item = Get-Command -ListImported -Name $item.Definition -CommandType Cmdlet,Function
}
if ($item) {
    if (!$env:CNL_METADATA_PARAMETER) {
        $item.Parameters.Keys | ForEach-Object { "-$_" }
    } elseif ($env:CNL_METADATA_PARAMETER -eq 'Name' -and $item.Name -in @('Get-Module', 'Import-Module', 'Remove-Module')) {
        Get-Module | ForEach-Object { $_.Name }
    } else {
        $parameter = $item.Parameters[$env:CNL_METADATA_PARAMETER]
        if ($parameter -and $parameter.ParameterType.IsEnum) {
            [Enum]::GetNames($parameter.ParameterType)
        }
    }
}
"#;

pub(super) fn powershell(command: Option<&str>, parameter: Option<&str>) -> Vec<Entry> {
    fn safe_identifier(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 128
            && name
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'_'))
    }

    let parameter = parameter.map(|name| name.trim_start_matches('-'));
    if command.is_some_and(|name| !safe_identifier(name))
        || parameter.is_some_and(|name| !safe_identifier(name))
    {
        return Vec::new();
    }

    let Some(executable) = powershell_executable() else {
        return Vec::new();
    };
    let Some(mut probe) = isolated_command(&executable) else {
        return Vec::new();
    };
    probe.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        POWERSHELL_METADATA,
    ]);
    if let Some(command) = command {
        probe.env("CNL_METADATA_COMMAND", command);
    }
    if let Some(parameter) = parameter {
        probe.env("CNL_METADATA_PARAMETER", parameter);
    }
    probe
        .env("POWERSHELL_TELEMETRY_OPTOUT", "1")
        .env("POWERSHELL_UPDATECHECK", "Off");
    output(&mut probe, POWERSHELL_TIMEOUT).map_or_else(Vec::new, |output| {
        named_entries(output.lines().map(str::trim).map(str::to_owned))
    })
}

pub(super) fn bash_commands() -> Vec<Entry> {
    #[cfg(windows)]
    let (executable, commands) = if let Some(git) = git_executable()
        && let Some(root) = git.ancestors().nth(3)
    {
        let bin = root.join("usr/bin");
        let executable = bin.join("bash.exe");
        let names = super::paths(&bin)
            .into_iter()
            .chain(git.parent().into_iter().flat_map(super::paths))
            .filter(|entry| !entry.directory)
            .filter_map(|entry| entry.name.strip_suffix(".exe").map(str::to_owned))
            .collect();
        (
            installed_file(&[executable.to_str().unwrap_or_default()]),
            names,
        )
    } else {
        (None, Vec::new())
    };
    #[cfg(not(windows))]
    let (executable, commands) = (
        installed_file(&["/usr/bin/bash", "/bin/bash", "/usr/local/bin/bash"]),
        Vec::new(),
    );
    let Some(executable) = executable else {
        return Vec::new();
    };
    let Some(mut command) = isolated_command(&executable) else {
        return Vec::new();
    };
    command.args(["--noprofile", "--norc", "-c", "builtin compgen -b"]);
    let builtins = output(&mut command, METADATA_TIMEOUT).unwrap_or_default();
    named_entries(builtins.lines().map(str::to_owned).chain(commands))
}

pub(super) fn wsl_distributions() -> Vec<Entry> {
    #[cfg(windows)]
    return named_entries(crate::cli::native::wsl_distributions());

    #[cfg(not(windows))]
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_helper_flags_and_subcommands_preserve_value_suffixes() {
        assert!(git_option_query("remote set-url").is_some());
        assert!(git_option_query("submodule").is_none());
        assert!(git_option_query("remote --help").is_none());
        assert!(git_option_query("rebase; echo").is_none());
        let names = git_option_entries("--onto= --verbose -- add\n--bad<<")
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["--onto=", "--verbose", "add"]);
    }

    #[test]
    fn installed_git_reports_advanced_builtin_options() {
        if git_executable().is_none() {
            return;
        }
        assert!(git_commands().iter().any(|entry| entry.name == "rebase"));
        assert!(
            git_options("rebase")
                .iter()
                .any(|entry| entry.name == "--onto=")
        );
        assert!(
            git_config_keys()
                .iter()
                .any(|entry| entry.name == "core.autocrlf")
        );
    }

    #[test]
    fn installed_powershell_reports_imported_cmdlets_and_static_parameters() {
        if powershell_executable().is_none() {
            return;
        }
        assert!(
            powershell(None, None)
                .iter()
                .any(|entry| entry.name == "Get-Process")
        );
        assert!(
            powershell(Some("Get-Process"), None)
                .iter()
                .any(|entry| entry.name == "-Id")
        );
    }
}
