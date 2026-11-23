use std::{borrow::Cow, ops::Range};

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::{Dashboard, Frame, Overlay, Row, Tone, tr};
use crate::tui::{
    terminal::{Line, MenuRows},
    view::input_line,
};

const CHECKBOX: usize = 6;
pub(super) const DETAIL_ROWS: usize = 14;

pub(super) struct Layout {
    margin: usize,
    width: usize,
    header: Option<u16>,
    pub(super) table: Range<u16>,
    feedback: Option<u16>,
    pub(super) shortcuts: Option<u16>,
    details: usize,
}

impl Layout {
    pub(super) fn new((columns, height): (u16, u16)) -> Self {
        let columns = usize::from(columns.saturating_sub(1));
        let margin = if columns >= 45 { 2 } else { 0 };
        let width = columns.saturating_sub(2 * margin);
        let shortcuts = (height >= 4).then(|| height - 1);
        let feedback = (height >= 6).then(|| height - 2);
        let header = (height >= 7).then_some(2);
        let end = feedback.or(shortcuts).unwrap_or(height);
        let start = if header.is_some() { 3 } else { height.min(2) };
        Self {
            margin,
            width,
            header,
            table: start..end,
            feedback,
            shortcuts,
            details: if width >= 125 { 35 } else { 0 },
        }
    }

    fn table_width(&self) -> usize {
        self.width - self.details
    }

    fn label(&self, frame: &mut Frame, row: u16, tone: Tone, value: &str) {
        let Some(line) = frame.rows.get_mut(usize::from(row)) else {
            return;
        };
        let mut styled = self.line();
        styled.push(tone, clip(value, self.width));
        *line = styled;
    }

    fn line(&self) -> Line {
        let mut line = Line::default();
        line.push(Tone::Plain, " ".repeat(self.margin));
        line
    }
}

fn printable(value: &str) -> Cow<'_, str> {
    if value.chars().any(char::is_control) {
        Cow::Owned(value.chars().filter(|ch| !ch.is_control()).collect())
    } else {
        Cow::Borrowed(value)
    }
}

fn clip(value: &str, width: usize) -> Cow<'_, str> {
    let value = printable(value);
    let mut used = 0;
    let mut end = value.len();
    for (index, grapheme) in value.grapheme_indices(true) {
        used += grapheme.width();
        if used > width {
            end = index;
            break;
        }
    }
    match value {
        Cow::Borrowed(value) => Cow::Borrowed(&value[..end]),
        Cow::Owned(mut value) => {
            value.truncate(end);
            Cow::Owned(value)
        }
    }
}

fn shorten_path(value: &str, width: usize) -> String {
    let value = printable(value);
    if value.width() <= width {
        return value.into_owned();
    }
    let mut used = 1;
    let mut start = value.len();
    for (index, grapheme) in value.grapheme_indices(true).rev() {
        used += grapheme.width();
        if used > width {
            break;
        }
        start = index;
    }
    format!("…{}", &value[start..])
}

fn cell(line: &mut Line, width: usize, tone: Tone, value: &str) {
    if width == 0 {
        return;
    }
    let text = clip(value, width.saturating_sub(1));
    let pad = width - text.width();
    line.push(tone, text);
    line.push(Tone::Plain, " ".repeat(pad));
}

pub(super) fn state_label(state: &str, vi: bool) -> (&'static str, Tone) {
    match state {
        "clean" => (tr(vi, "Clean", "Sạch"), Tone::Success),
        "modified" | "dirty" => (tr(vi, "Modified", "Đã sửa"), Tone::Warning),
        "untracked" => (tr(vi, "Untracked", "Chưa theo dõi"), Tone::Warning),
        "conflicted" => (tr(vi, "Conflicted", "Xung đột"), Tone::Error),
        "detached" => (tr(vi, "Detached", "HEAD rời"), Tone::Warning),
        "missing" => (tr(vi, "Missing", "Bị thiếu"), Tone::Error),
        "invalid" => (tr(vi, "Invalid", "Không hợp lệ"), Tone::Error),
        "local" => (tr(vi, "Local checkout", "Bản sao cục bộ"), Tone::Accent),
        "available" => (tr(vi, "Available", "Có sẵn"), Tone::Accent),
        "uncloned" => (tr(vi, "Uncloned", "Chưa sao chép"), Tone::Muted),
        _ => (tr(vi, "Unavailable", "Không khả dụng"), Tone::Muted),
    }
}

struct Columns {
    name: usize,
    path: usize,
    branch: usize,
    state: usize,
    divergence: usize,
    revision: usize,
}

