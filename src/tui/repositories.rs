mod actions;
#[cfg(test)]
mod tests;
mod view;

use std::{
    collections::{BTreeSet, HashMap},
    fs,
    io::{self, IsTerminal},
    path::PathBuf,
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use clap::ArgMatches;
use crossterm::{
    event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    terminal as console,
};
use serde::{Deserialize, Serialize};

use self::{
    actions::{Action, Plan, Target},
    view::{Layout, TableHits},
};
use super::{
    History, OutputView,
    clipboard::{Clipboard, SystemClipboard},
    control,
    editor::Input,
    read_events,
    repository_prompt::{PromptEvent, RepositoryPrompt},
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

#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Filters {
    search: String,
    group: Option<String>,
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
    CommandPrompt,
    Run {
        args: Vec<String>,
        action: Action,
    },
    Terminal {
        selector: String,
        shell: Option<String>,
    },
}

struct Dashboard {
    rows: Vec<Row>,
    search_keys: Vec<String>,
    visible: Vec<usize>,
    groups: Vec<String>,
    checked: BTreeSet<String>,
    selected: Option<String>,
    cursor: usize,
    scroll: usize,
    table_hits: TableHits,
    rendered_overlay: Option<std::mem::Discriminant<Overlay>>,
    filters: Filters,
    search: Input,
    searching: bool,
    update_only: bool,
    update_jobs: Option<u8>,
    vi: bool,
    overlay: Overlay,
    parents: Vec<Overlay>,
    overlay_scroll_max: usize,
    output: Option<OutputView>,
    notice: String,
    loading: bool,
}

impl Dashboard {
    fn new(vi: bool, update_only: bool) -> Self {
        Self {
            rows: Vec::new(),
            search_keys: Vec::new(),
            visible: Vec::new(),
            groups: Vec::new(),
            checked: BTreeSet::new(),
            selected: None,
            cursor: 0,
            scroll: 0,
            table_hits: TableHits::default(),
            rendered_overlay: None,
            filters: Filters::default(),
            search: Input::default(),
            searching: false,
            update_only,
            update_jobs: None,
            vi,
            overlay: Overlay::None,
            parents: Vec::new(),
            overlay_scroll_max: usize::MAX,
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

    fn replace_rows(&mut self, mut rows: Vec<Row>) {
        // Derive view order once per snapshot. The selector makes matching display names stable.
        rows.sort_by_cached_key(|row| {
            (
                row.alias.as_deref().unwrap_or(&row.name).to_lowercase(),
                row.selector.clone(),
            )
        });
        let identities: BTreeSet<_> = rows.iter().map(|row| row.selector.as_str()).collect();
        let old: HashMap<_, _> = self
            .rows
            .iter()
            .map(|row| (row.selector.as_str(), row))
            .collect();
        let local: HashMap<_, _> = rows
            .iter()
            .filter_map(|row| {
                row.path_identity()
                    .map(|path| (path, row.selector.as_str()))
            })
            .collect();
        let remap = |selector: &str| {
            if identities.contains(selector) {
                return Some(selector.to_owned());
            }
            let path = old.get(selector)?.path_identity()?;
            local.get(&path).map(|selector| (*selector).to_owned())
        };
        self.selected = self.selected.as_deref().and_then(remap);
        self.checked = self
            .checked
            .iter()
            .filter_map(|selector| remap(selector))
            .collect();
        self.rows = rows;
        self.search_keys = self
            .rows
            .iter()
            .map(|row| {
                let local = row.display_path().unwrap_or_default();
                let status = view::state_label(&row.state, self.vi).0;
                format!(
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
                .to_lowercase()
            })
            .collect();
        self.rebuild();
    }

    fn refilter(&mut self) {
        self.filters.search.clone_from(&self.search.value);
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.clear_table_hits();
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
            .filter(|(index, _)| {
                terms
                    .iter()
                    .all(|term| self.search_keys[*index].contains(term))
            })
            .map(|(index, _)| index)
            .collect();
        self.cursor = self
            .visible
            .iter()
            .position(|index| self.selected.as_deref() == Some(self.rows[*index].selector.as_str()))
            .unwrap_or(self.cursor)
            .min(self.visible.len().saturating_sub(1));
        if self.selected.is_none() {
            self.selected = self.focused().map(|row| row.selector.clone());
        }
    }

    fn focus(&mut self, index: usize) {
        self.cursor = index.min(self.visible.len().saturating_sub(1));
        self.selected = self.focused().map(|row| row.selector.clone());
    }

    fn clear_table_hits(&mut self) {
        self.table_hits = TableHits::default();
    }

    fn push_overlay(&mut self, next: Overlay) {
        self.parents
            .push(std::mem::replace(&mut self.overlay, next));
        self.overlay_scroll_max = usize::MAX;
        self.notice.clear();
        self.clear_table_hits();
    }

    fn back(&mut self) -> Effect {
        if matches!(self.overlay, Overlay::Output) {
            if self.output.as_ref().is_some_and(|output| output.running) {
                return Effect::Cancel;
            }
            self.output = None;
        }
        self.overlay = self.parents.pop().unwrap_or(Overlay::None);
        self.overlay_scroll_max = usize::MAX;
        self.notice.clear();
        self.clear_table_hits();
        Effect::Redraw
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
        match &mut self.overlay {
            Overlay::Actions { items, cursor } => {
                let Some(action) = items.get(index).copied() else {
                    return Effect::Idle;
                };
                *cursor = index;
                self.activate(action)
            }
            Overlay::Groups { .. } if index <= self.groups.len() => {
                self.filters.group = index
                    .checked_sub(1)
                    .and_then(|group| self.groups.get(group).cloned());
                self.back();
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
        } else if self.searching
            && matches!(self.overlay, Overlay::None)
            && self.search.edit(key, SEARCH_LIMIT)
        {
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
            && matches!(self.overlay, Overlay::None)
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
        let frame =
            frame.filter(|_| self.rendered_overlay == Some(std::mem::discriminant(&self.overlay)));
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key, &layout),
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
                    if matches!(self.overlay, Overlay::Details { .. })
                        && frame.suggestion_hit(mouse.column, mouse.row).is_some()
                    {
                        return self.open_actions();
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
                            if self.table_hits.size != Some(frame.size) {
                                return Effect::Idle;
                            }
                            self.focus(index);
                            if self.table_hits.checkbox.contains(&mouse.column) {
                                self.toggle(index);
                            }
                            self.searching = false;
                            return Effect::Redraw;
                        }
                        let width = unicode_width::UnicodeWidthStr::width(
                            frame.rows[usize::from(mouse.row)].contents().trim_end(),
                        );
                        if usize::from(mouse.column) < width {
                            return self.choose(index);
                        }
                    }
                    if matches!(self.overlay, Overlay::None)
                        && let Some(code) = self.table_hits.shortcut(frame, mouse.column, mouse.row)
                    {
                        return self.table_shortcut(code);
                    }
                    Effect::Idle
                }
                MouseEventKind::ScrollUp if !mouse.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.move_by(-3)
                }
                MouseEventKind::ScrollDown if !mouse.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.move_by(3)
                }
                _ => Effect::Idle,
            },
            _ => Effect::Idle,
        }
    }

    fn command_context(&self, workspace: &Workspace) -> (Vec<String>, Option<String>) {
        let focused = self.focused();
        let candidates = focused
            .into_iter()
            .chain(self.rows.iter())
            .flat_map(|row| {
                let relative = row
                    .path
                    .as_deref()
                    .and_then(|path| path.strip_prefix(&workspace.root).ok())
                    .filter(|path| !path.as_os_str().is_empty())
                    .map(|path| path.to_string_lossy().into_owned());
                [
                    (!row.selector.starts_with(r"\\?\")).then(|| row.selector.clone()),
                    row.alias.clone(),
                    relative,
                    row.display_path().map(|path| path.into_owned()),
                ]
            })
            .flatten()
            .collect();
        (candidates, focused.map(|row| row.selector.clone()))
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
            Overlay::Preview { scroll, .. } | Overlay::Details { scroll } => {
                *scroll = scroll
                    .saturating_add_signed(lines)
                    .min(self.overlay_scroll_max);
            }
            Overlay::Output => {
                if let Some(output) = &mut self.output {
                    output.scroll_by(lines);
                }
            }
            Overlay::None => {
                let next = self
                    .cursor
                    .saturating_add_signed(lines)
                    .min(self.visible.len().saturating_sub(1));
                if next == self.cursor {
                    return Effect::Idle;
                }
                self.focus(next);
                self.clear_table_hits();
            }
            Overlay::Prompt { .. } => return Effect::Idle,
        }
        Effect::Redraw
    }

    fn confirm(&mut self) -> Effect {
        let Overlay::Preview { plan, .. } = &self.overlay else {
            unreachable!()
        };
        Effect::Run {
            args: plan.args.clone(),
            action: plan.action,
        }
    }

    fn copy_path(&mut self, clipboard: &mut impl Clipboard) -> Effect {
        self.notice = self.focused().and_then(Row::display_path).map_or_else(
            || {
                tr(
                    self.vi,
                    "This repository has no local path.",
                    "Kho mã này không có đường dẫn cục bộ.",
                )
                .into()
            },
            |path| match clipboard.set_text(&path) {
                Ok(()) => tr(
                    self.vi,
                    "Repository path copied.",
                    "Đã sao chép đường dẫn kho mã.",
                )
                .into(),
                Err(error) => format!(
                    "{} ({error})",
                    tr(
                        self.vi,
                        "Clipboard unavailable",
                        "Không thể dùng bảng nhớ tạm"
                    )
                ),
            },
        );
        Effect::Redraw
    }

    fn key(&mut self, key: KeyEvent, layout: &Layout) -> Effect {
        let page = layout.table.len().max(1);
        if key.code == KeyCode::Esc || control(key, 'c') {
            if !matches!(self.overlay, Overlay::None) {
                return self.back();
            }
            if self.searching {
                self.searching = false;
                return Effect::Redraw;
            }
            return Effect::Exit;
        }
        if control(key, 'v')
            && ((self.searching && matches!(self.overlay, Overlay::None))
                || matches!(self.overlay, Overlay::Prompt { .. }))
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
        if key.kind == KeyEventKind::Press
            && super::text_modifiers(key)
            && (key.code == KeyCode::Char(':')
                || key.code == KeyCode::Char(';') && key.modifiers.contains(KeyModifiers::SHIFT))
            && matches!(
                self.overlay,
                Overlay::None
                    | Overlay::Details { .. }
                    | Overlay::Actions { .. }
                    | Overlay::Groups { .. }
            )
        {
            return Effect::CommandPrompt;
        }
        if super::text_modifiers(key)
            && key.code == KeyCode::Char('c')
            && matches!(self.overlay, Overlay::None | Overlay::Details { .. })
        {
            return self.copy_path(&mut SystemClipboard);
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
            Overlay::Details { .. }
                if matches!(
                    key.code,
                    KeyCode::Char('a') | KeyCode::Enter | KeyCode::F(2)
                ) =>
            {
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
        self.table_shortcut(key.code)
    }

    fn table_shortcut(&mut self, code: KeyCode) -> Effect {
        match code {
            KeyCode::Char('q') => return Effect::Exit,
            KeyCode::Esc if self.searching => self.searching = false,
            KeyCode::Esc => return Effect::Exit,
            KeyCode::Char(':') => return Effect::CommandPrompt,
            KeyCode::Char('c') => return self.copy_path(&mut SystemClipboard),
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
                self.push_overlay(Overlay::Groups { cursor });
            }
            KeyCode::F(5) => return Effect::Refresh,
            KeyCode::F(2) | KeyCode::Char('a') => return self.open_actions(),
            KeyCode::Char('t') if !self.update_only => return self.activate(Action::Terminal),
            KeyCode::Enter if self.update_only => return self.activate(Action::Update),
            KeyCode::Enter | KeyCode::Char('d') if self.focused().is_some() => {
                self.push_overlay(Overlay::Details { scroll: 0 })
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
    history: &mut History,
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
    let mut prompt = RepositoryPrompt::new(vi, history);
    let result = drive(workspace, flags, update, sessions, vi, &mut prompt);
    prompt.finish(history);
    result
}

fn drive(
    workspace: &Workspace,
    flags: &ArgMatches,
    update: bool,
    sessions: &mut Manager,
    vi: bool,
    prompt: &mut RepositoryPrompt,
) -> Result<()> {
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
    let mut rendered_prompt = false;
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
                    let result = sessions.open_with_repositories(
                        (&key, &directory, shell.as_deref()),
                        workspace,
                        flags,
                        Some(&dashboard.rows),
                        vi,
                    );
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
        redraw |= prompt.poll()?;
        if redraw {
            let frame = dashboard.frame(size);
            rendered_prompt = prompt.active();
            let frame = if rendered_prompt {
                prompt.frame(frame, color)
            } else {
                frame
            };
            painter.draw(&mut screen.out, frame, color)?;
        }
        let timeout =
            if running.is_some() || scanner.is_some() || context.is_some() || prompt.polling() {
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
            && (prompt.active()
                || dashboard.searching && matches!(dashboard.overlay, Overlay::None)
                || matches!(dashboard.overlay, Overlay::Prompt { .. }))
        {
            if batch.len() == 128 {
                paste_continuation = previous_text + batch.iter().filter(|event| {
                    matches!(event, Event::Key(key) if key.kind != KeyEventKind::Release && super::printable(*key).is_some())
                }).count();
            }
            if let Some(paste) = super::windows_paste(&batch, previous_text) {
                redraw |= if prompt.active() {
                    matches!(prompt.paste(&paste, true), PromptEvent::Redraw)
                } else {
                    matches!(dashboard.paste(&paste), Effect::Redraw)
                };
                continue;
            }
        }
        let host_paste = batch.iter().any(|event| matches!(event, Event::Paste(_)));
        let mut pasted = false;
        for event in batch.drain(..) {
            if super::skip_paste_companion(&event, host_paste, &mut pasted) {
                continue;
            }
            let frame = painter
                .last_frame(size)
                .filter(|_| prompt.active() == rendered_prompt);
            let effect = if prompt.active() {
                match prompt.event(event, frame, workspace, flags)? {
                    PromptEvent::Idle => Effect::Idle,
                    PromptEvent::Redraw => Effect::Redraw,
                    PromptEvent::Closed { refresh } => {
                        refresh_pending |= refresh;
                        Effect::Redraw
                    }
                    PromptEvent::Terminal {
                        selector,
                        shell,
                        refresh,
                    } => {
                        refresh_pending |= refresh;
                        Effect::Terminal { selector, shell }
                    }
                }
            } else {
                dashboard.handle(event, frame)
            };
            match effect {
                Effect::Idle => {}
                Effect::Redraw => redraw = true,
                Effect::Exit => break 'dashboard,
                Effect::CommandPrompt => {
                    let (selectors, focused) = dashboard.command_context(workspace);
                    let native = dashboard
                        .focused()
                        .and_then(|row| row.path.clone().map(|path| (path, row.shell.clone())));
                    prompt.open_on(selectors, focused, native);
                    redraw = true;
                }
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
                    dashboard.push_overlay(Overlay::Output);
                    redraw = true;
                }
                Effect::Terminal { selector, shell } => {
                    if context.is_none() {
                        let workspace = Workspace {
                            root: workspace.root.clone(),
                        };
                        let flags = flags.clone();
                        context = Some(thread::spawn(move || {
                            terminal_context(&workspace, &flags, &selector)
                                .map(|(key, path, configured)| (key, path, shell.or(configured)))
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
