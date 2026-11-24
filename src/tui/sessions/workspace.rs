use std::{
    collections::HashSet,
    env, io,
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    thread,
    time::Duration,
};

use clap::ArgMatches;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

use super::{
    super::{
        clipboard::{Clipboard, SystemClipboard},
        editor::Input,
        terminal::{Painter, Screen, color_enabled},
    },
    Manager, Result, Row, Session, SessionId, Workspace,
    backend::MAX_PASTE,
    layout::{Layout, Rect, Sidebar},
    locale, pty_size,
    render::{self, Hit, Hits},
    scrollback::{self, Match, Selection},
};

const EDIT_LIMIT: usize = 192;
const POLL_INTERVAL: Duration = Duration::from_millis(25);

pub(super) enum WorkspaceEvent {
    Input(io::Result<Event>),
    Wake,
    Job(Result<Completed>),
}

#[derive(Clone)]
pub(super) struct Choice {
    selector: String,
    path: PathBuf,
    pub(super) name: String,
    preferred: Option<String>,
}

pub(super) struct Catalog {
    root: PathBuf,
    flags: ArgMatches,
    choices: Option<Vec<Choice>>,
    pub(super) shells: Vec<&'static str>,
}

impl Catalog {
    pub(super) fn new(workspace: &Workspace, flags: &ArgMatches, rows: Option<&[Row]>) -> Self {
        Self {
            root: workspace.root.clone(),
            flags: flags.clone(),
            choices: rows.map(Self::choices),
            shells: Vec::new(),
        }
    }

    fn choices(rows: &[Row]) -> Vec<Choice> {
        let mut choices: Vec<_> = rows
            .iter()
            .filter_map(|row| {
                if matches!(
                    row.state.as_str(),
                    "available" | "missing" | "invalid" | "inaccessible" | "unavailable"
                ) {
                    return None;
                }
                Some(Choice {
                    selector: row.selector.clone(),
                    path: row.path.clone()?,
                    name: row.alias.as_ref().unwrap_or(&row.name).clone(),
                    preferred: row.shell.clone(),
                })
            })
            .collect();
        choices.sort_by_cached_key(|choice| choice.name.to_lowercase());
        choices
    }

    pub(super) fn name_for(&self, path: &Path, key: &str) -> Option<&str> {
        let selector = key.split_once(" · ").map_or(key, |(selector, _)| selector);
        self.choices
            .as_deref()?
            .iter()
            .find(|choice| choice.path == path || choice.selector == selector)
            .map(|choice| choice.name.as_str())
    }

    pub(super) fn filtered(&self, query: &str) -> Vec<&Choice> {
        let query = query.trim().to_lowercase();
        self.choices
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|choice| {
                query.is_empty()
                    || choice.name.to_lowercase().contains(&query)
                    || crate::workspace::user_path(&choice.path)
                        .to_lowercase()
                        .contains(&query)
            })
            .collect()
    }

    fn repository_snapshot(&self) -> (Workspace, ArgMatches) {
        (
            Workspace {
                root: self.root.clone(),
            },
            self.flags.clone(),
        )
    }

    fn preferred(&self, choice: &Choice) -> usize {
        choice
            .preferred
            .as_deref()
            .and_then(|shell| self.shells.iter().position(|available| *available == shell))
            .unwrap_or_default()
    }
}

fn installed_shells() -> Vec<&'static str> {
    let mut paths: Vec<_> = env::var_os("PATH")
        .as_ref()
        .map(|paths| env::split_paths(paths).collect())
        .unwrap_or_default();
    if cfg!(windows)
        && let Some(root) = env::var_os("SystemRoot")
    {
        paths.push(PathBuf::from(root).join("System32"));
    }
    crate::cli::native::SHELLS
        .into_iter()
        .filter_map(|name| {
            let command = crate::cli::native::shell(Some(name)).ok()?;
            paths
                .iter()
                .any(|path| executable(&path.join(command.get_program())))
                .then_some(name)
        })
        .collect()
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    metadata.is_file()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Command {
    New,
    Split,
    Sidebar,
    Menu,
    Previous,
    Next,
    Rename,
    Duplicate,
    MovePane,
    MoveUp,
    MoveDown,
    Close,
    Restart,
    FocusPane,
    Maximize,
    ResizeLess,
    ResizeMore,
    Find,
    Copy,
    Back,
}