impl Columns {
    fn new(width: usize) -> Self {
        let space = width.saturating_sub(CHECKBOX);
        let path = if width >= 72 { (width / 4).min(33) } else { 0 };
        let branch = if width >= 42 {
            (width / 6).clamp(8, 17)
        } else {
            0
        };
        let state = if width >= 25 { 14.min(space / 2) } else { 0 };
        let divergence = if width >= 54 { 9 } else { 0 };
        let revision = if width >= 88 { 15 } else { 0 };
        let name = space.saturating_sub(path + branch + state + divergence + revision);
        Self {
            name,
            path,
            branch,
            state,
            divergence,
            revision,
        }
    }

    fn header(&self, layout: &Layout, vi: bool) -> Line {
        let mut line = layout.line();
        cell(
            &mut line,
            CHECKBOX.min(layout.table_width()),
            Tone::Heading,
            "✓",
        );
        for (width, label) in [
            (self.name, tr(vi, "Repository", "Kho mã")),
            (self.path, tr(vi, "Local path", "Đường dẫn")),
            (self.branch, tr(vi, "Branch", "Nhánh")),
            (self.state, tr(vi, "Local status", "Trạng thái")),
            (self.divergence, "↑ / ↓"),
            (self.revision, tr(vi, "Last commit", "Commit cuối")),
        ] {
            cell(&mut line, width, Tone::Heading, label);
        }
        line
    }

    fn row(&self, layout: &Layout, row: &Row, current: bool, checked: bool, vi: bool) -> Line {
        let mut line = layout.line();
        let mark = if checked { "[x]" } else { "[ ]" };
        let pointer = if current { " ›" } else { "" };
        cell(
            &mut line,
            CHECKBOX.min(layout.table_width()),
            Tone::Accent,
            &format!("{mark}{pointer}"),
        );
        let name = row.alias.as_deref().unwrap_or(&row.name);
        if row.active {
            cell(&mut line, self.name, Tone::Accent, &format!("{name} *"));
        } else {
            cell(
                &mut line,
                self.name,
                if current { Tone::Accent } else { Tone::Plain },
                name,
            );
        }
        let local = row.path.as_deref().map(std::path::Path::to_string_lossy);
        if self.path > 0 {
            cell(
                &mut line,
                self.path,
                Tone::Path,
                &shorten_path(local.as_deref().unwrap_or("—"), self.path - 1),
            );
        }
        cell(
            &mut line,
            self.branch,
            Tone::Key,
            row.branch.as_deref().unwrap_or("—"),
        );
        let (status, tone) = state_label(&row.state, vi);
        cell(&mut line, self.state, tone, status);
        cell(&mut line, self.divergence, Tone::Muted, &divergence(row));
        cell(
            &mut line,
            self.revision,
            Tone::Muted,
            row.revision.as_deref().unwrap_or("—"),
        );
        line
    }
}

fn divergence(row: &Row) -> String {
    match (row.ahead, row.behind) {
        (Some(ahead), Some(behind)) => format!("{ahead}↑ {behind}↓"),
        _ => "—".into(),
    }
}

