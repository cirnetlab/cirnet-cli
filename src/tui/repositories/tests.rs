use super::*;
use crossterm::event::{KeyModifiers, MouseEvent};
use unicode_width::UnicodeWidthStr;

fn row(index: usize) -> Row {
    Row {
        selector: (index + 1).to_string(),
        name: format!("repo-{index:03}"),
        alias: None,
        path: Some(PathBuf::from(format!("/workspace/repo-{index:03}"))),
        branch: Some(
            if index.is_multiple_of(2) {
                "main"
            } else {
                "topic"
            }
            .into(),
        ),
        state: "clean".into(),
        ahead: Some(0),
        behind: Some(0),
        revision: Some("c783d53b1fc11a1aaa0e72a441c9bba47d3c2674".into()),
        short_revision: Some("c783d53".into()),
        groups: vec![
            if index.is_multiple_of(2) {
                "Even"
            } else {
                "Odd"
            }
            .into(),
        ],
        remote: None,
        shell: None,
        active: false,
        registered: true,
        ..Row::default()
    }
}

fn populated() -> Dashboard {
    let mut dashboard = Dashboard::new(false, false);
    dashboard.replace_rows((0..160).map(row).collect());
    dashboard
}

#[test]
fn filters_are_incremental_and_keep_checked_repositories_outside_the_view() {
    let mut dashboard = populated();
    dashboard.toggle(0);
    dashboard.filters.group = Some("Odd".into());
    dashboard.search = Input::at_end("topic".into());
    dashboard.refilter();
    assert_eq!(dashboard.visible.len(), 80);
    assert_eq!(dashboard.focused().unwrap().name, "repo-001");
    let targets = dashboard.targets();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].selector, "1");
}

#[test]
fn scrolling_and_resize_paint_only_visible_rows_without_losing_focus() {
    let mut dashboard = populated();
    dashboard.focus(130);
    let small = dashboard.frame((44, 9));
    let small_hits = small.menu.as_ref().unwrap();
    assert!(small_hits.rows.len() < 9);
    assert!(small_hits.first <= 130);
    assert!(small_hits.first + small_hits.rows.len() > 130);
    assert!(
        small
            .rows
            .iter()
            .any(|line| line.contents().contains("repo-130"))
    );
    assert!(
        !small
            .rows
            .iter()
            .any(|line| line.contents().contains("repo-000"))
    );
    let wide = dashboard.frame((140, 32));
    assert!(wide.menu.as_ref().unwrap().rows.len() < 32);
    assert_eq!(dashboard.focused().unwrap().selector, "131");
    assert!(wide.rows.iter().all(|line| line.contents().width() < 140));
}

#[test]
fn keyboard_and_checkbox_mouse_click_toggle_the_same_selection() {
    let mut dashboard = populated();
    let frame = dashboard.frame((86, 20));
    let hit = frame.menu.as_ref().unwrap();
    let mouse = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: hit.columns.start + 1,
        row: hit.rows.start + 2,
        modifiers: KeyModifiers::NONE,
    };
    dashboard.handle(Event::Mouse(mouse), Some(&frame));
    assert_eq!(dashboard.focused().unwrap().selector, "3");
    assert!(dashboard.checked.contains("3"));
    dashboard.handle(
        Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
        Some(&frame),
    );
    assert!(dashboard.checked.is_empty());
}

#[test]
fn a_bulk_preview_requires_confirmation_and_captures_the_selected_targets() {
    let mut dashboard = populated();
    dashboard.toggle(0);
    dashboard.toggle(2);
    dashboard.activate(Action::Update);
    assert!(matches!(dashboard.overlay, Overlay::Preview { .. }));
    assert!(matches!(
        dashboard.handle(
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            None
        ),
        Effect::Redraw
    ));
    assert!(matches!(dashboard.overlay, Overlay::None));
    dashboard.activate(Action::Update);
    assert!(matches!(
        dashboard.handle(
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            None
        ),
        Effect::Idle
    ));
    let key = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
    let Effect::Run { args, .. } = dashboard.handle(Event::Key(key), None) else {
        panic!("Confirming starts the captured command");
    };
    assert_eq!(args, ["repos", "update", "1", "3"]);
}

