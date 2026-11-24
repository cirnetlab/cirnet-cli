use std::cell::Cell;
#[cfg(windows)]
use std::io;
use std::process::{Child, Command, ExitStatus, Stdio};

use crate::localization::{self, Diagnostic, START_FAILED, message, text};
use anyhow::{Context, Result, bail};

thread_local! {
    static JSON_OUTPUT: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn set_json_output(enabled: bool) {
    JSON_OUTPUT.set(enabled);
}

pub(crate) fn json_output() -> bool {
    JSON_OUTPUT.get()
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
        .env_remove("CNL_TUI_REPORT")
        .output()
        .with_context(|| Diagnostic::new(|vi| format!("{} {label}", text(START_FAILED, vi))))?;
    if !result.status.success() {
        let diagnostic = String::from_utf8_lossy(&result.stderr);
        bail!(Diagnostic::new(|vi| format!(
            "{label} {} {} · {}",
            text(localization::PROCESS_EXITED, vi),
            result.status,
            diagnostic.trim()
        )));
    }
    Ok(String::from_utf8(result.stdout)?.trim().to_owned())
}

pub fn run(command: &mut Command) -> Result<()> {
    let label = command.get_program().to_string_lossy().into_owned();
    command.env_remove("CNL_TUI_REPORT");
    if json_output() {
        // stdout is reserved for the CLI's single structured report.
        command.stdout(Stdio::null());
    }
    let status = command
        .status()
        .with_context(|| format!("{} {label}", message(START_FAILED)))?;
    if !status.success() {
        bail!("{label} {} {status}", message(localization::PROCESS_EXITED));
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

/// Capture fixed metadata queries with a shared process, byte, and time budget.
/// The caller chooses trusted executables and supplies an argv that only reads data.
pub(crate) fn metadata_output(
    command: &mut Command,
    timeout: std::time::Duration,
    limit: usize,
) -> Option<(ExitStatus, Vec<u8>)> {
    use std::{
        io::Read,
        sync::{Arc, Condvar, Mutex},
        thread,
        time::{Duration, Instant},
    };
    const MAX_PROCESSES: usize = 4;
    static READERS: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());
    struct Permit;
    impl Drop for Permit {
        fn drop(&mut self) {
            *READERS.0.lock().expect("metadata process count") -= 1;
            READERS.1.notify_one();
        }
    }

    let deadline = Instant::now() + timeout;
    let (mut active, _) = READERS
        .1
        .wait_timeout_while(READERS.0.lock().ok()?, timeout, |active| {
            *active >= MAX_PROCESSES
        })
        .ok()?;
    if *active >= MAX_PROCESSES || Instant::now() >= deadline {
        return None;
    }
    *active += 1;
    drop(active);
    let permit = Arc::new(Permit);
    let reader_permit = Arc::clone(&permit);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    hidden(
        command
            .env_remove("CNL_TUI_REPORT")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::piped()),
    );
    let mut child = command.spawn().ok()?;
    let job = ProcessJob::attach(&child).ok();
    let mut pipe = child.stdout.take().expect("piped metadata stdout");
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let reader = thread::Builder::new()
        .name("cnl-metadata-output".into())
        .spawn(move || {
            let _permit = reader_permit;
            let mut output = Vec::new();
            let result = pipe
                .by_ref()
                .take(limit.saturating_add(1) as u64)
                .read_to_end(&mut output);
            let _ = sender.send(
                result
                    .ok()
                    .filter(|_| output.len() <= limit)
                    .map(|_| output),
            );
        });

    let result = if reader.is_ok() {
        loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                _ => break None,
            }
        }
    } else {
        None
    };
    drop(job);
    let Some(status) = result else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let output = receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .ok()??;
    Some((status, output))
}

#[cfg(windows)]
pub(crate) struct ProcessJob {
    _handle: std::os::windows::io::OwnedHandle,
}

#[cfg(windows)]
impl ProcessJob {
    pub(crate) fn attach(child: &Child) -> Result<Self> {
        use std::os::windows::io::AsRawHandle;
        Self::attach_handle(child.as_raw_handle())
    }

    pub(crate) fn attach_handle(child: std::os::windows::io::RawHandle) -> Result<Self> {
        use std::{mem, os::windows::io::FromRawHandle, ptr};
        use windows_sys::Win32::System::JobObjects::*;
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        // The newly created job handle was checked for null and has a single owner.
        let guard = Self {
            _handle: unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) },
        };
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                mem::size_of_val(&limits) as u32,
            )
        } == 0
            || unsafe { AssignProcessToJobObject(handle, child) } == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        Ok(guard)
    }
}

#[cfg(unix)]
pub(crate) struct ProcessJob(libc::pid_t);

#[cfg(unix)]
impl ProcessJob {
    pub(crate) fn attach(child: &Child) -> Result<Self> {
        Ok(Self(child.id().try_into()?))
    }
}

#[cfg(unix)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        // Cancellable children were started as leaders of their own process groups.
        unsafe { libc::kill(-self.0, libc::SIGKILL) };
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) struct ProcessJob;
#[cfg(not(any(unix, windows)))]
impl ProcessJob {
    pub(crate) fn attach(_: &Child) -> Result<Self> {
        bail!(
            "{}",
            crate::localization::message(crate::localization::terminal::CANCELLATION_WINDOWS_ONLY)
        )
    }
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
                    .context(message(localization::STDIO_ISOLATION_FAILED));
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
    fn bounded_metadata_fixture() {
        match std::env::var("CNL_BOUNDED_METADATA_FIXTURE").as_deref() {
            Ok("overflow") => println!("{}", "a".repeat(16 * 1024)),
            Ok("timeout") => std::thread::sleep(std::time::Duration::from_secs(5)),
            Ok("nonzero") => {
                println!("metadata");
                std::process::exit(17);
            }
            _ => {}
        }
    }

    #[test]
    fn metadata_preserves_status_and_enforces_byte_and_time_limits() {
        use std::time::{Duration, Instant};
        let fixture = |mode| {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "process::tests::bounded_metadata_fixture",
                    "--nocapture",
                ])
                .env("CNL_BOUNDED_METADATA_FIXTURE", mode);
            command
        };
        let timeout = Duration::from_secs(3);
        let (status, output) = metadata_output(&mut fixture("nonzero"), timeout, 1024).unwrap();
        assert_eq!(status.code(), Some(17));
        assert!(String::from_utf8_lossy(&output).contains("metadata"));
        assert!(metadata_output(&mut fixture("overflow"), timeout, 1024).is_none());
        let start = Instant::now();
        assert!(
            metadata_output(&mut fixture("timeout"), Duration::from_millis(150), 1024).is_none()
        );
        assert!(start.elapsed() < Duration::from_secs(3));
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
