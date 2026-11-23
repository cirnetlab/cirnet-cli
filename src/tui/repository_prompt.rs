use std::io;

use clap::ArgMatches;
use crossterm::event::Event;

use super::{Effect, History, Input, Menu, SystemClipboard, runner::Runner, terminal::Frame, view};
use crate::workspace::Workspace;

pub(super) enum PromptEvent {
    Idle,
    Redraw,
    Closed {
        refresh: bool,
    },
    Terminal {
        selector: String,
        shell: Option<String>,
        refresh: bool,
    },
}

pub(super) struct RepositoryPrompt {
    menu: Menu,
    worker: Option<Runner>,
    refresh: bool,
    focused: Option<String>,
    animate: bool,
}

impl RepositoryPrompt {
    pub(super) fn new(vi: bool, history: &mut History) -> Self {
        let mut menu = Menu::new(vi);
        menu.history = std::mem::take(history);
        let animate =
            super::terminal::color_enabled() && std::env::var_os("CNL_REDUCED_MOTION").is_none();
        Self {
            menu,
            worker: None,
            refresh: false,
            focused: None,
            animate,
        }
    }

    pub(super) fn finish(self, history: &mut History) {
        *history = self.menu.history;
    }

    pub(super) fn open_on(&mut self, mut selectors: Vec<String>, focused: Option<String>) {
        selectors.extend(crate::workspace::Repo::KEYS.into_iter().map(str::to_owned));
        let mut unique = std::collections::HashSet::new();
        selectors.retain(|value| unique.insert(value.clone()));
        self.menu.completer.set_repository_values(selectors);
        self.focused = focused;
        // The repository panel has its own selection and search state. Only open a fresh editor.
        self.menu.open_prompt(Input::default());
    }

    pub(super) fn active(&self) -> bool {
        self.menu.prompt.is_some() || self.menu.output.is_some()
    }

    pub(super) fn frame(&mut self, dashboard: Frame, color: bool) -> Frame {
        if self.menu.output.is_some() {
            view::frame(&mut self.menu, dashboard.size, false, color)
        } else {
            view::overlay_prompt(&mut self.menu, dashboard, color)
        }
    }

    pub(super) fn polling(&self) -> bool {
        self.worker.is_some() || self.menu.completer.metadata_pending()
    }

    pub(super) fn poll(&mut self) -> io::Result<bool> {
        let mut redraw = false;
        if self.menu.completer.metadata_ready()
            && self
                .menu
                .prompt
                .as_ref()
                .is_some_and(|prompt| prompt.accepted.is_none() && prompt.error.is_none())
        {
            self.menu.refresh_completion_labels();
            redraw = true;
        }
        if let Some(worker) = &mut self.worker {
            worker.set_language(self.menu.vi);
            let update = worker.poll()?;
            let output = self.menu.output.as_mut().expect("worker has output");
            if let Some(captured) = update.captured {
                self.menu.interaction.invalidate_output_selection();
                output.captured = Some(captured);
                output.render_capture(self.menu.vi);
                redraw = true;
            }
            if let Some(result) = update.result {
                output.finish(Ok(result), self.menu.vi);
                self.worker = None;
                redraw = true;
            } else if self.animate {
                redraw |= output.animate();
            }
        }
        Ok(redraw)
    }

    pub(super) fn paste(&mut self, value: &str, suppress_newline: bool) -> PromptEvent {
        let effect = if suppress_newline {
            self.menu.paste_suppressed_newline(value)
        } else {
            self.menu.paste(value)
        };
        if matches!(effect, Effect::Redraw) {
            PromptEvent::Redraw
        } else {
            PromptEvent::Idle
        }
    }

