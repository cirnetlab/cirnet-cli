use super::{Output, Result, locale};
use anyhow::{Context, anyhow, ensure};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    io::{self, Read, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[cfg(unix)]
#[path = "unix_io.rs"]
mod unix_io;
#[cfg(unix)]
use unix_io::pty_streams;

pub(super) const MAX_PASTE: usize = 32 * 1024;
const INPUT_PACKETS: usize = 32;
type PtyReader = Box<dyn Read + Send>;
type PtyWriter = Box<dyn Write + Send>;

struct IoStop {
    #[cfg(unix)]
    _signal: std::os::unix::net::UnixStream,
}

#[cfg(windows)]
fn pty_streams(master: &dyn MasterPty) -> Result<(PtyReader, PtyWriter, IoStop)> {
    Ok((master.try_clone_reader()?, master.take_writer()?, IoStop {}))
}

pub(super) struct Backend {
    input: SyncSender<Vec<u8>>,
    resize: Option<SyncSender<PtySize>>,
    monitor: Option<JoinHandle<()>>,
    pub(super) output: Arc<Mutex<Output>>,
    #[cfg(all(test, windows))]
    pub(super) process_id: u32,
}

impl Backend {
    pub(super) fn start(shell: &str, cwd: &Path, size: PtySize, vi: bool) -> Result<Self> {
        validate_directory(cwd, vi)?;
        #[cfg(windows)]
        if shell == "cmd" {
            use std::path::{Component, Prefix};
            ensure!(
                !matches!(cwd.components().next(), Some(Component::Prefix(prefix))
                    if matches!(prefix.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..))),
                "{}",
                locale(
                    [
                        "CMD cannot use a UNC working directory. Select PowerShell for this checkout.",
                        "CMD không thể dùng thư mục làm việc UNC. Hãy chọn PowerShell cho checkout này.",
                    ],
                    vi
                )
            );
        }
        let command = command(shell, cwd, vi)?;
        let pair = native_pty_system().openpty(size).context(locale(
            [
                "The interactive terminal could not be allocated",
                "Không thể cấp terminal tương tác",
            ],
            vi,
        ))?;
        let (reader, writer, stop) = pty_streams(pair.master.as_ref())?;
        let output = Arc::new(Mutex::new(Output::new(size)));
        let (input, packets) = mpsc::sync_channel(INPUT_PACKETS);
        let (resize, sizes) = mpsc::sync_channel(1);
        let (process, launched) = mpsc::sync_channel::<Process>(0);

        let shared = Arc::clone(&output);
        let replies = input.clone();
        thread::Builder::new()
            .name("cnl-pty-output".into())
            .spawn(move || read_output(reader, &shared, &replies))?;
        let shared = Arc::clone(&output);
        thread::Builder::new()
            .name("cnl-pty-input".into())
            .spawn(move || write_input(writer, packets, &shared))?;
        let shared = Arc::clone(&output);
        let monitor = thread::Builder::new()
            .name("cnl-pty-child".into())
            .spawn(move || {
                if let Ok(mut process) = launched.recv() {
                    process.monitor(sizes, &shared);
                }
            })?;

        // ConPTY can produce output during process creation. Its output reader
        // must be running before the first child is launched.
        validate_directory(cwd, vi)?;
        let child = pair.slave.spawn_command(command).with_context(|| {
            format!(
                "{} {shell}. {}",
                locale(crate::localization::SHELL_START_FAILED, vi),
                locale(crate::localization::SHELL_INSTALL, vi)
            )
        })?;
        #[cfg(all(test, windows))]
        let process_id = child.process_id().expect("ConPTY returned a process id");
        drop(pair.slave);
        process
            .send(Process::new(pair.master, child, stop))
            .map_err(|_| {
                anyhow!(locale(
                    ["Terminal worker ended", "Tiến trình terminal đã dừng"],
                    vi
                ))
            })?;
        Ok(Self {
            input,
            resize: Some(resize),
            monitor: Some(monitor),
            output,
            #[cfg(all(test, windows))]
            process_id,
        })
    }

    pub(super) fn send(&self, packet: Vec<u8>) -> bool {
        self.input.try_send(packet).is_ok()
    }

    pub(super) fn request_size(&self, size: PtySize) -> bool {
        let Some(resize) = &self.resize else {
            return false;
        };
        match resize.try_send(size) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => false,
            Err(TrySendError::Disconnected(_)) => {
                self.output.lock().unwrap().resize(size, false);
                true
            }
        }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.resize.take();
        if let Some(monitor) = self.monitor.take() {
            let _ = monitor.join();
        }
    }
}

fn validate_directory(cwd: &Path, vi: bool) -> Result<()> {
    ensure!(
        cwd.is_dir() && std::fs::canonicalize(cwd)? == cwd,
        "{}",
        locale(
            [
                "The repository directory changed or is unavailable",
                "Thư mục kho đã thay đổi hoặc không khả dụng",
            ],
            vi
        )
    );
    Ok(())
}

