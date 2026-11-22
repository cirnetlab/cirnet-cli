use std::{
    collections::VecDeque,
    io::{self, IsTerminal, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::Duration,
};

use anyhow::{Result, bail};
use clap::ArgMatches;
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use unicode_width::UnicodeWidthChar;

use crate::{process::hidden, workspace::Workspace};

const OUTPUT_LINES: usize = 600;
const COMMANDS: &[&str] = &[
    "doctor",
    "env show",
    "env check",
    "setup --dry-run",
    "repos discover",
    "repos status",
    "build --all --dry-run",
    "build --all",
    "run --dry-run",
    "run --initialize-db --open",
    "dev --initialize-db --open",
    "status",
    "verify",
    "stop",
    "help",
];
const BANNER: &[&str] = &[
    "▄██████ ███████ ██████▄ ██▄  ██ ███████ ███████    ██     ▄████▄  ██████▄",
    "██        ███   ██   ██ ██▀▄ ██ ██        ███      ██     ██  ██  ██   ██",
    "██        ███   ████▀▀  ██ ████ ██▀▀▀▀    ███      ██     ██████  ██████▀",
    "██▄▄▄▄▄ ▄▄███▄▄ ██ ▀█▄  ██  ███ ██▄▄▄▄▄   ███      ██▄▄▄▄ ██  ██  ██▄▄▄██",
    " ▀▀▀▀▀▀ ▀▀▀▀▀▀▀ ▀▀   ▀▀ ▀▀   ▀▀ ▀▀▀▀▀▀▀   ▀▀▀      ▀▀▀▀▀▀ ▀▀  ▀▀  ▀▀▀▀▀▀",
];

struct Terminal;
impl Terminal {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        let terminal = Self;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(terminal)
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), Show, ResetColor, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

struct Running {
    child: Child,
    messages: Receiver<String>,
    job: Option<ProcessJob>,
}

pub fn open(workspace: &Workspace, flags: &ArgMatches) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("cnl tui requires an interactive terminal");
    }
    let _screen = Terminal::enter()?;
    let mut input = String::new();
    let mut history = Vec::<String>::new();
    let mut history_index = 0;
    let mut output = VecDeque::new();
    let mut scroll = 0;
    let mut current: Option<Running> = None;
    let mut dirty = true;
    loop {
        if let Some(running) = &mut current {
            for message in running.messages.try_iter() {
                push(&mut output, message);
                dirty = true;
            }
            if let Some(status) = running.child.try_wait()? {
                for message in running.messages.try_iter() {
                    push(&mut output, message);
                }
                push(&mut output, format!("Exit status {status}"));
                current = None;
                dirty = true;
            }
        }
        if dirty {
            draw(&input, &output, scroll, current.is_some())?;
            dirty = false;
        }
        if !event::poll(Duration::from_millis(80))? {
            continue;
        }
        let key = match event::read()? {
            Event::Resize(..) => {
                dirty = true;
                continue;
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => key,
            _ => continue,
        };
        dirty = true;
        match key.code {
            KeyCode::Esc | KeyCode::Char('c')
                if key.code == KeyCode::Esc || key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if let Some(running) = &mut current {
                    if let Some(job) = running.job.take() {
                        drop(job);
                        push(&mut output, "Cancellation requested".into());
                    } else {
                        push(
                            &mut output,
                            "Wait for this command to finish. Use stop for a running application."
                                .into(),
                        );
                    }
                } else if input.is_empty() {
                    return Ok(());
                } else {
                    input.clear();
                }
            }
            KeyCode::PageUp => scroll = (scroll + 8).min(output.len().saturating_sub(1)),
            KeyCode::PageDown => scroll = scroll.saturating_sub(8),
            KeyCode::Up if history_index > 0 => {
                history_index -= 1;
                input.clone_from(&history[history_index]);
            }
            KeyCode::Down if history_index < history.len() => {
                history_index += 1;
                input = history.get(history_index).cloned().unwrap_or_default();
            }
            KeyCode::Tab => {
                let suggestion = suggestions(&input).next();
                if let Some(suggestion) = suggestion {
                    input = suggestion.to_string();
                }
            }
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && input.len() < 2048 =>
            {
                input.push(character);
            }
            KeyCode::Enter if current.is_none() && !input.trim().is_empty() => {
                let command = std::mem::take(&mut input);
                history.push(command.clone());
                if history.len() > 100 {
                    history.remove(0);
                }
                history_index = history.len();
                scroll = 0;
                push(&mut output, format!("cnl> {command}"));
                match spawn(workspace, flags, &command) {
                    Ok(child) => current = Some(child),
                    Err(error) => push(&mut output, error.to_string()),
                }
            }
            _ => {}
        }
    }
}

fn suggestions(input: &str) -> impl Iterator<Item = &'static str> + '_ {
    COMMANDS
        .iter()
        .copied()
        .filter(move |item| item.starts_with(input) && *item != input)
}

