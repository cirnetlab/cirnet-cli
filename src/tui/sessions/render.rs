use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::{
    super::{
        editor::Input,
        terminal::{Frame, Line, PalettePart},
    },
    Manager, Session, SessionId, Tone,
    layout::{Layout, Pane, Rect},
    locale, terminal_line,
    workspace::{Command, MENU_GROUPS, Modal, State},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hit {
    Command(Command),
    Session(SessionId),
    SessionAction(SessionId, Command),
    PaneActions(SessionId),
    SidebarSearch,
    FindNext,
    FindPrevious,
    Step(usize),
    Accept,
    Cancel,
}

#[derive(Default)]
pub(super) struct Hits(Vec<(Rect, Hit)>);

impl Hits {
    pub(super) fn at(&self, x: u16, y: u16) -> Option<Hit> {
        self.0
            .iter()
            .rev()
            .find(|(area, _)| area.contains(x, y))
            .map(|(_, hit)| *hit)
    }

    fn row(&mut self, x: u16, y: u16, width: u16, hit: Hit) {
        self.0.push((
            Rect {
                x,
                y,
                width,
                height: 1,
            },
            hit,
        ));
    }
}

const SURFACE: Tone = Tone::Palette(PalettePart::ContextItem);
const SELECTED: Tone = Tone::Palette(PalettePart::ContextSelected);
const BORDER: Tone = Tone::Palette(PalettePart::ContextEdge);
const TERMINAL_ICON: &str = "›_";
const SPLIT_ICON: &str = "◫";

pub(super) fn frame(manager: &Manager, layout: &Layout, state: &State) -> (Frame, Hits) {
    let mut frame = Frame::new(state.size);
    let mut hits = Hits::default();
    let vi = state.vi;
    toolbar(&mut frame, &mut hits, manager, state);
    let cursor_enabled =
        state.modal.is_none() && state.find.is_none() && !manager.arrangement.sidebar_focused;
    for (index, pane) in layout.panes.into_iter().enumerate() {
        if index == 1
            && let Some(x) = layout.vertical_divider
        {
            vertical(&mut frame, x, layout.area.y, layout.area.bottom());
        }
        if let Some(pane) = pane
            && let Some(session) = manager.session(pane.id)
        {
            terminal(
                &mut frame,
                &mut hits,
                pane,
                session,
                manager.arrangement.active() == Some(pane.id),
                cursor_enabled,
                state,
            );
        }
    }
    if let Some(y) = layout.horizontal_divider {
        let width = layout
            .panes
            .into_iter()
            .flatten()
            .next()
            .unwrap()
            .body
            .width;
        frame.rows[usize::from(y)].push(BORDER, "─".repeat(usize::from(width)));
    }
    if manager.entries.is_empty() && layout.area.height > 0 && state.busy.is_none() {
        let width = layout
            .sidebar
            .map_or(layout.area.width, |area| area.x.saturating_sub(1));
        let message = locale(["  F1 opens a terminal", "  F1 mở terminal"], vi);
        write_span(
            &mut frame.rows[usize::from(layout.area.y)],
            Tone::Muted,
            message,
            width,
        );
    }
    if let Some(area) = layout.sidebar {
        if area.x > 0 {
            vertical(&mut frame, area.x - 1, area.y, area.bottom());
        }
        sidebar(&mut frame, &mut hits, manager, area, state);
    }
    if state.find.is_some() {
        find_bar(&mut frame, &mut hits, manager, state);
    } else {
        footer(&mut frame, manager, state);
    }
    if let Some(modal) = &state.modal {
        // Use only the modal's hit regions while it owns the screen.
        hits.0.clear();
        dialogue(&mut frame, &mut hits, manager, state, modal);
    }
    (frame, hits)
}

fn action_title(command: Command, manager: &Manager, vi: bool) -> &'static str {
    match command {
        Command::Split if manager.arrangement.split_active() => locale(["Unsplit", "Bỏ chia"], vi),
        Command::Maximize if manager.arrangement.is_maximized() => {
            locale(["Restore panes", "Khôi phục ô"], vi)
        }
        _ => command.title(vi),
    }
}

