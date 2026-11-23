mod banner;
mod clipboard;
pub(crate) mod completion;
mod editor;
mod highlight;
mod interaction;
mod runner;
mod terminal;
mod view;
mod viewport;

use self::{
    banner::Banner,
    clipboard::SystemClipboard,
    completion::{Completer, Kind, Source, Suggestion},
    editor::Input,
    interaction::Interaction,
    terminal::{Painter, Screen, Tone},
    viewport::Buffer,
};
use crate::workspace::Workspace;
use clap::ArgMatches;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal as console,
};
use std::io::IsTerminal;
use std::{
    io,
    ops::Range,
    time::{Duration, Instant},
};

use crate::cli::catalog::{ENTRIES, Entry, Localized, text};
use crate::localization;

const COMMAND_LIMIT: usize = 8192;
const SEARCH_LIMIT: usize = 256;
const HISTORY_LIMIT: usize = 100;
const SUGGESTION_LIMIT: usize = 7;
const COMMAND_TOO_LONG: Localized = ["Command exceeds 8 KiB.", "Lệnh vượt quá 8 KiB."];
const PASTE_NEWLINE_NOTICE: Localized = [
    "Input received. Press Enter to run.",
    "Đã nhận nội dung. Nhấn Enter để chạy.",
];

fn text_modifiers(key: KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
}

fn control(key: KeyEvent, ch: char) -> bool {
    key.code == KeyCode::Char(ch) && key.modifiers == KeyModifiers::CONTROL
}

fn printable(key: KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(ch) if !ch.is_control() && text_modifiers(key) => Some(ch),
        _ => None,
    }
}

#[derive(Default)]
struct Prompt {
    input: Input,
    error: Option<Localized>,
    completions: Vec<Suggestion>,
    edited: bool,
    accepted: Option<(usize, Input)>,
    focus: usize,
}

impl Prompt {
    fn changed(&mut self) {
        self.error = None;
        self.edited = true;
        self.accepted = None;
        self.focus = 0;
    }
}

struct OutputView {
    title: Option<String>,
    buffer: Buffer,
    scroll: usize,
    last_page: usize,
    page_size: usize,
    width: usize,
    follow: bool,
    started_at: Instant,
    running: bool,
    status: String,
    activity: Option<usize>,
    result: Option<runner::RunOutput>,
    tone: Tone,
    find: Option<Input>,
    query: String,
    hit: Option<Range<usize>>,
}

impl OutputView {
    fn new(title: String, vi: bool) -> Self {
        Self {
            title: Some(title),
            buffer: Buffer::default(),
            scroll: 0,
            last_page: 0,
            page_size: 0,
            width: 0,
            follow: false,
            started_at: Instant::now(),
            running: true,
            status: format!("{}…", text(localization::RUNNING, vi)),
            activity: None,
            result: None,
            tone: Tone::Accent,
            find: None,
            query: String::new(),
            hit: None,
        }
    }

    fn layout(&mut self, width: usize, height: usize) {
        let anchor = self.buffer.row_start(self.scroll);
        self.page_size = height;
        self.width = width;
        self.last_page = self.buffer.layout(width).saturating_sub(height.max(1));
        self.scroll = if self.follow {
            self.last_page
        } else {
            anchor
                .map_or(self.scroll, |offset| self.buffer.row_at(offset))
                .min(self.last_page)
        };
    }

    fn scroll_to(&mut self, row: usize, follow: bool) -> Effect {
        let row = row.min(self.last_page);
        self.follow = follow && row == self.last_page;
        if row == self.scroll {
            return Effect::Idle;
        }
        self.scroll = row;
        Effect::Redraw
    }

    fn scroll_by(&mut self, lines: isize) -> Effect {
        self.scroll_to(self.scroll.saturating_add_signed(lines), lines > 0)
    }

    fn search(&mut self, backwards: bool) {
        self.hit = self.buffer.find(&self.query, self.hit.as_ref(), backwards);
        if let Some(hit) = &self.hit {
            self.scroll_to(self.buffer.row_at(hit.start), false);
        }
    }

    fn animate(&mut self) -> bool {
        let phase = (self.started_at.elapsed().as_millis() / 125 % 8) as usize;
        self.activity.replace(phase) != Some(phase)
    }

    fn status_line(&self) -> String {
        const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
        self.activity.map_or_else(
            || self.status.clone(),
            |phase| format!("{} {}", SPINNER[phase], self.status),
        )
    }

    fn finish(&mut self, result: Result<runner::RunOutput, String>, vi: bool) {
        self.running = false;
        self.activity = None;
        self.result = Some(match result {
            Ok(result) => result,
            Err(error) => {
                self.buffer =
                    Buffer::from_streams(String::new(), &runner::sanitize(error.as_bytes()), None);
                runner::RunOutput {
                    outcome: runner::Outcome::Fail,
                    exit_code: None,
                    elapsed_ms: self.started_at.elapsed().as_millis(),
                }
            }
        });
        self.localize_status(vi);
    }

