// Derived from CIRNET Rules CNR, commit 646afa38516bbfd276b0aecb7a138ab427df5fa9 (GPL-3.0).
use super::{
    Input, Menu, Prompt,
    banner::BANNER_WIDTH,
    terminal::{Frame, InputHit, Line, MenuRows, PalettePart, Tone},
    text,
};
use std::{borrow::Cow, ops::Range, sync::LazyLock};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const NAME: &str = "CIRNETLAB";
const MAX_COLUMNS: usize = 132;

fn clip(value: &str, width: usize) -> Cow<'_, str> {
    let mut remaining = width;
    let mut end = value.len();
    let mut control = false;
    for (index, ch) in value.char_indices() {
        let Some(columns) = ch.width() else {
            control = true;
            continue;
        };
        if columns > remaining {
            end = index;
            break;
        }
        remaining -= columns;
    }
    let visible = &value[..end];
    if control {
        Cow::Owned(visible.chars().filter(|ch| ch.width().is_some()).collect())
    } else {
        Cow::Borrowed(visible)
    }
}

fn tail(value: &str, width: usize) -> &str {
    let mut remaining = width;
    let mut start = value.len();
    for (index, grapheme) in value.grapheme_indices(true).rev() {
        let columns = grapheme.width();
        if columns > remaining {
            break;
        }
        start = index;
        remaining -= columns;
    }
    &value[start..]
}

fn indented(margin: usize) -> Line {
    let mut line = Line::default();
    line.push(Tone::Plain, spaces(margin));
    line
}

fn spaces(count: usize) -> &'static str {
    const SPACES: &str = concat!(
        "                           ",
        "                           ",
        "                           ",
        "                           ",
        "                           "
    );
    &SPACES[..count]
}

fn label(frame: &mut Frame, row: u16, margin: usize, width: usize, tone: Tone, value: &str) {
    let mut line = indented(margin);
    line.push(tone, clip(value, width));
    frame.rows[usize::from(row)] = line;
}

pub(super) fn input_line(
    frame: &mut Frame,
    row: u16,
    margin: usize,
    width: usize,
    prefix: &str,
    input: &Input,
    feedback: &str,
) {
    let mut line = indented(margin);
    let layout = input_text(
        &mut line,
        width,
        prefix,
        Tone::Heading,
        Tone::Plain,
        input,
        feedback,
    );
    layout.attach(frame, row, margin);
    frame.rows[usize::from(row)] = line;
}

struct InputLayout {
    cursor: usize,
    used: usize,
    columns: Range<usize>,
    offset: usize,
}

impl InputLayout {
    fn attach(self, frame: &mut Frame, row: u16, margin: usize) {
        frame.cursor = Some(((margin + self.cursor) as u16, row));
        frame.input = Some(InputHit {
            row,
            columns: (margin + self.columns.start) as u16..(margin + self.columns.end) as u16,
            offset: self.offset,
        });
    }
}

fn input_text(
    line: &mut Line,
    width: usize,
    prefix: &str,
    marker: Tone,
    body: Tone,
    input: &Input,
    feedback: &str,
) -> InputLayout {
    let prefix = clip(prefix, width.saturating_sub(1));
    let available = width - prefix.width();
    let before = tail(&input.value[..input.cursor], available.saturating_sub(1));
    let offset = input.cursor - before.len();
    line.push(marker, &prefix);
    let mut used = 0;
    let selection = input.selection();
    for (index, grapheme) in input.value[offset..].grapheme_indices(true) {
        let columns = grapheme.width();
        if used + columns > available {
            break;
        }
        let tone = if selection
            .as_ref()
            .is_some_and(|range| range.contains(&(offset + index)))
        {
            Tone::Selection
        } else {
            body
        };
        line.push(tone, grapheme);
        used += columns;
    }
    if input.cursor == input.value.len() {
        let feedback = clip(feedback, available - used);
        line.push(Tone::Muted, &feedback);
        used += feedback.width();
    }
    let cursor = prefix.width() + before.width();
    InputLayout {
        cursor,
        used: prefix.width() + used,
        columns: prefix.width()..width,
        offset,
    }
}

