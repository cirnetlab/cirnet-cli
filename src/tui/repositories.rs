mod actions;
#[cfg(test)]
mod tests;
mod view;

use std::{
    collections::BTreeSet,
    fs,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use clap::ArgMatches;
use crossterm::{
    event::{Event, KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEventKind},
    terminal as console,
};
use serde::{Deserialize, Serialize};

use self::{
    actions::{Action, Plan, Target},
    view::Layout,
};
use super::{
    OutputView,
    clipboard::{Clipboard, SystemClipboard},
    control,
    editor::Input,
    read_events,
    runner::Runner,
    sessions::Manager,
    terminal::{Frame, Painter, Screen, Tone, color_enabled},
};
use crate::{
    environment::write_json,
    repositories::{Row, group_names, registry_directory, snapshot, terminal_context},
    workspace::Workspace,
};

const SEARCH_LIMIT: usize = 256;
const INPUT_LIMIT: usize = 8192;

fn tr(vi: bool, en: &'static str, vn: &'static str) -> &'static str {
    if vi { vn } else { en }
}

#[derive(Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Sort {
    #[default]
    Name,
    Branch,
    Status,
    Activity,
}

impl Sort {
    fn label(self, vi: bool) -> &'static str {
        match self {
            Self::Name => tr(vi, "Name", "Tên"),
            Self::Branch => tr(vi, "Branch", "Nhánh"),
            Self::Status => tr(vi, "Status", "Trạng thái"),
            Self::Activity => tr(vi, "Activity", "Hoạt động"),
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Name => Self::Branch,
            Self::Branch => Self::Status,
            Self::Status => Self::Activity,
            Self::Activity => Self::Name,
        }
    }
}

#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Filters {
    search: String,
    group: Option<String>,
    sort: Sort,
}

enum Overlay {
    None,
    Details {
        scroll: usize,
    },
    Actions {
        items: Vec<Action>,
        cursor: usize,
    },
    Groups {
        cursor: usize,
    },
    Prompt {
        action: Action,
        input: Input,
        targets: Vec<Target>,
    },
    Preview {
        plan: Plan,
        scroll: usize,
    },
    Output,
}

enum Effect {
    Idle,
    Redraw,
    Refresh,
    Cancel,
    Exit,
    Run { args: Vec<String>, action: Action },
    Terminal(String),
}

struct Dashboard {
    rows: Vec<Row>,
    visible: Vec<usize>,
    groups: Vec<String>,
    checked: BTreeSet<String>,
    cursor: usize,
    scroll: usize,
    filters: Filters,
    search: Input,
    searching: bool,
    update_only: bool,
    update_jobs: Option<u8>,
    vi: bool,
    overlay: Overlay,
    output: Option<OutputView>,
    notice: String,
    loading: bool,
}

impl Dashboard {
    fn new(vi: bool, update_only: bool) -> Self {
        Self {
            rows: Vec::new(),
            visible: Vec::new(),
            groups: Vec::new(),
            checked: BTreeSet::new(),
            cursor: 0,
            scroll: 0,
            filters: Filters::default(),
            search: Input::default(),
            searching: false,
            update_only,
            update_jobs: None,
            vi,
            overlay: Overlay::None,
            output: None,
            notice: String::new(),
            loading: true,
        }
    }

    fn row(&self, visible: usize) -> Option<&Row> {
        self.visible.get(visible).map(|index| &self.rows[*index])
    }

    fn focused(&self) -> Option<&Row> {
        self.row(self.cursor)
    }

    fn replace_rows(&mut self, rows: Vec<Row>) {
        let focus = self.focused().map(|row| row.selector.clone());
        self.rows = rows;
        let identifiers: BTreeSet<_> = self.rows.iter().map(|row| row.selector.as_str()).collect();
        self.checked
            .retain(|item| identifiers.contains(item.as_str()));
        self.rebuild(focus);
    }

    fn refilter(&mut self) {
        self.filters.search.clone_from(&self.search.value);
        self.rebuild(self.focused().map(|row| row.selector.clone()));
    }