    fn localize_status(&mut self, vi: bool) {
        let Some(result) = &self.result else {
            self.status = format!("{}…", text(localization::RUNNING, vi));
            return;
        };
        let (symbol, tone, label) = match result.outcome {
            runner::Outcome::Pass => ("✓", Tone::Success, localization::SUCCEEDED),
            runner::Outcome::Fail => ("✗", Tone::Error, localization::FAILED),
            runner::Outcome::Cancelled => ("!", Tone::Warning, localization::CANCELLED),
        };
        let code = result
            .exit_code
            .map_or_else(|| "?".into(), |code| code.to_string());
        self.tone = tone;
        self.status = format!(
            "{symbol} {} · {} {code} · {} {} ms",
            text(label, vi),
            text(localization::EXIT, vi),
            text(localization::DURATION, vi),
            result.elapsed_ms,
        );
    }
}

enum Effect {
    Idle,
    Redraw,
    Run(Vec<String>),
    Native(String),
    Cancel,
    Exit,
}

struct Menu {
    vi: bool,
    items: Vec<&'static Entry>,
    selected: usize,
    first: usize,
    search: Option<Input>,
    prompt: Option<Prompt>,
    help: bool,
    output: Option<OutputView>,
    history: Vec<String>,
    history_index: Option<usize>,
    history_draft: Input,
    completer: Completer,
    banner: Banner,
    interaction: Interaction,
}

impl Menu {
    fn new(vi: bool) -> Self {
        Self {
            vi,
            items: ENTRIES.iter().collect(),
            selected: 0,
            first: 0,
            search: None,
            prompt: None,
            help: false,
            output: None,
            history: Vec::new(),
            history_index: None,
            history_draft: Input::default(),
            completer: Completer::new(vi),
            banner: Banner::default(),
            interaction: Interaction::default(),
        }
    }

    fn refilter(&mut self) {
        let query = self
            .search
            .as_ref()
            .map_or_else(String::new, |input| input.value.to_lowercase());
        self.items = ENTRIES
            .iter()
            .filter(|entry| {
                text(entry.label, self.vi).to_lowercase().contains(&query)
                    || text(entry.detail, self.vi).to_lowercase().contains(&query)
                    || entry.command.contains(&query)
                    || entry
                        .command
                        .split_whitespace()
                        .next()
                        .and_then(|name| completion::grammar().find_subcommand(name))
                        .is_some_and(|command| {
                            command
                                .get_subcommands()
                                .any(|child| child.get_name().contains(&query))
                        })
            })
            .collect();
        self.selected = 0;
        self.first = 0;
    }

    fn reveal_selection(&mut self, visible: usize) {
        self.first = self
            .first
            .min(self.items.len().saturating_sub(visible))
            .clamp(
                self.selected.saturating_sub(visible.saturating_sub(1)),
                self.selected,
            );
    }

    fn select(&mut self, index: usize) -> Effect {
        let index = index.min(self.items.len().saturating_sub(1));
        let effect = if index == self.selected {
            Effect::Idle
        } else {
            Effect::Redraw
        };
        self.selected = index;
        effect
    }

    fn activate(&mut self) -> Effect {
        let Some(entry) = self.items.get(self.selected) else {
            return Effect::Idle;
        };
        if entry.command.is_empty() {
            return Effect::Exit;
        }
        let input = Input::at_end(format!("{} ", entry.command));
        self.open_prompt(input);
        Effect::Redraw
    }

    fn open_prompt(&mut self, input: Input) {
        self.prompt = Some(Prompt {
            input,
            edited: true,
            ..Prompt::default()
        });
        self.help = false;
        self.history_index = None;
        self.history_draft = Input::default();
    }

    fn begin_output(&mut self, args: &[String]) {
        self.interaction.reset_output();
        self.output = Some(OutputView::new(output_title(args), self.vi));
        self.open_prompt(Input::default());
    }

    fn close_main_prompt(&mut self) -> bool {
        let closed = self.prompt.take().is_some();
        self.history_index = None;
        self.history_draft = Input::default();
        closed
    }

    fn switch_language(&mut self, vi: bool) {
        let selected = self.items.get(self.selected).copied();
        if let Some(output) = &mut self.output {
            let transient = [["Cancelling…", "Đang hủy…"], runner::CANNOT_CANCEL]
                .into_iter()
                .find(|status| output.status == text(*status, self.vi));
            if let Some(status) = transient {
                output.status = text(status, vi).into();
            } else {
                output.localize_status(vi);
            }
        }
        self.vi = vi;
        localization::set_language(vi);
        self.completer = Completer::new(vi);
        self.refilter();
        if let Some(index) = self
            .items
            .iter()
            .position(|entry| selected.is_some_and(|selected| std::ptr::eq(*entry, selected)))
        {
            self.selected = index;
        }
        self.open_prompt(Input::default());
    }