impl Command {
    pub(super) fn title(self, vi: bool) -> &'static str {
        locale(
            match self {
                Self::New => ["New Terminal", "Terminal mới"],
                Self::Split => ["Split", "Chia ô"],
                Self::Sidebar => ["Sessions", "Phiên"],
                Self::Menu => ["Actions", "Thao tác"],
                Self::Previous => ["Previous session", "Phiên trước"],
                Self::Next => ["Next session", "Phiên sau"],
                Self::Rename => ["Rename", "Đổi tên"],
                Self::Duplicate => ["Duplicate", "Tạo bản sao"],
                Self::MovePane => ["Move to other pane", "Chuyển sang ô kia"],
                Self::MoveUp => ["Move up in list", "Đưa lên trong danh sách"],
                Self::MoveDown => ["Move down in list", "Đưa xuống trong danh sách"],
                Self::Close => ["Close", "Đóng"],
                Self::Restart => ["Restart", "Khởi động lại"],
                Self::FocusPane => ["Focus other pane", "Chuyển tiêu điểm ô"],
                Self::Maximize => ["Maximize pane", "Phóng to ô"],
                Self::ResizeLess => ["Shrink first pane", "Thu nhỏ ô đầu"],
                Self::ResizeMore => ["Grow first pane", "Mở rộng ô đầu"],
                Self::Find => ["Find in terminal", "Tìm trong terminal"],
                Self::Copy => ["Copy selection", "Sao chép vùng chọn"],
                Self::Back => ["Back", "Quay lại"],
            },
            vi,
        )
    }

    pub(super) fn shortcut(self) -> &'static str {
        match self {
            Self::New => "F1",
            Self::Rename => "F2",
            Self::Split => "F3",
            Self::Sidebar => "F4",
            Self::Menu => "F5",
            Self::Previous => "F6",
            Self::Next => "F7",
            Self::Close => "F8",
            Self::Restart => "F9",
            Self::Back => "F10",
            Self::FocusPane => "Ctrl+Shift+P",
            Self::Maximize => "Ctrl+Shift+M",
            Self::ResizeLess => "Ctrl+Shift+←",
            Self::ResizeMore => "Ctrl+Shift+→",
            Self::Find => "Ctrl+Shift+F",
            Self::Copy => "Ctrl+Shift+C",
            Self::MoveUp => "Ctrl+Shift+↑",
            Self::MoveDown => "Ctrl+Shift+↓",
            Self::MovePane | Self::Duplicate => "",
        }
    }

    fn from_key(key: KeyEvent) -> Option<Self> {
        if key.modifiers == KeyModifiers::CONTROL | KeyModifiers::SHIFT {
            return match key.code {
                KeyCode::Up => Some(Self::MoveUp),
                KeyCode::Down => Some(Self::MoveDown),
                KeyCode::Left => Some(Self::ResizeLess),
                KeyCode::Right => Some(Self::ResizeMore),
                KeyCode::Char('p' | 'P') => Some(Self::FocusPane),
                KeyCode::Char('m' | 'M') => Some(Self::Maximize),
                KeyCode::Char('f' | 'F') => Some(Self::Find),
                KeyCode::Char('c' | 'C') => Some(Self::Copy),
                _ => None,
            };
        }
        if !key.modifiers.is_empty() {
            return None;
        }
        Some(match key.code {
            KeyCode::F(1) => Self::New,
            KeyCode::F(2) => Self::Rename,
            KeyCode::F(3) => Self::Split,
            KeyCode::F(4) => Self::Sidebar,
            KeyCode::F(5) => Self::Menu,
            KeyCode::F(6) => Self::Previous,
            KeyCode::F(7) => Self::Next,
            KeyCode::F(8) => Self::Close,
            KeyCode::F(9) => Self::Restart,
            KeyCode::F(10) => Self::Back,
            _ => return None,
        })
    }
}

pub(super) const MENU_GROUPS: &[&[Command]] = &[
    &[
        Command::New,
        Command::Duplicate,
        Command::Rename,
        Command::Restart,
        Command::Close,
    ],
    &[
        Command::Split,
        Command::MovePane,
        Command::FocusPane,
        Command::Maximize,
        Command::ResizeLess,
        Command::ResizeMore,
    ],
    &[
        Command::Sidebar,
        Command::Previous,
        Command::Next,
        Command::MoveUp,
        Command::MoveDown,
    ],
    &[Command::Find, Command::Copy, Command::Back],
];

fn menu_commands() -> impl Iterator<Item = Command> {
    MENU_GROUPS.iter().flat_map(|group| group.iter().copied())
}

pub(super) struct FindBar {
    pub(super) id: SessionId,
    pub(super) query: Input,
    pub(super) hit: Option<Match>,
    generation: u64,
    columns: u16,
    excerpt: Option<String>,
}

impl FindBar {
    pub(super) fn visible(&self, output: &super::Output) -> Option<Match> {
        let hit = self.hit?;
        let screen = output.parser.screen();
        (self.generation == output.generation
            || self.columns == screen.size().1.get()
                && self
                    .excerpt
                    .as_ref()
                    .is_some_and(|text| hit.visible_text(screen).as_ref() == Some(text)))
        .then_some(hit)
    }
}

pub(super) struct Marked {
    pub(super) id: SessionId,
    pub(super) cells: Selection,
    generation: u64,
    offset: usize,
    size: (u16, u16),
    snapshot: String,
    anchor: (u16, u16),
    pub(super) has_range: bool,
    dragging: bool,
}

impl Marked {
    pub(super) fn valid(&self, id: SessionId, output: &super::Output) -> bool {
        let screen = output.parser.screen();
        let (rows, cols) = screen.size();
        self.id == id
            && self.offset == screen.scrollback()
            && self.size == (rows.get(), cols.get())
            && (self.generation == output.generation || self.snapshot == screen.contents())
    }
}

pub(super) enum Modal {
    Menu {
        selected: usize,
        origin: Option<(u16, u16)>,
    },
    Repositories {
        query: Input,
        selected: usize,
    },
    Shells {
        repository: Choice,
        query: Input,
        previous: usize,
        selected: usize,
    },
    Rename(SessionId, Input),
    Confirm(SessionId, Command),
}

#[derive(Clone, Copy)]
pub(super) enum Busy {
    Catalog,
    Opening,
    Restarting,
    Closing,
}

impl Busy {
    pub(super) fn message(self, vi: bool) -> &'static str {
        locale(
            match self {
                Self::Catalog => [
                    "Finding repositories and shells…",
                    "Đang tìm kho mã và shell…",
                ],
                Self::Opening => ["Starting terminal…", "Đang mở terminal…"],
                Self::Restarting => ["Restarting terminal…", "Đang khởi động lại terminal…"],
                Self::Closing => ["Closing terminal…", "Đang đóng terminal…"],
            },
            vi,
        )
    }
}

pub(super) enum Placement {
    Independent,
    Split(SessionId),
    Replace(SessionId),
}