fn toolbar(frame: &mut Frame, hits: &mut Hits, manager: &Manager, state: &State) {
    let Some(row) = frame.rows.first_mut() else {
        return;
    };
    let vi = state.vi;
    let width = frame.size.0;
    let controls = [
        Command::New,
        Command::Split,
        Command::Sidebar,
        Command::Menu,
    ];
    let compact = width < 45;
    let labels = if compact {
        ["+", SPLIT_ICON, "≡", "⋯"]
    } else if width >= 76 {
        controls.map(|cmd| action_title(cmd, manager, vi))
    } else {
        [
            locale(["New", "Mới"], vi),
            action_title(Command::Split, manager, vi),
            Command::Sidebar.title(vi),
            Command::Menu.title(vi),
        ]
    };
    // let reserved = labels.iter().map(|label| label.width() + 3).sum::<usize>() as u16;
    // let end = width.saturating_sub(reserved);
    // let title = if width >= 76 {
    //     "Terminal"
    // } else {
    //     " Terminal"
    // };
    // let caption = manager.active().map_or_else(
    //     || title.to_owned(),
    //     |active| format!("{title} · {}", active.repository),
    // );
    // write_span(row, Tone::Accent, &ellipsize(&caption, end as usize), end);
    for (command, label) in controls.into_iter().zip(labels) {
        let text = if command == Command::Split && !compact {
            format!(" {SPLIT_ICON} {label} ")
        } else {
            format!(" {label} ")
        };
        let start = row_width(row);
        let button_width = text.width() as u16;
        if start + 1 + button_width > width {
            break;
        }
        row.push(BORDER, "│");
        row.push(
            if !state.enabled(manager, command) {
                Tone::Muted
            } else if state.hovered_command == Some(command) {
                SELECTED
            } else {
                Tone::Key
            },
            &text,
        );
        hits.row(start + 1, 0, button_width, Hit::Command(command));
    }
}

fn terminal(
    frame: &mut Frame,
    hits: &mut Hits,
    pane: Pane,
    session: &Session,
    focused: bool,
    cursor_enabled: bool,
    state: &State,
) {
    let body = pane.body;
    let output = session.output.lock().unwrap();
    let screen = output.parser.screen();
    let header = &mut frame.rows[usize::from(body.y - 1)];
    pad(header, body.x, Tone::Plain);
    let header_tone = if focused { Tone::Accent } else { Tone::Heading };
    let marker = if focused { "▌" } else { " " };
    let label = if session.label.is_some() {
        format!("{marker} {} / {}", session.repository, session.title())
    } else {
        format!("{marker} {}", session.repository)
    };
    let indicator = if output.exit.is_some() { "×" } else { "●" };
    let status = format!(" {indicator} {} ", shell_label(session.shell.as_str()));
    let actions = if body.width >= 14 { 3 } else { 0 };
    let status_end = body.right() - actions;
    let status_start = status_end.saturating_sub(status.width() as u16).max(body.x);
    write_span(
        header,
        header_tone,
        &ellipsize(&label, usize::from(status_start - body.x)),
        status_start,
    );
    pad(header, status_start, SURFACE);
    write_span(
        header,
        if output.exit.is_some() {
            Tone::Warning
        } else {
            Tone::Success
        },
        &status,
        status_end,
    );
    pad(header, status_end, SURFACE);
    if actions != 0 {
        write_span(header, Tone::Key, " ⋯ ", body.right());
        hits.row(status_end, body.y - 1, actions, Hit::PaneActions(pane.id));
    }
    let mark = state
        .marked
        .as_ref()
        .filter(|mark| mark.has_range && mark.valid(pane.id, &output));
    let found = state
        .find
        .as_ref()
        .filter(|find| find.id == pane.id)
        .and_then(|find| find.visible(&output));
    let (rows, cols) = screen.size();
    let visible_rows = rows.get().min(body.height);
    let visible_cols = cols.get().min(body.width);
    for row in 0..body.height {
        let line = &mut frame.rows[usize::from(row + body.y)];
        pad(line, body.x, Tone::Plain);
        if row < visible_rows {
            let contents = terminal_line(screen, row, visible_cols, |column| {
                if mark.is_some_and(|mark| mark.cells.contains(row, column)) {
                    Some(Tone::Selection)
                } else if found.is_some_and(|found| found.contains(screen, row, column)) {
                    Some(Tone::Match)
                } else {
                    None
                }
            });
            for (tone, segment) in contents.segments() {
                line.push(tone, segment);
            }
        }
    }
    let (row, col) = screen.cursor_position();
    if focused
        && cursor_enabled
        && output.exit.is_none()
        && screen.scrollback() == 0
        && !screen.hide_cursor()
        && row < visible_rows
        && visible_cols > 0
    {
        frame.cursor = Some((body.x + col.min(visible_cols - 1), body.y + row));
    }
}