    fn rebuild(&mut self, focus: Option<String>) {
        let query = self.filters.search.to_lowercase();
        let terms: Vec<_> = query.split_whitespace().collect();
        self.visible = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| !self.update_only || row.registered)
            .filter(|(_, row)| {
                self.filters
                    .group
                    .as_ref()
                    .is_none_or(|group| row.groups.contains(group))
            })
            .filter(|(_, row)| {
                terms.is_empty() || {
                    let local = row
                        .path
                        .as_deref()
                        .map(Path::to_string_lossy)
                        .unwrap_or_default();
                    let status = view::state_label(&row.state, self.vi).0;
                    let fields = format!(
                        "{} {} {} {} {} {} {} {}",
                        row.name,
                        row.alias.as_deref().unwrap_or_default(),
                        local,
                        row.branch.as_deref().unwrap_or_default(),
                        row.state,
                        status,
                        row.remote.as_deref().unwrap_or_default(),
                        row.groups.join(" ")
                    )
                    .to_lowercase();
                    terms.iter().all(|term| fields.contains(term))
                }
            })
            .map(|(index, _)| index)
            .collect();
        // Tie-break by identity so a refresh keeps the same view order.
        self.visible.sort_by_cached_key(|index| {
            let row = &self.rows[*index];
            let key = match self.filters.sort {
                Sort::Branch => row.branch.as_deref().unwrap_or("~").to_lowercase(),
                Sort::Status => row.state.clone(),
                _ => String::new(),
            };
            let activity =
                (self.filters.sort == Sort::Activity).then_some(std::cmp::Reverse(row.last_sync));
            (
                key,
                activity,
                row.name.to_lowercase(),
                row.selector.as_str(),
            )
        });
        self.cursor = self
            .visible
            .iter()
            .position(|index| focus.as_deref() == Some(self.rows[*index].selector.as_str()))
            .unwrap_or(self.cursor)
            .min(self.visible.len().saturating_sub(1));
    }

    fn focus(&mut self, index: usize) {
        self.cursor = index.min(self.visible.len().saturating_sub(1));
    }

    fn reveal(&mut self, page: usize) {
        let page = page.max(1);
        self.scroll = self.scroll.min(self.visible.len().saturating_sub(page));
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + page {
            self.scroll = self.cursor + 1 - page;
        }
    }

    fn toggle(&mut self, index: usize) {
        if let Some(row) = self.row(index) {
            let selector = row.selector.clone();
            if !self.checked.remove(&selector) {
                self.checked.insert(selector);
            }
        }
    }

    fn targets(&self) -> Vec<&Row> {
        if self.checked.is_empty() {
            self.focused().into_iter().collect()
        } else {
            self.rows
                .iter()
                .filter(|row| self.checked.contains(&row.selector))
                .collect()
        }
    }

    fn choose(&mut self, index: usize) -> Effect {
        match &self.overlay {
            Overlay::Actions { items, .. } => items
                .get(index)
                .copied()
                .map_or(Effect::Idle, |action| self.activate(action)),
            Overlay::Groups { .. } if index <= self.groups.len() => {
                self.filters.group = index
                    .checked_sub(1)
                    .and_then(|group| self.groups.get(group).cloned());
                self.overlay = Overlay::None;
                self.refilter();
                Effect::Redraw
            }
            _ => Effect::Idle,
        }
    }

    fn edit(&mut self, key: KeyEvent) -> Effect {
        if let Overlay::Prompt { input, .. } = &mut self.overlay {
            if input.edit(key, INPUT_LIMIT) {
                self.notice.clear();
                return Effect::Redraw;
            }
        } else if self.searching && self.search.edit(key, SEARCH_LIMIT) {
            self.refilter();
            return Effect::Redraw;
        }
        Effect::Idle
    }

    fn paste(&mut self, text: &str) -> Effect {
        if let Overlay::Prompt { input, .. } = &mut self.overlay {
            return if input
                .paste_sanitized(text, INPUT_LIMIT)
                .is_ok_and(|changed| changed)
            {
                Effect::Redraw
            } else {
                Effect::Idle
            };
        }
        if self.searching
            && self
                .search
                .paste_sanitized(text, SEARCH_LIMIT)
                .is_ok_and(|changed| changed)
        {
            self.refilter();
            return Effect::Redraw;
        }
        Effect::Idle
    }

    fn handle(&mut self, event: Event, frame: Option<&Frame>) -> Effect {
        let layout = Layout::new(frame.map_or((80, 20), |frame| frame.size));
        let page = layout.table.len().max(1);
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key, page),
            Event::Paste(value) => self.paste(&value),
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    let Some(frame) = frame else {
                        return Effect::Idle;
                    };
                    if matches!(self.overlay, Overlay::Preview { .. })
                        && frame.suggestion_hit(mouse.column, mouse.row).is_some()
                    {
                        return self.confirm();
                    }
                    if let Some(hit) = frame
                        .input
                        .as_ref()
                        .filter(|hit| hit.row == mouse.row && hit.columns.contains(&mouse.column))
                    {
                        let input = match &mut self.overlay {
                            Overlay::Prompt { input, .. } => Some(input),
                            Overlay::None => {
                                self.searching = true;
                                Some(&mut self.search)
                            }
                            _ => None,
                        };
                        if let Some(input) = input {
                            let offset = input.offset_at_column(
                                hit.offset,
                                usize::from(mouse.column - hit.columns.start),
                            );
                            input.move_to(offset, false);
                            return Effect::Redraw;
                        }
                    }
                    if let Some(index) = frame.menu_hit(mouse.column, mouse.row) {
                        if matches!(self.overlay, Overlay::None) {
                            self.focus(index);
                            if frame
                                .menu
                                .as_ref()
                                .is_some_and(|menu| mouse.column < menu.columns.start + 4)
                            {
                                self.toggle(index);
                            }
                            self.searching = false;
                            return Effect::Redraw;
                        }
                        return self.choose(index);
                    }
                    if matches!(self.overlay, Overlay::None) && Some(mouse.row) == layout.shortcuts
                    {
                        return self.open_actions();
                    }
                    Effect::Idle
                }
                MouseEventKind::ScrollUp => self.move_by(-3),
                MouseEventKind::ScrollDown => self.move_by(3),
                _ => Effect::Idle,
            },
            _ => Effect::Idle,
        }
    }

    fn move_by(&mut self, lines: isize) -> Effect {
        match &mut self.overlay {
            Overlay::Actions { cursor, items } => {
                *cursor = cursor
                    .saturating_add_signed(lines)
                    .min(items.len().saturating_sub(1))
            }
            Overlay::Groups { cursor } => {
                *cursor = cursor.saturating_add_signed(lines).min(self.groups.len())
            }
            Overlay::Preview { plan, scroll } => {
                *scroll = scroll
                    .saturating_add_signed(lines)
                    .min(plan.labels.len().saturating_sub(1))
            }
            Overlay::Details { scroll } => {
                *scroll = scroll
                    .saturating_add_signed(lines)
                    .min(view::DETAIL_ROWS.saturating_sub(1))
            }
            Overlay::Output => {
                if let Some(output) = &mut self.output {
                    output.scroll_by(lines);
                }
            }
            Overlay::None => self.focus(self.cursor.saturating_add_signed(lines)),
            Overlay::Prompt { .. } => return Effect::Idle,
        }
        Effect::Redraw
    }

    fn confirm(&mut self) -> Effect {
        let Overlay::Preview { plan, .. } = std::mem::replace(&mut self.overlay, Overlay::None)
        else {
            unreachable!()
        };
        Effect::Run {
            args: plan.args,
            action: plan.action,
        }
    }

    fn key(&mut self, key: KeyEvent, page: usize) -> Effect {
        if key.code == KeyCode::Esc || control(key, 'c') {
            if matches!(self.overlay, Overlay::Output)
                && self.output.as_ref().is_some_and(|output| output.running)
            {
                return Effect::Cancel;
            }
            if !matches!(self.overlay, Overlay::None) {
                self.overlay = Overlay::None;
                self.notice.clear();
                return Effect::Redraw;
            }
            if self.searching {
                self.searching = false;
                return Effect::Redraw;
            }
            return Effect::Exit;
        }
        if control(key, 'v')
            && (self.searching || matches!(self.overlay, Overlay::Prompt { .. }))
            && let Ok(value) = SystemClipboard.get_text()
        {
            return self.paste(&value);
        }
        if matches!(self.overlay, Overlay::Prompt { .. }) {
            return if key.code == KeyCode::Enter {
                self.submit_prompt()
            } else {
                self.edit(key)
            };
        }
        if self.searching && matches!(self.overlay, Overlay::None) {
            match key.code {
                KeyCode::Enter | KeyCode::Tab => {
                    self.searching = false;
                    return Effect::Redraw;
                }
                KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => {}
                _ => return self.edit(key),
            }
        }
        match key.code {
            KeyCode::Up => return self.move_by(-1),
            KeyCode::Down => return self.move_by(1),
            KeyCode::PageUp => return self.move_by(-(page as isize)),
            KeyCode::PageDown => return self.move_by(page as isize),
            KeyCode::Home => return self.move_by(isize::MIN),
            KeyCode::End => return self.move_by(isize::MAX),
            _ => {}
        }
        match &self.overlay {
            Overlay::Actions { cursor, .. } | Overlay::Groups { cursor }
                if key.code == KeyCode::Enter =>
            {
                return self.choose(*cursor);
            }
            Overlay::Preview { .. }
                if key.code == KeyCode::Char('y')
                    && key.kind == KeyEventKind::Press
                    && super::text_modifiers(key) =>
            {
                return self.confirm();
            }
            Overlay::Details { .. } if matches!(key.code, KeyCode::Char('a') | KeyCode::Enter) => {
                return self.open_actions();
            }
            Overlay::None => {}
            _ => return Effect::Idle,
        }
        if control(key, 'a') {
            self.checked.extend(
                self.visible
                    .iter()
                    .map(|index| self.rows[*index].selector.clone()),
            );
            return Effect::Redraw;
        }
        if control(key, 'd') {
            self.checked.clear();
            return Effect::Redraw;
        }
        if !super::text_modifiers(key) {
            return Effect::Idle;
        }
        match key.code {
            KeyCode::Char('q') => return Effect::Exit,
            KeyCode::Char(' ') => self.toggle(self.cursor),
            KeyCode::Char('/') | KeyCode::Tab => {
                self.notice.clear();
                self.searching = true;
            }
            KeyCode::Char('g') => {
                let cursor = self
                    .filters
                    .group
                    .as_ref()
                    .and_then(|group| self.groups.iter().position(|name| name == group))
                    .map_or(0, |index| index + 1);
                self.overlay = Overlay::Groups { cursor };
            }
            KeyCode::Char('s') => {
                self.filters.sort = self.filters.sort.next();
                self.refilter();
            }
            KeyCode::F(5) => return Effect::Refresh,
            KeyCode::F(2) | KeyCode::Char('a') => return self.open_actions(),
            KeyCode::Char('t') if !self.update_only => return self.activate(Action::Terminal),
            KeyCode::Enter if self.update_only => return self.activate(Action::Update),
            KeyCode::Enter | KeyCode::Char('d') if self.focused().is_some() => {
                self.overlay = Overlay::Details { scroll: 0 }
            }
            _ => return Effect::Idle,
        }
        Effect::Redraw
    }
}

