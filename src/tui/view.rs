use super::{
    Input, Menu, Prompt,
    banner::BANNER_WIDTH,
    terminal::{
        Frame, InputBorder, InputHit, Line, MenuRows, PalettePart, Readable, SourceRow, Tone,
    },
    text,
};
use crate::localization::terminal as labels;
use std::{borrow::Cow, ops::Range};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const NAME: &str = "CIRNET LAB";

struct Layout {
    columns: usize,
    margin: usize,
    width: usize,
    height: u16,
}

impl Layout {
    fn new(size: (u16, u16)) -> Self {
        // Leave the physical last column unused to avoid terminal autowrap.
        let columns = usize::from(size.0.saturating_sub(1));
        let margin = if columns >= 24 { 2 } else { 0 };
        Self {
            columns,
            margin,
            width: columns - 2 * margin,
            height: size.1,
        }
    }
}

fn clip(value: &str, width: usize) -> Cow<'_, str> {
    let mut remaining = width;
    let mut end = value.len();
    let mut control = false;
    for (index, grapheme) in value.grapheme_indices(true) {
        if grapheme.chars().any(|ch| ch.width().is_none()) {
            control = true;
            continue;
        }
        let columns = grapheme.width();
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
    pad(&mut line, Tone::Plain, margin);
    line
}

fn repeat(line: &mut Line, tone: Tone, block: &str, cell_bytes: usize, count: usize) {
    let bytes = count * cell_bytes;
    for _ in 0..bytes / block.len() {
        line.push(tone, block);
    }
    line.push(tone, &block[..bytes % block.len()]);
}