fn vertical(frame: &mut Frame, x: u16, start: u16, end: u16) {
    for y in start..end {
        let line = &mut frame.rows[usize::from(y)];
        pad(line, x, Tone::Plain);
        line.push(BORDER, "│");
    }
}

fn shell_label(shell: &str) -> &str {
    match shell {
        "powershell" => "PWSh",
        "cmd" => "CMD",
        "bash" => "Bash",
        "wsl" => "WSL",
        _ => shell,
    }
}

fn ellipsize(value: &str, columns: usize) -> std::borrow::Cow<'_, str> {
    if value.width() <= columns {
        return value.into();
    }
    if columns == 0 {
        return "".into();
    }
    let mut end = 0;
    let mut used = 1;
    for grapheme in value.graphemes(true) {
        used += grapheme.width();
        if used > columns {
            break;
        }
        end += grapheme.len();
    }
    format!("{}…", &value[..end]).into()
}

fn sidebar(frame: &mut Frame, hits: &mut Hits, manager: &Manager, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let vi = state.vi;
    let (ids, groups) = state.sidebar_rows(manager);
    let active = manager.arrangement.active();
    let selected = state.sidebar_selected.or(active);
    let title = format!(
        " {} · {}",
        locale(["SESSIONS", "PHIÊN"], vi),
        manager.entries.len()
    );
    sidebar_line(frame, area, area.y, SURFACE, &title);
    if area.width >= 16 {
        let x = area.right() - 3;
        let mut search = Line::default();
        search.push(SELECTED, " / ");
        overlay(frame, x, area.y, 3, &search);
        hits.row(x, area.y, 3, Hit::SidebarSearch);
    }
    let mut list_y = area.y + 1;
    if let Some(query) = &state.sidebar_query
        && area.height > 2
    {
        let prefix = "/ ";
        let (left, column) = input_window(query, area.width.saturating_sub(3));
        sidebar_line(
            frame,
            area,
            list_y,
            SELECTED,
            &format!(
                "{prefix}{}",
                if query.value.is_empty() {
                    locale(["Filter sessions", "Lọc phiên"], vi)
                } else {
                    &query.value[left..]
                }
            ),
        );
        if state.modal.is_none() && state.find.is_none() && manager.arrangement.sidebar_focused {
            frame.cursor = Some(((area.x + 2 + column).min(area.right() - 1), list_y));
        }
        list_y += 1;
    }
    let list_end = area.bottom();
    let capacity = usize::from(list_end.saturating_sub(list_y));
    let index = selected
        .and_then(|id| ids.iter().position(|entry| *entry == id))
        .unwrap_or_default();
    let mut start = list_start(index, capacity, ids.len());
    if let Some(active) = active.and_then(|id| ids.iter().position(|entry| *entry == id))
        && active.abs_diff(index) < capacity
    {
        start = start
            .min(active)
            .max(active.saturating_add(1).saturating_sub(capacity));
    }
    for (offset, y) in (list_y..list_end).enumerate() {
        let index = start + offset;
        let Some(session) = ids.get(index).and_then(|id| manager.session(*id)) else {
            let text = if offset == 0 && ids.is_empty() {
                locale([" No matching sessions", " Không có phiên phù hợp"], vi)
            } else {
                ""
            };
            sidebar_line(frame, area, y, SURFACE, text);
            continue;
        };
        let id = session.id;
        let focused = Some(id) == selected && manager.arrangement.sidebar_focused;
        let running = session.running();
        let hovered = state.sidebar_hover == Some(id);
        let marker = if Some(id) == active { "▌" } else { " " };
        let tone = if Some(id) == active {
            Tone::Palette(PalettePart::SessionActive)
        } else if hovered || focused {
            Tone::Palette(PalettePart::SessionHover)
        } else {
            SURFACE
        };
        let actions = hovered && area.width >= 16;
        let right = if actions {
            "      "
        } else if running {
            "  "
        } else {
            " ×"
        };
        let shell = if session.shell == "powershell" {
            "pwsh"
        } else {
            &session.shell
        };
        let tree = match groups.iter().find(|group| group.contains(&index)) {
            Some(group) if index == group.start => "┌ ",
            Some(group) if index + 1 == group.end => "└ ",
            Some(_) => "├ ",
            None => "  ",
        };
        let prefix = format!("{marker}{tree}{TERMINAL_ICON} ");
        let room = usize::from(area.width).saturating_sub(right.width());
        let left = if room >= prefix.width() {
            let label = format!("{shell} {}", session.title());
            format!("{prefix}{}", ellipsize(&label, room - prefix.width()))
        } else {
            marker.into()
        };
        let line = format!(
            "{left}{}{right}",
            " ".repeat(room.saturating_sub(left.width()))
        );
        sidebar_line(frame, area, y, tone, &line);
        hits.row(area.x, y, area.width, Hit::Session(id));
        if actions {
            for (offset, command, icon) in
                [(6, Command::Split, SPLIT_ICON), (3, Command::Close, "×")]
            {
                let x = area.right() - offset;
                let enabled = state.sidebar_action_enabled(manager, id, command);
                let mut button = Line::default();
                button.push(
                    if enabled {
                        tone
                    } else {
                        Tone::Palette(PalettePart::Detail)
                    },
                    format!(" {icon} "),
                );
                overlay(frame, x, y, 3, &button);
                // Disabled buttons still own their cells, so a click cannot select the row underneath.
                hits.row(x, y, 3, Hit::SessionAction(id, command));
            }
        }
    }
}