#[test]
fn worktree_preview_preserves_a_quoted_destination_in_the_shared_grammar() {
    let command = super::super::completion::grammar();
    let mut dashboard = populated();
    dashboard.activate(Action::WorktreeAdd);
    let Overlay::Prompt { input, .. } = &mut dashboard.overlay else {
        panic!("Expected a destination and branch prompt");
    };
    *input = Input::at_end(r#""C:\My Worktrees\topic" feature/status"#.into());
    dashboard.submit_prompt();
    let frame = dashboard.frame((95, 20));
    let confirm = frame.suggestions.as_ref().expect("clickable confirmation");
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: confirm.columns.start + 1,
        row: confirm.rows.start,
        modifiers: KeyModifiers::NONE,
    };
    let Effect::Run { args, .. } = dashboard.handle(Event::Mouse(click), Some(&frame)) else {
        panic!("Clicking confirmation starts the worktree command");
    };
    let matches = command
        .clone()
        .try_get_matches_from(std::iter::once("cnl").chain(args.iter().map(String::as_str)))
        .unwrap();
    let worktree = matches
        .subcommand_matches("repos")
        .unwrap()
        .subcommand_matches("worktree")
        .unwrap();
    assert_eq!(
        worktree.get_one::<PathBuf>("path"),
        Some(&PathBuf::from(r"C:\My Worktrees\topic"))
    );
    assert_eq!(
        worktree.get_one::<String>("branch").map(String::as_str),
        Some("feature/status")
    );
}