fn command(shell: &str, cwd: &Path, vi: bool) -> Result<CommandBuilder> {
    let template = crate::cli::native::shell(Some(shell))?;
    let shell_cwd = if cfg!(windows) && matches!(shell, "cmd" | "wsl") {
        // CMD treats a verbatim drive prefix like a network path. WSL's
        // --cd adapter also expects the standard Windows path spelling.
        let standard = crate::workspace::canonical(cwd)?;
        ensure!(
            std::fs::canonicalize(&standard)? == cwd,
            "{}",
            locale(
                [
                    "The selected shell cannot use this working directory",
                    "Shell đã chọn không thể dùng thư mục làm việc này"
                ],
                vi
            )
        );
        std::borrow::Cow::Owned(standard)
    } else if cfg!(windows)
        && shell == "powershell"
        && let Some(original) = cwd.to_str()
    {
        // PowerShell exposes verbatim paths as provider-qualified locations.
        // Keep the original path when the standard spelling cannot reach the same directory.
        let visible = crate::workspace::user_path(cwd);
        let standard = Path::new(visible.as_ref());
        if visible != original && std::fs::canonicalize(standard).is_ok_and(|path| path == cwd) {
            std::borrow::Cow::Owned(standard.to_path_buf())
        } else {
            std::borrow::Cow::Borrowed(cwd)
        }
    } else {
        std::borrow::Cow::Borrowed(cwd)
    };
    let mut command = CommandBuilder::new(template.get_program());
    if shell == "wsl" {
        // WSL accepts an absolute Windows path for --cd and requires it
        // before --exec. Keep it one argv value, including embedded spaces.
        command.arg("--cd");
        command.arg(shell_cwd.as_ref());
        command.args([
            "--exec",
            "env",
            "HISTFILE=",
            "bash",
            "--noprofile",
            "--norc",
        ]);
    } else {
        command.args(template.get_args());
    }
    command.cwd(shell_cwd.as_ref());
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env_remove("CNL_TUI_REPORT");
    match shell {
        "bash" => {
            command.env("HISTFILE", "");
            command.args(["--noprofile", "--norc"]);
        }
        "powershell" => {
            command.args([
                "-NoExit",
                "-Command",
                "Import-Module PSReadLine -ErrorAction SilentlyContinue; if (Get-Module PSReadLine) { Set-PSReadLineOption -HistorySaveStyle SaveNothing }",
            ]);
        }
        _ => {}
    }
    Ok(command)
}

fn read_output(mut reader: PtyReader, output: &Mutex<Output>, replies: &SyncSender<Vec<u8>>) {
    let mut chunk = [0_u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => return,
            Ok(count) => {
                let packets = {
                    let mut output = output.lock().unwrap();
                    output.process(&chunk[..count])
                };
                for packet in packets {
                    let _ = replies.try_send(packet);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

fn write_input(mut writer: PtyWriter, packets: Receiver<Vec<u8>>, output: &Mutex<Output>) {
    for packet in packets {
        if writer
            .write_all(&packet)
            .and_then(|()| writer.flush())
            .is_err()
        {
            let mut output = output.lock().unwrap();
            output.input_failed = true;
            output.changed();
            return;
        }
    }
}

struct Process {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    exited: bool,
    _io_stop: IoStop,
    #[cfg(windows)]
    job: Option<crate::tui::runner::ProcessJob>,
}

impl Process {
    fn new(
        master: Box<dyn MasterPty + Send>,
        child: Box<dyn Child + Send + Sync>,
        stop: IoStop,
    ) -> Self {
        #[cfg(windows)]
        let job = child
            .as_raw_handle()
            .and_then(|handle| crate::tui::runner::ProcessJob::attach_handle(handle).ok());
        Self {
            master,
            child,
            exited: false,
            _io_stop: stop,
            #[cfg(windows)]
            job,
        }
    }

    fn monitor(&mut self, sizes: Receiver<PtySize>, output: &Mutex<Output>) {
        loop {
            match self.child.try_wait() {
                Ok(None) => {}
                Ok(Some(status)) => {
                    self.exited = true;
                    let mut output = output.lock().unwrap();
                    output.exit = Some(Ok(status));
                    output.changed();
                    return;
                }
                Err(error) => {
                    let mut output = output.lock().unwrap();
                    output.exit = Some(Err(error.to_string()));
                    output.changed();
                    return;
                }
            }
            match sizes.recv_timeout(Duration::from_millis(150)) {
                Ok(size) => {
                    // Publish the new parser size only after the native resize returns.
                    let failed = self.master.resize(size).is_err();
                    output.lock().unwrap().resize(size, failed);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        #[cfg(windows)]
        self.job.take();
        if !self.exited {
            #[cfg(unix)]
            if let Some(foreground) = self.master.process_group_leader()
                && u32::try_from(foreground).ok() != self.child.process_id()
            {
                // This is the foreground group of our still-owned PTY.
                let _ = unsafe { libc::kill(-foreground, libc::SIGHUP) };
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