fn sidebar_line(frame: &mut Frame, area: Rect, y: u16, tone: Tone, text: &str) {
    let row = &mut frame.rows[usize::from(y)];
    pad(row, area.x, Tone::Plain);
    write_span(row, tone, text, area.right());
    pad(row, area.right(), tone);
}

fn footer(frame: &mut Frame, manager: &Manager, state: &State) {
    if frame.size.1 < 2 {
        return;
    }
    let vi = state.vi;
    let mut note = state.notice.clone();
    let mut tone = Tone::Warning;
    if let Some(busy) = state.busy {
        note = Some(busy.message(vi).into());
    } else if note.is_none()
        && let Some(session) = manager.active()
    {
        let output = session.output.lock().unwrap();
        if output.resize_failed {
            note = Some(
                locale(
                    [
                        "Shell resize failed · F9 Restart",
                        "Không thể đổi kích thước shell · F9 Khởi động lại",
                    ],
                    vi,
                )
                .into(),
            );
        } else if let Some(exit) = &output.exit {
            let detail = match exit {
                Ok(status) => status.to_string(),
                Err(error) => error.clone(),
            };
            note = Some(format!(
                "{} {detail} · {} {}",
                locale(["Exited", "Đã thoát"], vi),
                Command::Restart.shortcut(),
                Command::Restart.title(vi)
            ));
        } else if output.input_failed {
            note = Some(
                locale(
                    [
                        "Terminal input stopped · F9 Restart",
                        "Thiết bị nhập terminal đã dừng · F9 Khởi động lại",
                    ],
                    vi,
                )
                .into(),
            );
        } else if output.parser.screen().scrollback() != 0 {
            tone = Tone::Muted;
            note = Some(format!(
                "{} +{}",
                locale(["Scrollback", "Lịch sử cuộn"], vi),
                output.parser.screen().scrollback()
            ));
        }
    }
    let footer = frame.rows.last_mut().unwrap();
    if let Some(note) = note {
        write_span(footer, tone, &format!(" {note}"), frame.size.0);
        return;
    }
    let hint = if let Some(command) = state.hovered_command
        && state.sidebar_hover.is_some()
    {
        match command {
            Command::Split
                if state
                    .sidebar_hover
                    .is_some_and(|id| manager.arrangement.is_split_member(id)) =>
            {
                locale(
                    [
                        " Split Terminal · two panes open",
                        " Chia terminal · đang mở hai pane",
                    ],
                    vi,
                )
            }
            Command::Split => locale([" Split Terminal", " Chia terminal"], vi),
            Command::Close => locale([" Kill Terminal", " Đóng terminal"], vi),
            _ => "",
        }
        .into()
    } else if let Some(command) = state.hovered_command {
        format!(
            " {} · {}",
            action_title(command, manager, vi),
            command.shortcut()
        )
    } else if let Some(session) = state.sidebar_hover.and_then(|id| manager.session(id)) {
        format!(" {} · {} · #{}", session.title(), session.shell, session.id)
    } else if manager.arrangement.sidebar_focused {
        locale(
            [
                " / Filter  ↑↓ Select  Enter Focus  Esc Back",
                " / Lọc  ↑↓ Chọn  Enter Tiêu điểm  Esc Quay lại",
            ],
            vi,
        )
        .into()
    } else if frame.size.0 >= 80 {
        locale([" F10 Back   F6/F7", " F10 Quay lại   F6/F7"], vi).into()
    } else {
        locale([" F10 Back", " F10 Quay lại"], vi).into()
    };
    write_span(footer, Tone::Muted, &hint, frame.size.0);
}

