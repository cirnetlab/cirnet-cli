use std::cell::Cell;
use std::io;
use std::process::{Command, Stdio};

use crate::localization::{START_FAILED, message};
use anyhow::{Context, Result, bail};

thread_local! {
    static JSON_OUTPUT: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn set_json_output(enabled: bool) {
    JSON_OUTPUT.set(enabled);
}

fn redact_sensitive_line(line: &str) -> &str {
    let lower = line.to_ascii_lowercase();
    if [
        "authorization:",
        "bearer ",
        "github_pat_",
        "ghp_",
        "gho_",
        "ghs_",
        "-----begin private key",
        "password=",
        "token=",
        "secret=",
        "\"password\"",
        "\"token\"",
        "\"secret\"",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || line.split_whitespace().any(|word| {
            word.split_once("://").is_some_and(|(_, url)| {
                url.split('/').next().is_some_and(|host| host.contains('@')) || url.contains('?')
            })
        })
    {
        crate::localization::LOG_REDACTED
    } else {
        line
    }
}

// A child may emit arbitrary terminal control sequences. Strip them before
// putting the captured text through the TUI painter.
pub(crate) fn sanitize_console_output(bytes: &[u8]) -> String {
    enum State {
        Text,
        Escape,
        Intermediate,
        Csi,
        ControlString { osc: bool, escape: bool },
    }
    let control_string = |osc| State::ControlString { osc, escape: false };
    let chars = bytes.utf8_chunks().flat_map(|chunk| {
        let invalid = match chunk.invalid() {
            [] => None,
            [control @ 0x80..=0x9f] => Some(char::from(*control)),
            _ => Some('\u{fffd}'),
        };
        chunk.valid().chars().chain(invalid)
    });
    let mut output = String::with_capacity(bytes.len());
    let mut state = State::Text;
    let mut progress = false;
    for ch in chars {
        state = match (state, ch) {
            (State::ControlString { osc, escape }, ch) => {
                if ch == '\u{9c}' || (escape && ch == '\\') || (osc && ch == '\x07') {
                    State::Text
                } else {
                    State::ControlString {
                        osc,
                        escape: ch == '\x1b',
                    }
                }
            }
            (_, '\x1b') => State::Escape,
            (State::Escape, '[') | (_, '\u{9b}') => State::Csi,
            (State::Escape, ']') | (_, '\u{9d}') => control_string(true),
            (State::Escape, 'P' | 'X' | '^' | '_')
            | (_, '\u{90}' | '\u{98}' | '\u{9e}' | '\u{9f}') => control_string(false),
            (State::Escape | State::Intermediate, '\x20'..='\x2f') => State::Intermediate,
            (State::Escape | State::Intermediate, _) => State::Text,
            (State::Csi, '@'..='~') => State::Text,
            (State::Csi, _) => State::Csi,
            (State::Text, '\r') => {
                progress = true;
                State::Text
            }
            (State::Text, ch) => {
                if matches!(ch, '\n' | '\t') || !ch.is_control() {
                    if std::mem::take(&mut progress) && ch != '\n' {
                        output.push('\n');
                    }
                    if ch == '\t' {
                        output.push_str("    ");
                    } else {
                        output.push(ch);
                    }
                }
                State::Text
            }
        };
    }
    let mut redacted = String::with_capacity(output.len());
    let mut private_key = false;
    for record in output.split_inclusive('\n') {
        let line = record.strip_suffix('\n').unwrap_or(record);
        if private_key {
            private_key = !(line.contains("-----END ") && line.contains("PRIVATE KEY-----"));
            continue;
        }
        private_key = line
            .split_once("-----BEGIN ")
            .is_some_and(|(_, marker)| marker.contains("PRIVATE KEY-----"));
        if private_key {
            redacted.push_str(crate::localization::LOG_REDACTED);
        } else {
            redacted.push_str(redact_sensitive_line(line));
        }
        if record.ends_with('\n') {
            redacted.push('\n');
        }
    }
    redacted
}

pub(crate) fn isolate_git(command: &mut Command) {
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CEILING_DIRECTORIES",
        "GIT_DISCOVERY_ACROSS_FILESYSTEM",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
    ] {
        command.env_remove(name);
    }
}

pub fn capture(command: &mut Command) -> Result<String> {
    let label = command.get_program().to_string_lossy().into_owned();
    let result = command
        .output()
        .with_context(|| format!("{} {label}", message(START_FAILED)))?;
    if !result.status.success() {
        let diagnostic = String::from_utf8_lossy(&result.stderr);
        bail!(
            "{label} {} {} · {}",
            message(["exited with", "đã thoát với"]),
            result.status,
            diagnostic.trim()
        );
    }
    Ok(String::from_utf8(result.stdout)?.trim().to_owned())
}

pub fn run(command: &mut Command) -> Result<()> {
    let label = command.get_program().to_string_lossy().into_owned();
    if JSON_OUTPUT.get() {
        // stdout is reserved for the CLI's single structured report.
        command.stdout(Stdio::null());
    }
    let status = command
        .status()
        .with_context(|| format!("{} {label}", message(START_FAILED)))?;
    if !status.success() {
        bail!(
            "{label} {} {status}",
            message(["exited with", "đã thoát với"])
        );
    }
    Ok(())
}

pub fn powershell() -> Command {
    let mut command = Command::new("pwsh.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ]);
    command
}

pub fn hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    let _ = command;
}

pub fn isolate_stdio() -> Result<()> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::{HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation},
            System::Console::{
                GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
            },
        };

        // Windows may inherit the caller's pipe handles in addition to the file
        // handles attached to the supervisor. Close inheritance of the former.
        for kind in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let handle = unsafe { GetStdHandle(kind) };
            if !handle.is_null()
                && handle != INVALID_HANDLE_VALUE
                && unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0
            {
                return Err(io::Error::last_os_error()).context(crate::localization::message([
                    "Cannot isolate the caller's standard handles",
                    "Không thể cô lập các handle chuẩn của tiến trình gọi",
                ]));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_stream_fixture() {
        let Ok(mode) = std::env::var("CNL_PROCESS_OUTPUT_FIXTURE") else {
            return;
        };
        if mode == "child" {
            println!("normal progress");
            eprintln!("actual diagnostic");
            return;
        }
        set_json_output(mode == "json");
        run(Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process::tests::child_stream_fixture",
                "--nocapture",
            ])
            .env("CNL_PROCESS_OUTPUT_FIXTURE", "child"))
        .unwrap();
    }

    #[test]
    fn progress_and_diagnostics_stay_separate_with_json_stdout_reserved() {
        for mode in ["normal", "json"] {
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "process::tests::child_stream_fixture",
                    "--nocapture",
                ])
                .env("CNL_PROCESS_OUTPUT_FIXTURE", mode)
                .output()
                .unwrap();
            assert!(output.status.success());
            let stdout = String::from_utf8(output.stdout).unwrap();
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert_eq!(stdout.contains("normal progress"), mode == "normal");
            assert!(!stderr.contains("normal progress"));
            assert!(!stdout.contains("actual diagnostic"));
            assert!(stderr.contains("actual diagnostic"));
        }
    }
}