pub(super) enum Completed {
    Picker(Vec<Choice>, Vec<&'static str>),
    Session(Box<Session>, Placement),
    Closed,
}

#[derive(Clone, Copy)]
struct MouseCapture {
    id: SessionId,
    area: Rect,
    event: MouseEvent,
}

fn button_index(button: MouseButton) -> usize {
    match button {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
    }
}

pub(super) struct State {
    pub(super) modal: Option<Modal>,
    pub(super) busy: Option<Busy>,
    pub(super) notice: Option<String>,
    pub(super) catalog: Catalog,
    pub(super) size: (u16, u16),
    pub(super) vi: bool,
    pub(super) find: Option<FindBar>,
    pub(super) marked: Option<Marked>,
    pub(super) sidebar_query: Option<Input>,
    pub(super) sidebar_selected: Option<SessionId>,
    pub(super) sidebar_hover: Option<SessionId>,
    pub(super) hovered_command: Option<Command>,
    divider_drag: Option<[SessionId; 2]>,
    native_mouse: [Option<MouseCapture>; 3],
    events: SyncSender<WorkspaceEvent>,
}

impl State {
    pub(super) fn new(
        size: (u16, u16),
        catalog: Catalog,
        vi: bool,
        events: SyncSender<WorkspaceEvent>,
    ) -> Self {
        Self {
            modal: None,
            busy: None,
            notice: None,
            size,
            catalog,
            vi,
            find: None,
            marked: None,
            sidebar_query: None,
            sidebar_selected: None,
            sidebar_hover: None,
            hovered_command: None,
            divider_drag: None,
            native_mouse: [None; 3],
            events,
        }
    }

    fn begin(&mut self, busy: Busy, work: impl FnOnce() -> Result<Completed> + Send + 'static) {
        let events = self.events.clone();
        let spawn = thread::Builder::new()
            .name("cnl-pty-workspace".into())
            .spawn(move || {
                let result = work();
                let _ = events.send(WorkspaceEvent::Job(result));
            });
        match spawn {
            Ok(_) => self.busy = Some(busy),
            Err(error) => self.notice = Some(error.to_string()),
        }
    }

    pub(super) fn complete(&mut self, manager: &mut Manager, result: Result<Completed>) {
        self.busy = None;
        match result {
            Ok(Completed::Picker(choices, shells)) => {
                self.catalog.choices = Some(choices);
                self.catalog.shells = shells;
                self.modal = Some(Modal::Repositories {
                    query: Input::default(),
                    selected: 0,
                });
            }
            Ok(Completed::Session(session, placement)) => {
                self.release_mouse(manager);
                self.find = None;
                self.marked = None;
                self.sidebar_query = None;
                self.sidebar_selected = None;
                self.sidebar_hover = None;
                let mut session = *session;
                session.watch(Some(&self.events));
                match placement {
                    Placement::Independent => {
                        let id = manager.insert(session);
                        manager.arrangement.activate(id);
                    }
                    Placement::Split(source) => {
                        manager.arrangement.activate(source);
                        let id = manager.insert(session);
                        manager.arrangement.split(id);
                    }
                    Placement::Replace(id) => {
                        if let Some(index) = manager.position(id) {
                            session.id = id;
                            let old = std::mem::replace(&mut manager.entries[index], session);
                            self.discard(old);
                        }
                    }
                }
                manager.arrangement.sidebar_focused = false;
                if let Some(id) = manager.arrangement.active() {
                    manager.remember(id);
                }
            }
            Ok(Completed::Closed) => {}
            Err(error) => self.notice = Some(format!("{error:#}")),
        }
    }

    fn discard(&mut self, session: Session) {
        session.watch(None);
        self.begin(Busy::Closing, move || {
            drop(session);
            Ok(Completed::Closed)
        });
    }

    fn picker(&mut self, manager: &Manager) {
        let (workspace, flags) = self.catalog.repository_snapshot();
        let choices = self.catalog.choices.clone();
        let mut existing: Vec<_> = manager
            .entries
            .iter()
            .map(|session| Choice {
                selector: session
                    .key
                    .split_once(" · ")
                    .map_or_else(|| session.key.clone(), |(selector, _)| selector.into()),
                path: session.cwd.clone(),
                name: session.repository.clone(),
                preferred: Some(session.shell.clone()),
            })
            .collect();
        let recent = manager.recent.clone();
        self.begin(Busy::Catalog, move || {
            let mut choices = match choices {
                Some(choices) => choices,
                None => Catalog::choices(&crate::repositories::snapshot(&workspace, &flags)?),
            };
            let mut paths: HashSet<_> = choices.iter().map(|choice| choice.path.clone()).collect();
            existing.retain(|entry| paths.insert(entry.path.clone()));
            choices.extend(existing);
            choices.sort_by_cached_key(|choice| {
                (
                    recent
                        .iter()
                        .position(|path| path == &choice.path)
                        .unwrap_or(usize::MAX),
                    choice.name.to_lowercase(),
                )
            });
            Ok(Completed::Picker(choices, installed_shells()))
        });
    }

    fn pane_size(&self, manager: &Manager, placement: &Placement) -> portable_pty::PtySize {
        let mut arrangement = manager.arrangement.clone();
        let id = match placement {
            Placement::Independent => {
                arrangement.activate(0);
                0
            }
            Placement::Split(source) => {
                arrangement.activate(*source);
                arrangement.split(0);
                0
            }
            Placement::Replace(id) => *id,
        };
        Layout::new(self.size, &arrangement).pane(id).map_or_else(
            || {
                manager
                    .session(id)
                    .map_or(pty_size(self.size), |session| session.size)
            },
            |pane| pane.size(),
        )
    }

    fn start_existing(&mut self, manager: &Manager, placement: Placement, id: SessionId) {
        let Some(active) = manager.session(id) else {
            return;
        };
        let mut session_label = None;
        let busy = if matches!(placement, Placement::Replace(_)) {
            session_label = active.label.clone();
            Busy::Restarting
        } else {
            Busy::Opening
        };
        let (key, cwd, shell, name) = (
            active.key.clone(),
            active.cwd.clone(),
            active.shell.clone(),
            active.repository.clone(),
        );
        let size = self.pane_size(manager, &placement);
        let vi = self.vi;
        self.begin(busy, move || {
            let mut session = Session::start(&key, &cwd, Some(&shell), size, vi)?;
            session.repository = name;
            session.label = session_label;
            Ok(Completed::Session(Box::new(session), placement))
        });
    }