fn find_bar(frame: &mut Frame, hits: &mut Hits, manager: &Manager, state: &State) {
    let Some(find) = &state.find else { return };
    let Some(row) = frame.size.1.checked_sub(1) else {
        return;
    };
    if frame.size.0 < 10 {
        return;
    }
    let compact = frame.size.0 < 28;
    let text = if compact {
        " / "
    } else {
        locale([" Find ", " Tìm "], state.vi)
    };
    let visible_hit = manager
        .session(find.id)
        .is_some_and(|s| find.visible(&s.output.lock().unwrap()).is_some());
    let status = if visible_hit {
        locale(["Match", "Có kết quả"], state.vi)
    } else if find.hit.is_some() {
        locale(["Output changed", "Kết quả đã đổi"], state.vi)
    } else if find.query.value.is_empty() {
        ""
    } else {
        locale(["No match", "Không thấy"], state.vi)
    };
    let width = frame.size.0;
    let status = if width >= 48 { status } else { "" };
    let line = &mut frame.rows[row as usize];
    line.push(Tone::Accent, text);
    let query_end = if compact {
        width
    } else {
        width.saturating_sub(13 + status.width() as u16)
    };
    let (offset, cursor) = input_window(
        &find.query,
        query_end.saturating_sub(row_width(line)).saturating_sub(1),
    );
    write_span(line, SURFACE, &find.query.value[offset..], query_end);
    pad(line, query_end, SURFACE);
    if !status.is_empty() {
        write_span(line, Tone::Muted, status, width);
    }
    for (label, hit) in [
        (" ↑ ", Hit::FindPrevious),
        (" ↓ ", Hit::FindNext),
        (" Esc ", Hit::Cancel),
    ] {
        let x = row_width(line);
        if !compact && x + label.width() as u16 <= width {
            line.push(SELECTED, label);
            hits.row(x, row, label.width() as u16, hit);
        }
    }
    frame.cursor = Some(((text.width() as u16 + cursor).min(width - 1), row));
}

struct OptionRow {
    title: String,
    shortcut: &'static str,
    enabled: bool,
    step: Option<usize>,
}

impl OptionRow {
    fn choice(index: usize, title: String) -> Self {
        Self {
            title,
            shortcut: "",
            enabled: true,
            step: Some(index),
        }
    }
}

