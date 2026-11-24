use std::{
    io::{self, Read},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, bail};
use clap::ArgMatches;

use super::viewport::Buffer;
pub(super) use crate::process::sanitize_console_output as sanitize;
use crate::{
    localization::terminal as labels,
    process::{ProcessJob, hidden},
    workspace::Workspace,
};

const STREAM_LIMIT: usize = 128 * 1024;
const LOG_FRAME_INTERVAL: Duration = Duration::from_millis(160);
pub(super) enum Outcome {
    Pass,
    Fail,
    Cancelled,
}

pub(super) struct RunOutput {
    pub(super) outcome: Outcome,
    pub(super) exit_code: Option<i32>,
    pub(super) elapsed_ms: u128,
}

pub(super) struct Update {
    pub(super) captured: Option<CapturedOutput>,
    pub(super) result: Option<RunOutput>,
}

pub(super) struct CapturedOutput {
    stdout: String,
    stderr: String,
    report: Option<serde_json::Value>,
    truncated: bool,
}

impl CapturedOutput {
    fn new(stdout: &[u8], stderr: &[u8], truncated: bool, marker: Option<&str>) -> Self {
        let mut report = None;
        let mut external = Vec::new();
        let stdout = if let Some(marker) = marker {
            for line in stdout.split_inclusive(|byte| *byte == b'\n') {
                if let Some(body) = line.strip_prefix(marker.as_bytes()) {
                    if !line.ends_with(b"\n") {
                        continue;
                    }
                    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body)
                        && value.is_object()
                    {
                        report = Some(value);
                        continue;
                    }
                }
                external.extend_from_slice(line);
            }
            external.as_slice()
        } else {
            stdout
        };
        Self {
            stdout: sanitize(stdout),
            stderr: sanitize(stderr),
            report,
            truncated,
        }
    }

    pub(super) fn render(&self, vi: bool) -> Buffer {
        let mut stdout = self.stdout.clone();
        if let Some(report) = &self.report {
            if !stdout.is_empty() && !stdout.ends_with('\n') {
                stdout.push('\n');
            }
            stdout.push_str(&sanitize(crate::cli::human_report(report, vi).as_bytes()));
        }
        Buffer::from_streams(
            stdout,
            &self.stderr,
            self.truncated
                .then_some(super::text(labels::OUTPUT_OMITTED, vi)),
        )
    }
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
    report_marker: Option<String>,
}

impl Runner {
    pub(super) fn start(
        workspace: &Workspace,
        flags: &ArgMatches,
        args: &[String],
        vi: bool,
    ) -> Result<Self> {
        let parsed = crate::cli::app()
            .try_get_matches_from(std::iter::once("cnl").chain(args.iter().map(String::as_str)));
        let action = parsed.as_ref().ok().and_then(ArgMatches::subcommand_name);
        if action == Some("tui") {
            bail!(
                "{}",
                crate::localization::message(labels::TERMINAL_ALREADY_OPEN)
            );
        }
        let mut command = Command::new(std::env::current_exe()?);
        let supplied = |name: &str| {
            parsed.as_ref().is_ok_and(|matches| {
                matches.value_source(name) == Some(clap::parser::ValueSource::CommandLine)
            })
        };
        if !supplied("workspace") {
            command.arg("--workspace").arg(&workspace.root);
        }
        if !supplied("profile") {
            command
                .arg("--profile")
                .arg(flags.get_one::<String>("profile").expect("default profile"));
        }
        if !supplied("state-dir")
            && let Some(path) = flags.get_one::<PathBuf>("state-dir")
        {
            command.arg("--state-dir").arg(path);
        }
        if !supplied("lang") {
            command.args(["--lang", if vi { "vi" } else { "en" }]);
        }
        let report_nonce = parsed
            .as_ref()
            .is_ok_and(|matches| !matches.get_flag("json") && action != Some("logs"))
            .then(|| {
                let instant = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default();
                format!("{}x{}", std::process::id(), instant.as_nanos())
            });
        command
            .args(args)
            .env("NO_COLOR", "1")
            .env_remove("CNL_TUI_REPORT");
        if let Some(nonce) = &report_nonce {
            command.env("CNL_TUI_REPORT", nonce);
        }
        let mut worker = Self::spawn(command, !matches!(action, Some("run" | "dev" | "stop")), vi)?;
        worker.report_marker =
            report_nonce.map(|nonce| format!("{}{nonce} ", crate::cli::TUI_REPORT_MARKER));
        Ok(worker)
    }