fn pad(line: &mut Line, tone: Tone, count: usize) {
    repeat(
        line,
        tone,
        "                                                                ",
        1,
        count,
    );
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

fn menu_footer(frame: &mut Frame, margin: usize, width: usize, vi: bool) -> u16 {
    let height = frame.size.1;
    if height < 6 {
        return height;
    }
    let full = text(labels::MENU_HINT, vi);
    let labeled = text(labels::MENU_HINT_SHORT, vi);
    let keys = "Tab : / Esc ^C ? F8";
    let last = height - 1;
    frame.feedback_row = Some(last);
    for candidate in [full, labeled, keys] {
        if candidate.width() <= width {
            label(frame, last, margin, width, Tone::Muted, candidate);
            return last;
        }
    }
    let brief = if width >= 8 {
        "Esc ? F8"
    } else if width >= 5 {
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
            || width < 5
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

    fn choice_start(&self) -> usize {
        self.prompt
            .focus
            .min(self.prompt.completions.len().saturating_sub(1))
            .saturating_add(1)
            .saturating_sub(usize::from(self.choices))
    }

    fn attach(&self, frame: &mut Frame, row: u16, margin: usize, color: bool) {
        if self.bordered && color {
            frame.input_border = Some(InputBorder {
                left: margin as u16,
                top: row,
                width: self.width as u16,
            });
        }
        if self.choices != 0 && self.prompt.error.is_none() {
            let first_row = row + if self.bordered { 3 } else { 1 };
            frame.suggestions = Some(MenuRows {
                columns: margin as u16..(margin + self.width) as u16,
                rows: first_row..first_row + self.choices,
                first: self.choice_start(),
            });
        }
    }

    fn draw_cap(&self, line: &mut Line, top: bool, color: bool) {
        const RULE: &str = "────────────────────────────────────────────────────────────────";

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
        repeat(line, rule_tone, RULE, '─'.len_utf8(), self.width - 2);
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
                pad(line, body_tone, inner - layout.used + 1);
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
            pad(line, body_tone, inner - used);
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
        let focus = self.prompt.focus;
        let offset = row - if self.bordered { 3 } else { 1 };
        let index = self.choice_start() + usize::from(offset);
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
        let column = self.label_width.min(width.saturating_sub(17));
        let detail_width = width.saturating_sub(column + 5);
        let show_detail = width >= 28 && detail_width >= 12;
        let label = clip(&item.label, if show_detail { column } else { width - 2 });
        let label_width = label.width();
        line.push(tone, label);
        if !show_detail {
            return label_width + 2;
        }
        let detail = if item.description.is_empty() {
            item.kind.label(vi)
        } else {
            &item.description
        };
        let detail = clip(detail, detail_width);
        let used = column + 5 + detail.width();
        pad(line, detail_tone, column - label_width + 3);
        line.push(detail_tone, detail);
        used
    }
}

fn palette_rows(
    frame: &mut Frame,
    palette: &Palette<'_>,
    row: u16,
    margin: usize,
    vi: bool,
    color: bool,
) {
    palette.attach(frame, row, margin, color);
    for offset in 0..palette.rows() {
        let y = row + offset;
        let mut line = indented(margin);
        if let Some(layout) = palette.draw(&mut line, offset, vi, color) {
            layout.attach(frame, y, margin);
        }
        frame.rows[usize::from(y)] = line;
    }
}

fn output_frame(menu: &mut Menu, size: (u16, u16), color: bool) -> Frame {
    let mut frame = Frame::new(size);
    let Layout {
        margin,
        width,
        height,
        ..
    } = Layout::new(size);
    let inset = if margin > 0 { "  " } else { "" };
    let output = menu.output.as_mut().expect("output mode");
    let find = output.find.as_ref();
    let command = menu.prompt.as_ref().filter(|_| find.is_none());
    let editing = command.is_some() || find.is_some();
    let body = u16::from(!editing || height >= 2);
    let has_status = height >= 2 + u16::from(editing);
    let header = u16::from(height >= if editing { 7 } else { 4 } && width >= 24);
    let status_gap = u16::from(height >= 16 && width >= 24 && command.is_some());
    let palette = command.map(|prompt| {
        let available = height - header - u16::from(has_status) - body - status_gap;
        Palette::new(prompt, width, available, !output.running, true)
    });
    let input_rows = palette
        .as_ref()
        .map_or(u16::from(find.is_some()), Palette::rows);
    let command_row = height - input_rows;
    let status_row = has_status.then(|| command_row - status_gap - 1);
    let body_end = status_row.unwrap_or(command_row);
    let visible = body_end.saturating_sub(header);
    let mut readable = output.buffer.copy_safe().then(|| Readable {
        first: header,
        rows: Vec::new(),
        inset: margin,
    });
    if header != 0 {
        label(
            &mut frame,
            0,
            margin,
            width,
            Tone::Accent,
            &format!(
                "{NAME}  /  {}",
                output
                    .title
                    .as_deref()
                    .unwrap_or_else(|| text(labels::NATIVE_SHELL, menu.vi)),
            ),
        );
    }
    output.layout(width, usize::from(visible));
    for row in 0..visible {
        let index = output.scroll + usize::from(row);
        if let Some(line) = output.buffer.line(index, inset, output.hit.as_ref()) {
            if let Some(readable) = &mut readable
                && let Some(bytes) = output.buffer.row_range(index)
            {
                readable.rows.push(SourceRow {
                    substituted: bytes.len() != line.contents().len() - margin,
                    bytes,
                });
            }
            frame.rows[usize::from(header + row)] = line;
        }
    }
    frame.readable = readable;
    if let Some(row) = status_row {
        frame.feedback_row = Some(row);
        label(
            &mut frame,
            row,
            margin,
            width,
            output.tone,
            &output.status_line(),
        );
    }
    if let Some(palette) = palette {
        palette_rows(&mut frame, &palette, command_row, margin, menu.vi, color);
    }
    if let Some(find) = &output.find {
        let feedback = if !find.value.is_empty() && output.hit.is_none() {
            text(labels::NO_FIND_MATCHES, menu.vi)
        } else {
            ""
        };
        input_line(
            &mut frame,
            command_row,
            margin,
            width,
            text(labels::FIND_PROMPT, menu.vi),
            find,
            feedback,
        );
    }
    frame
}

pub(super) fn frame(menu: &mut Menu, size: (u16, u16), banner: bool, color: bool) -> Frame {
    menu.refresh_suggestions();
    let mut frame = if size.1 == 0 || size.0 <= 1 {
        let mut frame = Frame::new(size);
        // A one-column terminal has no safe cell to print, but still hosts the editor.
        let editor = menu
            .output
            .as_ref()
            .and_then(|output| output.find.as_ref())
            .or(menu.prompt.as_ref().map(|prompt| &prompt.input))
            .or(menu.search.as_ref());
        if let Some(editor) = editor.filter(|_| size.0 == 1 && size.1 > 0) {
            frame.cursor = Some((0, size.1 - 1));
            frame.input = Some(InputHit {
                row: size.1 - 1,
                columns: 0..1,
                offset: editor.cursor,
            });
        }
        frame
    } else if menu.output.is_some() {
        output_frame(menu, size, color)
    } else {
        menu_frame(menu, size, banner, color)
    };
    menu.interaction.decorate(&mut frame, menu.vi);
    frame
}

fn menu_frame(menu: &mut Menu, size: (u16, u16), banner: bool, color: bool) -> Frame {
    let mut frame = Frame::new(size);
    let Layout {
        columns,
        margin,
        width,
        height,
    } = Layout::new(size);
    let mut row = u16::from(height >= 16);
    if banner && columns >= BANNER_WIDTH && height >= 22 {
        let inset = if width >= BANNER_WIDTH { margin } else { 0 };
        let lines = menu
            .banner
            .lines(inset, color, columns.saturating_sub(inset));
        let start = usize::from(row);
        frame.rows[start..start + lines.len()].clone_from_slice(lines);
        row += lines.len() as u16 + 1;
    } else if height >= 5 {
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
            text(labels::MENU_COMMANDS, menu.vi),
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
            text(labels::SEARCH_PROMPT, menu.vi),
            search,
            "",
        );
    }

    let wide = width >= 72 && end.saturating_sub(row) >= 2;
    let left_width = if wide {
        (width * 2 / 5).clamp(26, 46)
    } else {
        width
    };
    let divider = if wide { "  │  " } else { "" };
    let right_start = left_width + divider.width();
    let right_width = width.saturating_sub(right_start);
    let mut preview = Vec::new();
    if menu.help && menu.prompt.is_none() {
        let help = text(labels::COMMAND_REFERENCE, menu.vi);
        if wide {
            preview.push((Tone::Heading, help.to_owned()));
            preview.push((Tone::Plain, "cnl --help · F8 EN/VI".to_owned()));
        } else if end.saturating_sub(row) >= 2 {
            label(
                &mut frame,
                row,
                margin,
                width,
                Tone::Heading,
                &format!("{help} · cnl --help · F8 EN/VI"),
            );
            row += 1;
        }
    } else if (wide
        || (width >= 40
            && end.saturating_sub(row) >= 9
            && menu.prompt.is_none()
            && menu.search.is_none()))
        && let Some(entry) = menu.items.get(menu.selected)
    {
        if !entry.command.is_empty() {
            preview.push((Tone::Accent, format!("cnl {}", entry.command)));
        }
        preview.extend(
            wrap(
                text(entry.detail, menu.vi),
                if wide { right_width.min(86) } else { width },
            )
            .into_iter()
            .map(|line| (Tone::Plain, line)),
        );
    }

    if !wide && !preview.is_empty() {
        let lines = preview.len().min(3) as u16;
        let start = end - lines;
        for (offset, (tone, value)) in preview.iter().take(usize::from(lines)).enumerate() {
            label(
                &mut frame,
                start + offset as u16,
                margin,
                width,
                *tone,
                value,
            );
        }
        end = start - 1;
    }

    let mut wide_start = None;
    if let Some(prompt) = &menu.prompt {
        let available = end.saturating_sub(row);
        if wide {
            let minimum_palette = if available >= 5 { 4 } else { 2 };
            let start = (preview.len() as u16 + 1)
                .max(5)
                .min(available - minimum_palette);
            wide_start = Some(start);
        } else if available > 0 {
            let menu_rows = if available >= 6 {
                2
            } else {
                (available - 1).min(1)
            };
            let palette = Palette::new(prompt, width, available - menu_rows, true, true);
            end -= palette.rows();
            palette_rows(&mut frame, &palette, end, margin, menu.vi, color);
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
    if let Some((start, palette)) = &wide_palette {
        palette.attach(&mut frame, row + start, margin + right_start, color);
    }
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
            let label = text(entry.label, menu.vi);
            let value = if left_width < 24
                && label.width() + marker.width() > left_width
                && !entry.command.is_empty()
            {
                entry.command
            } else {
                label
            };
            let value = clip(value, left_width - marker.width());
            let tone = if selected { Tone::Accent } else { Tone::Plain };
            used = marker.width() + value.width();
            line.push(tone, marker);
            line.push(tone, value);
        } else if i == 0 && items.is_empty() {
            let message = text(labels::NO_MATCHING_COMMANDS, menu.vi);
            let value = clip(message, left_width);
            used = value.width();
            line.push(Tone::Muted, value);
        }
        if wide {
            pad(&mut line, Tone::Plain, left_width - used);
            line.push(Tone::Muted, divider);
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
                layout.attach(&mut frame, y, margin + right_start);
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
    fn resize_keeps_editor_focus_selection_and_menu_hit_regions() {
        let mut menu = Menu::new(true);
        let mut input = Input::at_end("run --desk".into());
        input.select_all();
        menu.open_prompt(input);
        menu.refresh_suggestions();
        let before = menu.prompt.as_ref().unwrap().input.selection();
        for size in [
            (180, 29),
            (66, 12),
            (32, 7),
            (18, 3),
            (2, 1),
            (1, 1),
            (120, 26),
        ] {
            let frame = frame(&mut menu, size, true, true);
            let (x, y) = frame.cursor.expect("active editor stays visible");
            assert!(x < size.0 && y < size.1, "{size:?}: ({x}, {y})");
            let hit = frame.input.as_ref().unwrap();
            assert!(hit.columns.contains(&x));
            assert_eq!(hit.row, y);
            if size.1 >= 3 {
                let rows = frame.menu.as_ref().expect("visible navigation");
                assert!(
                    frame
                        .menu_hit(rows.columns.start, rows.rows.start)
                        .is_some()
                );
            }
            if size.0 == 180 {
                assert!(hit.columns.end > 170);
            }
            assert_eq!(menu.prompt.as_ref().unwrap().input.selection(), before);
        }
    }

    #[test]
    fn narrow_palette_clips_long_suggestions_and_keeps_focused_row_reachable() {
        let mut menu = Menu::new(true);
        menu.open_prompt(Input::at_end("run --".into()));
        menu.refresh_suggestions();
        let prompt = menu.prompt.as_mut().unwrap();
        assert!(prompt.completions.len() >= 2);
        prompt.focus = 1;
        prompt.completions[0].label = "một-gợi-ý-rất-dài-🧑‍💻".into();
        let frame = frame(&mut menu, (12, 6), false, true);
        let target = frame.suggestions.unwrap();
        assert!(target.rows.clone().any(|row| row >= 2));
        assert!((target.first..target.first + target.rows.len()).contains(&1));
        assert!(frame.rows.iter().all(|line| line.contents().width() < 12));
    }

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