fn dialogue(frame: &mut Frame, hits: &mut Hits, manager: &Manager, state: &State, modal: &Modal) {
    frame.cursor = None;
    let vi = state.vi;
    let (title, origin, options, selected, input, note, confirm) = match modal {
        Modal::Menu { selected, origin } => {
            let mut options = Vec::new();
            let mut index = 0;
            for group in MENU_GROUPS {
                if !options.is_empty() {
                    options.push(OptionRow {
                        title: String::new(),
                        shortcut: "",
                        enabled: false,
                        step: None,
                    });
                }
                for &command in *group {
                    options.push(OptionRow {
                        title: action_title(command, manager, vi).into(),
                        shortcut: command.shortcut(),
                        enabled: state.enabled(manager, command),
                        step: Some(index),
                    });
                    index += 1;
                }
            }
            let active_row = options
                .iter()
                .position(|option| option.step == Some(*selected))
                .unwrap_or_default();
            (
                Command::Menu.title(vi).into(),
                *origin,
                options,
                active_row,
                None,
                None,
                false,
            )
        }
        Modal::Repositories { query, selected } => {
            let options: Vec<_> = state
                .catalog
                .filtered(&query.value)
                .into_iter()
                .enumerate()
                .map(|(index, repo)| OptionRow::choice(index, repo.name.clone()))
                .collect();
            let note = options.is_empty().then(|| {
                locale(
                    [
                        "No matching local repositories",
                        "Không có kho mã cục bộ phù hợp",
                    ],
                    vi,
                )
            });
            (
                locale(["Choose repository", "Chọn kho mã"], vi).into(),
                None,
                options,
                *selected,
                Some(query),
                note,
                false,
            )
        }
        Modal::Shells {
            repository,
            selected,
            ..
        } => {
            let options: Vec<_> = state
                .catalog
                .shells
                .iter()
                .enumerate()
                .map(|(index, shell)| OptionRow::choice(index, (*shell).to_owned()))
                .collect();
            let note = options.is_empty().then(|| {
                locale(
                    [
                        "No supported shell is installed",
                        "Chưa cài đặt shell được hỗ trợ",
                    ],
                    vi,
                )
            });
            (
                format!(
                    "{} · {}",
                    locale(["Choose shell", "Chọn shell"], vi),
                    repository.name
                ),
                None,
                options,
                *selected,
                None,
                note,
                false,
            )
        }
        Modal::Rename(_, input) => (
            Command::Rename.title(vi).into(),
            None,
            Vec::new(),
            0,
            Some(input),
            Some(locale(
                [
                    "Empty restores the repository label",
                    "Để trống để dùng lại nhãn kho mã",
                ],
                vi,
            )),
            true,
        ),
        Modal::Confirm(id, command) => (
            manager.session(*id).map_or_else(
                || command.title(vi).into(),
                |session| format!("{} · {}", command.title(vi), session.title()),
            ),
            None,
            Vec::new(),
            0,
            None,
            Some(if *command == Command::Close {
                locale(
                    [
                        "Close the shell and its attached processes?",
                        "Đóng shell và các tiến trình liên quan?",
                    ],
                    vi,
                )
            } else {
                locale(
                    [
                        "Restart the shell and clear its state?",
                        "Khởi động lại shell và xóa trạng thái?",
                    ],
                    vi,
                )
            }),
            true,
        ),
    };
    if frame.size.0 < 2 || frame.size.1 < 2 {
        return;
    }
    let chrome = u16::from(frame.size.1 >= 7);
    let available = frame.size.1 - 2 * chrome;
    let decorations = 2 + u16::from(input.is_some()) + u16::from(note.is_some());
    let capacity = usize::from(available.saturating_sub(decorations))
        .min(options.len())
        .min(if matches!(modal, Modal::Menu { .. }) {
            22
        } else {
            11
        });
    let width = frame.size.0.min(match modal {
        Modal::Menu { .. } => 43,
        _ => 60,
    });
    let height = (decorations + capacity as u16).min(available);
    let (x, y) = origin.map_or(
        ((frame.size.0 - width) / 2, (frame.size.1 - height) / 2),
        |(x, y)| {
            (
                x.min(frame.size.0 - width),
                y.clamp(chrome, frame.size.1 - chrome - height),
            )
        },
    );
    let interior = width.saturating_sub(2);
    let mut row = y;
    let mut card = |row, content: Line| overlay(frame, x, row, width, &content);
    let mut header = Line::default();
    header.push(SURFACE, " ");
    write_span(&mut header, Tone::Accent, &title, width);
    pad(&mut header, width, SURFACE);
    card(row, header);
    row += 1;
    if let Some(input) = input.filter(|_| row < y + height - 1) {
        let (left, cursor) = input_window(input, interior.saturating_sub(1));
        let mut content = Line::default();
        content.push(SURFACE, " ");
        let placeholder = if matches!(modal, Modal::Repositories { .. }) && input.value.is_empty() {
            locale(["Search local repositories", "Tìm kho mã cục bộ"], vi)
        } else {
            &input.value[left..]
        };
        write_span(&mut content, SURFACE, placeholder, width);
        if let Some(selection) = input.selection() {
            let start = selection.start.saturating_sub(left) + 1;
            let end = (selection.end.saturating_sub(left) + 1).min(content.contents().len());
            if start < end {
                content.select(start..end);
            }
        }
        pad(&mut content, width, SURFACE);
        card(row, content);
        frame.cursor = Some((x + 1 + cursor, row));
        row += 1;
    }
    if let Some(note) = note.filter(|_| row < y + height - 1) {
        let mut content = Line::default();
        content.push(SURFACE, " ");
        write_span(&mut content, SURFACE, note, width);
        pad(&mut content, width, SURFACE);
        overlay(frame, x, row, width, &content);
        row += 1;
    }
    let start = list_start(selected, capacity, options.len());
    for (index, option) in options.iter().enumerate().skip(start).take(capacity) {
        if row >= y + height - 1 {
            break;
        }
        let mut content = Line::default();
        if option.step.is_none() {
            content.push(SURFACE, " ");
            content.push(BORDER, "─".repeat(width.saturating_sub(2).into()));
            pad(&mut content, width, SURFACE);
        } else {
            let tone = if index == selected && option.enabled {
                SELECTED
            } else {
                SURFACE
            };
            content.push(
                tone,
                if index == selected && option.enabled {
                    " › "
                } else {
                    "   "
                },
            );
            let boundary = width.saturating_sub(option.shortcut.width() as u16 + 1);
            let label_tone = if option.enabled { tone } else { Tone::Muted };
            write_span(
                &mut content,
                label_tone,
                &ellipsize(&option.title, usize::from(boundary.saturating_sub(3))),
                boundary,
            );
            pad(&mut content, boundary, tone);
            write_span(&mut content, label_tone, option.shortcut, width);
            pad(&mut content, width, tone);
            if let Some(step) = option.step.filter(|_| option.enabled) {
                hits.row(x, row, width, Hit::Step(step));
            }
        }
        overlay(frame, x, row, width, &content);
        row += 1;
    }
    let row = y + height - 1;
    let mut footer = Line::default();
    for (hint, hit) in [
        (locale([" Esc Back ", " Esc Quay lại "], vi), Hit::Cancel),
        (
            locale([" Enter Confirm ", " Enter Xác nhận "], vi),
            Hit::Accept,
        ),
    ] {
        if hit == Hit::Accept && !confirm {
            continue;
        }
        let offset = row_width(&footer);
        if offset + hint.width() as u16 <= width {
            footer.push(SELECTED, hint);
            hits.row(x + offset, row, hint.width() as u16, hit);
        }
        write_span(&mut footer, SURFACE, " ", width);
    }
    pad(&mut footer, width, SURFACE);
    overlay(frame, x, row, width, &footer);
}

