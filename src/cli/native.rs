use std::{
    io,
    path::Path,
    process::{Command, ExitStatus},
};

use crate::localization::{self, message};
use anyhow::{Context, Result, bail};

pub(crate) const SHELLS: [&str; 4] = ["powershell", "cmd", "bash", "wsl"];

pub(crate) fn shell(selected: Option<&str>) -> Result<Command> {
    match selected.unwrap_or(if cfg!(windows) { "powershell" } else { "bash" }) {
        "powershell" if cfg!(windows) => {
            let mut command = Command::new("pwsh.exe");
            command.args(["-NoLogo", "-NoProfile"]);
            Ok(command)
        }
        "cmd" if cfg!(windows) => {
            let mut command = Command::new("cmd.exe");
            command.arg("/D");
            Ok(command)
        }
        "bash" if !cfg!(windows) => Ok(Command::new("bash")),
        "wsl" if cfg!(windows) => {
            let mut command = Command::new("wsl.exe");
            command.args(["--exec", "bash"]);
            Ok(command)
        }
        name => bail!(
            "{} ({name}). {}",
            message(localization::SHELL_PLATFORM),
            message(localization::SHELL_HELP)
        ),
    }
}

pub(crate) fn interactive_in(
    selected: Option<&str>,
    directory: Option<&Path>,
) -> Result<ExitStatus> {
    let mut command = shell(selected)?;
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    let name = command.get_program().to_string_lossy().into_owned();
    command.status().with_context(|| {
        format!(
            "{} {name}. {}",
            message(localization::SHELL_START_FAILED),
            message(localization::SHELL_INSTALL)
        )
    })
}

pub(crate) fn line(
    input: &str,
    arguments: impl FnOnce(&str) -> Option<Vec<String>>,
) -> Result<Command> {
    // Direct execution keeps shell-free Git arguments literal. Syntax that
    // belongs to the host shell is forwarded exactly as typed.
    if input.split_whitespace().next() == Some("git")
        && !input.contains([
            '|', '&', '<', '>', ';', '$', '\u{0060}', '%', '*', '?', '(', ')',
        ])
        && !(cfg!(windows) && input.contains("''") || !cfg!(windows) && input.contains('\\'))
        && let Some(args) = arguments(input)
    {
        let mut command = Command::new("git");
        command.args(args.into_iter().skip(1));
        return Ok(command);
    }
    let mut command = shell(None)?;
    if cfg!(windows) {
        command.args(["-NonInteractive", "-Command", input]);
    } else {
        command.args(["-c", input, "cnl"]);
    }
    Ok(command)
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
    fn git_receives_arguments_and_operators_go_to_the_original_native_shell() {
        let git = line("git log --oneline", |_| {
            Some(["git", "log", "--oneline"].map(str::to_owned).to_vec())
        })
        .unwrap();
        assert_eq!(git.get_program(), "git");
        assert_eq!(git.get_args().collect::<Vec<_>>(), ["log", "--oneline"]);
        let raw = "git status | Select-Object -First 1";
        let shell = line(raw, |_| panic!("native commands must retain shell syntax")).unwrap();
        assert!(shell.get_args().any(|arg| arg == raw));
    }

    #[test]
    fn git_with_shell_quoted_apostrophe_uses_the_host_shell() {
        let raw = if cfg!(windows) {
            "git add 'O''Connor.txt'"
        } else {
            "git add 'O'\\''Connor.txt'"
        };
        let command = line(raw, |_| panic!("the host shell owns this quote syntax")).unwrap();
        assert!(command.get_args().any(|arg| arg == raw));
    }
}
