use std::{borrow::Cow, ops::Range};

use crossterm::event::KeyCode;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::{Dashboard, Frame, Overlay, Row, Tone, tr};
use crate::tui::{
    terminal::{Line, MenuRows},
    view::input_line,
};

const CHECKBOX: usize = 6;

#[derive(Default)]
pub(super) struct TableHits {
    size: Option<(u16, u16)>,
    pub(super) checkbox: Range<u16>,
    pub(super) path: Range<u16>,
    tooltip: Range<u16>,
    shortcut_row: Option<u16>,
    shortcuts: Vec<(Range<u16>, KeyCode)>,
}

impl TableHits {
    pub(super) fn contains_row(&self, frame: &Frame, column: u16, row: u16) -> bool {
        self.size == Some(frame.size)
            && !self.tooltip.contains(&row)
            && frame.menu_hit(column, row).is_some()
    }

    pub(super) fn shortcut(&self, frame: &Frame, column: u16, row: u16) -> Option<KeyCode> {
        if self.size != Some(frame.size) || self.shortcut_row != Some(row) {
            return None;
        }
        self.shortcuts
            .iter()
            .find_map(|(columns, code)| columns.contains(&column).then_some(*code))
    }
}

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
            details: if width >= 133 { 35 } else { 0 },
        }
    }

    fn table_width(&self) -> usize {
        self.width - self.details
    }

    pub(super) fn horizontal_max(&self) -> usize {
        Columns::new(self.table_width(), 0).limit()
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
    let prefix = if value.as_bytes().get(1) == Some(&b':') {
        value
            .get(..3)
            .filter(|prefix| prefix.ends_with(['/', '\\']))
            .unwrap_or("")
    } else if value.starts_with(r"\\") {
        r"\\"
    } else if value.starts_with('/') {
        "/"
    } else {
        ""
    };
    let prefix = if width > prefix.width() + 3 {
        prefix
    } else {
        ""
    };
    let mut used = prefix.width() + 1;
    let mut start = value.len();
    for (index, grapheme) in value.grapheme_indices(true).rev() {
        used += grapheme.width();
        if used > width {
            break;
        }
        start = index;
    }
    format!("{prefix}…{}", &value[start..])
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
        "inaccessible" => (tr(vi, "Inaccessible", "Không truy cập được"), Tone::Error),
        "local" => (tr(vi, "Local checkout", "Bản sao cục bộ"), Tone::Accent),
        "available" => (tr(vi, "Remote only", "Chỉ có từ xa"), Tone::Accent),
        "uncloned" => (tr(vi, "Uncloned", "Chưa sao chép"), Tone::Muted),
        _ => (tr(vi, "Unavailable", "Không khả dụng"), Tone::Muted),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    State,
    Branch,
    Path,
    Revision,
    Source,
}

struct Columns {
    checkbox: usize,
    name: usize,
    viewport: usize,
    offset: usize,
    fields: [(Field, usize); 5],
}

impl Columns {
    fn new(width: usize, offset: usize) -> Self {
        let checkbox = CHECKBOX.min(width / 2);
        let space = width.saturating_sub(checkbox);
        let name = (width / 4)
            .clamp(13, 29)
            .min(if space > 10 { space - 4 } else { space });
        let viewport = space - name;
        let mut fields = [
            (Field::State, 19),
            (Field::Branch, 12),
            (Field::Path, 27),
            (Field::Revision, 13),
            (Field::Source, 12),
        ];
        let extra = viewport.saturating_sub(fields.iter().map(|(_, width)| width).sum());
        let branch = (extra / 3).min(10);
        fields[1].1 += branch;
        fields[2].1 += extra - branch;
        let mut columns = Self {
            checkbox,
            name,
            viewport,
            offset,
            fields,
        };
        columns.offset = offset.min(columns.limit());
        columns
    }

    fn limit(&self) -> usize {
        if self.viewport == 0 {
            return 0;
        }
        self.fields.iter().map(|(_, width)| width).sum::<usize>() - self.viewport
    }