    pub(super) fn event(
        &mut self,
        event: Event,
        frame: Option<&Frame>,
        workspace: &Workspace,
        flags: &ArgMatches,
    ) -> io::Result<PromptEvent> {
        let previous_output = self.menu.output.is_some();
        let effect = match event {
            Event::Key(key) => self.menu.handle(key),
            Event::Mouse(mouse) => self.menu.mouse(mouse, frame, &mut SystemClipboard),
            Event::Paste(value) => return Ok(self.paste(&value, false)),
            Event::FocusLost => self.menu.focus_lost(),
            _ => Effect::Idle,
        };
        let vi = self.menu.vi;
        let result = match effect {
            Effect::Idle => PromptEvent::Idle,
            Effect::Redraw if previous_output && self.menu.output.is_none() => {
                self.menu.open_prompt(Input::default());
                PromptEvent::Redraw
            }
            Effect::Exit => {
                self.menu.close_main_prompt();
                self.menu.output = None;
                self.menu.interaction.reset_output();
                PromptEvent::Closed {
                    refresh: std::mem::take(&mut self.refresh),
                }
            }
            Effect::Redraw if !self.active() => PromptEvent::Closed {
                refresh: std::mem::take(&mut self.refresh),
            },
            Effect::Redraw => PromptEvent::Redraw,
            Effect::Cancel => {
                if let Some(worker) = &mut self.worker
                    && let Err(error) = worker.cancel()
                    && let Some(output) = &mut self.menu.output
                {
                    output.status = error.to_owned();
                }
                PromptEvent::Redraw
            }
            Effect::Run(args) => {
                if let Some(shell) = super::native_shell_request(&args) {
                    if let Some(selector) = self.focused.clone() {
                        return Ok(self.terminal(selector, shell));
                    }
                    self.context_hint();
                } else {
                    self.refresh |= args.iter().any(|word| word == "repos");
                    self.menu.begin_output(&args);
                    self.start(Runner::start(workspace, flags, &args, vi));
                }
                PromptEvent::Redraw
            }
            Effect::Native(line) => {
                self.menu.begin_output(&[]);
                self.menu.output.as_mut().expect("native output").title = None;
                self.start(Runner::start_native(&line, vi, None));
                PromptEvent::Redraw
            }
            Effect::Repositories(_) => {
                self.context_hint();
                PromptEvent::Redraw
            }
            Effect::RepositoryTerminal(selector, shell) => self.terminal(selector, shell),
        };
        Ok(result)
    }

    fn start(&mut self, result: anyhow::Result<Runner>) {
        let output = self.menu.output.as_mut().expect("command output");
        match result {
            Ok(worker) => {
                output.follow = true;
                self.worker = Some(worker);
            }
            Err(error) => output.finish(Err(format!("{error:#}")), self.menu.vi),
        }
    }

    fn terminal(&mut self, selector: String, shell: Option<String>) -> PromptEvent {
        self.menu.close_main_prompt();
        self.menu.output = None;
        self.menu.interaction.reset_output();
        PromptEvent::Terminal {
            selector,
            shell,
            refresh: std::mem::take(&mut self.refresh),
        }
    }

    fn context_hint(&mut self) {
        self.menu.open_prompt(Input::default());
        if let Some(prompt) = &mut self.menu.prompt {
            prompt.error = Some([
                "Use Repository Manager actions here. Use explicit arguments for CLI commands.",
                "Dùng thao tác trong Trình quản lý repo. Chỉ định đối số cho lệnh CLI.",
            ]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn shared_prompt_reuses_history_and_overlays_the_dashboard() {
        let workspace = Workspace {
            root: std::env::temp_dir(),
        };
        let flags = crate::cli::app().try_get_matches_from(["cnl"]).unwrap();
        let mut history = History::default();
        history.record_cnl("status", &["status".into()]);
        let mut editor = RepositoryPrompt::new(false, &mut history);
        editor.open_on(vec!["r:42".into()], Some("r:42".into()));
        editor
            .event(key(KeyCode::Up), None, &workspace, &flags)
            .unwrap();
        assert_eq!(editor.menu.prompt.as_ref().unwrap().input.value, "status");
        editor
            .event(key(KeyCode::Down), None, &workspace, &flags)
            .unwrap();
        editor.paste("repos status r:", false);
        let mut background = Frame::new((91, 25));
        background.rows[0].push(super::super::Tone::Accent, "Repository table");
        let frame = editor.frame(background, false);
        assert_eq!(frame.rows[0].contents(), "Repository table");
        assert!(frame.input.is_some());
        assert!(
            editor
                .menu
                .prompt
                .as_ref()
                .unwrap()
                .completions
                .iter()
                .any(|item| item.text == "r:42")
        );
        assert!(matches!(
            editor
                .event(key(KeyCode::Esc), Some(&frame), &workspace, &flags)
                .unwrap(),
            PromptEvent::Closed { refresh: false }
        ));
        assert!(!editor.active());
        editor.finish(&mut history);
        assert_eq!(history.entries().count(), 1);
    }

    #[test]
    fn completed_output_returns_to_the_command_field_before_the_dashboard() {
        let workspace = Workspace {
            root: std::env::temp_dir(),
        };
        let flags = crate::cli::app().try_get_matches_from(["cnl"]).unwrap();
        let mut history = History::default();
        let mut editor = RepositoryPrompt::new(false, &mut history);
        editor.open_on(vec![], None);
        editor.menu.begin_output(&["status".into()]);
        editor.menu.output.as_mut().unwrap().running = false;
        assert!(matches!(
            editor
                .event(key(KeyCode::Esc), None, &workspace, &flags)
                .unwrap(),
            PromptEvent::Redraw
        ));
        assert!(editor.menu.prompt.is_some() && editor.menu.output.is_none());
        assert!(matches!(
            editor
                .event(key(KeyCode::Esc), None, &workspace, &flags)
                .unwrap(),
            PromptEvent::Closed { refresh: false }
        ));
    }
}
