use super::{
    COMMAND_LIMIT, COMMAND_TOO_LONG, Effect, Input, Menu, SEARCH_LIMIT,
    clipboard::{Clipboard, ClipboardError},
    control,
    editor::byte_at_column,
    terminal::{Frame, InputHit, Line, PalettePart, SourceRow, Tone},
    text,
};
use crate::localization::{Localized, terminal as labels};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Editor {
    Prompt,
    MenuSearch,
    OutputSearch,
}

impl Editor {
    fn limit(self) -> usize {
        if self == Self::Prompt {
            COMMAND_LIMIT
        } else {
            SEARCH_LIMIT
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Action {
    Copy,
    Cut,
    Paste,
    Delete,
    SelectAll,
    Undo,
    Redo,
    CopyLine,
}

impl Action {
    fn label(self, vi: bool) -> &'static str {
        text(
            match self {
                Self::Copy => labels::COPY,
                Self::Cut => labels::CUT,
                Self::Paste => labels::PASTE,
                Self::Delete => labels::DELETE,
                Self::SelectAll => labels::SELECT_ALL,
                Self::Undo => labels::UNDO,
                Self::Redo => labels::REDO,
                Self::CopyLine => labels::COPY_LINE,
            },
            vi,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Selection {
    anchor: usize,
    end: usize,
}

impl Selection {
    fn bytes(self) -> Range<usize> {
        self.anchor.min(self.end)..self.anchor.max(self.end)
    }

    fn active(self) -> bool {
        self.anchor != self.end
    }
}

#[derive(Default)]
struct Visible {
    first: u16,
    inset: usize,
    lines: Vec<VisibleLine>,
}

struct VisibleLine {
    text: Line,
    source: SourceRow,
}

impl Visible {
    fn capture(frame: &Frame) -> Self {
        let Some(view) = &frame.readable else {
            return Self::default();
        };
        let mut bytes = 0;
        let lines = view
            .rows
            .iter()
            .take(512)
            .zip(&frame.rows[usize::from(view.first)..])
            .take_while(|(row, line)| {
                bytes += row.bytes.len().max(line.contents().len());
                bytes <= super::clipboard::MAX_BYTES
            })
            .map(|(source, line)| VisibleLine {
                text: line.clone(),
                source: source.clone(),
            })
            .collect();
        Self {
            first: view.first,
            inset: view.inset,
            lines,
        }
    }

    fn row(&self, row: u16, clamp: bool) -> Option<&VisibleLine> {
        let last = self.lines.len().checked_sub(1)?;
        let index = if clamp {
            usize::from(row.saturating_sub(self.first)).min(last)
        } else {
            usize::from(row.checked_sub(self.first)?)
        };
        self.lines.get(index)
    }

    fn point(&self, column: u16, row: u16, clamp: bool) -> Option<usize> {
        let line = self.row(row, clamp)?;
        let offset = byte_at_column(
            &line.text.contents()[self.inset..],
            usize::from(column).saturating_sub(self.inset),
        );
        if line.source.substituted && offset != 0 {
            Some(line.source.bytes.end)
        } else {
            Some(line.source.bytes.start + offset)
        }
    }

    fn copy_line(&self, from: Option<usize>) -> Option<&str> {
        let line = from
            .and_then(|byte| {
                self.lines
                    .iter()
                    .rfind(|line| line.source.bytes.start <= byte)
            })
            .or_else(|| self.lines.first())?;
        Some(line.text.contents().trim())
    }

    fn all(&self) -> Option<Selection> {
        Some(Selection {
            anchor: self.lines.first()?.source.bytes.start,
            end: self.lines.last()?.source.bytes.end,
        })
    }

    fn decorate(&self, frame: &mut Frame, selection: Selection) {
        let selected = selection.bytes();
        for (index, line) in self.lines.iter().enumerate() {
            let source = &line.source.bytes;
            let left = selected.start.max(source.start);
            let right = selected.end.min(source.end);
            if left >= right {
                continue;
            }
            let range = if line.source.substituted {
                self.inset..line.text.contents().len()
            } else {
                self.inset + left - source.start..self.inset + right - source.start
            };
            frame.rows[usize::from(self.first) + index].select(range);
        }
    }
}

#[derive(Clone, Copy)]
enum Press {
    Input(Editor),
    Read,
    Menu(usize),
    Suggestion(usize),
    Popup(usize),
}

struct Popup {
    items: Vec<Action>,
    selected: usize,
    position: (u16, u16),
}

struct PopupLayout {
    left: u16,
    top: u16,
    width: u16,
    first: usize,
    count: usize,
    border: u16,
}

impl Popup {
    fn bounds(&self, size: (u16, u16), vi: bool) -> PopupLayout {
        let width = (self
            .items
            .iter()
            .map(|action| action.label(vi).width())
            .max()
            .unwrap_or(0)
            + 5)
        .min(usize::from(size.0.saturating_sub(1))) as u16;
        let border = u16::from(width >= 5 && size.1 >= 3);
        let count = self.items.len().min(usize::from(size.1 - 2 * border));
        let first = self.selected.saturating_sub(count.saturating_sub(1));
        let left = self.position.0.min(size.0.saturating_sub(width + 1));
        let top = self.position.1.min(size.1 - count as u16 - 2 * border);
        PopupLayout {
            left,
            top,
            width,
            first,
            count,
            border,
        }
    }

    fn hit(&self, size: (u16, u16), vi: bool, column: u16, row: u16) -> Option<usize> {
        let PopupLayout {
            left,
            top,
            width,
            first,
            count,
            border,
        } = self.bounds(size, vi);
        let start_row = top + border;
        (width >= 2
            && (left + border..left + width - border).contains(&column)
            && (start_row..start_row + count as u16).contains(&row))
        .then(|| first + usize::from(row - start_row))
    }

    fn step(&mut self, offset: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(offset)
            .min(self.items.len() - 1);
    }

    fn draw(&self, frame: &mut Frame, vi: bool) -> bool {
        let PopupLayout {
            left,
            top,
            width,
            first,
            count,
            border,
        } = self.bounds(frame.size, vi);
        if width < 2 || count == 0 {
            return false;
        }
        frame.cursor = None;
        let edge = Tone::Palette(PalettePart::ContextEdge);
        let border = usize::from(border);
        let inner = usize::from(width) - 2 * border;
        let marker = if inner >= 4 { 2 } else { 1 };
        for offset in 0..count + 2 * border {
            let row = &mut frame.rows[usize::from(top) + offset];
            let mut overlay = Line::default();
            match offset {
                0 if border != 0 => overlay.push(edge, format!("╭{}╮", "─".repeat(inner))),
                end if border != 0 && end == count + 1 => {
                    overlay.push(edge, format!("╰{}╯", "─".repeat(inner)));
                }
                _ => {
                    let index = first + offset - border;
                    let selected = index == self.selected;
                    let tone = if selected {
                        Tone::Palette(PalettePart::ContextSelected)
                    } else {
                        Tone::Palette(PalettePart::ContextItem)
                    };
                    let label = fit(self.items[index].label(vi), inner - marker);
                    if border != 0 {
                        overlay.push(edge, "│");
                    }
                    let prefix = match (selected, marker) {
                        (true, 2) => "› ",
                        (true, _) => "›",
                        (false, 2) => "  ",
                        (false, _) => " ",
                    };
                    overlay.push(tone, prefix);
                    overlay.push(tone, label);
                    overlay.push(tone, " ".repeat(inner - marker - label.width()));
                    if border != 0 {
                        overlay.push(edge, "│");
                    }
                }
            }
            overlay_line(row, usize::from(left), usize::from(width), &overlay);
        }
        true
    }
}

fn overlay_line(row: &mut Line, left: usize, width: usize, overlay: &Line) {
    let right = left + width;
    let mut column = 0;
    let mut prefix = Line::default();
    let mut suffix = Line::default();
    let mut prefix_width = 0;
    for (tone, segment) in row.segments() {
        for symbol in segment.graphemes(true) {
            let next = column + symbol.width();
            if next <= left {
                prefix.push(tone, symbol);
                prefix_width = next;
            } else if column >= right {
                suffix.push(tone, symbol);
            } else if next > right {
                suffix.push(Tone::Plain, " ".repeat(next - right));
            }
            column = next;
        }
    }
    prefix.push(Tone::Plain, " ".repeat(left - prefix_width));
    for (tone, segment) in overlay.segments().chain(suffix.segments()) {
        prefix.push(tone, segment);
    }
    *row = prefix;
}

fn fit(value: &str, width: usize) -> &str {
    let mut used = 0;
    for (offset, grapheme) in value.grapheme_indices(true) {
        used += grapheme.width();
        if used > width {
            return &value[..offset];
        }
    }
    value
}

#[derive(Default)]
pub(super) struct Interaction {
    visible: Visible,
    selection: Option<Selection>,
    line: Option<usize>,
    press: Option<Press>,
    popup: Option<Popup>,
    feedback: Option<Feedback>,
    pub(super) read_focus: bool,
    last_cursor: Option<(u16, u16)>,
}

#[derive(Clone, Copy)]
enum Feedback {
    Notice(Localized),
    Clipboard(&'static str),
}

impl Interaction {
    pub(super) fn reset_output(&mut self) {
        *self = Self::default();
    }

    pub(super) fn invalidate_output_selection(&mut self) {
        self.selection = None;
        self.line = None;
        if matches!(self.press, Some(Press::Read)) {
            self.press = None;
        }
    }

    pub(super) fn decorate(&mut self, frame: &mut Frame, vi: bool) {
        self.last_cursor = frame.cursor;
        self.visible = Visible::capture(frame);
        if let Some(selection) = self.selection.filter(|selection| selection.active()) {
            self.visible.decorate(frame, selection);
        }
        if self.read_focus && frame.input.is_some() {
            frame.cursor = None;
        }
        if let Some(feedback) = self.feedback
            && let Some(row) = frame.feedback_row
        {
            let message = match feedback {
                Feedback::Notice(label) => std::borrow::Cow::Borrowed(text(label, vi)),
                Feedback::Clipboard(code) => std::borrow::Cow::Owned(format!(
                    "{code} · {}",
                    text(labels::CLIPBOARD_SHORTCUT, vi)
                )),
            };
            let mut line = Line::default();
            line.push(
                Tone::Warning,
                fit(&message, usize::from(frame.size.0.saturating_sub(1))),
            );
            frame.rows[usize::from(row)] = line;
        }
        if let Some(popup) = &self.popup
            && !popup.draw(frame, vi)
        {
            self.popup = None;
            self.press = None;
        }
        if frame.cursor.is_none() {
            frame.input_border = None;
        }
    }
}

pub(super) fn clipboard_safe_output(value: &str) -> bool {
    // Inspect the full source before allowing selection, so a partial token
    // or one of its wrapped rows cannot evade the check by losing its label.
    let lower = value.to_ascii_lowercase();
    !lower.contains("-----begin")
        && !lower.contains("bearer ")
        && !lower.contains("password")
        && !lower.contains("token")
        && !lower.contains("secret")
        && !lower.contains("github_pat_")
        && !lower.contains("ghp_")
        && !lower.contains("ghs_")
        && !value.split_whitespace().any(|word| {
            word.split_once("://").is_some_and(|(_, rest)| {
                rest.split('/')
                    .next()
                    .is_some_and(|host| host.contains('@'))
                    || rest.contains('?')
            })
        })
}

impl Menu {
    fn editor_focus(&self) -> Option<Editor> {
        if self
            .output
            .as_ref()
            .is_some_and(|output| output.find.is_some())
        {
            return Some(Editor::OutputSearch);
        }
        if self.interaction.read_focus {
            return None;
        }
        if self.prompt.is_some() {
            Some(Editor::Prompt)
        } else if self.search.is_some() {
            Some(Editor::MenuSearch)
        } else {
            None
        }
    }

    fn editor_mut(&mut self, editor: Editor) -> Option<&mut Input> {
        match editor {
            Editor::Prompt => self.prompt.as_mut().map(|prompt| &mut prompt.input),
            Editor::MenuSearch => self.search.as_mut(),
            Editor::OutputSearch => self.output.as_mut()?.find.as_mut(),
        }
    }

    fn editor_changed(&mut self, editor: Editor) {
        match editor {
            Editor::Prompt => {
                if let Some(prompt) = &mut self.prompt {
                    prompt.changed();
                }
                self.history.reset_navigation();
            }
            Editor::MenuSearch => self.refilter(),
            Editor::OutputSearch => {
                if let Some(output) = &mut self.output
                    && let Some(find) = &output.find
                {
                    output.query.clone_from(&find.value);
                    output.hit = None;
                    output.search(false);
                }
            }
        }
    }

    pub(super) fn paste_editor(&mut self, value: &str) -> Effect {
        if self.interaction.popup.is_some() {
            return Effect::Idle;
        }
        let Some(editor) = self.editor_focus() else {
            return Effect::Idle;
        };
        let Some(input) = self.editor_mut(editor) else {
            return Effect::Idle;
        };
        match input.paste_sanitized(value, editor.limit()) {
            Ok(true) => {
                self.editor_changed(editor);
                Effect::Redraw
            }
            Ok(false) => Effect::Idle,
            Err(_) => {
                if editor == Editor::Prompt
                    && let Some(prompt) = &mut self.prompt
                {
                    prompt.error = Some(COMMAND_TOO_LONG);
                } else {
                    self.interaction.feedback = Some(Feedback::Notice(labels::SEARCH_TOO_LONG));
                }
                Effect::Redraw
            }
        }
    }

    fn clipboard_error(&mut self, error: ClipboardError) -> Effect {
        self.interaction.feedback = Some(Feedback::Clipboard(error.code()));
        Effect::Redraw
    }

    fn write_clipboard(&mut self, clipboard: &mut impl Clipboard, value: &str) -> bool {
        if value.is_empty() {
            return false;
        }
        if let Err(error) = clipboard.set_text(value) {
            self.clipboard_error(error);
            return false;
        }
        self.interaction.feedback = Some(Feedback::Notice(labels::COPIED));
        true
    }

    fn clipboard_action(&mut self, action: Action, clipboard: &mut impl Clipboard) -> Effect {
        if let Some(editor) = self.editor_focus() {
            if action == Action::Paste {
                return match clipboard.get_text() {
                    Ok(value) => self.paste_editor(&value),
                    Err(error) => self.clipboard_error(error),
                };
            }
            let Some(input) = self.editor_mut(editor) else {
                return Effect::Idle;
            };
            if matches!(action, Action::Copy | Action::Cut) {
                let Some(value) = input.selected_text().map(str::to_owned) else {
                    return Effect::Idle;
                };
                if !clipboard_safe_output(&input.value) {
                    self.interaction.feedback = Some(Feedback::Notice(labels::SENSITIVE_TEXT));
                    return Effect::Redraw;
                }
                if self.write_clipboard(clipboard, &value)
                    && action == Action::Cut
                    && self.editor_mut(editor).is_some_and(Input::delete_selection)
                {
                    self.editor_changed(editor);
                }
                return Effect::Redraw;
            }
            let changed = match action {
                Action::Delete => input.delete_selection(),
                Action::SelectAll => {
                    return if input.select_all() {
                        Effect::Redraw
                    } else {
                        Effect::Idle
                    };
                }
                Action::Undo => input.undo(),
                Action::Redo => input.redo(),
                _ => false,
            };
            return if changed {
                self.editor_changed(editor);
                Effect::Redraw
            } else {
                Effect::Idle
            };
        }

        if self.interaction.visible.lines.is_empty() {
            return Effect::Idle;
        }
        let visible = &self.interaction.visible;
        let value = match action {
            Action::Copy => self
                .interaction
                .selection
                .filter(|selection| selection.active())
                .and_then(|selection| {
                    self.output
                        .as_ref()?
                        .buffer
                        .copy(selection.bytes())
                        .map(str::to_owned)
                }),
            Action::CopyLine => visible.copy_line(self.interaction.line).map(str::to_owned),
            Action::SelectAll => {
                self.interaction.selection = visible.all();
                return Effect::Redraw;
            }
            _ => None,
        };
        if let Some(value) = value {
            self.write_clipboard(clipboard, &value);
            Effect::Redraw
        } else {
            Effect::Idle
        }
    }

    fn open_context_menu(&mut self, position: Option<(u16, u16)>) -> Effect {
        let mut items = Vec::new();
        if let Some(input) = self
            .editor_focus()
            .and_then(|editor| self.editor_mut(editor))
        {
            if input.selection().is_some() {
                items.extend([Action::Copy, Action::Cut, Action::Paste, Action::Delete]);
            } else {
                items.extend([Action::Paste, Action::SelectAll]);
                if input.can_undo() {
                    items.push(Action::Undo);
                }
                if input.can_redo() {
                    items.push(Action::Redo);
                }
            }
        } else if !self.interaction.visible.lines.is_empty() {
            if self.interaction.selection.is_some_and(Selection::active) {
                items.push(Action::Copy);
            }
            items.push(Action::CopyLine);
            items.push(Action::SelectAll);
        }
        if items.is_empty() {
            return Effect::Idle;
        }
        self.interaction.press = None;
        self.interaction.popup = Some(Popup {
            items,
            selected: 0,
            position: position.or(self.interaction.last_cursor).unwrap_or((2, 2)),
        });
        Effect::Redraw
    }

    fn choose_context_action(&mut self, index: usize, clipboard: &mut impl Clipboard) -> Effect {
        if let Some(action) = self
            .interaction
            .popup
            .take()
            .and_then(|popup| popup.items.get(index).copied())
        {
            self.clipboard_action(action, clipboard);
        }
        Effect::Redraw
    }

    pub(super) fn interaction_key(
        &mut self,
        key: KeyEvent,
        clipboard: &mut impl Clipboard,
    ) -> Option<Effect> {
        self.interaction.feedback = None;
        if let Some(popup) = &mut self.interaction.popup {
            return Some(match key.code {
                KeyCode::Esc => {
                    self.interaction.popup = None;
                    Effect::Redraw
                }
                KeyCode::Up => {
                    popup.step(-1);
                    Effect::Redraw
                }
                KeyCode::Down => {
                    popup.step(1);
                    Effect::Redraw
                }
                KeyCode::Enter => {
                    let index = popup.selected;
                    self.choose_context_action(index, clipboard)
                }
                _ => Effect::Idle,
            });
        }
        if key.code == KeyCode::F(10) && key.modifiers == KeyModifiers::SHIFT
            || key.code == KeyCode::Menu
        {
            return Some(self.open_context_menu(None));
        }
        let editor = self.editor_focus();
        if key.code == KeyCode::F(6)
            && self.output.is_some()
            && self
                .output
                .as_ref()
                .is_some_and(|output| output.find.is_none())
        {
            self.interaction.read_focus = !self.interaction.read_focus;
            self.interaction.selection = None;
            return Some(Effect::Redraw);
        }
        if key.code == KeyCode::Esc {
            if editor
                .and_then(|editor| self.editor_mut(editor))
                .is_some_and(Input::clear_selection)
            {
                return Some(Effect::Redraw);
            }
            if self
                .interaction
                .selection
                .take()
                .is_some_and(Selection::active)
            {
                return Some(Effect::Redraw);
            }
        }
        let selection = if let Some(editor) = editor {
            self.editor_mut(editor)
                .is_some_and(|input| input.selection().is_some())
        } else {
            self.interaction.selection.is_some_and(Selection::active)
        };
        let action = if control(key, 'c') && selection {
            Action::Copy
        } else if control(key, 'v') {
            Action::Paste
        } else if control(key, 'x') {
            Action::Cut
        } else if control(key, 'a') && editor.is_none() {
            Action::SelectAll
        } else {
            return None;
        };
        Some(self.clipboard_action(action, clipboard))
    }

    fn wheel(&mut self, lines: isize) -> Effect {
        self.interaction.selection = None;
        self.interaction.press = None;
        self.output
            .as_mut()
            .map_or(Effect::Idle, |output| output.scroll_by(lines))
    }

    fn move_input(&mut self, editor: Editor, hit: &InputHit, column: u16, extend: bool) -> bool {
        let column = column
            .clamp(hit.columns.start, hit.columns.end)
            .saturating_sub(hit.columns.start);
        let Some(input) = self.editor_mut(editor) else {
            return false;
        };
        let offset = input.offset_at_column(hit.offset, usize::from(column));
        let changed = input.move_to(offset, extend);
        if changed && editor == Editor::Prompt {
            self.editor_changed(editor);
        }
        changed
    }

    fn focus_input_at(&mut self, frame: &Frame, column: u16, row: u16) -> Option<Editor> {
        if frame
            .input
            .as_ref()
            .is_some_and(|hit| hit.row == row && hit.columns.contains(&column))
        {
            self.interaction.read_focus = false;
            return self.editor_focus();
        }
        None
    }

    pub(super) fn mouse(
        &mut self,
        event: MouseEvent,
        frame: Option<&Frame>,
        clipboard: &mut impl Clipboard,
    ) -> Effect {
        self.interaction.feedback = None;
        let MouseEvent {
            kind, column, row, ..
        } = event;
        if let Some(popup) = &mut self.interaction.popup {
            let hit = frame.and_then(|frame| popup.hit(frame.size, self.vi, column, row));
            return match kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(index) = hit {
                        popup.selected = index;
                        self.interaction.press = Some(Press::Popup(index));
                    } else {
                        self.interaction.popup = None;
                        self.interaction.press = None;
                    }
                    Effect::Redraw
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    if let Some(Press::Popup(index)) = self.interaction.press.take()
                        && hit == Some(index)
                    {
                        self.choose_context_action(index, clipboard)
                    } else {
                        Effect::Idle
                    }
                }
                MouseEventKind::Drag(MouseButton::Left) => {
                    self.interaction.press = None;
                    Effect::Idle
                }
                MouseEventKind::Moved if hit.is_some_and(|index| index != popup.selected) => {
                    popup.selected = hit.expect("checked popup hit");
                    Effect::Redraw
                }
                _ => Effect::Idle,
            };
        }
        match kind {
            MouseEventKind::ScrollUp => return self.wheel(-3),
            MouseEventKind::ScrollDown => return self.wheel(3),
            _ => {}
        }
        let Some(frame) = frame else {
            self.interaction.press = None;
            return Effect::Idle;
        };
        let suggestion = frame.suggestion_hit(column, row);
        match kind {
            MouseEventKind::Moved if suggestion.is_some() => {
                self.focus_completion(suggestion.expect("checked suggestion"))
            }
            MouseEventKind::Down(MouseButton::Right) => {
                let editor = self.focus_input_at(frame, column, row);
                if editor.is_none() {
                    let Some(line) = self.interaction.visible.row(row, false) else {
                        return Effect::Idle;
                    };
                    self.interaction.read_focus = true;
                    self.interaction.line = Some(line.source.bytes.start);
                }
                self.open_context_menu(Some((column, row)))
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.interaction.press = None;
                self.interaction.selection = None;
                if let Some(index) = suggestion
                    && self
                        .prompt
                        .as_ref()
                        .is_some_and(|prompt| !prompt.edited && prompt.error.is_none())
                {
                    self.interaction.press = Some(Press::Suggestion(index));
                    return self.focus_completion(index);
                }
                if let Some(editor) = self.focus_input_at(frame, column, row)
                    && let Some(hit) = &frame.input
                {
                    self.move_input(editor, hit, column, false);
                    self.interaction.press = Some(Press::Input(editor));
                    return Effect::Redraw;
                }
                if let Some(point) = self.interaction.visible.point(column, row, false) {
                    self.interaction.read_focus = true;
                    self.interaction.line = self
                        .interaction
                        .visible
                        .row(row, false)
                        .map(|line| line.source.bytes.start);
                    self.interaction.selection = Some(Selection {
                        anchor: point,
                        end: point,
                    });
                    self.interaction.press = Some(Press::Read);
                    return Effect::Redraw;
                }
                if let Some(index) = frame.menu_hit(column, row) {
                    self.interaction.press = Some(Press::Menu(index));
                }
                Effect::Redraw
            }
            MouseEventKind::Drag(MouseButton::Left) => match self.interaction.press {
                Some(Press::Input(editor)) if self.editor_focus() == Some(editor) => {
                    let changed = frame
                        .input
                        .as_ref()
                        .is_some_and(|hit| self.move_input(editor, hit, column, true));
                    if changed {
                        Effect::Redraw
                    } else {
                        Effect::Idle
                    }
                }
                Some(Press::Read) => self.extend_output(column, row),
                _ => {
                    self.interaction.press = None;
                    Effect::Idle
                }
            },
            MouseEventKind::Up(MouseButton::Left) => match self.interaction.press.take() {
                Some(Press::Suggestion(index)) if suggestion == Some(index) => {
                    self.accept_completion(index)
                }
                Some(Press::Menu(index))
                    if frame.menu_hit(column, row) == Some(index) && index < self.items.len() =>
                {
                    self.close_main_prompt();
                    self.select(index);
                    self.activate()
                }
                Some(Press::Read) => self.extend_output(column, row),
                Some(Press::Input(editor))
                    if self.editor_focus() == Some(editor)
                        && self
                            .editor_mut(editor)
                            .is_some_and(|input| input.selection().is_some()) =>
                {
                    if let Some(hit) = &frame.input {
                        self.move_input(editor, hit, column, true);
                    }
                    Effect::Redraw
                }
                _ => Effect::Idle,
            },
            _ => Effect::Idle,
        }
    }

    fn extend_output(&mut self, column: u16, row: u16) -> Effect {
        if let Some(selection) = &mut self.interaction.selection
            && let Some(point) = self.interaction.visible.point(column, row, true)
        {
            selection.end = point;
            Effect::Redraw
        } else {
            Effect::Idle
        }
    }

    pub(super) fn focus_lost(&mut self) -> Effect {
        self.interaction.popup = None;
        self.interaction.selection = None;
        self.interaction.press = None;
        if let Some(editor) = self.editor_focus()
            && let Some(input) = self.editor_mut(editor)
        {
            input.clear_selection();
        }
        Effect::Redraw
    }
}

#[cfg(test)]
mod popup_tests {
    use super::*;

    #[derive(Default)]
    struct TestClipboard(String);

    impl Clipboard for TestClipboard {
        fn get_text(&mut self) -> Result<String, ClipboardError> {
            Ok(self.0.clone())
        }

        fn set_text(&mut self, text: &str) -> Result<(), ClipboardError> {
            self.0 = text.to_owned();
            Ok(())
        }
    }

    #[test]
    fn mouse_selection_copies_original_graphemes_after_reflow() {
        use crate::tui::{view, viewport::Buffer};

        let mut menu = Menu::new(true);
        menu.begin_output(&["env".into()]);
        let output = menu.output.as_mut().unwrap();
        output.buffer = Buffer::from_streams("đầu 🧑‍💻 cuối dòng\nnext\n".into(), "", None);
        output.running = false;
        let original = view::frame(&mut menu, (40, 10), false, true);
        let row = original.readable.as_ref().unwrap().first;
        let mut clipboard = TestClipboard::default();
        for (kind, column) in [
            (MouseEventKind::Down(MouseButton::Left), 2),
            (MouseEventKind::Up(MouseButton::Left), 8),
        ] {
            menu.mouse(
                MouseEvent {
                    kind,
                    column,
                    row,
                    modifiers: KeyModifiers::NONE,
                },
                Some(&original),
                &mut clipboard,
            );
        }
        let compact = view::frame(&mut menu, (6, 6), false, true);
        assert_eq!(
            compact
                .rows
                .iter()
                .filter(|line| line.has(Tone::Selection))
                .count(),
            2
        );
        menu.clipboard_action(Action::Copy, &mut clipboard);
        assert_eq!(clipboard.0, "đầu 🧑‍💻");
    }

    #[test]
    fn popup_clamps_to_terminal_and_never_hits_its_border() {
        let popup = Popup {
            items: vec![Action::Copy, Action::Paste],
            selected: 0,
            position: (58, 18),
        };
        let mut frame = Frame::new((60, 20));
        let PopupLayout {
            left,
            top,
            width,
            count,
            ..
        } = popup.bounds(frame.size, true);
        assert_eq!(count, 2);
        assert!(left + width < frame.size.0);
        assert!(top + 3 < frame.size.1);
        assert_eq!(popup.hit(frame.size, true, left + 1, top), None);
        assert_eq!(popup.hit(frame.size, true, left, top + 1), None);
        assert_eq!(popup.hit(frame.size, true, left + 1, top + 1), Some(0));
        assert_eq!(popup.hit((2, 2), true, 0, 0), None);
        popup.draw(&mut frame, true);
        assert!(
            frame.rows[usize::from(top + 1)]
                .contents()
                .contains("Sao chép")
        );

        let mut compact = Frame::new((6, 2));
        assert!(popup.draw(&mut compact, true));
        assert_eq!(popup.hit(compact.size, true, 0, 0), Some(0));
        assert_eq!(popup.hit(compact.size, true, 0, 1), Some(1));
        assert!(
            compact
                .rows
                .iter()
                .all(|row| !row.has(Tone::Palette(PalettePart::ContextEdge)))
        );
    }
}
