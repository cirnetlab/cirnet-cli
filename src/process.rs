use std::io::{self, Read, Write};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

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
    let mut child = command
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("Could not start {label}"))?;
    let mut output = child.stdout.take().expect("piped stdout");
    let mut bytes = [0; 8192];
    loop {
        let count = output.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        io::stderr().lock().write_all(&bytes[..count])?;
    }
    let status = child.wait()?;
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