fn wrap(value: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut line = String::new();
    for word in value.split_whitespace() {
        if !line.is_empty() && line.width() + word.width() + 1 > width {
            rows.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        rows.push(line);
    }
    rows
}

const MENU_HINT: [&str; 2] = [
    "Tab complete · : command · / search · Esc back · Ctrl+C exit · ? help",
    "Tab gợi ý · : lệnh · / tìm · Esc quay lại · Ctrl+C thoát · ? trợ giúp",
];

fn menu_footer(frame: &mut Frame, margin: usize, width: usize, vi: bool) -> u16 {
    let height = frame.size.1;
    if height < 2 {
        return height;
    }
    let full = text(MENU_HINT, vi);
    let labeled = text(
        [
            "Tab · : cmd · / find · Esc · Ctrl+C · ?",
            "Tab · : lệnh · / tìm · Esc · Ctrl+C · ?",
        ],
        vi,
    );
    let keys = "Tab : / Esc Ctrl+C ?";
    let last = height - 1;
    for candidate in [full, labeled, keys] {
        if candidate.width() <= width {
            label(frame, last, margin, width, Tone::Muted, candidate);
            return last;
        }
    }
    if width >= 8 && height >= 4 {
        label(frame, last - 1, margin, width, Tone::Muted, "Tab : /");
        let rest = if width >= 12 {
            "Esc Ctrl+C ?"
        } else {
            "Esc ^C ?"
        };
        label(frame, last, margin, width, Tone::Muted, rest);
        return last - 1;
    }
    let brief = if width >= 5 {
        "Esc ?"
    } else if width >= 3 {
        "Esc"
    } else {
        "?"
    };
    label(frame, last, margin, width, Tone::Muted, brief);
    last
}

struct Palette<'a> {
    prompt: &'a Prompt,
    width: usize,
    choices: u16,
    label_width: usize,
    bordered: bool,
}

impl<'a> Palette<'a> {
    fn new(
        prompt: &'a Prompt,
        width: usize,
        available: u16,
        show_choices: bool,
        show_notice: bool,
    ) -> Self {
        let bordered = width >= 12 && available >= 4;
        let label_width = prompt
            .completions
            .iter()
            .map(|candidate| candidate.label.width())
            .max()
            .unwrap_or(0);
        let choices = if show_notice && prompt.error.is_some() {
            1
        } else if !show_choices
            || prompt.error.is_some()
            || prompt.input.value.trim().is_empty()
            || width < label_width.saturating_add(if bordered { 6 } else { 2 })
        {
            0
        } else {
            prompt.completions.len().min(5) as u16
        };
        Self {
            prompt,
            width,
            choices: choices.min(available.saturating_sub(if bordered { 3 } else { 1 })),
            label_width,
            bordered,
        }
    }

    fn rows(&self) -> u16 {
        1 + self.choices + 2 * u16::from(self.bordered)
    }

    fn draw_cap(&self, line: &mut Line, top: bool, color: bool) {
        static RULE: LazyLock<String> = LazyLock::new(|| "─".repeat(MAX_COLUMNS));

        let (left, right) = if top { ("╭", "╮") } else { ("╰", "╯") };
        let tone = if color {
            Tone::Palette(PalettePart::InputRim)
        } else {
            Tone::Plain
        };
        let rule_tone = if color {
            Tone::Palette(PalettePart::InputEdge)
        } else {
            Tone::Plain
        };
        line.push(tone, left);
        line.push(rule_tone, &RULE[..(self.width - 2) * '─'.len_utf8()]);
        line.push(tone, right);
    }