    fn start_choice(&mut self, manager: &Manager, choice: Choice, shell: &'static str) {
        if !self.capacity(manager) {
            return;
        }
        let (workspace, flags) = self.catalog.repository_snapshot();
        let size = self.pane_size(manager, &Placement::Independent);
        let vi = self.vi;
        self.begin(Busy::Opening, move || {
            let (key, cwd, _) =
                crate::repositories::terminal_context(&workspace, &flags, &choice.selector)?;
            // A picker entry is a snapshot. Revalidate and refuse to launch if
            // the registry selector was relinked after it was displayed.
            anyhow::ensure!(
                cwd == choice.path,
                "{}",
                locale(
                    [
                        "Repository location changed. Reopen the terminal workspace.",
                        "Vị trí kho mã đã thay đổi. Hãy mở lại không gian terminal."
                    ],
                    vi
                )
            );
            let mut session = Session::start(&key, &cwd, Some(shell), size, vi)?;
            session.repository = choice.name;
            Ok(Completed::Session(
                Box::new(session),
                Placement::Independent,
            ))
        });
    }

    fn capacity(&mut self, manager: &Manager) -> bool {
        if let Err(error) = manager.ensure_capacity(self.vi) {
            self.notice = Some(error.to_string());
            false
        } else {
            true
        }
    }

    pub(super) fn enabled(&self, manager: &Manager, command: Command) -> bool {
        let active = manager.active();
        let split = manager.arrangement.split_active();
        let capacity = manager.entries.len() < super::MAX_SESSIONS;
        let idle = self.busy.is_none();
        match command {
            Command::New => idle && capacity,
            Command::Split => idle && (split || active.is_some() && capacity),
            Command::Duplicate => idle && active.is_some() && capacity,
            Command::Rename | Command::Close | Command::Restart => idle && active.is_some(),
            Command::FocusPane | Command::MovePane | Command::Maximize => split,
            Command::ResizeLess | Command::ResizeMore => {
                split && !manager.arrangement.is_maximized()
            }
            Command::MoveUp => active
                .and_then(|s| manager.position(s.id))
                .is_some_and(|i| i > 0),
            Command::MoveDown => active
                .and_then(|s| manager.position(s.id))
                .is_some_and(|i| i + 1 < manager.entries.len()),
            Command::Previous | Command::Next => manager.entries.len() > 1,
            Command::Find => active.is_some(),
            Command::Copy => active.zip(self.marked.as_ref()).is_some_and(|(s, mark)| {
                mark.has_range && mark.valid(s.id, &s.output.lock().unwrap())
            }),
            Command::Back => idle,
            Command::Sidebar | Command::Menu => true,
        }
    }

    pub(super) fn sidebar_action_enabled(
        &self,
        manager: &Manager,
        id: SessionId,
        command: Command,
    ) -> bool {
        self.busy.is_none()
            && match command {
                Command::Split => {
                    !manager.arrangement.is_split_member(id)
                        && manager.entries.len() < super::MAX_SESSIONS
                }
                Command::Close => true,
                _ => false,
            }
    }

    fn close_session(&mut self, manager: &mut Manager, id: SessionId) {
        if let Some(session) = manager.session(id) {
            if session.needs_close_confirmation() {
                self.modal = Some(Modal::Confirm(id, Command::Close));
            } else if let Some(closed) = manager.take(id) {
                self.discard(closed);
            }
        }
    }

    pub(super) fn command(&mut self, manager: &mut Manager, command: Command) -> bool {
        if !self.enabled(manager, command) {
            return false;
        }
        let id = manager.arrangement.active();
        self.notice = None;
        match command {
            Command::Back => return true,
            Command::New if self.capacity(manager) => self.picker(manager),
            Command::Sidebar => {
                let arrangement = &mut manager.arrangement;
                if arrangement.sidebar_focused {
                    arrangement.sidebar = Sidebar::Hidden;
                    arrangement.sidebar_focused = false;
                    self.sidebar_query = None;
                    self.sidebar_selected = None;
                } else {
                    arrangement.sidebar = Sidebar::Visible;
                    arrangement.sidebar_focused = true;
                    self.sidebar_selected = arrangement.active();
                }
            }
            Command::Menu => self.open_menu(manager, None),
            Command::Previous | Command::Next => {
                manager.cycle(if command == Command::Previous { -1 } else { 1 });
                if let Some(id) = manager.arrangement.active() {
                    manager.remember(id);
                    self.sidebar_selected = Some(id);
                }
            }
            Command::Split if manager.arrangement.split_active() => manager.arrangement.unsplit(),
            Command::Split | Command::Duplicate if self.capacity(manager) => {
                if let Some(id) = id {
                    let place = if command == Command::Split {
                        Placement::Split(id)
                    } else {
                        Placement::Independent
                    };
                    self.start_existing(manager, place, id);
                }
            }
            Command::MovePane => manager.arrangement.move_pane(),
            Command::FocusPane => {
                manager.arrangement.focus_other();
                if let Some(id) = manager.arrangement.active() {
                    manager.remember(id);
                }
            }
            Command::Maximize => manager.arrangement.toggle_maximized(),
            Command::ResizeLess => manager.arrangement.adjust_split(-5),
            Command::ResizeMore => manager.arrangement.adjust_split(5),
            Command::Find => {
                if let Some(id) = id {
                    self.focus_session(manager, id);
                    self.find = Some(FindBar {
                        id,
                        query: Input::default(),
                        hit: None,
                        generation: 0,
                        columns: 0,
                        excerpt: None,
                    });
                }
            }
            Command::Copy => self.copy_selection(manager, &mut SystemClipboard),
            Command::MoveUp | Command::MoveDown => {
                if let Some(id) = id {
                    manager.reorder(id, if command == Command::MoveUp { -1 } else { 1 });
                }
            }
            Command::Rename => {
                if let Some(session) = manager.active() {
                    let mut input = Input::at_end(session.title().to_owned());
                    input.select_all();
                    self.modal = Some(Modal::Rename(session.id, input));
                }
            }
            Command::Close => {
                if let Some(id) = id {
                    self.close_session(manager, id);
                }
            }
            Command::Restart => {
                if let Some(id) = id {
                    self.modal = Some(Modal::Confirm(id, Command::Restart));
                }
            }
            _ => {}
        }
        false
    }