    fn refresh_suggestions(&mut self) {
        let Some(prompt) = self.prompt.as_mut().filter(|prompt| prompt.edited) else {
            return;
        };
        prompt.edited = false;
        let input = &prompt.input;
        prompt.completions = if input.value.is_empty() {
            self.output
                .is_none()
                .then(|| self.items.get(self.selected))
                .flatten()
                .filter(|entry| !entry.command.is_empty())
                .map(|entry| Suggestion {
                    text: entry.command.into(),
                    insert_range: 0..0,
                    label: entry.command.into(),
                    kind: Kind::Command,
                    description: text(entry.detail, self.vi).into(),
                    source: Source::Menu,
                })
                .into_iter()
                .collect()
        } else {
            self.completer.suggestions(&input.value, input.cursor)
        };
        if self.output.is_none()
            && input.cursor == input.value.len()
            && let Some(entry) = self.items.get(self.selected)
            && input.value.trim_end() == entry.command
            && let Some(recommendation) = entry.recommendation
        {
            let separator = if input.value.ends_with(char::is_whitespace) {
                ""
            } else {
                " "
            };
            prompt
                .completions
                .retain(|item| item.text != recommendation);
            prompt.completions.insert(
                0,
                Suggestion {
                    text: format!("{separator}{recommendation}"),
                    insert_range: input.cursor..input.cursor,
                    label: recommendation.into(),
                    kind: Kind::Option,
                    description: text(["Recommended action", "Thao tác gợi ý"], self.vi).into(),
                    source: Source::Menu,
                },
            );
            prompt.completions.truncate(SUGGESTION_LIMIT);
        }
        if input.cursor == input.value.len()
            && !input.value.is_empty()
            && !input.value.contains("-- ")
        {
            // The completer only falls back to fuzzy matches if no exact prefix
            // matches exist. Safe history takes precedence over those matches.
            let fuzzy = if prompt.completions.iter().all(|item| {
                !item
                    .text
                    .starts_with(&input.value[item.insert_range.start..input.cursor])
            }) {
                std::mem::take(&mut prompt.completions)
            } else {
                Vec::new()
            };
            for entry in self
                .history
                .iter()
                .rev()
                .filter(|entry| entry.starts_with(&input.value) && *entry != &input.value)
            {
                if prompt.completions.len() >= SUGGESTION_LIMIT {
                    break;
                }
                if prompt.completions.iter().any(|suggestion| {
                    let range = &suggestion.insert_range;
                    entry.len() == input.value.len() - range.len() + suggestion.text.len()
                        && entry.starts_with(&input.value[..range.start])
                        && entry.ends_with(&input.value[range.end..])
                        && entry[range.start..].starts_with(&suggestion.text)
                }) {
                    continue;
                }
                prompt.completions.push(Suggestion {
                    text: entry.clone(),
                    insert_range: 0..input.value.len(),
                    label: entry.clone(),
                    kind: Kind::History,
                    description: text(["Recent command", "Lệnh gần đây"], self.vi).into(),
                    source: Source::History,
                });
            }
            for suggestion in fuzzy {
                if prompt.completions.len() == SUGGESTION_LIMIT {
                    break;
                }
                if !prompt.completions.iter().any(|existing| {
                    existing.text == suggestion.text
                        && existing.insert_range == suggestion.insert_range
                }) {
                    prompt.completions.push(suggestion);
                }
            }
        }
    }

    fn complete(&mut self, backwards: bool) -> Effect {
        self.refresh_suggestions();
        let prompt = self.prompt.as_mut().expect("prompt is active");
        let count = prompt.completions.len();
        if count == 0 || prompt.error == Some(COMMAND_TOO_LONG) {
            return Effect::Idle;
        }
        let index = if let Some((index, _)) = &prompt.accepted {
            (index + if backwards { count - 1 } else { 1 }) % count
        } else if backwards && prompt.focus == 0 {
            count - 1
        } else {
            prompt.focus.min(count - 1)
        };
        self.accept_completion(index)
    }

    fn focus_completion(&mut self, index: usize) -> Effect {
        let Some(prompt) = self
            .prompt
            .as_mut()
            .filter(|prompt| !prompt.edited && prompt.error.is_none())
        else {
            return Effect::Idle;
        };
        if index >= prompt.completions.len() || prompt.focus == index {
            return Effect::Idle;
        }
        prompt.focus = index;
        Effect::Redraw
    }

    fn accept_completion(&mut self, index: usize) -> Effect {
        let Some(prompt) = self
            .prompt
            .as_mut()
            .filter(|prompt| !prompt.edited && prompt.error.is_none())
        else {
            return Effect::Idle;
        };
        let Some(completion) = prompt.completions.get(index) else {
            return Effect::Idle;
        };
        let base = prompt
            .accepted
            .as_ref()
            .map_or(&prompt.input, |(_, base)| base)
            .clone();
        let range = completion.insert_range.clone();
        if !prompt
            .input
            .replace_from(&base, range, &completion.text, COMMAND_LIMIT)
        {
            return Effect::Idle;
        }
        prompt.accepted = Some((index, base));
        prompt.focus = index;
        prompt.error = None;
        self.history_index = None;
        Effect::Redraw
    }

