use std::cell::Cell;
use std::io;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

thread_local! {
    static JSON_OUTPUT: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn set_json_output(enabled: bool) {
    JSON_OUTPUT.set(enabled);
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
        .with_context(|| format!("Could not start {label}"))?;
    if !result.status.success() {
        let diagnostic = String::from_utf8_lossy(&result.stderr);
        bail!(
            "{label} exited with {}: {}",
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
        .with_context(|| format!("Could not start {label}"))?;
    if !status.success() {
        bail!("{label} exited with {status}");
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
                return Err(io::Error::last_os_error())
                    .context("Cannot isolate the caller's standard handles");
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
