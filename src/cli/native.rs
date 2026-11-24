use std::{
    env,
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    time::Duration,
};

use crate::localization::{self, message, text};
use anyhow::{Context, Result, bail, ensure};
use clap::{
    Arg, Command as ClapCommand, Error,
    builder::{PossibleValue, StringValueParser, TypedValueParser, ValueParser},
    error::{ContextKind, ContextValue, ErrorKind},
};

const SHELLS: [&str; 5] = ["powershell", "cmd", "bash", "git-bash", "wsl"];
const SHELL_CHECK: Duration = Duration::from_secs(3);
const WSL_CHECK: Duration = Duration::from_secs(6);
const POWERSHELL_PROFILE: &str = r#"if (Test-Path -LiteralPath "$PSHOME/Modules/PSReadLine/PSReadLine.psd1") { Import-Module "$PSHOME/Modules/PSReadLine/PSReadLine.psd1" -ErrorAction SilentlyContinue }; if (Get-Module -Name PSReadLine) { Set-PSReadLineOption -HistorySaveStyle SaveNothing }"#;

// Initialize after Bash owns the terminal. PROMPT_COMMAND is fixed application code,
// and startup never sources a repository or user profile. The distributor's completion
// scripts can still provide native completion, including Readline's normal fallback.
const BASH_PROFILE: &str = r#"
builtin unset PROMPT_COMMAND HISTFILE
BASH_COMPLETION_USER_FILE=/dev/null
BASH_COMPLETION_USER_DIR=/dev/null
BASH_COMPLETION_COMPAT_DIR=/etc/bash_completion.d
if [[ -r /usr/share/bash-completion/bash_completion ]]; then
    builtin source /usr/share/bash-completion/bash_completion
elif [[ $MSYSTEM == MINGW64 || $MSYSTEM == MINGW32 || $MSYSTEM == CLANGARM64 ]]; then
    case "$MSYSTEM" in
        MINGW64) _cnl_git_completion=/mingw64/share/git/completion/git-completion.bash ;;
        MINGW32) _cnl_git_completion=/mingw32/share/git/completion/git-completion.bash ;;
        CLANGARM64) _cnl_git_completion=/clangarm64/share/git/completion/git-completion.bash ;;
    esac
    if [[ -r $_cnl_git_completion ]]; then builtin source "$_cnl_git_completion"; fi
    builtin unset _cnl_git_completion
fi
builtin unset HISTFILE
"#;

pub(crate) fn shell_parser() -> ValueParser {
    ValueParser::new(ShellValueParser)
}

#[derive(Clone)]
struct ShellValueParser;

impl TypedValueParser for ShellValueParser {
    type Value = String;

    fn parse_ref(
        &self,
        cmd: &ClapCommand,
        arg: Option<&Arg>,
        value: &OsStr,
    ) -> Result<String, Error> {
        let value = StringValueParser::new().parse_ref(cmd, arg, value)?;
        if valid_shell(&value) {
            return Ok(value);
        }
        let supported = SHELLS
            .into_iter()
            .chain(["wsl:<distribution>"])
            .map(str::to_owned)
            .collect();
        let mut error = Error::new(ErrorKind::InvalidValue).with_cmd(cmd);
        if let Some(arg) = arg {
            error.insert(
                ContextKind::InvalidArg,
                ContextValue::String(arg.to_string()),
            );
        }
        error.insert(ContextKind::InvalidValue, ContextValue::String(value));
        error.insert(ContextKind::ValidValue, ContextValue::Strings(supported));
        Err(error)
    }

    fn possible_values(&self) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
        Some(Box::new(SHELLS.into_iter().map(PossibleValue::new)))
    }
}

pub(crate) fn valid_shell(value: &str) -> bool {
    SHELLS.contains(&value) || value.strip_prefix("wsl:").is_some_and(valid_distribution)
}