fn input_window(input: &Input, columns: u16) -> (usize, u16) {
    let mut start = input.cursor;
    let mut width = 0;
    for (index, grapheme) in input.value[..input.cursor].grapheme_indices(true).rev() {
        let next = width + grapheme.width();
        if next > usize::from(columns) {
            break;
        }
        start = index;
        width = next;
    }
    (start, width as u16)
}

fn list_start(selected: usize, capacity: usize, count: usize) -> usize {
    selected
        .saturating_sub(capacity / 2)
        .min(count.saturating_sub(capacity))
}

fn overlay(frame: &mut Frame, x: u16, y: u16, width: u16, content: &Line) {
    let original = std::mem::take(&mut frame.rows[usize::from(y)]);
    let mut row = Line::default();
    copy_cells(&mut row, &original, 0, x);
    pad(&mut row, x, Tone::Plain);
    copy_cells(&mut row, content, 0, width);
    pad(&mut row, x + width, SURFACE);
    copy_cells(&mut row, &original, x + width, frame.size.0);
    frame.rows[usize::from(y)] = row;
}

fn copy_cells(target: &mut Line, source: &Line, start: u16, end: u16) {
    let mut column = 0;
    for (tone, segment) in source.segments() {
        for grapheme in segment.graphemes(true) {
            let next = column + grapheme.width() as u16;
            if column >= end {
                return;
            }
            if column >= start && next <= end {
                target.push(tone, grapheme);
            } else if next > start {
                let space = next.min(end) - column.max(start);
                target.push(Tone::Plain, " ".repeat(usize::from(space)));
            }
            column = next;
        }
    }
}

fn row_width(line: &Line) -> u16 {
    line.contents().width().min(usize::from(u16::MAX)) as u16
}

fn pad(line: &mut Line, width: u16, tone: Tone) {
    let spaces = width.saturating_sub(row_width(line));
    line.push(tone, " ".repeat(usize::from(spaces)));
}

fn write_span(line: &mut Line, tone: Tone, value: &str, end_column: u16) {
    let mut width = row_width(line);
    let clean = value.replace(char::is_control, " ");
    let end = clean.grapheme_indices(true).find_map(|(index, grapheme)| {
        width = width.saturating_add(grapheme.width() as u16);
        (width > end_column).then_some(index)
    });
    line.push(tone, &clean[..end.unwrap_or(clean.len())]);
}
