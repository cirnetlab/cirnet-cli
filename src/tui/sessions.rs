mod backend;
mod input;
mod layout;
mod render;
mod workspace;

use std::{
    io::{self, IsTerminal},
    num::NonZeroU16,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc::SyncSender},
};

use anyhow::{Context, Result, ensure};
use clap::ArgMatches;
use crossterm::{
    event::{KeyEvent, MouseEvent},
    style::{Attribute, Attributes, Color},
    terminal as console,
};
use portable_pty::{ExitStatus, PtySize};

use self::{
    backend::Backend,
    input::{key_bytes, mouse_bytes},
    layout::{Arrangement, Layout, Sidebar},
    workspace::{Catalog, WorkspaceEvent},
};
use super::terminal::{Line, Screen, Tone};
use crate::{localization::text as locale, repositories::Row, workspace::Workspace};

const MAX_SESSIONS: usize = 12;
const SCROLLBACK_ROWS: usize = 500;
type SessionId = u64;
type Target<'a> = (&'a str, &'a Path, Option<&'a str>);

#[derive(Default)]
pub(super) struct Manager {
    entries: Vec<Session>,
    arrangement: Arrangement,
    next_id: SessionId,
}

impl Manager {
    pub(super) fn open_with_repositories(
        &mut self,
        (key, cwd, shell): Target<'_>,
        workspace: &Workspace,
        flags: &ArgMatches,
        rows: Option<&[Row]>,
        vi: bool,
    ) -> Result<()> {
        ensure!(
            io::stdin().is_terminal() && io::stdout().is_terminal(),
            "{}",
            locale(crate::localization::SHELL_INTERACTIVE, vi)
        );
        let cwd = std::fs::canonicalize(cwd)?;
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
        let size = console::size()?;
        let index = self.attach(key, &cwd, shell, pty_size(size), vi)?;
        let id = self.entries[index].id;
        self.arrangement.activate(id);
        self.arrangement.sidebar_focused = false;
        if size.0 < 54 {
            self.arrangement.sidebar = Sidebar::Auto;
        }
        let catalog = Catalog::new(workspace, flags, rows);
        if let Some(name) = catalog.name_for(&cwd, key) {
            self.entries[index].repository = name.into();
        }
        let mut screen = Screen::enter(io::BufWriter::with_capacity(64 * 1024, io::stdout()))?;
        workspace::interact(self, &mut screen, size, catalog, vi)
    }

    fn attach(
        &mut self,
        key: &str,
        cwd: &Path,
        shell: Option<&str>,
        size: PtySize,
        vi: bool,
    ) -> Result<usize> {
        // Once open, shell changes happen in a new independent session.
        if let Some(index) = self.entries.iter().position(|session| {
            session.cwd == cwd && shell.is_none_or(|shell| session.shell == shell)
        }) {
            return Ok(index);
        }
        self.ensure_capacity(vi)?;
        let index = self.entries.len();
        let mut arrangement = self.arrangement.clone();
        arrangement.activate(self.next_id + 1);
        let size = Layout::new((size.cols, size.rows), &arrangement)
            .pane(self.next_id + 1)
            .map_or(size, |pane| pane.size());
        let session = Session::start(key, cwd, shell, size, vi)?;
        self.insert(session);
        Ok(index)
    }

    fn ensure_capacity(&self, vi: bool) -> Result<()> {
        ensure!(
            self.entries.len() < MAX_SESSIONS,
            "{}",
            locale(
                [
                    "Terminal session limit reached. Close a session with F8.",
                    "Đã đạt giới hạn phiên terminal. Đóng một phiên bằng F8.",
                ],
                vi
            )
        );
        Ok(())
    }

    fn insert(&mut self, mut session: Session) -> SessionId {
        self.next_id += 1;
        session.id = self.next_id;
        self.entries.push(session);
        self.next_id
    }

    fn position(&self, id: SessionId) -> Option<usize> {
        self.entries.iter().position(|entry| entry.id == id)
    }

    fn session(&self, id: SessionId) -> Option<&Session> {
        self.position(id).map(|index| &self.entries[index])
    }

    fn active(&self) -> Option<&Session> {
        self.arrangement.active().and_then(|id| self.session(id))
    }

    fn resize_visible(&mut self, layout: &Layout) {
        for pane in layout.panes.into_iter().flatten() {
            if let Some(index) = self.position(pane.id) {
                self.entries[index].resize(pane.size());
            }
        }
    }

    fn cycle(&mut self, step: isize) {
        if self.entries.is_empty() {
            return;
        }
        let current = self
            .arrangement
            .active()
            .and_then(|id| self.position(id))
            .unwrap_or_default();
        let next = (current as isize + step).rem_euclid(self.entries.len() as isize) as usize;
        self.arrangement.activate(self.entries[next].id);
    }

    fn reorder(&mut self, id: SessionId, step: isize) {
        if let Some(index) = self.position(id)
            && let Some(next) = index.checked_add_signed(step)
            && next < self.entries.len()
        {
            self.entries.swap(index, next);
        }
    }

    fn take(&mut self, id: SessionId) -> Option<Session> {
        let index = self.position(id)?;
        let session = self.entries.remove(index);
        let fallback = self
            .entries
            .get(index.min(self.entries.len().saturating_sub(1)))
            .map(|session| session.id);
        self.arrangement.close(id, fallback);
        Some(session)
    }
}

fn repository_name(key: &str, cwd: &Path) -> String {
    let identity = key.split_once(" · ").map_or(key, |(name, _)| name);
    if Path::new(identity).is_absolute() || identity.is_empty() {
        cwd.file_name()
            .map_or_else(|| "Repository".into(), |name| name.to_string_lossy().into())
    } else {
        identity.into()
    }
}

struct Session {
    id: SessionId,
    key: String,
    cwd: PathBuf,
    repository: String,
    label: Option<String>,
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
            id: 0,
            key: key.into(),
            cwd: cwd.into(),
            repository: repository_name(key, cwd),
            label: None,
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

    fn watch(&self, wake: Option<&SyncSender<WorkspaceEvent>>) {
        self.output.lock().unwrap().wake = wake.cloned();
    }

    fn title(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.repository)
    }

    fn rename(&mut self, label: &str) {
        let label = label.trim();
        self.label = (!label.is_empty()).then(|| label.to_owned());
    }

    fn running(&self) -> bool {
        self.output.lock().unwrap().exit.is_none()
    }

    fn needs_close_confirmation(&self) -> bool {
        self.output
            .lock()
            .unwrap()
            .exit
            .as_ref()
            .is_none_or(|exit| exit.is_err())
    }

    fn mouse(&self, mouse: MouseEvent, origin: (u16, u16)) -> Option<bool> {
        let packet = {
            let output = self.output.lock().unwrap();
            let screen = output.parser.screen();
            if output.exit.is_some() || screen.scrollback() != 0 {
                return None;
            }
            mouse_bytes(mouse, screen, origin)?
        };
        Some(self.send(packet))
    }
}

fn pty_size((width, height): (u16, u16)) -> PtySize {
    PtySize {
        cols: width.clamp(1, 256),
        rows: height.clamp(1, 120),
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
    wake: Option<SyncSender<WorkspaceEvent>>,
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
            wake: None,
            exit: None,
            input_failed: false,
            resize_failed: false,
        }
    }

    fn changed(&mut self) {
        if let Some(wake) = &self.wake {
            let _ = wake.try_send(WorkspaceEvent::Wake);
        }
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
