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
        revision: Some("2026-11-23".into()),
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
fn select_all_tracks_filtered_scope_and_recent_sort_uses_sync_time() {
    let mut dashboard = Dashboard::new(false, false);
    let mut first = row(0);
    let mut third = row(2);
    first.last_sync = Some(100);
    third.last_sync = Some(200);
    dashboard.replace_rows(vec![first, row(1), third]);
    dashboard.filters.group = Some("Even".into());
    dashboard.filters.sort = Sort::Activity;
    dashboard.refilter();
    assert_eq!(dashboard.row(0).unwrap().selector, "3");
    let select = Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    dashboard.handle(select, None);
    assert_eq!(dashboard.checked, ["1".to_owned(), "3".to_owned()].into());
    let clear = Event::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    dashboard.handle(clear, None);
    assert!(dashboard.checked.is_empty());
}