    fn draw(&self, line: &mut Line, row: u16, vi: bool, color: bool) -> Option<InputLayout> {
        use PalettePart::*;
        if self.bordered && (row == 0 || row == 2) {
            self.draw_cap(line, row == 0, color);
            return None;
        }

        let input_row = row == u16::from(self.bordered);
        if input_row {
            let body_tone = if self.bordered {
                Tone::Palette(Input)
            } else {
                Tone::Plain
            };
            let inset = usize::from(self.bordered) * 2;
            let inner = self.width - 2 * inset;
            let edge_tone = if color {
                Tone::Palette(InputEdge)
            } else {
                Tone::Plain
            };
            if self.bordered {
                line.push(edge_tone, "│");
                line.push(body_tone, " ");
            }
            let mut layout = input_text(
                line,
                inner,
                "› ",
                body_tone,
                body_tone,
                &self.prompt.input,
                "",
            );
            if self.bordered {
                line.push(body_tone, spaces(inner - layout.used + 1));
                line.push(edge_tone, "│");
            }
            layout.cursor += inset;
            layout.columns = inset + layout.columns.start..inset + layout.columns.end;
            Some(layout)
        } else {
            let body_tone = if self.bordered || color {
                Tone::Palette(Detail)
            } else {
                Tone::Muted
            };
            let edge = if color {
                Tone::Palette(SuggestionEdge)
            } else {
                Tone::Plain
            };
            let bordered = usize::from(self.bordered);
            let inner = self.width - 3 * bordered;
            if self.bordered {
                line.push(edge, if color { "▐" } else { " " });
                line.push(body_tone, " ");
            }
            let used = if let Some(error) = self.prompt.error {
                let value = clip(text(error, vi), inner);
                let used = value.width();
                line.push(body_tone, value);
                used
            } else {
                self.draw_choice(line, row, inner, vi, color)
            };
            line.push(body_tone, spaces(inner - used));
            if self.bordered {
                line.push(edge, if color { "▌" } else { " " });
            }
            None
        }
    }

    fn draw_choice(&self, line: &mut Line, row: u16, width: usize, vi: bool, color: bool) -> usize {
        let detail_tone = if self.bordered || color {
            Tone::Palette(PalettePart::Detail)
        } else {
            Tone::Muted
        };
        let focus = self.prompt.accepted.as_ref().map_or(0, |(index, _)| *index);
        let offset = row - if self.bordered { 3 } else { 1 };
        let index = (focus + 1).saturating_sub(usize::from(self.choices)) + usize::from(offset);
        let item = &self.prompt.completions[index];
        let selected = index == focus;
        line.push(
            detail_tone,
            if selected && (!self.bordered || !color) {
                "› "
            } else {
                "  "
            },
        );
        let tone = match (self.bordered || color, selected) {
            (true, true) => Tone::Palette(PalettePart::Selected),
            (true, false) => Tone::Palette(PalettePart::Suggestion),
            (false, true) => Tone::Heading,
            (false, false) => Tone::Plain,
        };
        line.push(tone, &item.label);
        let label = item.label.width();
        let detail_width = width.saturating_sub(self.label_width + 5);
        if detail_width < 12 {
            return label + 2;
        }
        let detail = if item.description.is_empty() {
            item.kind.label(vi)
        } else {
            &item.description
        };
        let detail = clip(detail, detail_width);
        let used = self.label_width + 5 + detail.width();
        line.push(detail_tone, spaces(self.label_width - label + 3));
        line.push(detail_tone, detail);
        used
    }
}

