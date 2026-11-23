use std::{
    collections::HashSet,
    env, io,
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
        editor::Input,
        terminal::{Painter, Screen, color_enabled},
    },
    Manager, Result, Row, Session, SessionId, Workspace,
    backend::MAX_PASTE,
    layout::{Layout, Sidebar},
    locale, pty_size,
    render::{self, Hit, Hits},
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

pub(super) fn menu_commands(manager: &Manager) -> Vec<Command> {
    let mut commands = vec![Command::New];
    if let Some(session) = manager.active() {
        commands.push(Command::Split);
        if manager.arrangement.split_active() {
            commands.push(Command::MovePane);
        }
        commands.extend([Command::Rename, Command::Duplicate]);
        let current = manager.position(session.id).unwrap();
        if current > 0 {
            commands.push(Command::MoveUp);
        }
        if current + 1 < manager.entries.len() {
            commands.push(Command::MoveDown);
        }
        commands.extend([Command::Restart, Command::Close]);
    }
    commands.push(Command::Back);
    commands
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
    Active,
    Split,
    Replace(SessionId),
}

pub(super) enum Completed {
    Picker(Vec<Choice>, Vec<&'static str>),
    Session(Box<Session>, Placement),
    Closed,
}

pub(super) struct State {
    pub(super) modal: Option<Modal>,
    pub(super) busy: Option<Busy>,
    pub(super) notice: Option<String>,
    pub(super) catalog: Catalog,
    pub(super) size: (u16, u16),
    pub(super) vi: bool,
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
                let mut session = *session;
                session.watch(Some(&self.events));
                match placement {
                    Placement::Active => {
                        let id = manager.insert(session);
                        manager.arrangement.activate(id);
                    }
                    Placement::Split => {
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
                if self.size.0 < 54 {
                    manager.arrangement.sidebar = Sidebar::Auto;
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
        self.begin(Busy::Catalog, move || {
            let mut choices = match choices {
                Some(choices) => choices,
                None => Catalog::choices(&crate::repositories::snapshot(&workspace, &flags)?),
            };
            let mut paths: HashSet<_> = choices.iter().map(|choice| choice.path.clone()).collect();
            existing.retain(|entry| paths.insert(entry.path.clone()));
            choices.extend(existing);
            choices.sort_by_cached_key(|choice| choice.name.to_lowercase());
            Ok(Completed::Picker(choices, installed_shells()))
        });
    }

    fn pane_size(&self, manager: &Manager, placement: &Placement) -> portable_pty::PtySize {
        let mut arrangement = manager.arrangement.clone();
        let id = match placement {
            Placement::Active => {
                arrangement.activate(0);
                0
            }
            Placement::Split => {
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
        let size = self.pane_size(manager, &Placement::Active);
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
            Ok(Completed::Session(Box::new(session), Placement::Active))
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

    pub(super) fn command(&mut self, manager: &mut Manager, command: Command) -> bool {
        let id = manager.arrangement.active();
        if self.busy.is_some()
            && matches!(
                command,
                Command::Back
                    | Command::New
                    | Command::Split
                    | Command::Duplicate
                    | Command::Close
                    | Command::Restart
            )
        {
            return false;
        }
        self.notice = None;
        match command {
            Command::Back => return true,
            Command::New if self.capacity(manager) => self.picker(manager),
            Command::Sidebar => {
                let arrangement = &mut manager.arrangement;
                if arrangement.sidebar_focused {
                    arrangement.sidebar = Sidebar::Hidden;
                    arrangement.sidebar_focused = false;
                } else {
                    arrangement.sidebar = Sidebar::Visible;
                    arrangement.sidebar_focused = true;
                }
            }
            Command::Menu => {
                self.modal = Some(Modal::Menu {
                    selected: 0,
                    origin: None,
                });
            }
            Command::Previous => manager.cycle(-1),
            Command::Next => manager.cycle(1),
            Command::Split if manager.arrangement.split_active() => manager.arrangement.unsplit(),
            Command::Split | Command::Duplicate if self.capacity(manager) => {
                if let Some(id) = id {
                    let place = if command == Command::Split {
                        Placement::Split
                    } else {
                        Placement::Active
                    };
                    self.start_existing(manager, place, id);
                }
            }
            Command::MovePane => manager.arrangement.move_pane(),
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
            Command::Close | Command::Restart => {
                if let Some(session) = manager.active() {
                    if command == Command::Restart || session.needs_close_confirmation() {
                        self.modal = Some(Modal::Confirm(session.id, command));
                    } else if let Some(closed) = manager.take(session.id) {
                        self.discard(closed);
                    }
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
        if self.modal.is_some() {
            return self.modal_key(manager, key);
        }
        if let Some(command) = Command::from_key(key) {
            return self.command(manager, command);
        }
        self.notice = None;
        if manager.arrangement.sidebar_focused {
            match key.code {
                KeyCode::Esc => {
                    manager.arrangement.sidebar_focused = false;
                    if self.size.0 < 54 {
                        manager.arrangement.sidebar = Sidebar::Auto;
                    }
                }
                KeyCode::Up => manager.cycle(-1),
                KeyCode::Down => manager.cycle(1),
                KeyCode::Enter => {
                    manager.arrangement.sidebar_focused = false;
                    if self.size.0 < 54 {
                        manager.arrangement.sidebar = Sidebar::Auto;
                    }
                }
                _ => {}
            }
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
                shift_index(selected, key.code, menu_commands(manager).len());
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
            Some(Modal::Menu { selected, .. }) => {
                if let Some(command) = menu_commands(manager).get(index.unwrap_or(selected)) {
                    return self.command(manager, *command);
                }
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
        use MouseEventKind::{Down, ScrollDown, ScrollUp};

        let (x, y) = (mouse.column, mouse.row);
        let hit = hits.at(x, y);
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

        match (hit, mouse.kind) {
            (Some(Hit::Command(command)), Down(Left)) => return self.command(manager, command),
            (Some(Hit::Session(id)), Down(Left | Right)) => {
                manager.arrangement.activate(id);
                manager.arrangement.sidebar_focused = false;
                if self.size.0 < 54 {
                    manager.arrangement.sidebar = Sidebar::Auto;
                }
                if mouse.kind == Down(Right) {
                    self.context_at(x, y);
                }
                return false;
            }
            _ => {}
        }
        if layout.sidebar.is_some_and(|area| area.contains(x, y)) {
            if matches!(mouse.kind, ScrollUp | ScrollDown) {
                manager.cycle(if mouse.kind == ScrollUp { -1 } else { 1 });
            }
            return false;
        }
        let Some(pane) = layout
            .panes
            .into_iter()
            .flatten()
            .find(|pane| pane.contains(x, y))
        else {
            return false;
        };
        let on_header = !pane.body.contains(x, y);
        let changing_focus =
            manager.arrangement.active() != Some(pane.id) || manager.arrangement.sidebar_focused;
        if matches!(mouse.kind, Down(Left | Right)) {
            manager.arrangement.activate(pane.id);
            manager.arrangement.sidebar_focused = false;
            if mouse.kind == Down(Right) && (on_header || changing_focus) {
                self.context_at(x, y);
                return false;
            }
        }
        if changing_focus || on_header {
            return false;
        }
        let Some(session) = manager.session(pane.id) else {
            return false;
        };
        match session.mouse(mouse, (pane.body.x, pane.body.y)) {
            Some(false) => self.input_failed(),
            Some(true) => {}
            None => match mouse.kind {
                Down(Right) => self.context_at(x, y),
                ScrollUp => session.scroll(3),
                ScrollDown => session.scroll(-3),
                _ => {}
            },
        }
        false
    }

    fn context_at(&mut self, x: u16, y: u16) {
        self.modal = Some(Modal::Menu {
            selected: 0,
            origin: Some((x, y)),
        });
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
                            Event::Resize(cols, rows) => state.size = (cols, rows),
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