    fn history_move(&mut self, older: bool, prefix: bool) -> Effect {
        if self.history.is_empty() || (!older && self.history_index.is_none()) {
            return Effect::Idle;
        }
        let prompt = self.prompt.as_mut().expect("prompt is active");
        if self.history_index.is_none() {
            self.history_draft.clone_from(&prompt.input);
        }
        self.history_index = if older {
            let before = self.history_index.unwrap_or(self.history.len());
            let matches = |entry: &String| !prefix || entry.starts_with(&self.history_draft.value);
            let Some(index) = self.history[..before].iter().rposition(matches) else {
                return Effect::Idle;
            };
            Some(index)
        } else {
            self.history_index
                .and_then(|index| (index + 1 < self.history.len()).then_some(index + 1))
        };
        prompt.input = self.history_index.map_or_else(
            || self.history_draft.clone(),
            |index| Input::at_end(self.history[index].clone()),
        );
        prompt.changed();
        Effect::Redraw
    }

    fn submit_prompt(&mut self) -> Effect {
        let prompt = self.prompt.as_mut().expect("prompt is active");
        if prompt.input.value.trim().is_empty() || prompt.error == Some(COMMAND_TOO_LONG) {
            return Effect::Idle;
        }
        let first = prompt
            .input
            .value
            .split_whitespace()
            .next()
            .unwrap_or_default();
        if first != "cnl"
            && first != "cnl.exe"
            && first != "clear"
            && !first.starts_with('-')
            && completion::grammar().find_subcommand(first).is_none()
        {
            if self.output.as_ref().is_some_and(|output| output.running) {
                prompt.error = Some(["A command is still running.", "Một lệnh đang chạy."]);
                return Effect::Redraw;
            }
            let native = prompt.input.value.clone();
            self.close_main_prompt();
            return Effect::Native(native);
        }
        let Ok(mut args) = completion::arguments(&prompt.input.value) else {
            prompt.error = Some([
                "Close the quoted argument.",
                "Hãy đóng dấu nháy của đối số.",
            ]);
            return Effect::Redraw;
        };
        if args
            .first()
            .is_some_and(|word| matches!(word.as_str(), "cnl" | "cnl.exe"))
        {
            args.remove(0);
        }
        if let Some(vi) = crate::cli::local_terminal_language(&args) {
            self.switch_language(vi);
            return Effect::Redraw;
        }
        if self.output.as_ref().is_some_and(|output| output.running) {
            prompt.error = Some(["A command is still running.", "Một lệnh đang chạy."]);
            return Effect::Redraw;
        }
        if args.len() == 1 && args[0] == "clear" {
            if let Some(output) = &mut self.output {
                output.buffer = Buffer::default();
                output.scroll = 0;
                output.follow = false;
                output.hit = None;
            }
            self.open_prompt(Input::default());
            return Effect::Redraw;
        }
        if args.is_empty() || args[0] == "tui" {
            prompt.error = Some(["Enter a CIRNETLAB command.", "Nhập một lệnh CIRNETLAB."]);
            return Effect::Redraw;
        }
        if safe_history(&args) {
            let command = prompt.input.value.trim().to_owned();
            if self.history.last() != Some(&command) {
                self.history.push(command);
                if self.history.len() > HISTORY_LIMIT {
                    self.history.remove(0);
                }
            }
        }
        self.history_index = None;
        self.history_draft = Input::default();
        self.prompt = None;
        Effect::Run(args)
    }

