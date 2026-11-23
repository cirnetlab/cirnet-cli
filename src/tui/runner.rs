// Derived from CIRNET Rules CNR, commit 646afa38516bbfd276b0aecb7a138ab427df5fa9 (GPL-3.0).
use std::{
    io::{self, Read},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use clap::ArgMatches;

use super::viewport::Buffer;
use crate::{process::hidden, workspace::Workspace};

const STREAM_LIMIT: usize = 128 * 1024;
const LOG_FRAME_INTERVAL: Duration = Duration::from_millis(160);

pub(super) enum Outcome {
    Pass,
    Fail,
    Cancelled,
}

impl Outcome {
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Cancelled => "CANCELLED",
        }
    }
}

pub(super) struct RunOutput {
    pub(super) outcome: Outcome,
    pub(super) exit_code: Option<i32>,
    pub(super) elapsed_ms: u128,
}

pub(super) struct Update {
    pub(super) buffer: Option<Buffer>,
    pub(super) result: Option<RunOutput>,
}

#[derive(Default)]
struct Stream {
    bytes: Vec<u8>,
    truncated: bool,
}

impl Stream {
    fn append(&mut self, bytes: &[u8]) {
        if bytes.len() >= STREAM_LIMIT {
            self.bytes.clear();
            self.bytes
                .extend_from_slice(&bytes[bytes.len() - STREAM_LIMIT..]);
            self.truncated = true;
            return;
        }
        let excess = (self.bytes.len() + bytes.len()).saturating_sub(STREAM_LIMIT);
        if excess > 0 {
            self.bytes.drain(..excess);
            self.truncated = true;
        }
        self.bytes.extend_from_slice(bytes);
    }
}

#[derive(Default)]
struct Capture {
    stdout: Stream,
    stderr: Stream,
    dirty: bool,
}

pub(super) struct Runner {
    child: Child,
    job: Option<ProcessJob>,
    capture: Arc<Mutex<Capture>>,
    readers: [JoinHandle<()>; 2],
    exit: Option<ExitStatus>,
    cancelled: bool,
    started: Instant,
    rendered: Instant,
    vi: bool,
}

impl Runner {
    pub(super) fn start(
        workspace: &Workspace,
        flags: &ArgMatches,
        args: &[String],
    ) -> Result<Self> {
        let parsed = crate::cli::app()
            .try_get_matches_from(std::iter::once("cnl").chain(args.iter().map(String::as_str)));
        let action = parsed.as_ref().ok().and_then(ArgMatches::subcommand_name);
        if action == Some("tui") {
            bail!("The CIRNETLAB terminal is already open.");
        }
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("--workspace")
            .arg(&workspace.root)
            .arg("--profile")
            .arg(flags.get_one::<String>("profile").expect("default profile"));
        if let Some(path) = flags.get_one::<PathBuf>("state-dir") {
            command.arg("--state-dir").arg(path);
        }
        let vi = flags
            .get_one::<String>("lang")
            .is_some_and(|language| language == "vi");
        command.args(args).env("NO_COLOR", "1");
        Self::spawn(command, !matches!(action, Some("run" | "dev" | "stop")), vi)
    }

    fn spawn(mut command: Command, cancellable: bool, vi: bool) -> Result<Self> {
        hidden(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        );
        let mut child = command.spawn()?;
        let job = if cancellable {
            ProcessJob::attach(&child).ok()
        } else {
            None
        };
        let capture = Arc::new(Mutex::new(Capture::default()));
        let readers = [
            Self::reader(
                child.stdout.take().expect("piped stdout"),
                capture.clone(),
                false,
            ),
            Self::reader(
                child.stderr.take().expect("piped stderr"),
                capture.clone(),
                true,
            ),
        ];
        let started = Instant::now();
        Ok(Self {
            child,
            job,
            capture,
            readers,
            exit: None,
            cancelled: false,
            started,
            rendered: started,
            vi,
        })
    }

    fn reader(
        mut pipe: impl Read + Send + 'static,
        capture: Arc<Mutex<Capture>>,
        stderr: bool,
    ) -> JoinHandle<()> {
        thread::spawn(move || {
            let mut buffer = [0; 16384];
            loop {
                let count = match pipe.read(&mut buffer) {
                    Ok(0) => return,
                    Ok(count) => count,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => return,
                };
                let mut capture = capture.lock().expect("output capture");
                let stream = if stderr {
                    &mut capture.stderr
                } else {
                    &mut capture.stdout
                };
                stream.append(&buffer[..count]);
                capture.dirty = true;
            }
        })
    }

