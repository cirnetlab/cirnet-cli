mod backend;
mod input;

use std::{
    io::{self, IsTerminal},
    num::NonZeroU16,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind},
    style::{Attribute, Attributes, Color},
    terminal as console,
};
use portable_pty::{ExitStatus, PtySize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use self::{
    backend::{Backend, MAX_PASTE},
    input::{key_bytes, mouse_bytes},
};
use super::terminal::{Frame, Line, Painter, Screen, Tone, color_enabled};
use crate::localization::text as locale;

const MAX_SESSIONS: usize = 12;
const SCROLLBACK_ROWS: usize = 500;
const HEADER: u16 = 2;
const CHROME: u16 = HEADER + 1;

#[derive(Default)]
pub(super) struct Manager {
    entries: Vec<Session>,
}

impl Manager {
    pub(super) fn open(
        &mut self,
        key: &str,
        cwd: &Path,
        shell: Option<&str>,
        vi: bool,
    ) -> Result<()> {
        ensure!(
            io::stdin().is_terminal() && io::stdout().is_terminal(),
            "{}",
            locale(crate::localization::SHELL_INTERACTIVE, vi)
        );
        let cwd = crate::workspace::canonical(cwd)?;
        ensure!(
            cwd.is_dir(),
            "{}",
            locale(
                [
                    "Repository directory is unavailable",
                    "Thư mục kho không khả dụng"
                ],
                vi
            )
        );
        let mut screen = Screen::enter(io::stdout())?;
        let size = console::size()?;
        let index = self.attach(key, &cwd, shell, pty_size(size), vi)?;
        self.interact(&mut screen, index, size, vi)
    }

    fn attach(
        &mut self,
        key: &str,
        cwd: &Path,
        shell: Option<&str>,
        size: PtySize,
        vi: bool,
    ) -> Result<usize> {
        if let Some(index) = self
            .entries
            .iter()
            .position(|item| item.key == key && item.cwd == cwd)
        {
            return Ok(index);
        }
        ensure!(
            self.entries.len() < MAX_SESSIONS,
            "{}",
            locale(
                [
                    "Terminal session limit reached. Open an existing session and close it with F8.",
                    "Đã đạt giới hạn phiên terminal. Mở một phiên hiện có và đóng bằng F8.",
                ],
                vi
            )
        );
        let index = self.entries.len();
        self.entries
            .push(Session::start(key, cwd, shell, size, vi)?);
        Ok(index)
    }

    fn interact(
        &mut self,
        screen: &mut Screen<io::Stdout>,
        mut index: usize,
        mut size: (u16, u16),
        vi: bool,
    ) -> Result<()> {
        let mut painter = Painter::default();
        let mut drawn = None;
        let mut prompt = None;
        let mut feedback = None;
        loop {
            let session = &mut self.entries[index];
            session.resize(pty_size(size));
            let frame = {
                let output = session.output.lock().unwrap();
                let stamp = (index, size, output.revision, prompt, feedback);
                if drawn == Some(stamp) {
                    None
                } else {
                    drawn = Some(stamp);
                    Some(session.frame(&output, size, prompt, feedback, vi))
                }
            };
            if let Some(mut frame) = frame {
                if let Some(header) = frame.rows.first_mut() {
                    push_clipped(
                        header,
                        Tone::Muted,
                        &format!("  [{}/{}]", index + 1, self.entries.len()),
                        size.0,
                    );
                }
                painter.draw(&mut screen.out, frame, color_enabled())?;
            }
            if !event::poll(Duration::from_millis(45))? {
                continue;
            }
            match event::read()? {
                Event::Resize(width, height) => size = (width, height),
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if let Some(action) = prompt {
                        if key.code == KeyCode::Esc {
                            prompt = None;
                        }
                        if key.code == KeyCode::Enter {
                            prompt = None;
                            let previous = self.entries.remove(index);
                            if action == Action::Close {
                                return Ok(());
                            }
                            let (key, cwd, shell) = (
                                previous.key.clone(),
                                previous.cwd.clone(),
                                previous.shell.clone(),
                            );
                            drop(previous);
                            let replacement =
                                Session::start(&key, &cwd, Some(&shell), pty_size(size), vi)?;
                            self.entries.insert(index, replacement);
                            drawn = None;
                        }
                        continue;
                    }
                    let no_modifiers = key.modifiers.is_empty();
                    feedback = None;
                    match key.code {
                        KeyCode::F(10) if no_modifiers => return Ok(()),
                        KeyCode::F(8) if no_modifiers => prompt = Some(Action::Close),
                        KeyCode::F(9) if no_modifiers => prompt = Some(Action::Restart),
                        KeyCode::F(6) if no_modifiers => {
                            index = if index == 0 {
                                self.entries.len() - 1
                            } else {
                                index - 1
                            };
                        }
                        KeyCode::F(7) if no_modifiers => {
                            index = (index + 1) % self.entries.len();
                        }
                        KeyCode::PageUp | KeyCode::PageDown
                            if key.modifiers == KeyModifiers::SHIFT =>
                        {
                            let page = i32::from(pty_size(size).rows);
                            self.entries[index].scroll(if key.code == KeyCode::PageUp {
                                page
                            } else {
                                -page
                            });
                        }
                        _ => {
                            if !self.entries[index].key(key) {
                                feedback = Some(Feedback::Input);
                            }
                        }
                    }
                }
                Event::Paste(value) if prompt.is_none() => {
                    feedback = if value.len() > MAX_PASTE {
                        Some(Feedback::Paste)
                    } else if !self.entries[index].paste(value) {
                        Some(Feedback::Input)
                    } else {
                        None
                    };
                }
                Event::Mouse(mouse) if prompt.is_none() => {
                    let session = &self.entries[index];
                    let packet = {
                        let output = session.output.lock().unwrap();
                        let screen = output.parser.screen();
                        if screen.scrollback() != 0 {
                            None
                        } else {
                            mouse_bytes(mouse, screen, HEADER)
                        }
                    };
                    if let Some(packet) = packet {
                        if !session.send(packet) {
                            feedback = Some(Feedback::Input);
                        }
                    } else {
                        match mouse.kind {
                            MouseEventKind::ScrollUp => session.scroll(3),
                            MouseEventKind::ScrollDown => session.scroll(-3),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Close,
    Restart,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Feedback {
    Input,
    Paste,
}

struct Session {
    key: String,
    cwd: PathBuf,
    shell: String,
    size: PtySize,
    backend: Backend,
    output: Arc<Mutex<Output>>,
}

impl Session {
    fn start(
        key: &str,
        cwd: &Path,
        requested: Option<&str>,
        size: PtySize,
        vi: bool,
    ) -> Result<Self> {
        let default_shell = if cfg!(windows) { "powershell" } else { "bash" };
        let mut shell = requested.unwrap_or(default_shell);
        let backend = match Backend::start(shell, cwd, size, vi) {
            Err(powershell) if cfg!(windows) && requested.is_none() => {
                shell = "cmd";
                Backend::start(shell, cwd, size, vi).with_context(|| powershell.to_string())?
            }
            result => result?,
        };
        Ok(Self {
            key: key.into(),
            cwd: cwd.into(),
            shell: shell.into(),
            size,
            output: Arc::clone(&backend.output),
            backend,
        })
    }

    fn resize(&mut self, size: PtySize) {
        if self.size != size && self.backend.request_size(size) {
            self.size = size;
        }
    }

    fn send(&self, bytes: Vec<u8>) -> bool {
        self.backend.send(bytes)
    }

    fn key(&self, key: KeyEvent) -> bool {
        let packet = {
            let mut output = self.output.lock().unwrap();
            if output.exit.is_some() {
                return false;
            }
            output.scroll_to_live();
            key_bytes(key, output.parser.screen().application_cursor())
        };
        packet.is_none_or(|bytes| self.send(bytes))
    }

    fn paste(&self, text: String) -> bool {
        let bracketed = {
            let mut output = self.output.lock().unwrap();
            if output.exit.is_some() {
                return false;
            }
            output.scroll_to_live();
            output.parser.screen().bracketed_paste()
        };
        let mut packet = Vec::with_capacity(text.len() + 16);
        if bracketed {
            packet.extend_from_slice(b"\x1b[200~");
            packet.extend_from_slice(text.as_bytes());
            packet.extend_from_slice(b"\x1b[201~");
        } else {
            let mut previous = None;
            for byte in text.bytes() {
                if byte != b'\n' || previous != Some(b'\r') {
                    packet.push(if byte == b'\n' { b'\r' } else { byte });
                }
                previous = Some(byte);
            }
        }
        self.send(packet)
    }

    fn scroll(&self, lines: i32) {
        let mut output = self.output.lock().unwrap();
        let screen = output.parser.screen_mut();
        let before = screen.scrollback();
        let next = before.saturating_add_signed(lines as isize);
        screen.set_scrollback(next);
        if screen.scrollback() != before {
            output.changed();
        }
    }

    fn frame(
        &self,
        output: &Output,
        size: (u16, u16),
        action: Option<Action>,
        feedback: Option<Feedback>,
        vi: bool,
    ) -> Frame {
        let mut frame = Frame::new(size);
        let Some(header) = frame.rows.first_mut() else {
            return frame;
        };
        let screen = output.parser.screen();
        let status = match output.exit.as_ref() {
            Some(Ok(status)) => format!("{} {status}", locale(["Exited", "Đã thoát"], vi)),
            Some(Err(error)) => format!("{} {error}", locale(["Exited", "Đã thoát"], vi)),
            None => locale(["Running", "Đang chạy"], vi).into(),
        };
        let title = format!(
            "{} · {} · {} · {status}",
            locale(["Repository terminal", "Terminal kho mã"], vi),
            self.key,
            self.shell,
        );
        push_clipped(header, Tone::Accent, &title, size.0);
        if size.1 >= 3 {
            push_clipped(
                &mut frame.rows[1],
                Tone::Muted,
                &self.cwd.to_string_lossy(),
                size.0,
            );
        }
        if size.1 > HEADER {
            let (rows, columns) = screen.size();
            let view_rows = rows.get().min(size.1 - CHROME);
            let view_columns = columns.get().min(size.0);
            for row in 0..view_rows {
                frame.rows[usize::from(row + HEADER)] = terminal_line(screen, row, view_columns);
            }
            let (row, col) = screen.cursor_position();
            if action.is_none()
                && output.exit.is_none()
                && screen.scrollback() == 0
                && !screen.hide_cursor()
                && row < view_rows
                && view_columns > 0
            {
                frame.cursor = Some((col.min(view_columns - 1), row + HEADER));
            }
        }
        if size.1 > 1 {
            let (note, tone) = footer(output, action, feedback, vi);
            push_clipped(frame.rows.last_mut().unwrap(), tone, &note, size.0);
        }
        frame
    }
}

fn footer(
    output: &Output,
    action: Option<Action>,
    feedback: Option<Feedback>,
    vi: bool,
) -> (String, Tone) {
    let warning = match action {
        Some(Action::Close) => Some([
            "Close this shell and its attached processes? Enter closes · Esc returns",
            "Đóng shell và các tiến trình liên quan? Enter đóng · Esc quay lại",
        ]),
        Some(Action::Restart) => Some([
            "Restart this shell? Enter restarts and clears its state · Esc returns",
            "Khởi động lại shell? Enter khởi động lại và xóa trạng thái · Esc quay lại",
        ]),
        None => match feedback {
            Some(Feedback::Input) => Some([
                "Terminal input unavailable or busy · F9 restart · F10 back",
                "Thiết bị nhập terminal không khả dụng hoặc đang bận · F9 khởi động lại · F10 quay lại",
            ]),
            Some(Feedback::Paste) => Some([
                "Paste exceeds 32 KiB. Paste smaller parts.",
                "Nội dung dán vượt 32 KiB. Hãy chia thành các phần nhỏ.",
            ]),
            None if output.resize_failed => Some([
                "Shell resize failed · F9 restart · F10 back",
                "Không thể đổi kích thước shell · F9 khởi động lại · F10 quay lại",
            ]),
            None if output.input_failed && output.exit.is_none() => Some([
                "Terminal input stopped · F9 restart · F10 back",
                "Thiết bị nhập terminal đã dừng · F9 khởi động lại · F10 quay lại",
            ]),
            None => None,
        },
    };
    if let Some(warning) = warning {
        return (locale(warning, vi).into(), Tone::Warning);
    }
    let shortcuts = locale(
        [
            "F10 Back · F6/F7 Session · F8 Close · F9 Restart · Shift+PgUp/PgDn Scroll",
            "F10 Quay lại · F6/F7 Đổi phiên · F8 Đóng · F9 Khởi động lại · Shift+PgUp/PgDn Cuộn",
        ],
        vi,
    );
    let scrollback = output.parser.screen().scrollback();
    if scrollback == 0 {
        (shortcuts.into(), Tone::Muted)
    } else {
        let scroll = locale(["Scrollback", "Lịch sử cuộn"], vi);
        (format!("{scroll} +{scrollback} · {shortcuts}"), Tone::Muted)
    }
}

fn push_clipped(line: &mut Line, tone: Tone, value: &str, width: u16) {
    let mut used = line.contents().width();
    let clean = value.replace(char::is_control, " ");
    let end = clean.grapheme_indices(true).find_map(|(index, grapheme)| {
        used += grapheme.width();
        (used > usize::from(width)).then_some(index)
    });
    line.push(tone, &clean[..end.unwrap_or(clean.len())]);
}

fn pty_size((width, height): (u16, u16)) -> PtySize {
    PtySize {
        cols: width.clamp(1, 256),
        rows: height.saturating_sub(CHROME).clamp(1, 120),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn dimensions(size: PtySize) -> (NonZeroU16, NonZeroU16) {
    (
        NonZeroU16::new(size.rows).unwrap(),
        NonZeroU16::new(size.cols).unwrap(),
    )
}

struct Output {
    parser: vt100::Parser<Replies>,
    revision: u64,
    exit: Option<std::result::Result<ExitStatus, String>>,
    input_failed: bool,
    resize_failed: bool,
}

impl Output {
    fn new(size: PtySize) -> Self {
        let (rows, columns) = dimensions(size);
        Self {
            parser: vt100::Parser::new_with_callbacks(
                rows,
                columns,
                SCROLLBACK_ROWS,
                Replies::default(),
            ),
            revision: 0,
            exit: None,
            input_failed: false,
            resize_failed: false,
        }
    }

    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn process(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.parser.process(bytes);
        self.changed();
        std::mem::take(&mut self.parser.callbacks_mut().0)
    }

    fn resize(&mut self, size: PtySize) {
        let (rows, columns) = dimensions(size);
        self.parser.set_size(rows, columns);
        self.resize_failed = false;
        self.changed();
    }

    fn scroll_to_live(&mut self) {
        if self.parser.screen().scrollback() != 0 {
            self.parser.screen_mut().set_scrollback(0);
            self.changed();
        }
    }
}

#[derive(Default)]
struct Replies(Vec<Vec<u8>>);

impl vt100::Callbacks for Replies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        first: Option<u8>,
        second: Option<u8>,
        params: &[&[u16]],
        final_byte: char,
    ) {
        if second.is_some() || self.0.len() >= 16 {
            return;
        }
        let response = match (first, params, final_byte) {
            (None, [[5]], 'n') => "\x1b[0n".into(),
            (None | Some(b'?'), [[6]], 'n') => {
                let (row, column) = screen.cursor_position();
                let prefix = if first.is_some() { "?" } else { "" };
                format!("\x1b[{prefix}{};{}R", row + 1, column + 1)
            }
            (None, [] | [[0]], 'c') => "\x1b[?1;2c".into(),
            (Some(b'>'), [] | [[0]], 'c') => "\x1b[>0;0;0c".into(),
            _ => return,
        };
        self.0.push(response.into_bytes());
    }
}

fn terminal_line(screen: &vt100::Screen, row: u16, columns: u16) -> Line {
    let mut line = Line::default();
    for index in 0..columns {
        let Some(cell) = screen.cell(row, index) else {
            break;
        };
        if cell.is_wide_continuation() {
            continue;
        }
        let content = if !cell.has_contents() || (cell.is_wide() && index + 1 == columns) {
            " "
        } else {
            cell.contents()
        };
        let mut attributes = Attributes::default();
        for (enabled, attr) in [
            (cell.bold(), Attribute::Bold),
            (cell.dim(), Attribute::Dim),
            (cell.italic(), Attribute::Italic),
            (cell.underline(), Attribute::Underlined),
            (cell.inverse(), Attribute::Reverse),
        ] {
            if enabled {
                attributes.set(attr);
            }
        }
        let foreground = terminal_color(cell.fgcolor());
        let background = terminal_color(cell.bgcolor());
        let tone = if attributes.is_empty() && foreground.is_none() && background.is_none() {
            Tone::Plain
        } else {
            Tone::Pty {
                foreground,
                background,
                attributes,
            }
        };
        line.push(tone, content);
    }
    line
}

fn terminal_color(color: vt100::Color) -> Option<Color> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Idx(index) => Some(Color::AnsiValue(index)),
        vt100::Color::Rgb(r, g, b) => Some(Color::Rgb { r, g, b }),
    }
}

#[cfg(test)]
mod tests;