    fn edit_prompt(&mut self, key: KeyEvent) -> Effect {
        match key.code {
            KeyCode::Enter if text_modifiers(key) => {
                return self.submit_prompt();
            }
            KeyCode::Tab | KeyCode::BackTab if text_modifiers(key) => {
                return self.complete(
                    key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT),
                );
            }
            KeyCode::Up | KeyCode::Down if text_modifiers(key) => {
                return self.history_move(key.code == KeyCode::Up, false);
            }
            _ if control(key, 'r') => return self.history_move(true, true),
            _ => {}
        }
        let prompt = self.prompt.as_mut().expect("prompt is active");
        if printable(key)
            .is_some_and(|ch| !prompt.input.fits_replacement(ch.len_utf8(), COMMAND_LIMIT))
        {
            prompt.error = Some(COMMAND_TOO_LONG);
            return Effect::Redraw;
        }
        let previous = prompt.input.value.clone();
        if !prompt.input.edit(key, COMMAND_LIMIT) {
            return Effect::Idle;
        }
        if prompt.input.value != previous || prompt.error != Some(COMMAND_TOO_LONG) {
            prompt.changed();
            self.history_index = None;
        }
        Effect::Redraw
    }

    fn handle_output(&mut self, key: KeyEvent) -> Effect {
        let output = self.output.as_mut().expect("output is active");
        if control(key, 'f') {
            self.interaction.read_focus = false;
            output
                .find
                .get_or_insert_with(|| Input::at_end(output.query.clone()));
            return Effect::Redraw;
        }
        if key.code == KeyCode::F(3) && text_modifiers(key) {
            output.search(key.modifiers.contains(KeyModifiers::SHIFT));
            return Effect::Redraw;
        }
        let page = output.page_size.clamp(1, 10) as isize;
        match key.code {
            KeyCode::PageUp => return output.scroll_by(-page),
            KeyCode::PageDown => return output.scroll_by(page),
            KeyCode::Home if key.modifiers == KeyModifiers::CONTROL => {
                return output.scroll_to(0, false);
            }
            KeyCode::End if key.modifiers == KeyModifiers::CONTROL => {
                return output.scroll_to(output.last_page, true);
            }
            _ => {}
        }
        if let Some(find) = &mut output.find {
            return match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    output.find = None;
                    Effect::Redraw
                }
                _ if find.edit(key, SEARCH_LIMIT) => {
                    output.query.clone_from(&find.value);
                    output.hit = None;
                    output.search(false);
                    Effect::Redraw
                }
                _ => Effect::Idle,
            };
        }
        if key.code == KeyCode::Esc {
            if output.running {
                return self.cancel_output();
            }
            self.output = None;
            self.interaction.reset_output();
            self.close_main_prompt();
            return Effect::Redraw;
        }
        if self.interaction.read_focus {
            if printable(key) == Some(':') {
                self.interaction.read_focus = false;
                return Effect::Redraw;
            }
            return Effect::Idle;
        }
        self.edit_prompt(key)
    }

    fn handle(&mut self, key: KeyEvent) -> Effect {
        self.handle_with_clipboard(key, &mut SystemClipboard)
    }

    fn handle_with_clipboard(
        &mut self,
        key: KeyEvent,
        clipboard: &mut impl clipboard::Clipboard,
    ) -> Effect {
        if key.kind == KeyEventKind::Release {
            return Effect::Idle;
        }
        if let Some(effect) = self.interaction_key(key, clipboard) {
            return effect;
        }
        if control(key, 'c') {
            if self.output.as_ref().is_some_and(|output| output.running) {
                return self.cancel_output();
            }
            return Effect::Exit;
        }
        if self.output.is_some() {
            return self.handle_output(key);
        }
        if key.code == KeyCode::Esc {
            if self.close_main_prompt() {
                return Effect::Redraw;
            }
            if self.search.take().is_some() {
                self.refilter();
                return Effect::Redraw;
            }
            if std::mem::take(&mut self.help) {
                return Effect::Redraw;
            }
            return Effect::Exit;
        }
        if self.prompt.is_none()
            && text_modifiers(key)
            && !key.modifiers.contains(KeyModifiers::SHIFT)
        {
            let index = match key.code {
                KeyCode::Up => Some(self.selected.saturating_sub(1)),
                KeyCode::Down => Some(self.selected + 1),
                KeyCode::Home if self.prompt.is_none() && self.search.is_none() => Some(0),
                KeyCode::End if self.prompt.is_none() && self.search.is_none() => {
                    Some(self.items.len().saturating_sub(1))
                }
                _ => None,
            };
            if let Some(index) = index {
                let closed = self.close_main_prompt();
                let selection = self.select(index);
                return if closed { Effect::Redraw } else { selection };
            }
        }
        if self.prompt.is_some() {
            return self.edit_prompt(key);
        }
        if control(key, 'k') {
            self.search = Some(Input::default());
            self.help = false;
            self.refilter();
            return Effect::Redraw;
        }
        if control(key, 'r') && !self.history.is_empty() {
            self.open_prompt(Input::default());
            self.history_move(true, true);
            return Effect::Redraw;
        }
        if key.code == KeyCode::Enter && text_modifiers(key) {
            return self.activate();
        }
        if let Some(search) = &mut self.search {
            let previous = search.value.clone();
            if !search.edit(key, SEARCH_LIMIT) {
                return Effect::Idle;
            }
            if search.value != previous {
                self.refilter();
            }
            return Effect::Redraw;
        }
        match printable(key) {
            Some('/') => {
                self.search = Some(Input::default());
                self.help = false;
                Effect::Redraw
            }
            Some('?') => {
                self.help = !self.help;
                Effect::Redraw
            }
            Some(':') => {
                self.open_prompt(Input::default());
                Effect::Redraw
            }
            Some(ch) if !ch.is_whitespace() && !self.help => {
                self.open_prompt(Input::default());
                self.edit_prompt(key)
            }
            _ => Effect::Idle,
        }
    }

    fn cancel_output(&mut self) -> Effect {
        if let Some(output) = &mut self.output {
            output.status = text(["Cancelling…", "Đang hủy…"], self.vi).into();
            output.tone = Tone::Warning;
        }
        Effect::Cancel
    }

    fn paste(&mut self, value: &str) -> Effect {
        if self.prompt.is_none() && self.output.is_none() && self.search.is_none() {
            if self.help || value.trim().is_empty() {
                return Effect::Idle;
            }
            self.open_prompt(Input::default());
        }
        self.paste_editor(value)
    }

    fn paste_suppressed_newline(&mut self, value: &str) -> Effect {
        let command_input = (self.prompt.is_some() || self.search.is_none())
            && self
                .output
                .as_ref()
                .is_none_or(|output| output.find.is_none())
            && self
                .prompt
                .as_ref()
                .is_none_or(|prompt| prompt.error.is_none());
        let effect = self.paste(value);
        if command_input
            && matches!(effect, Effect::Redraw)
            && let Some(prompt) = &mut self.prompt
            && prompt.error.is_none()
        {
            prompt.error = Some(PASTE_NEWLINE_NOTICE);
        }
        effect
    }
}