#[test]
fn select_all_tracks_filtered_scope_with_alphabetical_order_from_legacy_settings() {
    let mut dashboard = Dashboard::new(false, false);
    let mut first = row(0);
    let mut third = row(2);
    first.last_sync = Some(100);
    third.last_sync = Some(200);
    dashboard.replace_rows(vec![third, row(1), first]);
    dashboard.filters =
        serde_json::from_str(r#"{"search":"","group":"Even","sort":"activity"}"#).unwrap();
    dashboard.refilter();
    assert_eq!(dashboard.row(0).unwrap().selector, "1");
    let select = Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    dashboard.handle(select, None);
    assert_eq!(dashboard.checked, ["1".to_owned(), "3".to_owned()].into());
    let clear = Event::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    dashboard.handle(clear, None);
    assert!(dashboard.checked.is_empty());
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn status_and_empty_footer_do_not_open_actions() {
    let mut dashboard = populated();
    dashboard.loading = false;
    dashboard.toggle(0);
    let frame = dashboard.frame((100, 20));
    let y = frame.size.1 - 1;
    let footer = frame.rows[usize::from(y)].contents();
    assert!(footer.ends_with("1 selected"), "{footer}");
    assert!(!footer.contains("Sort"), "{footer}");
    assert!(matches!(
        dashboard.handle(
            mouse(MouseEventKind::Down(MouseButton::Left), 95, y),
            Some(&frame)
        ),
        Effect::Idle
    ));
    assert!(matches!(dashboard.overlay, Overlay::None));
    assert!(matches!(
        dashboard.handle(
            mouse(MouseEventKind::Down(MouseButton::Left), 61, y),
            Some(&frame)
        ),
        Effect::Idle
    ));
    assert!(matches!(
        dashboard.handle(
            mouse(MouseEventKind::Down(MouseButton::Left), 3, y),
            Some(&frame)
        ),
        Effect::Redraw
    ));
    assert!(matches!(dashboard.overlay, Overlay::Actions { .. }));
}

#[test]
fn nested_escape_restores_prompt_actions_details_and_table() {
    let mut dashboard = populated();
    dashboard.focus(25);
    dashboard.toggle(25);
    dashboard.handle(key(KeyCode::Enter), None);
    dashboard.handle(key(KeyCode::Down), None);
    dashboard.handle(key(KeyCode::Enter), None);
    let Overlay::Actions { items, .. } = &dashboard.overlay else {
        panic!("Details can open Actions");
    };
    let index = items
        .iter()
        .position(|item| *item == Action::BranchCreate)
        .unwrap();
    for _ in 0..index {
        dashboard.handle(key(KeyCode::Down), None);
    }
    dashboard.handle(key(KeyCode::Enter), None);
    dashboard.handle(Event::Paste("topic/nested".into()), None);
    dashboard.handle(key(KeyCode::Enter), None);
    assert!(matches!(dashboard.overlay, Overlay::Preview { .. }));
    dashboard.handle(key(KeyCode::Esc), None);
    let Overlay::Prompt { input, .. } = &dashboard.overlay else {
        panic!("Preview should restore its prompt");
    };
    assert_eq!(input.value, "topic/nested");
    dashboard.handle(key(KeyCode::Esc), None);
    assert!(matches!(dashboard.overlay, Overlay::Actions { cursor, .. } if cursor == index));
    dashboard.handle(key(KeyCode::Esc), None);
    assert!(matches!(dashboard.overlay, Overlay::Details { scroll: 1 }));
    dashboard.handle(key(KeyCode::Esc), None);
    assert!(matches!(dashboard.overlay, Overlay::None));
    assert_eq!(dashboard.focused().unwrap().selector, "26");
    assert!(dashboard.checked.contains("26"));
    assert!(matches!(
        dashboard.handle(key(KeyCode::Esc), None),
        Effect::Exit
    ));
}

#[test]
fn compact_horizontal_scroll_leaves_identity_and_vertical_focus_in_place() {
    let mut dashboard = populated();
    let size = (44, 10);
    let initial = dashboard.frame(size);
    let y = initial.menu.as_ref().unwrap().rows.start;
    let before = initial.rows[usize::from(y)].contents();
    assert!(before.contains("repo-000"));
    assert!(matches!(
        dashboard.handle(key(KeyCode::Right), Some(&initial)),
        Effect::Redraw
    ));
    let shifted = dashboard.frame(size);
    assert_ne!(shifted.rows[usize::from(y)].contents(), before);
    assert!(shifted.rows[usize::from(y)].contents().contains("repo-000"));
    assert_eq!(shifted.menu.as_ref().unwrap().first, 0);
}

#[test]
fn clearing_filter_restores_an_unmodified_selection() {
    let mut dashboard = populated();
    dashboard.focus(130);
    dashboard.search = Input::at_end("repo-000".into());
    dashboard.refilter();
    assert_eq!(dashboard.focused().unwrap().selector, "1");
    dashboard.search = Input::default();
    dashboard.refilter();
    assert_eq!(dashboard.focused().unwrap().selector, "131");
}

#[test]
fn running_output_requires_cancellation_before_returning_to_the_preview() {
    let mut dashboard = populated();
    dashboard.activate(Action::Update);
    assert!(matches!(
        dashboard.handle(key(KeyCode::Char('y')), None),
        Effect::Run { .. }
    ));
    dashboard.output = Some(OutputView::new("update".into(), false));
    dashboard.push_overlay(Overlay::Output);
    assert!(matches!(
        dashboard.handle(key(KeyCode::Esc), None),
        Effect::Cancel
    ));
    assert!(matches!(dashboard.overlay, Overlay::Output));
    dashboard.output.as_mut().unwrap().running = false;
    dashboard.handle(key(KeyCode::Esc), None);
    assert!(matches!(dashboard.overlay, Overlay::Preview { .. }));
    dashboard.handle(key(KeyCode::Esc), None);
    assert!(matches!(dashboard.overlay, Overlay::None));
}

#[test]
fn scrolled_path_hits_show_the_normalized_path_and_invalidate_after_resize() {
    let mut first = row(0);
    first.path = Some(PathBuf::from(
        r"\\?\UNC\server\share\Vietnamese 東京\repo-000",
    ));
    let mut dashboard = Dashboard::new(false, false);
    dashboard.replace_rows(vec![first, row(1)]);
    dashboard.horizontal = 24;
    let frame = dashboard.frame((64, 12));
    let y = frame.menu.as_ref().unwrap().rows.start;
    let path = dashboard.table_hits.path.clone();
    assert!(!path.is_empty());
    assert!(
        frame.rows[usize::from(y) - 1]
            .contents()
            .contains("Local path")
    );
    assert!(matches!(
        dashboard.handle(mouse(MouseEventKind::Moved, path.start, y), Some(&frame)),
        Effect::Redraw
    ));
    let hovered = dashboard.frame(frame.size);
    let has_full_path = |frame: &Frame| {
        frame.rows.iter().any(|line| {
            line.contents()
                .contains(r"\\server\share\Vietnamese 東京\repo-000")
        })
    };
    assert!(has_full_path(&hovered));
    assert!(hovered.rows.iter().all(|line| line.contents().width() < 64));
    assert!(matches!(
        dashboard.handle(
            mouse(MouseEventKind::Down(MouseButton::Left), path.start, y + 1),
            Some(&hovered)
        ),
        Effect::Idle
    ));
    let resized = dashboard.frame((63, 12));
    assert!(!has_full_path(&resized));
    dashboard.handle(key(KeyCode::Char('p')), Some(&resized));
    let keyboard_tooltip = dashboard.frame(resized.size);
    assert!(has_full_path(&keyboard_tooltip));
    assert_eq!(
        dashboard.focused().unwrap().path.as_deref(),
        Some(std::path::Path::new(
            r"\\?\UNC\server\share\Vietnamese 東京\repo-000"
        ))
    );
}

#[test]
fn path_copy_and_refresh_keep_display_separate_from_checkout_identity() {
    struct Recorder(String);
    impl Clipboard for Recorder {
        fn get_text(&mut self) -> Result<String, super::super::clipboard::ClipboardError> {
            Ok(self.0.clone())
        }
        fn set_text(&mut self, text: &str) -> Result<(), super::super::clipboard::ClipboardError> {
            self.0 = text.to_owned();
            Ok(())
        }
    }
    let mut repo = row(0);
    repo.path = Some(PathBuf::from(r"\\?\E:\fixtures\repo"));
    let mut dashboard = Dashboard::new(false, false);
    dashboard.replace_rows(vec![repo.clone()]);
    dashboard.toggle(0);
    let mut clipboard = Recorder(String::new());
    dashboard.copy_path(&mut clipboard);
    assert_eq!(clipboard.0, r"E:\fixtures\repo");
    repo.selector = "new-registry-id".into();
    if cfg!(windows) {
        repo.path = Some(PathBuf::from(r"e:\fixtures\repo"));
    }
    dashboard.replace_rows(vec![repo]);
    assert_eq!(dashboard.focused().unwrap().selector, "new-registry-id");
    assert!(dashboard.checked.contains("new-registry-id"));
}

#[test]
fn table_uses_short_hash_and_details_wrap_commit_metadata() {
    let mut repo = row(0);
    repo.commit_message = Some("A meaningful commit message with a second line".into());
    repo.commit_author = Some("Git Fixture".into());
    repo.commit_timestamp = Some("2026-11-23T12:45:11Z".into());
    let full = repo.revision.clone().unwrap();
    let mut dashboard = Dashboard::new(false, false);
    dashboard.replace_rows(vec![repo]);
    let frame = dashboard.frame((149, 24));
    let y = frame.menu.as_ref().unwrap().rows.start;
    let first = frame.rows[usize::from(y)].contents();
    assert!(first.contains("c783d53"));
    assert!(!first.contains(&full));
    dashboard.handle(key(KeyCode::Enter), Some(&frame));
    let details = dashboard.frame((44, 32));
    let joined = details
        .rows
        .iter()
        .map(|row| row.contents().trim())
        .collect::<Vec<_>>()
        .join("");
    assert!(joined.contains(&full));
    assert!(joined.contains("meaningful commit message"));
    assert!(joined.contains("Git Fixture"));
    assert!(joined.contains("2026-11-23T12:45:11Z"));
    assert!(details.rows.iter().all(|line| line.contents().width() < 44));
}

#[test]
fn colon_requests_the_shared_prompt_without_changing_the_dashboard() {
    let mut dashboard = populated();
    dashboard.focus(26);
    dashboard.search.value = "repo".into();
    dashboard.refilter();
    dashboard.open_actions();
    let shifted = Event::Key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT));
    assert!(matches!(
        dashboard.handle(shifted, None),
        Effect::CommandPrompt
    ));
    assert!(matches!(dashboard.overlay, Overlay::Actions { .. }));
    let (selectors, focused) = dashboard.command_context(&Workspace {
        root: PathBuf::from("/workspace"),
    });
    assert_eq!(focused.as_deref(), Some("27"));
    assert_eq!(selectors.first().map(String::as_str), focused.as_deref());
    assert!(selectors.iter().any(|selector| selector == "repo-026"));
    assert_eq!(dashboard.search.value, "repo");
}