    pub(super) fn key(&mut self, manager: &mut Manager, layout: &Layout, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if layout.sidebar.is_none() {
            manager.arrangement.sidebar_focused = false;
            self.sidebar_query = None;
            self.sidebar_selected = None;
        }
        if self.modal.is_some() {
            return self.modal_key(manager, key);
        }
        if self.find.is_some() {
            self.key_find(manager, key);
            return false;
        }
        if let Some(command) = Command::from_key(key) {
            if command == Command::Menu {
                let origin = layout.sidebar.map(|area| (area.x, area.y + 1));
                self.open_menu(manager, origin);
                return false;
            }
            return self.command(manager, command);
        }
        self.notice = None;
        if manager.arrangement.sidebar_focused {
            self.key_sidebar(manager, key);
            return false;
        }
        let marked = self.marked.take();
        if key.code == KeyCode::Esc
            && let Some((mark, session)) = marked.zip(manager.active())
            && mark.has_range
            && mark.valid(session.id, &session.output.lock().unwrap())
        {
            return false;
        }
        if let Some(session) = manager.active()
            && let Some(pane) = layout.pane(session.id)
        {
            if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown)
                && key.modifiers == KeyModifiers::SHIFT
            {
                let rows = i32::from(pane.body.height);
                session.scroll(if key.code == KeyCode::PageUp {
                    rows
                } else {
                    -rows
                });
            } else if !session.key(key) {
                self.input_failed();
            }
        }
        false
    }

    fn modal_key(&mut self, manager: &mut Manager, key: KeyEvent) -> bool {
        let mut modal = self.modal.take().expect("modal");
        match (&mut modal, key.code) {
            (
                Modal::Shells {
                    query, previous, ..
                },
                KeyCode::Esc,
            ) => {
                self.modal = Some(Modal::Repositories {
                    query: std::mem::take(query),
                    selected: *previous,
                });
                return false;
            }
            (_, KeyCode::Esc) => return false,
            (_, KeyCode::Enter) => {
                self.modal = Some(modal);
                return self.accept(manager, None);
            }
            (Modal::Menu { selected, .. }, KeyCode::Up | KeyCode::Down) => {
                let commands: Vec<_> = menu_commands().collect();
                for _ in &commands {
                    shift_index(selected, key.code, commands.len());
                    if self.enabled(manager, commands[*selected]) {
                        break;
                    }
                }
            }
            (Modal::Repositories { query, selected }, KeyCode::Up | KeyCode::Down) => {
                shift_index(
                    selected,
                    key.code,
                    self.catalog.filtered(&query.value).len(),
                );
            }
            (Modal::Shells { selected, .. }, KeyCode::Up | KeyCode::Down) => {
                shift_index(selected, key.code, self.catalog.shells.len());
            }
            (Modal::Rename(_, input), _) => {
                input.edit(key, EDIT_LIMIT);
            }
            (Modal::Repositories { query, selected }, _) => {
                if query.edit(key, EDIT_LIMIT) {
                    *selected = 0;
                }
            }
            _ => {}
        }
        self.modal = Some(modal);
        false
    }

    fn accept(&mut self, manager: &mut Manager, index: Option<usize>) -> bool {
        match self.modal.take() {
            Some(Modal::Menu { selected, origin }) => {
                if let Some(command) = menu_commands().nth(index.unwrap_or(selected))
                    && self.enabled(manager, command)
                {
                    return self.command(manager, command);
                }
                self.modal = Some(Modal::Menu { selected, origin });
            }
            Some(Modal::Repositories { query, selected }) => {
                let previous = index.unwrap_or(selected);
                if let Some(choice) = self.catalog.filtered(&query.value).get(previous) {
                    self.modal = Some(Modal::Shells {
                        repository: (*choice).clone(),
                        selected: self.catalog.preferred(choice),
                        previous,
                        query,
                    });
                } else {
                    self.modal = Some(Modal::Repositories { query, selected });
                }
            }
            Some(Modal::Shells {
                repository,
                selected,
                query,
                previous,
            }) => {
                if let Some(&shell) = self.catalog.shells.get(index.unwrap_or(selected)) {
                    self.start_choice(manager, repository, shell);
                } else {
                    self.modal = Some(Modal::Shells {
                        repository,
                        selected,
                        query,
                        previous,
                    });
                }
            }
            Some(Modal::Rename(id, input)) => {
                if let Some(index) = manager.position(id) {
                    manager.entries[index].rename(&input.value);
                }
            }
            Some(Modal::Confirm(id, Command::Close)) => {
                if let Some(closed) = manager.take(id) {
                    self.discard(closed);
                }
            }
            Some(Modal::Confirm(id, Command::Restart)) => {
                self.start_existing(manager, Placement::Replace(id), id);
            }
            _ => {}
        }
        false
    }