fn output_frame(menu: &mut Menu, size: (u16, u16), color: bool) -> Frame {
    let mut frame = Frame::new(size);
    let columns = usize::from(size.0.saturating_sub(1)).min(MAX_COLUMNS);
    let height = size.1;
    if columns == 0 || height == 0 {
        return frame;
    }
    let inset = if columns >= 24 { "  " } else { "" };
    let margin = inset.len();
    let width = columns - 2 * margin;
    let output = menu.output.as_mut().expect("output mode");
    let find = output.find.as_ref();
    let separate_find = find.is_some() && height >= 3;
    let header = u16::from(height >= 4 + u16::from(separate_find));
    let has_status = height >= 2;
    let body = u16::from(height >= 3 + u16::from(separate_find));
    let status_gap = u16::from(height >= 16 && width >= 24 && menu.prompt.is_some());
    let palette = menu.prompt.as_ref().map(|prompt| {
        let available = height.saturating_sub(
            header + u16::from(has_status) + body + u16::from(separate_find) + status_gap,
        );
        Palette::new(
            prompt,
            width,
            available,
            find.is_none() && !output.running,
            find.is_none(),
        )
    });
    let command_row = height - palette.as_ref().map_or(0, Palette::rows);
    let status_row = has_status.then(|| command_row - status_gap - 1);
    let body_end = status_row
        .unwrap_or(command_row)
        .saturating_sub(u16::from(separate_find));
    let visible = body_end.saturating_sub(header);
    if output.buffer.copy_safe() {
        frame.readable = Some(header..body_end);
    }
    if header != 0 {
        label(
            &mut frame,
            0,
            margin,
            width,
            Tone::Accent,
            &format!("{NAME}  /  {}", output.title),
        );
    }
    output.layout(width, usize::from(visible));
    for row in 0..visible {
        if let Some(line) =
            output
                .buffer
                .line(output.scroll + usize::from(row), inset, output.hit.as_ref())
        {
            frame.rows[usize::from(header + row)] = line;
        }
    }
    if let Some(row) = status_row {
        label(&mut frame, row, margin, width, output.tone, &output.status);
    }
    if let Some(palette) = palette {
        for offset in 0..palette.rows() {
            let row = command_row + offset;
            let mut line = indented(margin);
            if let Some(layout) = palette.draw(&mut line, offset, menu.vi, color) {
                layout.attach(&mut frame, row, margin);
            }
            frame.rows[usize::from(row)] = line;
        }
    }
    if let Some(find) = &output.find {
        let feedback = if !find.value.is_empty() && output.hit.is_none() {
            text(["  (no matches)", "  (không có kết quả)"], menu.vi)
        } else {
            ""
        };
        let row = if separate_find { body_end } else { height - 1 };
        input_line(
            &mut frame,
            row,
            margin,
            width,
            text(["Find  › ", "Tìm  › "], menu.vi),
            find,
            feedback,
        );
    }
    frame
}

pub(super) fn frame(menu: &mut Menu, size: (u16, u16), banner: bool, color: bool) -> Frame {
    menu.refresh_suggestions();
    let mut frame = if menu.output.is_some() {
        output_frame(menu, size, color)
    } else {
        menu_frame(menu, size, banner, color)
    };
    menu.interaction.decorate(&mut frame, menu.vi);
    frame
}