type TerminalContext = (String, PathBuf, Option<String>);

struct Snapshot {
    rows: Vec<Row>,
    groups: Vec<String>,
    settings: Option<(PathBuf, Filters)>,
}

fn scan(workspace: &Workspace, flags: &ArgMatches, load: bool) -> JoinHandle<Result<Snapshot>> {
    let workspace = Workspace {
        root: workspace.root.clone(),
    };
    let flags = flags.clone();
    thread::spawn(move || {
        let rows = snapshot(&workspace, &flags)?;
        let mut groups = group_names(&workspace, &flags)?;
        groups.extend(rows.iter().flat_map(|row| row.groups.iter().cloned()));
        groups.sort();
        groups.dedup();
        let settings = if load {
            let path = registry_directory(&workspace, &flags)?.join("dashboard.json");
            let filters = fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Filters>(&bytes).ok())
                .unwrap_or_default();
            Some((path, filters))
        } else {
            None
        };
        Ok(Snapshot {
            rows,
            groups,
            settings,
        })
    })
}

fn save_settings(settings: Option<(PathBuf, Filters)>, dashboard: &Dashboard) -> Result<()> {
    if let Some((path, old)) = settings
        && dashboard.filters != old
    {
        fs::create_dir_all(path.parent().expect("dashboard settings directory"))?;
        write_json(&path, &dashboard.filters)?;
    }
    Ok(())
}