pub(crate) fn same_selector(left: &str, right: &str) -> bool {
    if left.starts_with("wsl:") && right.starts_with("wsl:") {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn valid_distribution(name: &str) -> bool {
    !name.is_empty()
        && name == name.trim()
        && !name.starts_with('-')
        && name.chars().count() <= 255
        && !name
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '/' | '\\' | ':'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    PowerShell,
    Cmd,
    Bash,
    GitBash,
    Wsl,
}

#[derive(Clone)]
pub(crate) struct ShellProfile {
    pub(crate) selector: String,
    kind: Kind,
    program: PathBuf,
    distribution: Option<String>,
    version: Option<String>,
    default_distribution: bool,
}

impl ShellProfile {
    fn host(kind: Kind, selector: &str) -> Option<Self> {
        let program = match kind {
            Kind::PowerShell => powershell_executable(),
            Kind::Cmd => windows_program("cmd.exe"),
            Kind::Bash => bash_executable(false),
            Kind::GitBash => bash_executable(true),
            Kind::Wsl => None,
        }?;
        Some(Self {
            selector: selector.into(),
            kind,
            program,
            distribution: None,
            version: None,
            default_distribution: false,
        })
    }

    fn wsl(program: &Path, distribution: Option<&str>) -> Self {
        Self {
            selector: distribution.map_or_else(|| "wsl".to_owned(), |name| format!("wsl:{name}")),
            kind: Kind::Wsl,
            program: program.to_path_buf(),
            distribution: distribution.map(str::to_owned),
            version: None,
            default_distribution: false,
        }
    }

    fn probe(&self) -> Option<String> {
        let mut command = Command::new(&self.program);
        command.env("LC_ALL", "C");
        match self.kind {
            Kind::PowerShell => {
                command.arg("--version");
            }
            Kind::Cmd => {
                command.args(["/D", "/C", "ver"]);
            }
            Kind::Bash | Kind::GitBash => {
                command.args(["--noprofile", "--norc", "--version"]);
            }
            Kind::Wsl => {
                command
                    .arg("--distribution")
                    .arg(self.distribution.as_deref()?)
                    .args([
                        "--exec",
                        "/usr/bin/env",
                        "LC_ALL=C",
                        "/bin/bash",
                        "--noprofile",
                        "--norc",
                        "--version",
                    ]);
            }
        }
        let timeout = if self.kind == Kind::Wsl {
            WSL_CHECK
        } else {
            SHELL_CHECK
        };
        let (status, output) = crate::process::metadata_output(&mut command, timeout, 16 * 1024)?;
        if !status.success() {
            return None;
        }
        let value = launcher_text(&output);
        let version = value.lines().map(str::trim).find(|line| !line.is_empty())?;
        match self.kind {
            Kind::PowerShell => version
                .strip_prefix("PowerShell 7.")
                .map(|_| version.to_owned()),
            Kind::Cmd => Some(version.to_owned()),
            Kind::Bash | Kind::GitBash | Kind::Wsl => version
                .strip_prefix("GNU bash, version ")
                .and_then(|value| value.split_whitespace().next())
                .map(str::to_owned),
        }
    }

    pub(crate) fn label(&self, vi: bool) -> String {
        let name = shell_label(&self.selector, vi);
        if let Some(version) = &self.version
            && matches!(self.kind, Kind::Bash | Kind::GitBash)
        {
            return format!("{name} {version}");
        }
        if let Some(version) = &self.version
            && self.kind == Kind::PowerShell
        {
            return version.clone();
        }
        name
    }

    pub(crate) fn matches(&self, requested: &str) -> bool {
        if self.kind == Kind::Wsl {
            same_selector(&self.selector, requested)
                || (self.default_distribution && requested == "wsl")
        } else {
            self.selector == requested
        }
    }

    pub(crate) fn interactive(&self, directory: Option<&Path>, vi: bool) -> Result<Command> {
        self.command(directory, None, vi)
    }

    fn command(&self, directory: Option<&Path>, line: Option<&str>, vi: bool) -> Result<Command> {
        let mut command = Command::new(&self.program);
        command.env_remove("CNL_TUI_REPORT");
        if let Some(distro) = &self.distribution {
            command.args(["--distribution", distro]);
        }
        if let Some(directory) = directory {
            let standard = host_directory(directory, self.kind, vi)?;
            if self.kind == Kind::Wsl {
                let target = wsl_directory(&standard, self.distribution.as_deref(), vi)?;
                command.arg("--cd").arg(target);
                if let Some(system_root) = self.program.parent() {
                    command.current_dir(system_root);
                }
            } else {
                command.current_dir(standard);
            }
        }
        match self.kind {
            Kind::PowerShell => {
                command.args(["-NoLogo", "-NoProfile"]);
                if let Some(line) = line {
                    command.args(["-NonInteractive", "-Command", line]);
                } else {
                    command.args(["-NoExit", "-Command", POWERSHELL_PROFILE]);
                }
            }
            Kind::Cmd => {
                command.arg("/D");
                #[cfg(windows)]
                if let Some(line) = line {
                    use std::os::windows::process::CommandExt;
                    command.args(["/S", "/C"]).raw_arg(format!(r#""{line}""#));
                }
            }
            Kind::Bash | Kind::GitBash | Kind::Wsl => self.bash_command(&mut command, line),
        }
        Ok(command)
    }

    fn bash_command(&self, command: &mut Command, line: Option<&str>) {
        if self.kind == Kind::Wsl {
            // The WSL launcher consumes its own options before the direct guest argv.
            command.args([
                "--exec",
                "/usr/bin/env",
                "-u",
                "BASH_ENV",
                "-u",
                "ENV",
                "HISTFILE=",
            ]);
            if line.is_none() {
                command.arg(format!("PROMPT_COMMAND={BASH_PROFILE}"));
            }
            command.arg("/bin/bash");
        } else {
            command
                .env_remove("BASH_ENV")
                .env_remove("ENV")
                .env("HISTFILE", "");
            if line.is_none() {
                command.env("PROMPT_COMMAND", BASH_PROFILE);
            }
            if self.kind == Kind::GitBash {
                configure_git_bash(command, &self.program);
            }
        }
        command.args(["--noprofile", "--norc"]);
        if let Some(line) = line {
            command.args(["-c", line, "cnl"]);
        } else {
            command.arg("-i");
        }
    }
}

pub(crate) fn shell_label(selector: &str, vi: bool) -> String {
    match selector {
        "powershell" => "PowerShell 7".into(),
        "cmd" => "CMD".into(),
        "bash" if cfg!(target_os = "linux") => "Linux Bash".into(),
        "bash" => "Bash".into(),
        "git-bash" => "Git Bash".into(),
        "wsl" => text(["WSL Bash (default)", "WSL Bash (mặc định)"], vi).into(),
        name => name
            .strip_prefix("wsl:")
            .map_or_else(|| name.to_owned(), |distro| format!("WSL Bash · {distro}")),
    }
}

pub(crate) fn installed_shells() -> Vec<ShellProfile> {
    let mut result = Vec::new();
    for (kind, selector) in [
        (Kind::PowerShell, "powershell"),
        (Kind::Cmd, "cmd"),
        (Kind::Bash, "bash"),
        (Kind::GitBash, "git-bash"),
    ] {
        if let Some(mut profile) = ShellProfile::host(kind, selector)
            && let Some(version) = profile.probe()
        {
            profile.version = Some(version);
            result.push(profile);
        }
    }
    if let Some(program) = windows_program("wsl.exe") {
        let distros = interactive_distributions(&program);
        let default = default_distribution(&program, &distros);
        result.extend(distros.iter().map(|name| {
            let mut profile = ShellProfile::wsl(&program, Some(name));
            profile.default_distribution = default.as_deref() == Some(name.as_str());
            profile
        }));
    }
    result
}

pub(crate) fn terminal_profile(selected: Option<&str>, vi: bool) -> Result<ShellProfile> {
    selected_profile(selected, None, vi, true)
}

fn selected_profile(
    selected: Option<&str>,
    directory: Option<&Path>,
    vi: bool,
    validate: bool,
) -> Result<ShellProfile> {
    if let Some(selected) = selected {
        return resolve_profile(selected, directory, vi, validate);
    }
    if cfg!(windows) {
        resolve_profile("powershell", directory, vi, validate).or_else(|pwsh| {
            resolve_profile("cmd", directory, vi, validate).with_context(|| pwsh.to_string())
        })
    } else {
        resolve_profile("bash", directory, vi, validate)
    }
}

fn resolve_profile(
    selected: &str,
    directory: Option<&Path>,
    vi: bool,
    validate: bool,
) -> Result<ShellProfile> {
    let profile = match selected {
        "powershell" => ShellProfile::host(Kind::PowerShell, selected),
        "cmd" => ShellProfile::host(Kind::Cmd, selected),
        "bash" => ShellProfile::host(Kind::Bash, selected),
        "git-bash" => ShellProfile::host(Kind::GitBash, selected),
        selector
            if valid_shell(selector)
                && let Some(program) = windows_program("wsl.exe") =>
        {
            let explicit = selector.strip_prefix("wsl:");
            let map_default_unc = explicit.is_none()
                && directory
                    .is_some_and(|path| crate::workspace::user_path(path).starts_with(r"\\"));
            if validate || map_default_unc {
                let installed = interactive_distributions(&program);
                let distribution = if let Some(requested) = explicit {
                    installed
                        .into_iter()
                        .find(|name| name.eq_ignore_ascii_case(requested))
                } else {
                    default_distribution(&program, &installed)
                };
                let Some(distribution) = distribution else {
                    bail!(
                        "{} ({selector}).",
                        text(
                            [
                                "The requested WSL distribution is unavailable. Select an installed Linux distribution",
                                "Bản phân phối WSL đã chọn không khả dụng. Hãy chọn một bản phân phối Linux đã cài đặt",
                            ],
                            vi
                        )
                    );
                };
                Some(ShellProfile::wsl(&program, Some(&distribution)))
            } else {
                Some(ShellProfile::wsl(&program, explicit))
            }
        }
        _ => None,
    };
    let Some(mut profile) = profile else {
        bail!(
            "{} ({selected}). {}",
            text(localization::SHELL_PLATFORM, vi),
            text(localization::SHELL_HELP, vi)
        );
    };
    if validate {
        profile.version = profile.probe();
        ensure!(
            profile.version.is_some(),
            "{} ({selected}). {}",
            text(
                [
                    "The selected shell did not pass its startup check",
                    "Shell đã chọn không vượt qua kiểm tra khởi động",
                ],
                vi,
            ),
            text(localization::SHELL_INSTALL, vi)
        );
    }
    Ok(profile)
}

pub(crate) fn bash_executable(git_bash: bool) -> Option<PathBuf> {
    if git_bash != cfg!(windows) {
        return None;
    }
    if !git_bash {
        return ["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| executable(path));
    }
    let usual = windows_install_roots()
        .map(|root| root.join("Git"))
        .chain(env::var_os("LOCALAPPDATA").map(|root| PathBuf::from(root).join("Programs/Git")));
    let via_path = search_paths()
        .filter(|path| executable(&path.join("git.exe")))
        .filter_map(|path| path.parent().map(Path::to_path_buf));
    usual
        .chain(via_path.filter(|root| trusted_install_path(root)))
        .find_map(|root| {
            let program = root.join("bin/bash.exe");
            (executable(&program)
                && executable(&root.join("usr/bin/bash.exe"))
                && root.join("etc/profile").is_file()
                && git_runtime(&root).is_some())
            .then_some(program)
        })
}

fn git_runtime(root: &Path) -> Option<(&'static str, &'static str)> {
    [
        ("MINGW64", "mingw64"),
        ("MINGW32", "mingw32"),
        ("CLANGARM64", "clangarm64"),
    ]
    .into_iter()
    .find(|(_, subdir)| executable(&root.join(subdir).join("bin/git.exe")))
}

fn configure_git_bash(command: &mut Command, program: &Path) {
    if let Some(root) = program.parent().and_then(Path::parent)
        && let Some((msystem, runtime)) = git_runtime(root)
    {
        command
            .env("MSYSTEM", msystem)
            .env("MSYS2_PATH_TYPE", "inherit");
        let entries = [root.join(runtime).join("bin"), root.join("usr/bin")]
            .into_iter()
            .chain(search_paths());
        if let Ok(path) = env::join_paths(entries) {
            command.env("PATH", path);
        }
    }
}

pub(crate) fn powershell_executable() -> Option<PathBuf> {
    if cfg!(windows) {
        let roots: Vec<_> = windows_install_roots().collect();
        return roots
            .iter()
            .map(|root| root.join("PowerShell/7/pwsh.exe"))
            .chain(
                search_paths()
                    .filter(|path| {
                        roots.iter().any(|root| path.starts_with(root))
                            || path
                                .to_string_lossy()
                                .to_ascii_lowercase()
                                .starts_with(r"c:\program files\windowsapps\")
                    })
                    .map(|path| path.join("pwsh.exe")),
            )
            .find(|path| executable(path));
    }
    [
        "/usr/bin/pwsh",
        "/usr/local/bin/pwsh",
        "/opt/microsoft/powershell/7/pwsh",
        "/opt/homebrew/bin/pwsh",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| executable(path))
}

fn trusted_install_path(path: &Path) -> bool {
    windows_install_roots()
        .chain(env::var_os("LOCALAPPDATA").map(|root| PathBuf::from(root).join("Programs")))
        .any(|root| path.starts_with(root))
}

fn search_paths() -> impl Iterator<Item = PathBuf> {
    env::var_os("PATH")
        .into_iter()
        .flat_map(|value| env::split_paths(&value).collect::<Vec<_>>())
        .filter(|path| path.is_absolute())
}

fn windows_install_roots() -> impl Iterator<Item = PathBuf> {
    ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(env::var_os)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn windows_program(name: &str) -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let root = PathBuf::from(env::var_os("SystemRoot")?);
    ["Sysnative", "System32"]
        .into_iter()
        .map(|dir| root.join(dir).join(name))
        .find(|path| path.is_absolute() && executable(path))
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    metadata.is_file()
}

pub(crate) fn wsl_distributions() -> Vec<String> {
    windows_program("wsl.exe").map_or_else(Vec::new, |program| installed_distributions(&program))
}

fn installed_distributions(program: &Path) -> Vec<String> {
    let mut command = Command::new(program);
    command.args(["--list", "--quiet"]);
    crate::process::metadata_output(&mut command, SHELL_CHECK, 64 * 1024)
        .filter(|(exit, _)| exit.success())
        .map_or_else(Vec::new, |(_, output)| parse_distributions(&output))
}

fn interactive_distributions(program: &Path) -> Vec<String> {
    installed_distributions(program)
        .into_iter()
        .filter(|name| is_interactive_distribution(name))
        .collect()
}

pub(crate) fn is_interactive_distribution(name: &str) -> bool {
    !["docker-desktop", "docker-desktop-data"]
        .iter()
        .any(|managed| name.eq_ignore_ascii_case(managed))
}

fn parse_distributions(output: &[u8]) -> Vec<String> {
    let mut distributions = Vec::<String>::new();
    for name in launcher_text(output).lines().map(str::trim) {
        if valid_distribution(name)
            && !distributions
                .iter()
                .any(|known| known.eq_ignore_ascii_case(name))
            && distributions.len() < 64
        {
            distributions.push(name.to_owned());
        }
    }
    distributions
}

fn default_distribution(program: &Path, installed: &[String]) -> Option<String> {
    if installed.is_empty() {
        return None;
    }
    let mut command = Command::new(program);
    command.args(["--list", "--verbose"]);
    let (exit, output) = crate::process::metadata_output(&mut command, SHELL_CHECK, 64 * 1024)?;
    exit.success().then_some(())?;
    let output = launcher_text(&output);
    let default = output
        .lines()
        .find_map(|line| line.trim().strip_prefix('*'))?
        .trim_start();
    installed
        .iter()
        .filter(|name| {
            default
                .strip_prefix(name.as_str())
                .is_some_and(|rest| rest.starts_with(char::is_whitespace))
        })
        .max_by_key(|name| name.len())
        .cloned()
}

fn launcher_text(output: &[u8]) -> String {
    if output.starts_with(&[0xff, 0xfe]) || output.contains(&0) {
        let words: Vec<_> = output
            .as_chunks::<2>()
            .0
            .iter()
            .copied()
            .map(u16::from_le_bytes)
            .collect();
        String::from_utf16_lossy(&words)
            .trim_start_matches('\u{feff}')
            .to_owned()
    } else {
        String::from_utf8_lossy(output)
            .trim_start_matches('\u{feff}')
            .to_owned()
    }
}

fn host_directory(directory: &Path, kind: Kind, vi: bool) -> Result<PathBuf> {
    let actual = directory.canonicalize()?;
    if !cfg!(windows) {
        return Ok(actual);
    }
    let visible = crate::workspace::user_path(&actual);
    let standard = Path::new(visible.as_ref());
    let network = visible.starts_with(r"\\");
    ensure!(
        kind != Kind::Cmd || !network,
        "{}",
        text(
            [
                "CMD cannot use a UNC working directory. Select PowerShell for this checkout.",
                "CMD không thể dùng thư mục làm việc UNC. Hãy chọn PowerShell cho checkout này.",
            ],
            vi,
        )
    );
    if standard.canonicalize().is_ok_and(|path| path == actual) {
        return Ok(standard.to_path_buf());
    }
    if kind == Kind::PowerShell {
        return Ok(actual);
    }
    bail!(
        "{}",
        text(
            [
                "The selected shell cannot use this working directory",
                "Shell đã chọn không thể dùng thư mục làm việc này",
            ],
            vi,
        )
    )
}

fn wsl_directory(directory: &Path, distro: Option<&str>, vi: bool) -> Result<PathBuf> {
    let value = directory.to_str().context(text(
        [
            "WSL requires a Unicode working directory",
            "WSL cần thư mục làm việc Unicode",
        ],
        vi,
    ))?;
    let Some(unc) = value.strip_prefix(r"\\") else {
        return Ok(directory.into());
    };
    let mut pieces = unc.splitn(3, '\\');
    let server = pieces.next().unwrap_or_default();
    let share = pieces.next().unwrap_or_default();
    ensure!(
        (server.eq_ignore_ascii_case("wsl.localhost") || server.eq_ignore_ascii_case("wsl$"))
            && distro.is_some_and(|name| share.eq_ignore_ascii_case(name)),
        "{}",
        text(
            [
                "The UNC checkout must belong to the selected WSL distribution",
                "Checkout UNC phải thuộc bản phân phối WSL đã chọn",
            ],
            vi,
        )
    );
    Ok(PathBuf::from(format!(
        "/{}",
        pieces.next().unwrap_or_default().replace('\\', "/")
    )))
}

pub(crate) fn interactive_in(
    selected: Option<&str>,
    directory: Option<&Path>,
    vi: bool,
) -> Result<ExitStatus> {
    let profile = terminal_profile(selected, vi)?;
    let mut command = profile.interactive(directory, vi)?;
    command.status().with_context(|| {
        format!(
            "{} {}. {}",
            text(localization::SHELL_START_FAILED, vi),
            profile.selector,
            text(localization::SHELL_INSTALL, vi)
        )
    })
}

pub(crate) fn line(
    input: &str,
    selected: Option<&str>,
    directory: Option<&Path>,
    vi: bool,
    arguments: impl FnOnce(&str) -> Option<Vec<String>>,
) -> Result<Command> {
    // Keep the original direct Git path for implicit host commands. Explicit
    // profiles own their own parser, executable lookup, and filesystem context.
    if selected.is_none()
        && input.split_whitespace().next() == Some("git")
        && !input.contains([
            '|', '&', '<', '>', ';', '$', '\u{0060}', '%', '*', '?', '(', ')',
        ])
        && !(cfg!(windows) && input.contains("''") || !cfg!(windows) && input.contains('\\'))
        && let Some(args) = arguments(input)
    {
        let mut command = Command::new("git");
        command.args(args.into_iter().skip(1));
        if let Some(directory) = directory {
            command.current_dir(directory);
        }
        return Ok(command);
    }
    // An explicitly run command can report launcher failure from the child. The
    // event loop does not run shell version probes before starting the command.
    selected_profile(selected, directory, vi, false)?.command(directory, Some(input), vi)
}

pub(crate) fn unavailable(error: io::Error) -> anyhow::Error {
    if error.kind() == io::ErrorKind::NotFound {
        anyhow::Error::new(error).context(message(localization::SHELL_UNAVAILABLE))
    } else {
        error.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_inventory_decodes_windows_launcher_output() {
        let listing: Vec<_> = "\u{feff}Ubuntu 24.04\r\nViệt Linux\r\nubuntu 24.04\r\n--unknown\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(
            parse_distributions(&listing),
            ["Ubuntu 24.04", "Việt Linux"]
        );
    }

    #[test]
    fn wsl_maps_only_the_selected_distribution_unc_checkout() {
        assert_eq!(
            wsl_directory(
                Path::new(r"\\wsl.localhost\Ubuntu 24.04\home\Việt Labs"),
                Some("ubuntu 24.04"),
                false
            )
            .unwrap(),
            Path::new("/home/Việt Labs")
        );
        assert_eq!(
            wsl_directory(Path::new(r"C:\Program Files\repo"), Some("Debian"), false).unwrap(),
            Path::new(r"C:\Program Files\repo")
        );
        for path in [r"\\wsl$\Ubuntu\home\demo", r"\\server\share\demo"] {
            assert!(wsl_directory(Path::new(path), Some("Debian"), false).is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn cmd_forwards_its_native_quoting_and_pipeline() {
        let command = r#"echo "cnl cmd" | findstr /C:"cnl cmd""#;
        let output = line(command, Some("cmd"), None, false, |_| {
            panic!("explicit CMD should own the line")
        })
        .unwrap()
        .output()
        .unwrap();
        assert!(output.status.success(), "{:?}", output);
        assert!(String::from_utf8_lossy(&output.stdout).contains(r#""cnl cmd""#));
    }

    #[test]
    fn shell_selectors_accept_git_bash_and_specific_wsl_distros() {
        for args in [
            vec!["cnl", "shell", "git-bash"],
            vec!["cnl", "shell", "wsl:Ubuntu 24.04"],
            vec![
                "cnl",
                "repos",
                "terminal",
                "fixture",
                "--shell",
                "wsl:Debian",
            ],
            vec!["cnl", "repos", "config", "fixture", "--shell", "wsl:Debian"],
        ] {
            assert!(
                crate::cli::app_for(false)
                    .try_get_matches_from(args)
                    .is_ok()
            );
        }
        for invalid in ["wsl:", "wsl:--exec", "wsl:Debian\n", "gitbash"] {
            assert!(
                crate::cli::app_for(false)
                    .try_get_matches_from(["cnl", "shell", invalid])
                    .is_err()
            );
        }
    }

    #[test]
    fn git_receives_arguments_and_operators_go_to_the_original_native_shell() {
        let git = line("git log --oneline", None, None, false, |_| {
            Some(["git", "log", "--oneline"].map(str::to_owned).to_vec())
        })
        .unwrap();
        assert_eq!(git.get_program(), "git");
        assert_eq!(git.get_args().collect::<Vec<_>>(), ["log", "--oneline"]);
        let raw = "git status | Select-Object -First 1";
        let shell = line(raw, None, None, false, |_| {
            panic!("native commands must retain shell syntax")
        })
        .unwrap();
        assert!(shell.get_args().any(|arg| arg == raw));
    }

    #[test]
    fn git_with_shell_quoted_apostrophe_uses_the_host_shell() {
        let raw = if cfg!(windows) {
            "git add 'O''Connor.txt'"
        } else {
            "git add 'O'\\''Connor.txt'"
        };
        let command = line(raw, None, None, false, |_| {
            panic!("the host shell owns this quote syntax")
        })
        .unwrap();
        assert!(command.get_args().any(|arg| arg == raw));
    }
}