#[test]
fn shifted_wheel_and_horizontal_wheel_leave_vertical_position_unchanged() {
    let mut dashboard = populated();
    dashboard.focus(20);
    let initial = dashboard.frame((48, 13));
    let initial_scroll = dashboard.scroll;
    let shifted_wheel = Event::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 20,
        row: 6,
        modifiers: KeyModifiers::SHIFT,
    });
    assert!(matches!(
        dashboard.handle(shifted_wheel, Some(&initial)),
        Effect::Redraw
    ));
    assert!(dashboard.horizontal > 0);
    let shifted = dashboard.frame(initial.size);
    assert!(matches!(
        dashboard.handle(mouse(MouseEventKind::ScrollLeft, 20, 6), Some(&shifted)),
        Effect::Redraw
    ));
    assert_eq!(dashboard.horizontal, 0);
    assert_eq!(dashboard.scroll, initial_scroll);
    assert_eq!(dashboard.focused().unwrap().selector, "21");
}

#[test]
fn mouse_actions_back_preserves_the_origin_and_ignores_choice_whitespace() {
    let mut dashboard = populated();
    dashboard.handle(key(KeyCode::Enter), None);
    let details = dashboard.frame((105, 26));
    let actions = details.suggestions.as_ref().unwrap();
    let click = |column, row| mouse(MouseEventKind::Down(MouseButton::Left), column, row);
    assert!(matches!(
        dashboard.handle(
            click(actions.columns.start, actions.rows.start),
            Some(&details)
        ),
        Effect::Redraw
    ));
    let menu = dashboard.frame(details.size);
    let choices = menu.menu.as_ref().unwrap();
    assert!(matches!(
        dashboard.handle(click(90, choices.rows.start), Some(&menu)),
        Effect::Idle
    ));
    dashboard.handle(key(KeyCode::Esc), Some(&menu));
    assert!(matches!(dashboard.overlay, Overlay::Details { .. }));
}