pub(super) fn open(
    workspace: &Workspace,
    flags: &ArgMatches,
    update: bool,
    sessions: &mut Manager,
    vi: bool,
) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!(
            "{}",
            tr(
                vi,
                "The Repository Manager requires an interactive terminal.",
                "Trình quản lý kho mã yêu cầu terminal tương tác."
            )
        );
    }
    let mut dashboard = Dashboard::new(vi, update);
    dashboard.update_jobs = flags
        .subcommand_matches("repos")
        .and_then(|matches| matches.subcommand_matches("update"))
        .and_then(|matches| matches.get_one::<u8>("jobs").copied());
    let mut scanner = Some(scan(workspace, flags, true));
    let mut refresh_pending = false;
    let mut context: Option<JoinHandle<Result<TerminalContext>>> = None;
    let mut settings = None;
    let mut running: Option<(Runner, Action)> = None;
    let color = color_enabled();
    let mut screen = Screen::enter(io::BufWriter::with_capacity(64 * 1024, io::stdout().lock()))?;
    let mut painter = Painter::default();
    let mut size = console::size()?;
    let mut redraw = true;
    let mut batch = Vec::with_capacity(128);
    let mut paste_continuation = 0;
    'dashboard: loop {
        if scanner.as_ref().is_some_and(JoinHandle::is_finished) {
            let result = scanner.take().expect("finished scan").join().map_err(|_| {
                anyhow!(
                    "{}",
                    tr(vi, "Repository scan failed.", "Không thể quét kho mã.")
                )
            })?;
            dashboard.loading = false;
            match result {
                Ok(snapshot) => {
                    dashboard.groups = snapshot.groups;
                    if let Some((path, mut initial)) = snapshot.settings {
                        if initial.search.len() > SEARCH_LIMIT
                            || initial.search.chars().any(char::is_control)
                        {
                            initial.search.clear();
                        }
                        let original = initial.clone();
                        if dashboard.filters == Filters::default() {
                            dashboard.search = Input::at_end(initial.search.clone());
                            dashboard.filters = initial;
                        }
                        settings = Some((path, original));
                    }
                    if dashboard
                        .filters
                        .group
                        .as_ref()
                        .is_some_and(|name| !dashboard.groups.contains(name))
                    {
                        dashboard.filters.group = None;
                    }
                    dashboard.replace_rows(snapshot.rows);
                }
                Err(error) => dashboard.notice = format!("{error:#}"),
            }
            redraw = true;
        }
        if running.is_none() && context.as_ref().is_some_and(JoinHandle::is_finished) {
            let result = context
                .take()
                .expect("finished validation")
                .join()
                .map_err(|_| {
                    anyhow!(
                        "{}",
                        tr(
                            vi,
                            "Terminal validation failed.",
                            "Không thể kiểm tra terminal."
                        )
                    )
                })?;
            match result {
                Ok((key, directory, shell)) => {
                    dashboard.notice.clear();
                    drop(screen);
                    let result = sessions.open(&key, &directory, shell.as_deref(), vi);
                    screen = Screen::enter(io::BufWriter::with_capacity(
                        64 * 1024,
                        io::stdout().lock(),
                    ))?;
                    painter = Painter::default();
                    size = console::size()?;
                    if let Err(error) = result {
                        dashboard.notice = format!("{error:#}");
                    }
                }
                Err(error) => dashboard.notice = format!("{error:#}"),
            }
            redraw = true;
        }
        if let Some((runner, action)) = &mut running {
            let output = dashboard.output.as_mut().expect("active command output");
            let result = runner.poll()?;
            if let Some(captured) = result.captured {
                output.captured = Some(captured);
                output.render_capture(vi);
                redraw = true;
            }
            if let Some(result) = result.result {
                output.finish(Ok(result), vi);
                dashboard.notice.clear();
                refresh_pending |= action.changes_view();
                running = None;
                redraw = true;
            } else {
                redraw |= output.animate();
            }
        }
        if refresh_pending && scanner.is_none() {
            refresh_pending = false;
            dashboard.loading = true;
            scanner = Some(scan(workspace, flags, false));
            redraw = true;
        }
        if redraw {
            painter.draw(&mut screen.out, dashboard.frame(size), color)?;
        }
        let timeout = if running.is_some() || scanner.is_some() || context.is_some() {
            Some(Duration::from_millis(50))
        } else {
            cfg!(windows).then_some(Duration::from_millis(160))
        };
        read_events(&mut batch, timeout)?;
        redraw = false;
        if cfg!(windows) || batch.iter().any(|event| matches!(event, Event::Resize(..))) {
            let resized = console::size()?;
            if resized != size && resized.0 > 0 && resized.1 > 0 {
                size = resized;
                redraw = true;
            }
        }
        let previous_text = std::mem::take(&mut paste_continuation);
        if cfg!(windows)
            && (dashboard.searching || matches!(dashboard.overlay, Overlay::Prompt { .. }))
        {
            if batch.len() == 128 {
                paste_continuation = previous_text + batch.iter().filter(|event| {
                    matches!(event, Event::Key(key) if key.kind != KeyEventKind::Release && super::printable(*key).is_some())
                }).count();
            }
            if let Some(paste) = super::windows_paste(&batch, previous_text) {
                redraw |= matches!(dashboard.paste(&paste), Effect::Redraw);
                continue;
            }
        }
        let host_paste = batch.iter().any(|event| matches!(event, Event::Paste(_)));
        let mut pasted = false;
        for event in batch.drain(..) {
            if super::skip_paste_companion(&event, host_paste, &mut pasted) {
                continue;
            }
            match dashboard.handle(event, painter.last_frame(size)) {
                Effect::Idle => {}
                Effect::Redraw => redraw = true,
                Effect::Exit => break 'dashboard,
                Effect::Cancel => {
                    if let Some((runner, _)) = &mut running {
                        dashboard.notice = runner.cancel().err().map_or_else(
                            || tr(vi, "Cancelling…", "Đang hủy…").into(),
                            str::to_owned,
                        );
                    }
                    redraw = true;
                }
                Effect::Refresh => {
                    if scanner.is_none() {
                        dashboard.notice.clear();
                        refresh_pending = true;
                    }
                }
                Effect::Run { args, action } => {
                    dashboard.notice.clear();
                    let mut output = OutputView::new(action.label(vi).into(), vi);
                    match Runner::start(workspace, flags, &args, vi) {
                        Ok(runner) => {
                            output.follow = true;
                            running = Some((runner, action));
                        }
                        Err(error) => output.finish(Err(format!("{error:#}")), vi),
                    }
                    dashboard.output = Some(output);
                    dashboard.overlay = Overlay::Output;
                    redraw = true;
                }
                Effect::Terminal(selector) => {
                    if context.is_none() {
                        let workspace = Workspace {
                            root: workspace.root.clone(),
                        };
                        let flags = flags.clone();
                        context = Some(thread::spawn(move || {
                            terminal_context(&workspace, &flags, &selector)
                        }));
                        dashboard.notice = tr(
                            vi,
                            "Validating repository terminal…",
                            "Đang kiểm tra terminal của kho mã…",
                        )
                        .into();
                        redraw = true;
                    }
                }
            }
        }
    }
    drop(screen);
    save_settings(settings, &dashboard).context(tr(
        vi,
        "Could not save repository view settings.",
        "Không thể lưu chế độ xem kho mã.",
    ))
}