fn draw(input: &str, output: &VecDeque<String>, scroll: usize, busy: bool) -> Result<()> {
    let (columns, rows) = terminal::size()?;
    let width = usize::from(columns).saturating_sub(2);
    let mut screen = io::stdout().lock();
    queue!(screen, Clear(ClearType::All), MoveTo(0, 0))?;
    let mut row = 0;
    if columns >= 75 && rows >= 13 {
        for (index, line) in BANNER.iter().enumerate() {
            let color = [
                Color::Cyan,
                Color::Cyan,
                Color::Blue,
                Color::DarkBlue,
                Color::DarkCyan,
            ][index];
            queue!(
                screen,
                MoveTo(1, row),
                SetForegroundColor(color),
                Print(clip(line, width))
            )?;
            row += 1;
        }
    }
    queue!(
        screen,
        MoveTo(1, row),
        SetForegroundColor(Color::Cyan),
        Print(clip("CIRNETLAB", width))
    )?;
    row += 1;
    if row + 1 < rows {
        let line = if busy {
            "Running  ·  Esc cancel  ·  PgUp/PgDn scroll"
        } else {
            "Tab complete  ·  ↑/↓ history  ·  Esc exit  ·  PgUp/PgDn scroll"
        };
        queue!(
            screen,
            MoveTo(1, row),
            SetForegroundColor(Color::DarkGrey),
            Print(clip(line, width)),
            ResetColor
        )?;
        row += 1;
    }
    let capacity = usize::from(rows.saturating_sub(row + 2));
    let end = output.len().saturating_sub(scroll);
    let start = end.saturating_sub(capacity);
    for line in output.range(start..end) {
        queue!(screen, MoveTo(1, row), Print(clip(line, width)))?;
        row += 1;
    }
    if rows >= 2 {
        let suggestions = suggestions(input).take(4).collect::<Vec<_>>().join("  ·  ");
        queue!(
            screen,
            MoveTo(1, rows - 2),
            SetForegroundColor(Color::DarkCyan),
            Print(clip(&suggestions, width))
        )?;
    }
    if rows > 0 {
        let prompt = format!("cnl> {input}_");
        queue!(
            screen,
            MoveTo(1, rows - 1),
            SetForegroundColor(Color::Cyan),
            Print(clip(&prompt, width)),
            ResetColor
        )?;
    }
    screen.flush()?;
    Ok(())
}

fn clip(line: &str, mut width: usize) -> String {
    line.chars()
        .filter_map(|character| character.width().map(|width| (character, width)))
        .take_while(|(_, columns)| {
            if *columns > width {
                return false;
            }
            width -= *columns;
            true
        })
        .map(|(character, _)| character)
        .collect()
}

fn push(output: &mut VecDeque<String>, line: String) {
    if output.len() == OUTPUT_LINES {
        output.pop_front();
    }
    output.push_back(line);
}

fn tokens(text: &str) -> Result<Vec<String>> {
    let mut result = vec![];
    let mut token = String::new();
    let mut quote = None;
    for character in text.chars() {
        if quote == Some(character) {
            quote = None;
        } else if quote.is_none() && matches!(character, '"' | '\'') {
            quote = Some(character);
        } else if quote.is_none() && character.is_whitespace() {
            if !token.is_empty() {
                result.push(std::mem::take(&mut token));
            }
        } else {
            token.push(character);
        }
    }
    if quote.is_some() {
        bail!("Close the quoted argument before running a command");
    }
    if !token.is_empty() {
        result.push(token);
    }
    Ok(result)
}

fn spawn(workspace: &Workspace, flags: &ArgMatches, text: &str) -> Result<Running> {
    let arguments = tokens(text)?;
    let parsed = crate::cli::app()
        .try_get_matches_from(std::iter::once("cnl").chain(arguments.iter().map(String::as_str)));
    let action = parsed.as_ref().ok().and_then(ArgMatches::subcommand_name);
    if action == Some("tui") {
        bail!("The CIRNETLAB terminal is already open");
    }
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--workspace")
        .arg(&workspace.root)
        .arg("--profile")
        .arg(flags.get_one::<String>("profile").unwrap());
    if let Some(path) = flags.get_one::<PathBuf>("state-dir") {
        command.arg("--state-dir").arg(path);
    }
    hidden(
        command
            .args(&arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    let mut child = command.spawn()?;
    let job = if matches!(action, Some("run" | "dev" | "stop")) {
        None
    } else {
        ProcessJob::attach(&child).ok()
    };
    let (send, messages) = mpsc::sync_channel(128);
    reader(child.stdout.take().unwrap(), send.clone());
    reader(child.stderr.take().unwrap(), send);
    Ok(Running {
        child,
        messages,
        job,
    })
}

fn reader(mut pipe: impl Read + Send + 'static, send: SyncSender<String>) {
    thread::spawn(move || {
        let mut buffer = [0; 1024];
        let mut line = Vec::new();
        while let Ok(count) = pipe.read(&mut buffer) {
            if count == 0 {
                break;
            }
            for &byte in &buffer[..count] {
                if byte == b'\n' {
                    let _ = send.try_send(String::from_utf8_lossy(&line).trim_end().into());
                    line.clear();
                } else if line.len() < 4096 {
                    line.push(byte);
                }
            }
        }
        if !line.is_empty() {
            let _ = send.try_send(String::from_utf8_lossy(&line).trim_end().into());
        }
    });
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
    fn input_keeps_quoted_paths_and_suggests_commands() {
        assert_eq!(
            tokens(r#"run --receipt "D:\CIRNET LAB\assembly-result.json""#).unwrap(),
            ["run", "--receipt", r"D:\CIRNET LAB\assembly-result.json"]
        );
        assert!(tokens("run 'unfinished").is_err());
        assert_eq!(
            suggestions("repos ").collect::<Vec<_>>(),
            ["repos discover", "repos status"]
        );
    }
}