fn safe_history(args: &[String]) -> bool {
    // Keep only known command tokens and flag values. Paths and arbitrary
    // arguments may contain credentials and should not be echoed or recalled.
    let words = [
        "doctor",
        "env",
        "show",
        "check",
        "set",
        "setup",
        "repos",
        "discover",
        "status",
        "clone",
        "update",
        "build",
        "run",
        "dev",
        "stop",
        "verify",
        "help",
        "interface",
        "en",
        "vi",
    ];
    args.iter().all(|arg| {
        words.contains(&arg.as_str())
            || crate::workspace::Repo::KEYS.contains(&arg.as_str())
            || [
                "--all",
                "--repo",
                "--offline",
                "--dry-run",
                "--initialize-db",
                "--open",
                "--desktop",
                "--help",
                "--version",
                "--json",
                "--lang",
                "--browser",
            ]
            .contains(&arg.as_str())
    })
}

fn output_title(args: &[String]) -> String {
    if safe_history(args) {
        return format!("cnl {}", args.join(" "));
    }
    args.iter()
        .find(|arg| {
            ENTRIES
                .iter()
                .any(|entry| entry.command.split_whitespace().next() == Some(arg))
        })
        .map_or_else(|| "cnl".into(), |command| format!("cnl {command}"))
}

fn read_events(batch: &mut Vec<Event>, running: bool) -> io::Result<()> {
    batch.clear();
    let timeout = if running {
        Some(Duration::from_millis(16))
    } else if cfg!(windows) {
        Some(Duration::from_millis(120))
    } else {
        None
    };
    if let Some(timeout) = timeout
        && !event::poll(timeout)?
    {
        return Ok(());
    }
    batch.push(event::read()?);
    while batch.len() < 128 && event::poll(Duration::ZERO)? {
        batch.push(event::read()?);
    }
    Ok(())
}