    fn visible_fields(&self) -> impl Iterator<Item = (Field, usize, Range<usize>)> + '_ {
        let mut start = 0;
        self.fields.iter().filter_map(move |&(field, width)| {
            let end = start + width;
            let left = start.max(self.offset) - start;
            let right = end.min(self.offset + self.viewport).saturating_sub(start);
            start = end;
            (left < right).then_some((field, width, left..right))
        })
    }

    fn path_hit(&self, margin: usize) -> Range<u16> {
        let mut start = margin + self.checkbox + self.name;
        for (field, _, visible) in self.visible_fields() {
            let end = start + visible.len();
            if field == Field::Path {
                return start as u16..end as u16;
            }
            start = end;
        }
        0..0
    }

    fn header(&self, layout: &Layout, vi: bool) -> Line {
        let mut line = layout.line();
        cell(&mut line, self.checkbox, Tone::Heading, "✓");
        cell(
            &mut line,
            self.name,
            Tone::Heading,
            tr(vi, "Repository", "Kho mã"),
        );
        for (field, width, visible) in self.visible_fields() {
            let label = match field {
                Field::State => tr(vi, "Git status", "Trạng thái Git"),
                Field::Branch => tr(vi, "Branch", "Nhánh"),
                Field::Path => tr(vi, "Local path", "Đường dẫn"),
                Field::Revision => tr(vi, "Last commit", "Commit cuối"),
                Field::Source => tr(vi, "Source", "Nguồn"),
            };
            cell_part(&mut line, width, visible, Tone::Heading, label);
        }
        line
    }

    fn row(&self, layout: &Layout, row: &Row, current: bool, checked: bool, vi: bool) -> Line {
        let mut line = layout.line();
        let mark = if checked { "[x]" } else { "[ ]" };
        let pointer = if current { " ›" } else { "" };
        cell(
            &mut line,
            self.checkbox,
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
        let (status, tone) = state_label(&row.state, vi);
        let branch = if row.detached {
            "(HEAD)"
        } else {
            row.branch.as_deref().unwrap_or("—")
        };
        for (field, width, visible) in self.visible_fields() {
            let full_path;
            let (style, value) = match field {
                Field::State => (tone, status),
                Field::Branch => (Tone::Key, branch),
                Field::Path => {
                    full_path =
                        shorten_path(row.display_path().as_deref().unwrap_or("—"), width - 1);
                    (Tone::Path, full_path.as_str())
                }
                Field::Revision => (Tone::Muted, row.short_revision.as_deref().unwrap_or("—")),
                Field::Source => (Tone::Muted, source(row, vi, true)),
            };
            cell_part(&mut line, width, visible, style, value);
        }
        line
    }
}

fn cell_part(line: &mut Line, width: usize, visible: Range<usize>, tone: Tone, value: &str) {
    let text = clip(value, width - 1);
    let mut at = 0;
    let mut written = visible.start;
    for grapheme in text.graphemes(true) {
        let end = at + grapheme.width();
        if end > visible.end {
            break;
        }
        if at >= visible.start {
            line.push(Tone::Plain, " ".repeat(at - written));
            line.push(tone, grapheme);
            written = end;
        }
        at = end;
    }
    line.push(Tone::Plain, " ".repeat(visible.end - written));
}

fn divergence(row: &Row) -> String {
    match (row.ahead, row.behind) {
        (Some(ahead), Some(behind)) => format!("{ahead}↑ {behind}↓"),
        _ => "—".into(),
    }
}