    pub(super) fn poll(&mut self) -> io::Result<Update> {
        if self.exit.is_none() {
            self.exit = self.child.try_wait()?;
        }
        let finished = self
            .exit
            .filter(|_| self.readers.iter().all(JoinHandle::is_finished));
        let buffer = if finished.is_some() || self.rendered.elapsed() >= LOG_FRAME_INTERVAL {
            let mut capture = self.capture.lock().expect("output capture");
            self.rendered = Instant::now();
            if std::mem::take(&mut capture.dirty) {
                let stdout = capture.stdout.bytes.clone();
                let stderr = capture.stderr.bytes.clone();
                let truncated = capture.stdout.truncated || capture.stderr.truncated;
                drop(capture);
                let notice = super::text(
                    [
                        "Earlier output was omitted.",
                        "Đã lược bớt phần đầu của kết quả.",
                    ],
                    self.vi,
                );
                Some(Buffer::from_streams(
                    sanitize(&stdout),
                    &sanitize(&stderr),
                    truncated.then_some(notice),
                ))
            } else {
                None
            }
        } else {
            None
        };
        let result = finished.map(|exit| RunOutput {
            outcome: if self.cancelled {
                Outcome::Cancelled
            } else if exit.success() {
                Outcome::Pass
            } else {
                Outcome::Fail
            },
            exit_code: exit.code(),
            elapsed_ms: self.started.elapsed().as_millis(),
        });
        Ok(Update { buffer, result })
    }

    pub(super) fn cancel(&mut self) -> Result<(), &'static str> {
        if self.cancelled || self.exit.is_some() {
            return Ok(());
        }
        let Some(job) = self.job.take() else {
            return Err(super::text(
                [
                    "Wait for this command to finish. Use stop for the application.",
                    "Đợi lệnh hoàn tất. Dùng stop để dừng ứng dụng.",
                ],
                self.vi,
            ));
        };
        self.cancelled = true;
        drop(job);
        Ok(())
    }
}

// A child may emit arbitrary terminal control sequences. Strip them before
// putting the captured text through the TUI painter.
pub(super) fn sanitize(bytes: &[u8]) -> String {
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
    output
}

#[cfg(windows)]
struct ProcessJob(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl ProcessJob {
    fn attach(child: &Child) -> Result<Self> {
        use std::{mem, os::windows::io::AsRawHandle, ptr};
        use windows_sys::Win32::System::JobObjects::*;
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let guard = Self(handle);
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
            || unsafe { AssignProcessToJobObject(handle, child.as_raw_handle()) } == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        Ok(guard)
    }
}

#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}
#[cfg(not(windows))]
struct ProcessJob;
#[cfg(not(windows))]
impl ProcessJob {
    fn attach(_: &Child) -> Result<Self> {
        bail!("Process tree cancellation is available on Windows")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_fixture() {
        use std::io::Write;
        match std::env::var("CNL_RUNNER_TEST").as_deref() {
            Ok("stream") => {
                let mut output = io::stdout().lock();
                for _ in 0..128 {
                    output.write_all(&[b'x'; 4096]).unwrap();
                }
                writeln!(output, "\nfinal stdout").unwrap();
                eprintln!("final stderr");
            }
            Ok("wait") => {
                println!("worker ready");
                thread::sleep(Duration::from_secs(30));
            }
            _ => {}
        }
    }

    fn fixture(mode: &str) -> Runner {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "tui::runner::tests::child_fixture",
                "--nocapture",
            ])
            .env("CNL_RUNNER_TEST", mode);
        Runner::spawn(command, true, false).unwrap()
    }

    fn result(runner: &mut Runner) -> RunOutput {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(result) = runner.poll().unwrap().result {
                return result;
            }
            assert!(Instant::now() < deadline, "worker did not exit");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn large_output_is_bounded_and_keeps_both_stream_tails() {
        let mut runner = fixture("stream");
        assert!(matches!(result(&mut runner).outcome, Outcome::Pass));
        let capture = runner.capture.lock().unwrap();
        assert!(capture.stdout.truncated);
        assert_eq!(capture.stdout.bytes.len(), STREAM_LIMIT);
        assert!(sanitize(&capture.stdout.bytes).contains("final stdout"));
        assert!(sanitize(&capture.stderr.bytes).contains("final stderr"));
    }

    #[cfg(windows)]
    #[test]
    fn cancellation_stops_the_owned_worker() {
        let mut runner = fixture("wait");
        runner.cancel().unwrap();
        assert!(matches!(result(&mut runner).outcome, Outcome::Cancelled));
    }
}