fn since(timestamp: Option<u64>, vi: bool) -> String {
    let Some(timestamp) = timestamp else {
        return "—".into();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let seconds = now.saturating_sub(timestamp);
    let interval = if seconds < 60 {
        format!("{seconds} {}", tr(vi, "s", "giây"))
    } else if seconds < 3_600 {
        format!("{} {}", seconds / 60, tr(vi, "min", "phút"))
    } else if seconds < 86_400 {
        format!("{} {}", seconds / 3_600, tr(vi, "h", "giờ"))
    } else {
        format!("{} {}", seconds / 86_400, tr(vi, "d", "ngày"))
    };
    if vi {
        format!("Cách đây {interval}")
    } else {
        format!("{interval} ago")
    }
}

fn details(row: &Row, vi: bool) -> Vec<(Tone, String)> {
    let parts = [
        (
            tr(vi, "Alias", "Bí danh"),
            row.alias.as_deref().unwrap_or("—").into(),
        ),
        (
            tr(vi, "Path", "Đường dẫn"),
            row.path
                .as_deref()
                .map(std::path::Path::to_string_lossy)
                .unwrap_or_else(|| "—".into()),
        ),
        (
            if row.state == "available" {
                tr(vi, "Default branch", "Nhánh mặc định")
            } else {
                tr(vi, "Branch", "Nhánh")
            },
            row.branch.as_deref().unwrap_or("—").into(),
        ),
        (
            tr(vi, "Local status", "Trạng thái cục bộ"),
            state_label(&row.state, vi).0.into(),
        ),
        (
            tr(vi, "Ahead / behind", "Đi trước / đi sau"),
            divergence(row).into(),
        ),
        (
            tr(vi, "Last commit", "Commit cuối"),
            row.revision.as_deref().unwrap_or("—").into(),
        ),
        ("Remote", row.remote.as_deref().unwrap_or("—").into()),
        (
            tr(vi, "Groups", "Nhóm"),
            if row.groups.is_empty() {
                "—".into()
            } else {
                row.groups.join(", ").into()
            },
        ),
        (
            tr(vi, "Last sync", "Đồng bộ lần cuối"),
            since(row.last_sync, vi).into(),
        ),
        (
            tr(vi, "Last verified", "Kiểm tra lần cuối"),
            since(row.last_verified, vi).into(),
        ),
        ("Shell", row.shell.as_deref().unwrap_or("—").into()),
        (
            tr(vi, "Registry", "Sổ đăng ký"),
            if row.registered {
                tr(vi, "Registered", "Đã đăng ký")
            } else {
                tr(vi, "Discovery candidate", "Ứng viên tìm thấy")
            }
            .into(),
        ),
    ];
    let mut output = vec![(Tone::Accent, row.name.clone())];
    output.extend(
        parts
            .into_iter()
            .map(|(label, value)| (Tone::Plain, format!("{label} · {value}"))),
    );
    if row.active {
        output.push((
            Tone::Success,
            tr(vi, "Active working context", "Ngữ cảnh làm việc hiện tại").into(),
        ));
    }
    output
}

fn fill_details(frame: &mut Frame, layout: &Layout, row: &Row, vi: bool) {
    if layout.details == 0 {
        return;
    }
    for (y, (tone, text)) in layout.table.clone().zip(details(row, vi)) {
        let line = &mut frame.rows[usize::from(y)];
        let used = line.contents().width();
        let start = layout.margin + layout.table_width();
        line.push(Tone::Plain, " ".repeat(start.saturating_sub(used)));
        line.push(Tone::Muted, " │ ");
        line.push(tone, clip(&text, layout.details - 3));
    }
}

fn state_notice(dashboard: &Dashboard) -> (&str, Tone) {
    if !dashboard.notice.is_empty() {
        (&dashboard.notice, Tone::Warning)
    } else if dashboard.loading {
        (
            tr(
                dashboard.vi,
                "Refreshing local snapshots…",
                "Đang làm mới trạng thái cục bộ…",
            ),
            Tone::Accent,
        )
    } else {
        ("", Tone::Muted)
    }
}

impl Dashboard {
    pub(super) fn frame(&mut self, size: (u16, u16)) -> Frame {
        let mut frame = Frame::new(size);
        let layout = Layout::new(size);
        if layout.width == 0 || size.1 == 0 {
            return frame;
        }
        let vi = self.vi;
        let title = if self.update_only {
            tr(
                vi,
                "CIRNET LAB · Select repositories to update",
                "CIRNET LAB · Chọn kho mã để cập nhật",
            )
        } else {
            tr(
                vi,
                "CIRNET LAB · Repository Manager",
                "CIRNET LAB · Trình quản lý kho mã",
            )
        };
        layout.label(&mut frame, 0, Tone::Accent, title);
        let (notice, tone) = state_notice(self);
        if let Some(y) = layout.feedback {
            layout.label(&mut frame, y, tone, notice);
        }
        match &mut self.overlay {
            Overlay::None => self.table_frame(&mut frame, &layout),
            Overlay::Actions { items, cursor } => {
                layout.label(
                    &mut frame,
                    1,
                    Tone::Heading,
                    tr(vi, "Repository actions", "Thao tác kho mã"),
                );
                choices(
                    &mut frame,
                    &layout,
                    *cursor,
                    items.iter().map(|action| action.label(vi)),
                );
                footer(
                    &mut frame,
                    &layout,
                    tr(
                        vi,
                        "↑↓ Move   Enter Choose   Esc Back",
                        "↑↓ Di chuyển   Enter Chọn   Esc Quay lại",
                    ),
                );
            }
            Overlay::Groups { cursor } => {
                layout.label(
                    &mut frame,
                    1,
                    Tone::Heading,
                    tr(vi, "Filter by group", "Lọc theo nhóm"),
                );
                let all = tr(vi, "All repositories", "Tất cả kho mã");
                choices(
                    &mut frame,
                    &layout,
                    *cursor,
                    std::iter::once(all).chain(self.groups.iter().map(String::as_str)),
                );
                footer(
                    &mut frame,
                    &layout,
                    tr(
                        vi,
                        "↑↓ Move   Enter Choose   Esc Back",
                        "↑↓ Di chuyển   Enter Chọn   Esc Quay lại",
                    ),
                );
            }
            Overlay::Details { scroll } => {
                layout.label(
                    &mut frame,
                    1,
                    Tone::Heading,
                    tr(vi, "Repository details", "Chi tiết kho mã"),
                );
                let scroll = *scroll;
                if let Some(row) = self.focused() {
                    let items = details(row, vi);
                    let start = scroll.min(items.len().saturating_sub(layout.table.len()));
                    for (y, (tone, line)) in layout.table.clone().zip(items.into_iter().skip(start))
                    {
                        layout.label(&mut frame, y, tone, &line);
                    }
                }
                footer(
                    &mut frame,
                    &layout,
                    tr(
                        vi,
                        "↑↓ Scroll   Enter Actions   Esc Back",
                        "↑↓ Cuộn   Enter Thao tác   Esc Quay lại",
                    ),
                );
            }
            Overlay::Prompt { action, input, .. } => {
                layout.label(&mut frame, 1, Tone::Heading, action.label(vi));
                if let Some(y) = layout.table.clone().next() {
                    let prompt = format!("{} › ", action.prompt(vi).expect("prompt label"));
                    input_line(
                        &mut frame,
                        y,
                        layout.margin,
                        layout.width,
                        &prompt,
                        input,
                        "",
                    );
                }
                footer(
                    &mut frame,
                    &layout,
                    tr(
                        vi,
                        "Enter Preview   Esc Back",
                        "Enter Xem trước   Esc Quay lại",
                    ),
                );
            }
            Overlay::Preview { plan, scroll } => {
                let heading = format!(
                    "{} · {} · {}",
                    tr(vi, "Preview", "Xem trước"),
                    plan.action.label(vi),
                    plan.labels.len()
                );
                layout.label(&mut frame, 1, Tone::Heading, &heading);
                let start = (*scroll).min(plan.labels.len().saturating_sub(layout.table.len()));
                for (y, label) in layout.table.clone().zip(plan.labels.iter().skip(start)) {
                    layout.label(&mut frame, y, Tone::Plain, &format!("• {label}"));
                }
                let confirm = tr(vi, "Y Confirm", "Y Xác nhận");
                if let Some(y) = layout.shortcuts {
                    let shortcuts = tr(vi, "↑↓ Scroll   Esc Cancel", "↑↓ Cuộn   Esc Hủy");
                    layout.label(
                        &mut frame,
                        y,
                        Tone::Key,
                        &format!("{confirm}   {shortcuts}"),
                    );
                    let start = layout.margin as u16;
                    frame.suggestions = Some(MenuRows {
                        columns: start..start + confirm.width().min(layout.width) as u16,
                        rows: y..y + 1,
                        first: 0,
                    });
                }
            }
            Overlay::Output => {
                if let Some(output) = &mut self.output {
                    let label = output.title.as_deref().unwrap_or_default();
                    layout.label(&mut frame, 1, Tone::Heading, label);
                    output.layout(layout.width, layout.table.len());
                    let inset = " ".repeat(layout.margin);
                    for (row, y) in layout.table.clone().enumerate() {
                        if let Some(line) = output.buffer.line(output.scroll + row, &inset, None) {
                            frame.rows[usize::from(y)] = line;
                        }
                    }
                    if let Some(y) = layout.feedback.filter(|_| self.notice.is_empty()) {
                        layout.label(&mut frame, y, output.tone, &output.status_line());
                    }
                    let footer_line = if output.running {
                        tr(vi, "↑↓ Scroll   Ctrl+C Cancel", "↑↓ Cuộn   Ctrl+C Hủy")
                    } else {
                        tr(vi, "↑↓ Scroll   Esc Back", "↑↓ Cuộn   Esc Quay lại")
                    };
                    footer(&mut frame, &layout, footer_line);
                }
            }
        }
        frame
    }

    fn table_frame(&mut self, frame: &mut Frame, layout: &Layout) {
        self.reveal(layout.table.len());
        let vi = self.vi;
        let columns = Columns::new(layout.table_width());
        if frame.size.1 >= 2 {
            let search = tr(vi, "Search › ", "Tìm kiếm › ");
            input_line(
                frame,
                1,
                layout.margin,
                layout.width,
                search,
                &self.search,
                "",
            );
            if !self.searching {
                frame.cursor = None;
            }
        }
        if let Some(y) = layout.header {
            frame.rows[usize::from(y)] = columns.header(layout, vi);
        }
        let visible = layout
            .table
            .len()
            .min(self.visible.len().saturating_sub(self.scroll));
        if visible > 0 {
            for (offset, y) in layout.table.clone().take(visible).enumerate() {
                let index = self.scroll + offset;
                let row = self.row(index).expect("visible repository");
                frame.rows[usize::from(y)] = columns.row(
                    layout,
                    row,
                    index == self.cursor,
                    self.checked.contains(&row.selector),
                    vi,
                );
            }
            frame.menu = Some(MenuRows {
                columns: layout.margin as u16..(layout.margin + layout.table_width()) as u16,
                rows: layout.table.start..layout.table.start + visible as u16,
                first: self.scroll,
            });
            if let Some(row) = self.focused() {
                fill_details(frame, layout, row, vi);
            }
        } else if let Some(y) = layout.table.clone().next() {
            let reason = if self.loading {
                tr(vi, "Loading repositories…", "Đang tải kho mã…")
            } else if self.update_only && !self.rows.iter().any(|row| row.registered) {
                tr(
                    vi,
                    "Register a checkout in Repository Manager to select it here.",
                    "Đăng ký bản sao trong Trình quản lý kho mã để chọn tại đây.",
                )
            } else if self.rows.is_empty() {
                tr(
                    vi,
                    "Open Actions to discover repositories or add a local checkout.",
                    "Mở Thao tác để tìm kho mã hoặc thêm bản sao cục bộ.",
                )
            } else {
                tr(
                    vi,
                    "No repositories match this view.",
                    "Không có kho mã phù hợp với chế độ xem này.",
                )
            };
            layout.label(frame, y, Tone::Muted, reason);
        }
        if let Some(y) = layout.feedback
            && self.notice.is_empty()
            && !self.loading
        {
            let group = self
                .filters
                .group
                .as_deref()
                .unwrap_or(tr(vi, "All groups", "Mọi nhóm"));
            let status = format!(
                "{} / {} {}  ·  {} {}  ·  {}  ·  {}",
                self.visible.len(),
                self.rows.len(),
                tr(vi, "repositories", "kho mã"),
                self.checked.len(),
                tr(vi, "selected", "đã chọn"),
                group,
                self.filters.sort.label(vi)
            );
            layout.label(frame, y, Tone::Muted, &status);
        }
        footer(
            frame,
            layout,
            if self.update_only {
                tr(
                    vi,
                    "Enter Preview   Space Select   / Search   A Actions   Esc Back",
                    "Enter Xem trước   Space Chọn   / Tìm   A Thao tác   Esc Quay lại",
                )
            } else if layout.width >= 104 {
                tr(
                    vi,
                    "A Actions   Space Select   Ctrl+A All   Ctrl+D Clear   / Search   G Groups   S Sort   T Terminal   F5 Refresh",
                    "A Thao tác   Space Chọn   Ctrl+A Tất cả   Ctrl+D Bỏ chọn   / Tìm   G Nhóm   S Sắp xếp   T Terminal   F5 Làm mới",
                )
            } else {
                tr(
                    vi,
                    "A Actions   / Search   Space Select   Enter Details   G Groups   S Sort   T Terminal",
                    "A Thao tác   / Tìm   Space Chọn   Enter Chi tiết   G Nhóm   S Sắp xếp   T Terminal",
                )
            },
        );
    }
}

fn choices<'a>(
    frame: &mut Frame,
    layout: &Layout,
    cursor: usize,
    labels: impl Iterator<Item = &'a str> + Clone,
) {
    let available = layout.table.len();
    let first = cursor.saturating_sub(available.saturating_sub(1));
    let count = labels.clone().skip(first).take(available).count();
    for (offset, (y, label)) in layout.table.clone().zip(labels.skip(first)).enumerate() {
        if first + offset == cursor {
            layout.label(frame, y, Tone::Accent, &format!("› {label}"));
        } else {
            layout.label(frame, y, Tone::Plain, &format!("  {label}"));
        }
    }
    if count > 0 {
        frame.menu = Some(MenuRows {
            columns: layout.margin as u16..(layout.margin + layout.width) as u16,
            rows: layout.table.start..layout.table.start + count as u16,
            first,
        });
    }
}

fn footer(frame: &mut Frame, layout: &Layout, text: &str) {
    if let Some(y) = layout.shortcuts {
        layout.label(frame, y, Tone::Key, text);
    }
}