fn windows_paste(batch: &[Event], previous_text: usize) -> Option<String> {
    // The Windows console input API reports unbracketed paste as queued keys.
    // A newline in a queued text burst is input, never a submit.
    let mut value = String::new();
    let mut characters = previous_text;
    let mut newline = false;
    let mut paste_key = false;
    for event in batch {
        let key = match event {
            Event::Key(key) => key,
            Event::Resize(_, _) | Event::FocusGained => continue,
            _ => return None,
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if control(*key, 'v') {
            paste_key = true;
        } else if let Some(ch) = printable(*key) {
            characters += 1;
            value.push(ch);
        } else if key.code == KeyCode::Enter && text_modifiers(*key) {
            newline = true;
            value.push(' ');
        } else {
            return None;
        }
    }
    ((newline || paste_key) && characters > 0).then_some(value)
}

fn skip_paste_companion(event: &Event, host_paste: bool, pasted: &mut bool) -> bool {
    match event {
        Event::Paste(_) => {
            *pasted = true;
            false
        }
        Event::Key(key) if control(*key, 'v') => {
            *pasted = true;
            host_paste
        }
        Event::Key(key) if key.code == KeyCode::Enter && *pasted => true,
        _ => false,
    }
}

fn show(
    menu: &mut Menu,
    color: bool,
    banner: bool,
    workspace: &Workspace,
    flags: &ArgMatches,
) -> io::Result<()> {
    // Restore the terminal before joining an owned worker on an I/O error.
    let mut running: Option<runner::Runner> = None;
    let mut screen = Screen::enter(io::BufWriter::with_capacity(64 * 1024, io::stdout().lock()))?;
    let mut painter = Painter::default();
    let mut size = console::size()?;
    let mut batch = Vec::with_capacity(128);
    let mut redraw = true;
    let mut last_size_check = Instant::now();
    let mut paste_continuation = 0;
    let mut clipboard = SystemClipboard;
    let animate = color && std::env::var_os("CNL_REDUCED_MOTION").is_none();
    loop {
        if let Some(worker) = &mut running {
            worker.set_language(menu.vi);
            let update = worker.poll()?;
            if let Some(output) = &mut menu.output {
                if let Some(buffer) = update.buffer {
                    output.buffer = buffer;
                    if !output.query.is_empty() {
                        output.last_page = output
                            .buffer
                            .layout(output.width)
                            .saturating_sub(output.page_size.max(1));
                        output.hit = None;
                        output.search(false);
                    }
                    redraw = true;
                }
                if let Some(result) = update.result {
                    output.finish(Ok(result), menu.vi);
                    running = None;
                    redraw = true;
                } else if animate {
                    redraw |= output.animate();
                }
            }
        }
        if redraw {
            painter.draw(
                &mut screen.out,
                view::frame(menu, size, banner, color),
                color,
            )?;
        }
        read_events(&mut batch, running.is_some())?;
        redraw = false;
        if batch
            .iter()
            .any(|event| matches!(event, Event::Resize(_, _)))
            || (cfg!(windows) && last_size_check.elapsed() >= Duration::from_millis(120))
        {
            let new_size = console::size()?;
            last_size_check = Instant::now();
            if new_size.0 > 0 && new_size.1 > 0 && new_size != size {
                size = new_size;
                redraw = true;
            }
        }

        let pending = std::mem::take(&mut paste_continuation);
        if cfg!(windows) {
            if batch.len() == 128 {
                paste_continuation = pending + batch.iter().filter(|event| {
                    matches!(event, Event::Key(key) if key.kind != KeyEventKind::Release && printable(*key).is_some())
                }).count();
            }
            if let Some(paste) = windows_paste(&batch, pending) {
                redraw |= matches!(menu.paste_suppressed_newline(&paste), Effect::Redraw);
                continue;
            }
        }

        let host_paste = batch.iter().any(|event| matches!(event, Event::Paste(_)));
        let mut pasted = false;
        for input in batch.drain(..) {
            if skip_paste_companion(&input, host_paste, &mut pasted) {
                continue;
            }
            let effect = match input {
                Event::Key(key) => menu.handle(key),
                Event::Paste(value) => menu.paste(&value),
                Event::Mouse(mouse) => menu.mouse(mouse, painter.last_frame(size), &mut clipboard),
                Event::FocusLost => menu.focus_lost(),
                _ => Effect::Idle,
            };
            match effect {
                Effect::Idle => {}
                Effect::Redraw => redraw = true,
                Effect::Cancel => {
                    if let Some(worker) = &mut running
                        && let Err(error) = worker.cancel()
                        && let Some(output) = &mut menu.output
                    {
                        output.status = error.to_owned();
                    }
                    redraw = true;
                }
                Effect::Native(line) => {
                    menu.begin_output(&[]);
                    let output = menu.output.as_mut().expect("output was created");
                    output.title = None;
                    match runner::Runner::start_native(&line, menu.vi) {
                        Ok(worker) => {
                            output.follow = true;
                            running = Some(worker);
                        }
                        Err(error) => output.finish(Err(error.to_string()), menu.vi),
                    }
                    redraw = true;
                }
                Effect::Run(args) => {
                    menu.begin_output(&args);
                    if let Some(shell) = native_shell_request(&args) {
                        drop(screen);
                        let started = Instant::now();
                        let result = crate::cli::native::interactive(shell.as_deref())
                            .map(|status| runner::RunOutput {
                                outcome: if status.success() {
                                    runner::Outcome::Pass
                                } else {
                                    runner::Outcome::Fail
                                },
                                exit_code: status.code(),
                                elapsed_ms: started.elapsed().as_millis(),
                            })
                            .map_err(|error| error.to_string());
                        screen = Screen::enter(io::BufWriter::with_capacity(
                            64 * 1024,
                            io::stdout().lock(),
                        ))?;
                        painter = Painter::default();
                        size = console::size()?;
                        menu.output
                            .as_mut()
                            .expect("output was created")
                            .finish(result, menu.vi);
                        redraw = true;
                        break;
                    }
                    match runner::Runner::start(workspace, flags, &args, menu.vi) {
                        Ok(worker) => {
                            menu.output.as_mut().expect("output was created").follow = true;
                            running = Some(worker);
                        }
                        Err(error) => menu
                            .output
                            .as_mut()
                            .expect("output was created")
                            .finish(Err(error.to_string()), menu.vi),
                    }
                    redraw = true;
                }
                Effect::Exit => return Ok(()),
            }
        }
    }
}

fn native_shell_request(args: &[String]) -> Option<Option<String>> {
    let matches = crate::cli::app()
        .try_get_matches_from(std::iter::once("cnl").chain(args.iter().map(String::as_str)))
        .ok()?;
    if matches.get_flag("json") {
        return None;
    }
    matches
        .subcommand_matches("shell")
        .map(|shell| shell.get_one::<String>("shell").cloned())
}

pub fn open(workspace: &Workspace, flags: &ArgMatches) -> anyhow::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!(
            "{}",
            crate::localization::message([
                "cnl tui requires an interactive terminal",
                "cnl tui yêu cầu terminal tương tác"
            ])
        );
    }
    let vi = flags
        .get_one::<String>("lang")
        .is_some_and(|language| language == "vi");
    let mut menu = Menu::new(vi);
    show(&mut menu, terminal::color_enabled(), true, workspace, flags)?;
    Ok(())
}