    pub(super) fn start_native(
        line: &str,
        vi: bool,
        directory: Option<&std::path::Path>,
        selected_shell: Option<&str>,
    ) -> Result<Self> {
        let mut command = crate::cli::native::line(line, selected_shell, directory, vi, |input| {
            super::completion::arguments(input).ok()
        })?;
        command.env_remove("CNL_TUI_REPORT");
        Self::spawn(command, true, vi)
    }

    fn spawn(mut command: Command, cancellable: bool, vi: bool) -> Result<Self> {
        #[cfg(unix)]
        if cancellable {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        hidden(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        );
        let mut child = command.spawn().map_err(crate::cli::native::unavailable)?;
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
            report_marker: None,
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

    pub(super) fn set_language(&mut self, vi: bool) {
        if self.vi != vi {
            self.vi = vi;
        }
    }

    pub(super) fn poll(&mut self) -> io::Result<Update> {
        if self.exit.is_none() {
            self.exit = self.child.try_wait()?;
        }
        let finished = self
            .exit
            .filter(|_| self.readers.iter().all(JoinHandle::is_finished));
        let captured = if finished.is_some() || self.rendered.elapsed() >= LOG_FRAME_INTERVAL {
            let mut capture = self.capture.lock().expect("output capture");
            self.rendered = Instant::now();
            if std::mem::take(&mut capture.dirty) {
                let stdout = capture.stdout.bytes.clone();
                let stderr = capture.stderr.bytes.clone();
                let truncated = capture.stdout.truncated || capture.stderr.truncated;
                drop(capture);
                Some(CapturedOutput::new(
                    &stdout,
                    &stderr,
                    truncated,
                    self.report_marker.as_deref(),
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
        Ok(Update { captured, result })
    }

    pub(super) fn cancel(&mut self) -> Result<(), &'static str> {
        if self.cancelled || self.exit.is_some() {
            return Ok(());
        }
        let Some(job) = self.job.take() else {
            return Err(super::text(labels::CANNOT_CANCEL, self.vi));
        };
        self.cancelled = true;
        drop(job);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_reports_relocalize_without_reinterpreting_raw_tool_output() {
        let marker = format!("{}fixture ", crate::cli::TUI_REPORT_MARKER);
        let line = format!(
            "external tool text\n{}raw external text\n{marker}{{\"state\":\"stopped\"}}\n",
            crate::cli::TUI_REPORT_MARKER
        );
        let output = CapturedOutput::new(
            line.as_bytes(),
            b"external stderr text\n",
            false,
            Some(&marker),
        );
        let en = output.render(false);
        let vi = output.render(true);
        assert!(en.find("Stopped", None, false).is_some());
        assert!(vi.find("Đã dừng", None, false).is_some());
        for buffer in [en, vi] {
            assert!(buffer.find("external tool text", None, false).is_some());
            assert!(buffer.find("external stderr text", None, false).is_some());
            assert!(buffer.find("raw external text", None, false).is_some());
            assert!(buffer.find(&marker, None, false).is_none());
        }
        let raw = CapturedOutput::new(line.as_bytes(), b"", false, None).render(false);
        assert!(
            raw.find(crate::cli::TUI_REPORT_MARKER, None, false)
                .is_some()
        );
    }

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
