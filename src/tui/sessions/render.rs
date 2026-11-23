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
    workspace::{Command, Modal, State, menu_commands},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hit {
    Command(Command),
    Session(SessionId),
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

pub(super) fn frame(manager: &Manager, layout: &Layout, state: &State) -> (Frame, Hits) {
    let mut frame = Frame::new(state.size);
    let mut hits = Hits::default();
    let vi = state.vi;
    toolbar(&mut frame, &mut hits, manager, vi);
    let focus = if state.modal.is_none() && !manager.arrangement.sidebar_focused {
        manager.arrangement.active()
    } else {
        None
    };
    for (index, pane) in layout.panes.into_iter().enumerate() {
        if index == 1
            && let Some(x) = layout.vertical_divider
        {
            vertical(&mut frame, x, layout.area.y, layout.area.bottom());
        }
        if let Some(pane) = pane
            && let Some(session) = manager.session(pane.id)
        {
            terminal(&mut frame, pane, session, focus == Some(pane.id), vi);
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
        sidebar(&mut frame, &mut hits, manager, area, vi);
    }
    footer(&mut frame, manager, state);
    if let Some(modal) = &state.modal {
        // Use only the modal's hit regions while it owns the screen.
        hits.0.clear();
        dialogue(&mut frame, &mut hits, manager, state, modal);
    }
    (frame, hits)
}

fn toolbar(frame: &mut Frame, hits: &mut Hits, manager: &Manager, vi: bool) {
    let Some(row) = frame.rows.first_mut() else {
        return;
    };
    let width = frame.size.0;
    let title = if width >= 80 {
        " CIRNET LAB  /  TERMINAL"
    } else {
        " TERMINAL"
    };
    write_span(row, Tone::Accent, title, width);
    for command in [
        Command::New,
        Command::Split,
        Command::Sidebar,
        Command::Menu,
    ] {
        let name = if command == Command::Split && manager.arrangement.split_active() {
            locale(["Unsplit", "Bỏ chia"], vi)
        } else {
            command.title(vi)
        };
        let text = if width >= 80 {
            format!(" {} {name} ", command.shortcut())
        } else {
            format!(" {} ", command.shortcut())
        };
        let start = row_width(row);
        let space = if start > 0 { 1 } else { 0 };
        if start + space + text.width() as u16 > width {
            continue;
        }
        row.push(BORDER, "│");
        row.push(Tone::Key, &text);
        hits.row(start + space, 0, text.width() as u16, Hit::Command(command));
    }
}

fn terminal(frame: &mut Frame, pane: Pane, session: &Session, focused: bool, vi: bool) {
    let body = pane.body;
    let output = session.output.lock().unwrap();
    let screen = output.parser.screen();
    let header = &mut frame.rows[usize::from(body.y - 1)];
    pad(header, body.x, Tone::Plain);
    let header_tone = if focused {
        Tone::Accent
    } else {
        Tone::Punctuation
    };
    let marker = if focused { "▌" } else { " " };
    let label = if session.label.is_some() {
        format!("{marker} {} / {} ", session.repository, session.title())
    } else {
        format!("{marker} {} ", session.repository)
    };
    let status = if output.exit.is_some() {
        format!(
            " {} · {} ",
            session.shell,
            locale(["Exited", "Đã thoát"], vi)
        )
    } else {
        format!(" {} ", session.shell)
    };
    let name_end = body.right().saturating_sub(status.width() as u16 + 1);
    write_span(header, header_tone, &label, name_end.max(body.x + 1));
    write_span(header, Tone::Muted, &status, body.right());
    let (rows, cols) = screen.size();
    let visible_rows = rows.get().min(body.height);
    let visible_cols = cols.get().min(body.width);
    for row in 0..body.height {
        let line = &mut frame.rows[usize::from(row + body.y)];
        pad(line, body.x, Tone::Plain);
        if row < visible_rows {
            let contents = terminal_line(screen, row, visible_cols);
            for (tone, segment) in contents.segments() {
                line.push(tone, segment);
            }
        }
    }
    let (row, col) = screen.cursor_position();
    if focused
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

fn sidebar(frame: &mut Frame, hits: &mut Hits, manager: &Manager, area: Rect, vi: bool) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let heading = format!(
        " {}  {:02}",
        locale(["SESSIONS", "PHIÊN"], vi),
        manager.entries.len()
    );
    let height = if area.height >= 11 { 2 } else { 1 };
    let capacity = usize::from(area.height.saturating_sub(1) / height);
    let focus = manager
        .arrangement
        .active()
        .and_then(|id| manager.position(id))
        .unwrap_or_default();
    let start = list_start(focus, capacity, manager.entries.len());
    sidebar_line(frame, area, area.y, SURFACE, &heading);
    for (offset, session) in manager
        .entries
        .iter()
        .skip(start)
        .take(capacity)
        .enumerate()
    {
        let row = area.y + 1 + offset as u16 * height;
        let active = manager.arrangement.active() == Some(session.id);
        let pane = manager
            .arrangement
            .panes
            .iter()
            .position(|id| *id == Some(session.id));
        let marker = if active {
            "›"
        } else if pane.is_some() {
            "·"
        } else {
            " "
        };
        let surface = if active { SELECTED } else { SURFACE };
        let running = session.running();
        let status = if running {
            locale(["Running", "Đang chạy"], vi)
        } else {
            locale(["Exited", "Đã thoát"], vi)
        };
        let title = if session.label.is_some() {
            format!(" {marker} {}  {}", session.title(), session.repository)
        } else {
            format!(" {marker} {}", session.repository)
        };
        if height == 2 {
            sidebar_line(frame, area, row, surface, &title);
            sidebar_line(
                frame,
                area,
                row + 1,
                surface,
                &format!("   #{}  {} · {status}", session.id, session.shell),
            );
        } else {
            let status = if area.width >= 30 {
                status
            } else if running {
                locale(["Run", "Chạy"], vi)
            } else {
                locale(["Exit", "Thoát"], vi)
            };
            let details = format!(" {} · {status}", session.shell);
            let line = &mut frame.rows[usize::from(row)];
            pad(line, area.x, Tone::Plain);
            let name_width = area
                .width
                .saturating_sub(details.width() as u16)
                .max(area.width.min(7));
            let name_end = area.x + name_width;
            write_span(line, surface, &title, name_end);
            pad(line, name_end, surface);
            write_span(line, surface, &details, area.right());
            pad(line, area.right(), surface);
        }
        hits.0.push((
            Rect {
                y: row,
                height,
                ..area
            },
            Hit::Session(session.id),
        ));
    }
    let end =
        area.y + 1 + (manager.entries.len().saturating_sub(start).min(capacity) as u16) * height;
    for y in end..area.bottom() {
        sidebar_line(frame, area, y, SURFACE, "");
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
    footer.push(Tone::Muted, " ");
    let commands: &[Command] = if frame.size.0 < 80 {
        &[Command::New, Command::Sidebar, Command::Menu, Command::Back]
    } else {
        &[
            Command::Rename,
            Command::Close,
            Command::Restart,
            Command::Back,
        ]
    };
    for command in commands {
        let shortcut = format!(" {} {} ", command.shortcut(), command.title(vi));
        if row_width(footer) as usize + shortcut.width() > usize::from(frame.size.0) {
            continue;
        }
        footer.push(Tone::Muted, &shortcut);
    }
    if frame.size.0 >= 80 {
        write_span(footer, Tone::Muted, "  F6/F7 ", frame.size.0);
    }
}

fn dialogue(frame: &mut Frame, hits: &mut Hits, manager: &Manager, state: &State, modal: &Modal) {
    frame.cursor = None;
    let vi = state.vi;
    let (title, origin, options, selected, input, note, confirm) = match modal {
        Modal::Menu { selected, origin } => {
            let options = menu_commands(manager)
                .into_iter()
                .map(|command| {
                    let title = if command == Command::Split && manager.arrangement.split_active() {
                        locale(["Unsplit", "Bỏ chia"], vi)
                    } else {
                        command.title(vi)
                    };
                    (title.to_owned(), command.shortcut())
                })
                .collect();
            (
                Command::Menu.title(vi).into(),
                *origin,
                options,
                *selected,
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
                .map(|repo| (repo.name.clone(), ""))
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
                .map(|shell| ((*shell).to_owned(), ""))
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
        Modal::Confirm(_, command) => (
            command.title(vi).into(),
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
        .min(11);
    let width = frame.size.0.min(if origin.is_some() { 40 } else { 60 });
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
        write_span(&mut content, SURFACE, &input.value[left..], width);
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
    for (index, (option, shortcut)) in options.iter().enumerate().skip(start).take(capacity) {
        if row >= y + height - 1 {
            break;
        }
        let tone = if index == selected { SELECTED } else { SURFACE };
        let mut content = Line::default();
        content.push(tone, if index == selected { " › " } else { "   " });
        let boundary = width.saturating_sub(shortcut.width() as u16 + 1);
        write_span(&mut content, tone, option, boundary);
        pad(&mut content, boundary, tone);
        write_span(&mut content, tone, shortcut, width);
        pad(&mut content, width, tone);
        overlay(frame, x, row, width, &content);
        hits.row(x, row, width, Hit::Step(index));
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