#[cfg(test)]
mod state_tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn choose(menu: &mut Menu, command: &str) {
        let index = menu
            .items
            .iter()
            .position(|entry| entry.command == command)
            .unwrap();
        menu.select(index);
        assert!(matches!(menu.activate(), Effect::Redraw));
    }

    #[test]
    fn menu_commands_open_fresh_base_prompts_and_offer_valid_actions() {
        let mut menu = Menu::new(false);
        for entry in ENTRIES.iter().filter(|entry| !entry.command.is_empty()) {
            choose(&mut menu, entry.command);
            assert_eq!(
                menu.prompt.as_ref().unwrap().input.value,
                format!("{} ", entry.command)
            );
            menu.refresh_suggestions();
            let prompt = menu.prompt.as_ref().unwrap();
            if let Some(action) = entry.recommendation {
                assert_eq!(prompt.completions.first().unwrap().text, action);
                let command = format!("{}{action}", prompt.input.value);
                assert!(
                    crate::cli::app()
                        .try_get_matches_from(
                            std::iter::once("cnl").chain(command.split_ascii_whitespace())
                        )
                        .is_ok(),
                    "{command}"
                );
            }
            menu.handle(key(KeyCode::Char('x')));
            menu.handle(key(KeyCode::Esc));
        }
    }

    #[test]
    fn output_navigation_clears_abandoned_input_without_losing_history() {
        let mut menu = Menu::new(false);
        menu.open_prompt(Input::at_end("status".into()));
        assert!(matches!(menu.submit_prompt(), Effect::Run(_)));
        menu.begin_output(&["status".into()]);
        menu.output.as_mut().unwrap().running = false;
        menu.open_prompt(Input::at_end("build --of".into()));
        assert!(matches!(menu.handle(key(KeyCode::PageUp)), Effect::Idle));
        assert_eq!(menu.prompt.as_ref().unwrap().input.value, "build --of");
        menu.handle(key(KeyCode::Esc));
        assert!(menu.output.is_none());
        choose(&mut menu, "run");
        assert_eq!(menu.prompt.as_ref().unwrap().input.value, "run ");
        menu.handle(key(KeyCode::Up));
        assert_eq!(menu.prompt.as_ref().unwrap().input.value, "status");
        menu.handle(key(KeyCode::Down));
        assert_eq!(menu.prompt.as_ref().unwrap().input.value, "run ");
    }

    #[test]
    fn language_switch_is_local_and_keeps_active_output_and_history() {
        let mut menu = Menu::new(false);
        menu.history.push("status".into());
        menu.begin_output(&["build".into(), "--all".into()]);
        menu.output.as_mut().unwrap().buffer =
            Buffer::from_streams("native work in flight".into(), "", None);
        for (command, vi) in [("cnl --lang vi", true), ("cnl tui --lang=en", false)] {
            menu.open_prompt(Input::at_end(command.into()));
            assert!(matches!(menu.handle(key(KeyCode::Enter)), Effect::Redraw));
            assert_eq!(menu.vi, vi);
            assert_eq!(menu.history, ["status"]);
            let output = menu.output.as_mut().unwrap();
            assert!(output.running);
            assert_eq!(output.status, text(["Running…", "Đang chạy…"], vi));
            assert!(
                output
                    .buffer
                    .find("native work in flight", None, false)
                    .is_some()
            );
        }
        menu.open_prompt(Input::at_end("status".into()));
        assert!(matches!(menu.submit_prompt(), Effect::Redraw));
        assert!(menu.output.as_ref().unwrap().running);
    }

    #[test]
    fn click_completion_inserts_only_and_enter_runs_it_in_both_layouts() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        for size in [(55, 18), (126, 31)] {
            let mut menu = Menu::new(true);
            menu.open_prompt(Input::at_end("repos clone re".into()));
            let frame = view::frame(&mut menu, size, false, true);
            let target = frame.suggestions.as_ref().expect("visible completion");
            let mut clipboard = SystemClipboard;
            for kind in [
                MouseEventKind::Moved,
                MouseEventKind::Down(MouseButton::Left),
                MouseEventKind::Up(MouseButton::Left),
            ] {
                let effect = menu.mouse(
                    MouseEvent {
                        kind,
                        column: target.columns.start,
                        row: target.rows.start,
                        modifiers: KeyModifiers::NONE,
                    },
                    Some(&frame),
                    &mut clipboard,
                );
                assert!(matches!(effect, Effect::Idle | Effect::Redraw));
            }
            assert_eq!(
                menu.prompt.as_ref().unwrap().input.value,
                "repos clone remote"
            );
            assert!(menu.output.is_none());
            assert!(matches!(menu.handle(key(KeyCode::Enter)), Effect::Run(_)));
        }
    }
}