fn source(row: &Row, vi: bool, compact: bool) -> &'static str {
    if row.registered {
        tr(vi, "Registered", "Đã đăng ký")
    } else if row.path.is_none() {
        if compact {
            "GitHub"
        } else {
            tr(vi, "Remote catalog", "Danh mục từ xa")
        }
    } else if compact {
        tr(vi, "Discovered", "Đã tìm thấy")
    } else {
        tr(vi, "Discovered checkout", "Bản sao đã tìm thấy")
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
    let branch = match (row.detached, row.unborn, row.branch.as_deref()) {
        (true, _, _) => tr(vi, "Detached HEAD", "HEAD rời").into(),
        (_, true, Some(branch)) => {
            format!("{branch} ({})", tr(vi, "unborn", "chưa có commit")).into()
        }
        (_, _, branch) => branch.unwrap_or("—").into(),
    };
    let parts = [
        (
            tr(vi, "Alias", "Bí danh"),
            row.alias.as_deref().unwrap_or("—").into(),
        ),
        (
            tr(vi, "Path", "Đường dẫn"),
            row.display_path().unwrap_or_else(|| "—".into()),
        ),
        (
            if row.path.is_none() && row.branch.is_some() {
                tr(vi, "Default branch", "Nhánh mặc định")
            } else {
                tr(vi, "Branch", "Nhánh")
            },
            branch,
        ),
        (
            tr(vi, "Preferred branch", "Nhánh ưu tiên"),
            row.preferred_branch.as_deref().unwrap_or("—").into(),
        ),
        (
            tr(vi, "Git status", "Trạng thái Git"),
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
        (
            tr(vi, "Commit message", "Nội dung commit"),
            row.commit_message.as_deref().unwrap_or("—").into(),
        ),
        (
            tr(vi, "Commit author", "Tác giả commit"),
            row.commit_author.as_deref().unwrap_or("—").into(),
        ),
        (
            tr(vi, "Commit time", "Thời điểm commit"),
            row.commit_timestamp.as_deref().unwrap_or("—").into(),
        ),
        (
            tr(vi, "Remote", "Kho từ xa"),
            row.remote.as_deref().unwrap_or("—").into(),
        ),
        ("Upstream", row.upstream.as_deref().unwrap_or("—").into()),
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
        (tr(vi, "Source", "Nguồn"), source(row, vi, false).into()),
    ];
    let mut output = vec![(Tone::Accent, row.name.clone())];
    output.extend(
        parts
            .into_iter()
            .map(|(label, value)| (Tone::Plain, format!("{label} · {value}"))),
    );
    if let Some(url) = row
        .remote_url
        .as_deref()
        .filter(|url| Some(*url) != row.remote.as_deref())
    {
        output.push((
            Tone::Plain,
            format!("{} · {url}", tr(vi, "Remote URL", "URL kho từ xa")),
        ));
    }
    if let Some(remote) = row.remote_name.as_deref() {
        output.push((
            Tone::Plain,
            format!("{} · {remote}", tr(vi, "Remote name", "Tên kho từ xa")),
        ));
    }
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

fn wrap(value: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for source in value.split('\n') {
        let source = printable(source);
        let mut line = String::new();
        let mut used = 0;
        for grapheme in source.graphemes(true) {
            let (grapheme, columns) = if grapheme.width() > width {
                ("�", 1)
            } else {
                (grapheme, grapheme.width())
            };
            if columns + used > width {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            line.push_str(grapheme);
            used += columns;
        }
        lines.push(line);
    }
    lines
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
        self.rendered_overlay = Some(std::mem::discriminant(&self.overlay));
        if self.table_hits.size != Some(size) {
            self.hover = None;
        }
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
                    let items: Vec<_> = details(row, vi)
                        .into_iter()
                        .flat_map(|(tone, line)| {
                            wrap(&line, layout.width)
                                .into_iter()
                                .map(move |line| (tone, line))
                        })
                        .collect();
                    self.overlay_scroll_max = items.len().saturating_sub(layout.table.len());
                    let start = scroll.min(self.overlay_scroll_max);
                    for (y, (tone, line)) in layout.table.clone().zip(items.into_iter().skip(start))
                    {
                        layout.label(&mut frame, y, tone, &line);
                    }
                }
                action_footer(
                    &mut frame,
                    &layout,
                    tr(vi, "Enter Actions", "Enter Thao tác"),
                    tr(
                        vi,
                        "↑↓ Scroll   C Copy path   Esc Back",
                        "↑↓ Cuộn   C Sao chép đường dẫn   Esc Quay lại",
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
                let items: Vec<_> = plan
                    .labels
                    .iter()
                    .flat_map(|label| wrap(&format!("• {label}"), layout.width))
                    .collect();
                self.overlay_scroll_max = items.len().saturating_sub(layout.table.len());
                let start = (*scroll).min(self.overlay_scroll_max);
                for (y, label) in layout.table.clone().zip(items.iter().skip(start)) {
                    layout.label(&mut frame, y, Tone::Plain, label);
                }
                action_footer(
                    &mut frame,
                    &layout,
                    tr(vi, "Y Confirm", "Y Xác nhận"),
                    tr(vi, "↑↓ Scroll   Esc Back", "↑↓ Cuộn   Esc Quay lại"),
                );
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
        let columns = Columns::new(layout.table_width(), self.horizontal);
        let mut hits = TableHits {
            size: Some(frame.size),
            checkbox: layout.margin as u16..(layout.margin + columns.checkbox.min(3)) as u16,
            path: columns.path_hit(layout.margin),
            ..TableHits::default()
        };
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
            && let Some(group) = &self.filters.group
        {
            layout.label(
                frame,
                y,
                Tone::Muted,
                &format!("{} · {group}", tr(vi, "Group", "Nhóm")),
            );
        }
        self.table_footer(frame, layout, &columns, &mut hits);
        if let Some(row) = self
            .path_to_copy()
            .filter(|_| self.hover.is_some() || self.path_focus)
            && let Some(path) = row.display_path()
        {
            let anchor = layout
                .table
                .clone()
                .take(visible)
                .enumerate()
                .find(|(index, _)| {
                    self.row(self.scroll + index)
                        .is_some_and(|candidate| candidate.selector == row.selector)
                })
                .map_or(layout.table.start, |(_, y)| y);
            hits.tooltip = tooltip(frame, layout, anchor, &path, self.vi);
        }
        self.table_hits = hits;
    }

    fn table_footer(
        &self,
        frame: &mut Frame,
        layout: &Layout,
        columns: &Columns,
        hits: &mut TableHits,
    ) {
        let Some(y) = layout.shortcuts else { return };
        let vi = self.vi;
        let full = format!(
            "{}/{} {} · {} {}",
            self.visible.len(),
            self.rows.len(),
            tr(vi, "repos", "kho mã"),
            self.checked.len(),
            tr(vi, "selected", "đã chọn")
        );
        let compact = format!(
            "{}/{} · {}✓",
            self.visible.len(),
            self.rows.len(),
            self.checked.len()
        );
        let count = if layout.width >= 78 { full } else { compact };
        let count = if count.width() <= layout.width {
            count.as_str()
        } else {
            ""
        };
        let available = layout.width.saturating_sub(count.width()).saturating_sub(2);
        let enter = if self.update_only {
            tr(vi, "Enter Preview", "Enter Xem trước")
        } else {
            tr(vi, "Enter Details", "Enter Chi tiết")
        };
        let shortcuts = [
            (KeyCode::Char('a'), tr(vi, "A Actions", "A Thao tác"), "A"),
            (KeyCode::Char(':'), ": CNL", ": CNL"),
            (KeyCode::Char('/'), tr(vi, "/ Search", "/ Tìm"), "/"),
            (KeyCode::Char('g'), tr(vi, "G Groups", "G Nhóm"), "G"),
            (KeyCode::Char('t'), "T Terminal", "T"),
            (KeyCode::Char('p'), tr(vi, "P Path", "P Đường dẫn"), "P"),
            (KeyCode::Char('c'), tr(vi, "C Copy", "C Sao chép"), "C"),
            (KeyCode::Left, "←", "←"),
            (KeyCode::Right, "→", "→"),
            (KeyCode::F(5), tr(vi, "F5 Refresh", "F5 Làm mới"), "F5"),
            (KeyCode::Enter, enter, "Enter"),
            (KeyCode::Esc, tr(vi, "Esc Back", "Esc Quay lại"), "Esc"),
        ];
        let mut line = layout.line();
        let mut used = 0;
        hits.shortcut_row = Some(y);
        for (code, full, short) in shortcuts {
            if (code == KeyCode::Char('t') && self.update_only)
                || (matches!(code, KeyCode::Left | KeyCode::Right) && columns.limit() == 0)
            {
                continue;
            }
            let separator = if used == 0 { "" } else { "  " };
            let start = used + separator.len();
            let text = if (code == KeyCode::Char('a') || layout.width >= 127)
                && start + full.width() <= available
            {
                full
            } else {
                short
            };
            let end = start + text.width();
            if end > available {
                break;
            }
            line.push(Tone::Plain, separator);
            line.push(Tone::Key, text);
            hits.shortcuts.push((
                (layout.margin + start) as u16..(layout.margin + end) as u16,
                code,
            ));
            used = end;
        }
        line.push(Tone::Plain, " ".repeat(layout.width - used - count.width()));
        line.push(Tone::Muted, count);
        frame.rows[usize::from(y)] = line;
    }
}

fn tooltip(frame: &mut Frame, layout: &Layout, anchor: u16, path: &str, vi: bool) -> Range<u16> {
    let prefix = tr(vi, "Path · ", "Đường dẫn · ");
    let mut lines = wrap(&format!("{prefix}{path}"), layout.width);
    let first = layout.table.start;
    let end = layout.shortcuts.unwrap_or(frame.size.1);
    let below = end.saturating_sub(anchor.saturating_add(1));
    let above = anchor.saturating_sub(first);
    let (start, usable) = if lines.len() <= usize::from(below) || below >= above {
        (anchor.saturating_add(1), below)
    } else {
        (
            anchor.saturating_sub(lines.len().min(usize::from(above)) as u16),
            above,
        )
    };
    if usable == 0 {
        return 0..0;
    }
    if lines.len() > usize::from(usable) {
        lines.truncate(usize::from(usable));
        let last = lines.last_mut().expect("tooltip line");
        *last = format!("{}…", clip(last, layout.width - 1));
    }
    let end = start + lines.len() as u16;
    for (y, text) in (start..end).zip(lines) {
        layout.label(frame, y, Tone::Path, &text);
    }
    start..end
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

fn action_footer(frame: &mut Frame, layout: &Layout, action: &str, rest: &str) {
    if let Some(y) = layout.shortcuts {
        layout.label(frame, y, Tone::Key, &format!("{action}   {rest}"));
        if action.width() <= layout.width {
            let start = layout.margin as u16;
            frame.suggestions = Some(MenuRows {
                columns: start..start + action.width() as u16,
                rows: y..y + 1,
                first: 0,
            });
        }
    }
}