    pub(super) fn paste(&mut self, manager: &Manager, layout: &Layout, value: String) {
        if let Some(find) = &mut self.find {
            if find.query.paste_sanitized(&value, EDIT_LIMIT) == Ok(true) {
                self.search_terminal(manager, false, false);
            }
            return;
        }
        if manager.arrangement.sidebar_focused && self.modal.is_none() {
            if let Some(query) = &mut self.sidebar_query
                && query.paste_sanitized(&value, EDIT_LIMIT) == Ok(true)
            {
                self.sidebar_selected = self.sidebar_ids(manager).first().copied();
            }
            return;
        }
        self.marked = None;
        match &mut self.modal {
            Some(Modal::Rename(_, input)) => {
                let _ = input.paste_sanitized(&value, EDIT_LIMIT);
            }
            Some(Modal::Repositories { query, selected }) => {
                if query.paste_sanitized(&value, EDIT_LIMIT) == Ok(true) {
                    *selected = 0;
                }
            }
            None if !manager.arrangement.sidebar_focused => {
                if let Some(session) = manager
                    .active()
                    .filter(|session| layout.pane(session.id).is_some())
                {
                    if value.len() > MAX_PASTE {
                        self.notice = Some(
                            locale(
                                [
                                    "Paste exceeds 32 KiB. Paste smaller parts.",
                                    "Nội dung dán vượt 32 KiB. Hãy chia thành các phần nhỏ.",
                                ],
                                self.vi,
                            )
                            .into(),
                        );
                    } else if !session.paste(value) {
                        self.input_failed();
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn mouse(
        &mut self,
        manager: &mut Manager,
        layout: &Layout,
        hits: &Hits,
        mouse: MouseEvent,
    ) -> bool {
        use MouseButton::{Left, Right};
        use MouseEventKind::{Down, Drag, Moved, ScrollDown, ScrollUp, Up};

        let (x, y) = (mouse.column, mouse.row);
        let hit = hits.at(x, y);
        if mouse.kind == Down(Left) {
            self.divider_drag = None;
            if let Some(mark) = &mut self.marked {
                mark.dragging = false;
            }
        }
        if let Drag(button) | Up(button) = mouse.kind
            && let Some(capture) = self.native_mouse[button_index(button)].as_mut()
        {
            if let Some(pane) = layout.pane(capture.id) {
                capture.area = pane.body;
            }
            capture.event = mouse;
            let sent = manager
                .session(capture.id)
                .and_then(|session| session.mouse(mouse, capture.area, true));
            if matches!(mouse.kind, Up(_)) {
                self.native_mouse[button_index(button)] = None;
            }
            if sent == Some(false) {
                self.input_failed();
            }
            return false;
        }
        if self.modal.is_some() {
            if matches!(mouse.kind, Down(Left)) {
                match hit {
                    Some(Hit::Step(index)) => return self.accept(manager, Some(index)),
                    Some(Hit::Accept) => return self.accept(manager, None),
                    Some(Hit::Cancel) => {
                        return self
                            .modal_key(manager, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                    }
                    None if matches!(self.modal, Some(Modal::Menu { .. })) => self.modal = None,
                    _ => {}
                }
            }
            if let (Moved, Some(Hit::Step(index)), Some(Modal::Menu { selected, .. })) =
                (mouse.kind, hit, &mut self.modal)
            {
                *selected = index;
            }
            if matches!(mouse.kind, ScrollUp | ScrollDown) {
                let code = if mouse.kind == ScrollUp {
                    KeyCode::Up
                } else {
                    KeyCode::Down
                };
                return self.modal_key(manager, KeyEvent::new(code, KeyModifiers::NONE));
            }
            return false;
        }
        if self.find.is_some() {
            if mouse.kind == Down(Left) {
                match hit {
                    Some(Hit::FindNext) => self.search_terminal(manager, false, true),
                    Some(Hit::FindPrevious) => self.search_terminal(manager, true, true),
                    Some(Hit::Cancel) => self.find = None,
                    _ => {}
                }
            }
            return false;
        }
        if let Some(panes) = self.divider_drag
            && matches!(mouse.kind, Drag(Left) | Up(Left))
        {
            let same_group = manager.arrangement.panes() == panes.map(Some);
            if same_group && let Some(ratio) = layout.ratio_at(x, y) {
                manager.arrangement.set_split_ratio(ratio);
            }
            if !same_group || mouse.kind == Up(Left) {
                self.divider_drag = None;
            }
            return false;
        }
        if matches!(mouse.kind, Drag(Left) | Up(Left))
            && let Some(marked) = &mut self.marked
            && marked.dragging
            && let Some(pane) = layout.pane(marked.id)
            && let Some(session) = manager.session(marked.id)
        {
            let output = session.output.lock().unwrap();
            if marked.valid(session.id, &output) && pane.body.width > 0 && pane.body.height > 0 {
                let row = y.clamp(pane.body.y, pane.body.bottom() - 1) - pane.body.y;
                let col = x.clamp(pane.body.x, pane.body.right() - 1) - pane.body.x;
                marked.cells.extend(output.parser.screen(), row, col);
                marked.has_range = (row, col) != marked.anchor;
            }
            marked.dragging = mouse.kind != Up(Left);
            return false;
        }
        if matches!(mouse.kind, Drag(_) | Up(_)) {
            return false;
        }
        if mouse.kind == Down(Left)
            && layout.divider_at(x, y)
            && let [Some(first), Some(second)] = manager.arrangement.panes()
        {
            self.divider_drag = Some([first, second]);
            self.marked = None;
            return false;
        }
        if mouse.kind == Moved {
            self.sidebar_hover = match hit {
                Some(Hit::Session(id) | Hit::SessionAction(id, _)) => Some(id),
                _ => None,
            };
            self.hovered_command = match hit {
                Some(Hit::Command(cmd) | Hit::SessionAction(_, cmd)) => Some(cmd),
                _ => None,
            };
        }
        match (hit, mouse.kind) {
            (Some(Hit::Command(Command::Menu)), Down(Left)) => {
                self.open_menu(manager, Some((x, y.saturating_add(1))));
                return false;
            }
            (Some(Hit::Command(command)), Down(Left)) => return self.command(manager, command),
            (Some(Hit::SidebarSearch), Down(Left)) => {
                manager.arrangement.sidebar_focused = true;
                self.sidebar_query.get_or_insert_with(Input::default);
                self.sidebar_selected = manager.arrangement.active();
                return false;
            }
            (Some(Hit::SessionAction(id, command)), Down(Left)) => {
                if manager.session(id).is_some()
                    && self.sidebar_action_enabled(manager, id, command)
                {
                    match command {
                        Command::Split => self.start_existing(manager, Placement::Split(id), id),
                        Command::Close => self.close_session(manager, id),
                        _ => {}
                    }
                }
                return false;
            }
            (Some(Hit::Session(id) | Hit::SessionAction(id, _)), Down(Left | Right)) => {
                self.focus_session(manager, id);
                if mouse.kind == Down(Right) {
                    self.open_menu(manager, Some((x, y.saturating_add(1))));
                }
                return false;
            }
            (Some(Hit::PaneActions(id)), Down(Left | Right)) => {
                self.focus_session(manager, id);
                self.open_menu(manager, Some((x, y.saturating_add(1))));
                return false;
            }
            _ => {}
        }
        if layout.sidebar.is_some_and(|area| area.contains(x, y)) {
            if matches!(mouse.kind, ScrollUp | ScrollDown) {
                self.sidebar_hover = None;
                self.hovered_command = None;
                self.step_sidebar(manager, if mouse.kind == ScrollUp { -1 } else { 1 });
            }
            return false;
        }
        let Some(pane) = layout
            .panes
            .into_iter()
            .flatten()
            .find(|pane| pane.contains(x, y))
        else {
            if mouse.kind == Down(Left) {
                self.marked = None;
            }
            return false;
        };
        let on_header = !pane.body.contains(x, y);
        let changing_focus =
            manager.arrangement.active() != Some(pane.id) || manager.arrangement.sidebar_focused;
        let point = (x.saturating_sub(pane.body.x), y.saturating_sub(pane.body.y));
        if matches!(mouse.kind, Down(Left | Right)) {
            if changing_focus {
                self.focus_session(manager, pane.id);
            }
            if mouse.kind == Down(Right) && (on_header || changing_focus) {
                self.open_menu(manager, Some((x, y.saturating_add(1))));
                return false;
            }
        }
        if on_header {
            return false;
        }
        let Some(session) = manager.session(pane.id) else {
            return false;
        };
        // Native applications get mouse events when mouse tracking is enabled.
        // Shift+drag explicitly selects terminal text instead.
        let native = if !changing_focus && !mouse.modifiers.contains(KeyModifiers::SHIFT) {
            session.mouse(mouse, pane.body, false)
        } else {
            None
        };
        match native {
            Some(false) => self.input_failed(),
            Some(true) => {
                self.marked = None;
                if let Down(button) = mouse.kind {
                    self.native_mouse[button_index(button)] = Some(MouseCapture {
                        id: pane.id,
                        area: pane.body,
                        event: mouse,
                    });
                }
            }
            None => match mouse.kind {
                Down(Left) => {
                    let output = session.output.lock().unwrap();
                    let screen = output.parser.screen();
                    let (rows, cols) = screen.size();
                    self.marked = Some(Marked {
                        id: pane.id,
                        cells: Selection::new(screen, point.1, point.0),
                        generation: output.generation,
                        offset: screen.scrollback(),
                        size: (rows.get(), cols.get()),
                        snapshot: screen.contents(),
                        anchor: (point.1, point.0),
                        has_range: false,
                        dragging: true,
                    });
                }
                Down(Right) => self.open_menu(manager, Some((x, y.saturating_add(1)))),
                ScrollUp | ScrollDown => {
                    self.marked = None;
                    session.scroll(if mouse.kind == ScrollUp { 3 } else { -3 });
                }
                _ => {}
            },
        }
        false
    }

    fn release_mouse(&mut self, manager: &Manager) {
        for (index, capture) in self.native_mouse.iter_mut().enumerate() {
            let Some(mut capture) = capture.take() else {
                continue;
            };
            let button = [MouseButton::Left, MouseButton::Right, MouseButton::Middle][index];
            capture.event.kind = MouseEventKind::Up(button);
            if let Some(session) = manager.session(capture.id) {
                session.mouse(capture.event, capture.area, true);
            }
        }
    }

    fn open_menu(&mut self, manager: &Manager, origin: Option<(u16, u16)>) {
        self.modal = Some(Modal::Menu {
            selected: menu_commands()
                .position(|command| self.enabled(manager, command))
                .unwrap_or_default(),
            origin,
        });
    }

    pub(super) fn sidebar_ids(&self, manager: &Manager) -> Vec<SessionId> {
        self.sidebar_rows(manager).0
    }

    pub(super) fn sidebar_rows(&self, manager: &Manager) -> (Vec<SessionId>, Vec<Range<usize>>) {
        let query = self
            .sidebar_query
            .as_ref()
            .map(|query| query.value.to_lowercase());
        let mut ids: Vec<_> = manager
            .entries
            .iter()
            .filter(|s| {
                query.as_ref().is_none_or(|q| {
                    s.title().to_lowercase().contains(q)
                        || s.repository.to_lowercase().contains(q)
                        || s.shell.contains(q)
                })
            })
            .map(|s| s.id)
            .collect();
        let groups = manager.arrangement.group_sidebar(&mut ids);
        (ids, groups)
    }

    fn step_sidebar(&mut self, manager: &Manager, step: isize) {
        let ids = self.sidebar_ids(manager);
        let current = self
            .sidebar_selected
            .or(manager.arrangement.active())
            .and_then(|id| ids.iter().position(|entry| *entry == id))
            .unwrap_or_default();
        self.sidebar_selected = ids
            .get(
                current
                    .saturating_add_signed(step)
                    .min(ids.len().saturating_sub(1)),
            )
            .copied();
    }

    fn focus_session(&mut self, manager: &mut Manager, id: SessionId) {
        manager.arrangement.activate(id);
        manager.remember(id);
        manager.arrangement.sidebar_focused = false;
        self.sidebar_selected = None;
        self.sidebar_query = None;
        self.marked = None;
    }

    fn key_sidebar(&mut self, manager: &mut Manager, key: KeyEvent) {
        match key.code {
            KeyCode::Esc if self.sidebar_query.take().is_some() => {
                self.sidebar_selected = manager.arrangement.active();
            }
            KeyCode::Esc => {
                manager.arrangement.sidebar_focused = false;
                self.sidebar_selected = None;
            }
            KeyCode::Up => self.step_sidebar(manager, -1),
            KeyCode::Down => self.step_sidebar(manager, 1),
            KeyCode::PageUp => self.step_sidebar(manager, -5),
            KeyCode::PageDown => self.step_sidebar(manager, 5),
            KeyCode::Enter => {
                if let Some(id) = self
                    .sidebar_selected
                    .or(manager.arrangement.active())
                    .filter(|id| self.sidebar_ids(manager).contains(id))
                {
                    self.focus_session(manager, id);
                }
            }
            KeyCode::Char('/') if self.sidebar_query.is_none() => {
                self.sidebar_query = Some(Input::default())
            }
            _ => {
                if let Some(query) = &mut self.sidebar_query
                    && query.edit(key, EDIT_LIMIT)
                {
                    self.sidebar_selected = self.sidebar_ids(manager).first().copied();
                }
            }
        }
    }

    fn key_find(&mut self, manager: &Manager, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.find = None,
            KeyCode::Enter | KeyCode::F(3) => {
                self.search_terminal(manager, key.modifiers.contains(KeyModifiers::SHIFT), true)
            }
            _ => {
                if self
                    .find
                    .as_mut()
                    .is_some_and(|find| find.query.edit(key, EDIT_LIMIT))
                {
                    self.search_terminal(manager, false, false);
                }
            }
        }
    }

    fn search_terminal(&mut self, manager: &Manager, reverse: bool, advance: bool) {
        let Some(find) = &mut self.find else { return };
        let Some(session) = manager.session(find.id) else {
            return;
        };
        let mut output = session.output.lock().unwrap();
        find.generation = output.generation;
        let screen = output.parser.screen_mut();
        let columns = screen.size().1.get();
        let previous = if advance && columns == find.columns {
            find.hit
        } else {
            None
        };
        find.columns = columns;
        find.hit = if find.query.value.trim().is_empty() {
            None
        } else {
            scrollback::find(screen, &find.query.value, reverse, previous)
        };
        find.excerpt = find.hit.and_then(|hit| hit.visible_text(screen));
        output.changed();
    }

    pub(super) fn copy_selection(&mut self, manager: &Manager, clipboard: &mut impl Clipboard) {
        let Some((marked, session)) = self.marked.as_ref().zip(manager.active()) else {
            return;
        };
        let text = {
            let output = session.output.lock().unwrap();
            if !marked.has_range || !marked.valid(session.id, &output) {
                return;
            }
            marked.cells.text(output.parser.screen())
        };
        if !text.is_empty() {
            self.notice = Some(
                if clipboard.set_text(&text).is_ok() {
                    locale(["Selection copied", "Đã sao chép vùng chọn"], self.vi)
                } else {
                    locale(
                        ["Clipboard unavailable", "Không thể dùng bảng nhớ tạm"],
                        self.vi,
                    )
                }
                .into(),
            );
        }
    }

    fn input_failed(&mut self) {
        self.notice = Some(
            locale(
                [
                    "Terminal input unavailable or busy",
                    "Thiết bị nhập terminal không khả dụng hoặc đang bận",
                ],
                self.vi,
            )
            .into(),
        );
    }
}

fn shift_index(index: &mut usize, code: KeyCode, length: usize) {
    if length == 0 {
        *index = 0;
        return;
    }
    *index = if code == KeyCode::Up {
        (*index + length - 1) % length
    } else {
        (*index + 1) % length
    };
}

pub(super) fn interact<W: io::Write>(
    manager: &mut Manager,
    screen: &mut Screen<W>,
    size: (u16, u16),
    catalog: Catalog,
    vi: bool,
) -> Result<()> {
    let (events, receiver) = mpsc::sync_channel(128);
    for session in &manager.entries {
        session.watch(Some(&events));
    }
    let mut state = State::new(size, catalog, vi, events.clone());
    let mut painter = Painter::default();
    let color = color_enabled();
    let stopping = AtomicBool::new(false);
    let result = thread::scope(|scope| -> Result<()> {
        let pump = thread::Builder::new()
            .name("cnl-pty-events".into())
            .spawn_scoped(scope, || input_events(&events, &stopping))?;
        let mut interact = || -> Result<()> {
            loop {
                let layout = Layout::new(state.size, &manager.arrangement);
                if layout.sidebar.is_none() {
                    manager.arrangement.sidebar_focused = false;
                    state.sidebar_query = None;
                    state.sidebar_selected = None;
                }
                manager.resize_visible(&layout);
                let (frame, hits) = render::frame(manager, &layout, &state);
                painter.draw(&mut screen.out, frame, color)?;
                let first = receiver.recv()?;
                // Coalesce a bounded batch. Mouse geometry reflects the frame
                // on screen until the current batch has been processed.
                for input in std::iter::once(first).chain(receiver.try_iter().take(31)) {
                    match input {
                        WorkspaceEvent::Wake => continue,
                        WorkspaceEvent::Job(result) => {
                            state.complete(manager, result);
                        }
                        WorkspaceEvent::Input(event) => match event? {
                            Event::Resize(cols, rows) => {
                                state.size = (cols, rows);
                                state.marked = None;
                            }
                            Event::FocusLost => {
                                state.release_mouse(manager);
                                state.divider_drag = None;
                                state.marked = None;
                                state.sidebar_hover = None;
                                state.hovered_command = None;
                            }
                            Event::Key(key) => {
                                if state.key(manager, &layout, key) {
                                    return Ok(());
                                }
                            }
                            Event::Mouse(mouse) => {
                                if state.mouse(manager, &layout, &hits, mouse) {
                                    return Ok(());
                                }
                            }
                            Event::Paste(value) => state.paste(manager, &layout, value),
                            _ => {}
                        },
                    }
                    // A modal, focus or arrangement may change its hit regions.
                    break;
                }
            }
        };
        let result = interact();
        stopping.store(true, Ordering::Release);
        let _ = pump.join();
        result
    });
    state.release_mouse(manager);
    for session in &manager.entries {
        session.watch(None);
    }
    result
}

fn input_events(events: &SyncSender<WorkspaceEvent>, stopping: &AtomicBool) {
    while !stopping.load(Ordering::Acquire) {
        let input = match event::poll(POLL_INTERVAL) {
            Ok(true) => event::read(),
            Ok(false) => continue,
            Err(error) => Err(error),
        };
        let failed = input.is_err();
        let mut pending = WorkspaceEvent::Input(input);
        loop {
            if stopping.load(Ordering::Acquire) {
                return;
            }
            match events.try_send(pending) {
                Ok(()) => break,
                Err(TrySendError::Full(value)) => {
                    pending = value;
                    thread::park_timeout(Duration::from_millis(2));
                }
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
        if failed {
            return;
        }
    }
}