fn menu_frame(menu: &mut Menu, size: (u16, u16), banner: bool, color: bool) -> Frame {
    let mut frame = Frame::new(size);
    let columns = usize::from(size.0.saturating_sub(1)).min(MAX_COLUMNS);
    let height = size.1;
    if columns == 0 || height == 0 {
        return frame;
    }
    let margin = if columns >= 24 { 2 } else { 0 };
    let width = columns - 2 * margin;
    let mut row = u16::from(height >= 16);
    if banner && columns >= BANNER_WIDTH && height >= 22 {
        let inset = if width >= BANNER_WIDTH { margin } else { 0 };
        let lines = menu
            .banner
            .lines(inset, color, columns.saturating_sub(inset));
        let start = usize::from(row);
        frame.rows[start..start + lines.len()].clone_from_slice(lines);
        row += lines.len() as u16 + 1;
    } else if height >= 3 {
        let name = if width >= NAME.len() { NAME } else { "CNL" };
        label(&mut frame, row, margin, width, Tone::Accent, name);
        row += 1 + u16::from(height >= 12);
    }
    if height >= 12 {
        label(
            &mut frame,
            row,
            margin,
            width,
            Tone::Heading,
            text(["Commands", "Lệnh"], menu.vi),
        );
        row += 1;
    }

    let mut end = menu_footer(&mut frame, margin, width, menu.vi);
    if menu.prompt.is_none()
        && let Some(search) = &menu.search
        && end > row
    {
        end -= 1;
        input_line(
            &mut frame,
            end,
            margin,
            width,
            text(["Search  › ", "Tìm kiếm  › "], menu.vi),
            search,
            "",
        );
    }

    let wide = width >= 80 && end.saturating_sub(row) >= 4;
    let left_width = if wide { 33 } else { width };
    let right_width = width.saturating_sub(left_width + 5);
    let mut preview = Vec::new();
    if menu.help && menu.prompt.is_none() {
        let help = text(["Command reference", "Tham chiếu lệnh"], menu.vi);
        if wide {
            preview.push((Tone::Heading, help.to_owned()));
            preview.push((Tone::Plain, "cnl --help".to_owned()));
        } else if end > row {
            label(
                &mut frame,
                row,
                margin,
                width,
                Tone::Heading,
                &format!("{help} · cnl --help"),
            );
            row += 1;
        }
    } else if wide && let Some(entry) = menu.items.get(menu.selected) {
        if !entry.command.is_empty() {
            preview.push((Tone::Accent, format!("cnl {}", entry.command)));
        }
        preview.extend(
            wrap(text(entry.detail, menu.vi), right_width)
                .into_iter()
                .map(|line| (Tone::Plain, line)),
        );
    }

    let mut wide_start = None;
    if let Some(prompt) = &menu.prompt {
        let available = end.saturating_sub(row);
        if wide {
            let start = (preview.len() as u16 + 1).max(5).min(available - 1);
            wide_start = Some(start);
        } else if available > 0 {
            let menu_rows = (available - 1).min(2);
            let palette = Palette::new(prompt, width, available - menu_rows, true, true);
            end -= palette.rows();
            for offset in 0..palette.rows() {
                let y = end + offset;
                let mut line = indented(margin);
                if let Some(layout) = palette.draw(&mut line, offset, menu.vi, color) {
                    layout.attach(&mut frame, y, margin);
                }
                frame.rows[usize::from(y)] = line;
            }
        }
    }

    let visible = usize::from(end.saturating_sub(row));
    if visible == 0 {
        return frame;
    }
    menu.reveal_selection(visible);
    let wide_palette = wide_start.zip(menu.prompt.as_ref()).map(|(start, prompt)| {
        (
            start,
            Palette::new(prompt, right_width, end - row - start, true, true),
        )
    });
    let items = &menu.items[menu.first..];
    frame.menu = Some(MenuRows {
        columns: margin as u16..(margin + left_width) as u16,
        rows: row..row + items.len().min(visible) as u16,
        first: menu.first,
    });
    let right_rows = wide_palette
        .as_ref()
        .map_or(0, |(start, palette)| usize::from(start + palette.rows()));
    let count = items
        .len()
        .max(1)
        .max(preview.len())
        .max(right_rows)
        .min(visible);
    for i in 0..count {
        let y = row + i as u16;
        let mut line = indented(margin);
        let mut used = 0;
        if let Some(entry) = items.get(i) {
            let selected = menu.first + i == menu.selected;
            let marker = clip(if selected { "› " } else { "  " }, left_width);
            let value = clip(text(entry.label, menu.vi), left_width - marker.width());
            let tone = if selected { Tone::Accent } else { Tone::Plain };
            used = marker.width() + value.width();
            line.push(tone, marker);
            line.push(tone, value);
        } else if i == 0 && items.is_empty() {
            let message = text(["No matching commands", "Không có lệnh phù hợp"], menu.vi);
            let value = clip(message, left_width);
            used = value.width();
            line.push(Tone::Muted, value);
        }
        if wide {
            line.push(Tone::Plain, spaces(left_width - used));
            line.push(Tone::Muted, "  │  ");
            if let Some((tone, value)) = preview
                .get(i)
                .filter(|_| wide_start.is_none_or(|start| i < usize::from(start)))
            {
                line.push(*tone, clip(value, right_width));
            } else if let Some((start, palette)) = &wide_palette
                && let Some(offset) = (i as u16).checked_sub(*start)
                && offset < palette.rows()
                && let Some(layout) = palette.draw(&mut line, offset, menu.vi, color)
            {
                layout.attach(&mut frame, y, margin + left_width + 5);
            }
        }
        frame.rows[usize::from(y)] = line;
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_and_wide_frames_keep_the_menu_and_command_editor_reachable() {
        let mut menu = Menu::new(true);
        menu.open_prompt(Input::at_end("run --desk".into()));
        for rows in 1..=35 {
            for columns in [1, 2, 8, 23, 32, 79, 84, 96, 120] {
                let rendered = frame(&mut menu, (columns, rows), true, true);
                assert_eq!(rendered.rows.len(), usize::from(rows));
            }
        }
        let rendered = frame(&mut menu, (120, 30), true, true);
        assert!(rendered.cursor.is_some());
        let text = rendered
            .rows
            .iter()
            .map(Line::contents)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains('│') && text.contains("Chẩn đoán") && text.contains("--desktop"));
        assert!(rendered.rows.iter().any(|line| line.has(Tone::Shadow)));
    }
}
