use super::*;
use crossterm::event::{KeyCode, KeyModifiers, MouseEventKind};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Repos(PathBuf);

impl Repos {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "cnl-pty-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir(&path).unwrap();
        for repo in ["fixture-one", "fixture-two"] {
            assert!(
                Command::new("git")
                    .args(["init", "-q"])
                    .arg(path.join(repo))
                    .status()
                    .unwrap()
                    .success()
            );
        }
        Self(path)
    }
}

impl Drop for Repos {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn output_contains(session: &Session, wanted: &str) {
    output_matches(session, wanted, |output| output.contains(wanted));
}

fn output_matches(session: &Session, wanted: &str, matches: impl Fn(&str) -> bool) {
    let timeout = Instant::now() + Duration::from_secs(10);
    loop {
        let output = session.output.lock().unwrap().parser.screen().contents();
        if matches(&output) {
            return;
        }
        assert!(Instant::now() < timeout, "missing {wanted:?}: {output:?}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn wait_until(mut observed: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !observed() {
        assert!(Instant::now() < deadline, "PTY did not update");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn interactive_shells_keep_working_directories_and_environment_isolated() {
    let repos = Repos::new();
    let shell = if cfg!(windows) { "cmd" } else { "bash" };
    let first = fs::canonicalize(repos.0.join("fixture-one")).unwrap();
    let second = fs::canonicalize(repos.0.join("fixture-two")).unwrap();
    let mut manager = Manager::default();
    let size = pty_size((100, 28));
    let index = manager
        .attach("org/one", &first, Some(shell), size, false)
        .unwrap();
    let session = &manager.entries[index];
    let cwd = if cfg!(windows) { "cd\r" } else { "pwd\r" };
    assert!(session.send(cwd.as_bytes().to_vec()));
    output_contains(session, "fixture-one");
    let setup = if cfg!(windows) {
        "set CNL_PTY_ISOLATION=session_a\r"
    } else {
        "export CNL_PTY_ISOLATION=session_a\r"
    };
    assert!(session.send(setup.as_bytes().to_vec()));

    let second_index = manager
        .attach("org/two", &second, Some(shell), size, false)
        .unwrap();
    assert_ne!(index, second_index);
    let session = &manager.entries[second_index];
    assert!(session.send(cwd.as_bytes().to_vec()));
    output_contains(session, "fixture-two");
    let query = if cfg!(windows) {
        "echo RESULT:[%CNL_PTY_ISOLATION%]\r"
    } else {
        "echo RESULT:[$CNL_PTY_ISOLATION]\r"
    };
    let empty = if cfg!(windows) {
        "if not defined CNL_PTY_ISOLATION echo RESULT:em^pty\r"
    } else {
        "if [ -z \"$CNL_PTY_ISOLATION\" ]; then echo RESULT:\"empty\"; fi\r"
    };
    assert!(session.send(empty.as_bytes().to_vec()));
    output_contains(session, "RESULT:empty");

    let restored = manager
        .attach("org/one", &first, Some(shell), size, false)
        .unwrap();
    assert_eq!(restored, index);
    let session = &manager.entries[restored];
    assert!(session.send(query.as_bytes().to_vec()));
    output_contains(session, "RESULT:[session_a]");
}

#[cfg(windows)]
#[test]
fn powershell_opens_in_the_checkout_with_a_standard_windows_prompt() {
    if Command::new("pwsh.exe")
        .args(["-NoLogo", "-NoProfile", "-Command", "exit 0"])
        .output()
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        return;
    }

    let repos = Repos::new();
    let checkout = repos.0.join("fixture with spaces");
    fs::create_dir(&checkout).unwrap();
    let cwd = fs::canonicalize(checkout).unwrap();
    let visible = crate::workspace::user_path(&cwd);
    assert_ne!(cwd.to_str().unwrap(), visible);
    let session = Session::start(
        "fixture",
        &cwd,
        Some("powershell"),
        pty_size((240, 24)),
        false,
    )
    .unwrap();

    let prompt = format!("PS {visible}>");
    output_contains(&session, &prompt);
    assert_eq!(session.cwd, cwd);
}

#[test]
fn terminal_shows_back_in_footer_and_omits_scroll_hints() {
    use super::workspace::State;

    let workspace = Workspace {
        root: std::env::temp_dir(),
    };
    let flags = crate::cli::app().get_matches_from(["cnl"]);
    let manager = Manager::default();
    for width in [64, 125] {
        let size = (width, 24);
        let (events, _receiver) = std::sync::mpsc::sync_channel(1);
        let state = State::new(
            size,
            Catalog::new(&workspace, &flags, Some(&[])),
            false,
            events,
        );
        let layout = Layout::new(size, &manager.arrangement);
        let (frame, _) = render::frame(&manager, &layout, &state);
        let toolbar = frame.rows[0].contents();
        let footer = frame.rows.last().unwrap().contents();

        assert!(toolbar.contains("F5"));
        assert!(!toolbar.contains("F10"));
        assert!(footer.contains("F10 Back"));
        assert!(!footer.contains("‹/›"));
        assert!(!footer.contains("Shift+PgUp/PgDn"));
        if width >= 80 {
            assert!(footer.contains("F6/F7"));
        }
    }
}

#[test]
fn terminal_keys_keep_native_interrupts_and_cursor_modes() {
    let control_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(key_bytes(control_c, false).unwrap(), b"\x03");
    let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(key_bytes(up, false).unwrap(), b"\x1b[A");
    assert_eq!(key_bytes(up, true).unwrap(), b"\x1bOA");
    let vietnamese = KeyEvent::new(KeyCode::Char('ộ'), KeyModifiers::NONE);
    assert_eq!(key_bytes(vietnamese, false).unwrap(), "ộ".as_bytes());
}

#[test]
fn parsed_output_keeps_unicode_and_ansi_styles_without_host_escape_codes() {
    let mut output = Output::new(pty_size((24, 8)));
    output.parser.process(b"\x1b]52;c;dGVzdA==\x07");
    output.parser.process("\x1b[1;31mViệt 界\x1b[0m".as_bytes());
    let line = terminal_line(output.parser.screen(), 0, 24);
    assert_eq!(line.contents().trim_end(), "Việt 界");
    assert!(!line.contents().contains('\x1b'));
    assert!(matches!(
        line.segments().next(),
        Some((Tone::Pty { .. }, "Việt 界"))
    ));
}

#[test]
fn terminal_replies_are_routed_to_the_child() {
    let mut output = Output::new(pty_size((80, 25)));
    assert_eq!(
        output.process(b"\x1b[4;8H\x1b[6n"),
        vec![b"\x1b[4;8R".to_vec()]
    );
    assert!(output.process(b"\x1b]52;c;dGVzdA==\x07").is_empty());
}

#[test]
fn running_shell_observes_resize_and_reports_exit_code() {
    let repos = Repos::new();
    let repo = fs::canonicalize(repos.0.join("fixture-one")).unwrap();
    let shell = if cfg!(windows) { "cmd" } else { "bash" };
    let mut session =
        Session::start("org/one", &repo, Some(shell), pty_size((85, 26)), false).unwrap();
    let target_size = pty_size((112, 31));
    session.resize(target_size);
    wait_until(|| {
        let output = session.output.lock().unwrap();
        assert!(!output.resize_failed);
        output.parser.screen().size() == dimensions(target_size)
    });
    let command = if cfg!(windows) {
        "mode con\r"
    } else {
        "stty size\r"
    };
    assert!(session.send(command.as_bytes().to_vec()));
    output_contains(&session, if cfg!(windows) { "112" } else { "31 112" });
    assert!(session.send(b"exit 7\r".to_vec()));
    wait_until(|| {
        let output = session.output.lock().unwrap();
        output
            .exit
            .as_ref()
            .is_some_and(|code| code.as_ref().unwrap().exit_code() == 7)
    });
}

#[test]
fn workspace_reflows_split_and_compact_sidebar_without_changing_focus() {
    use super::layout::{Arrangement, Layout, Sidebar};

    let mut arrangement = Arrangement::default();
    arrangement.activate(1);
    arrangement.split(2);
    assert_eq!(arrangement.active(), Some(2));
    let wide = Layout::new((120, 28), &arrangement);
    let first = wide.panes[0].unwrap();
    let second = wide.panes[1].unwrap();
    let sidebar = wide.sidebar.unwrap();
    assert_eq!(first.body.y, second.body.y);
    assert_eq!(second.body.x, first.body.x + first.body.width + 1);
    assert_eq!(sidebar.x, second.body.x + second.body.width + 1);

    let narrow = Layout::new((60, 28), &arrangement);
    assert!(narrow.sidebar.is_none());
    assert_eq!(narrow.panes[0].unwrap().body.width, 60);
    assert!(narrow.panes[0].unwrap().body.y < narrow.panes[1].unwrap().body.y);

    let tiny = Layout::new((24, 6), &arrangement);
    assert!(tiny.panes[0].is_none());
    assert_eq!(tiny.panes[1].unwrap().id, 2);
    assert_eq!(arrangement.active(), Some(2));

    arrangement.sidebar = Sidebar::Visible;
    arrangement.sidebar_focused = true;
    let picker = Layout::new((24, 8), &arrangement);
    assert_eq!(picker.sidebar.unwrap().width, 24);
    assert!(picker.panes.iter().all(Option::is_none));
}

#[test]
fn moving_session_swaps_visible_panes_without_changing_active_identity() {
    use super::layout::Arrangement;
    let mut arrangement = Arrangement::default();
    arrangement.activate(1);
    arrangement.split(2);
    arrangement.move_pane();
    assert_eq!(arrangement.panes, [Some(2), Some(1)]);
    assert_eq!(arrangement.active(), Some(2));
    arrangement.activate(3);
    assert_eq!(arrangement.panes, [Some(3), Some(1)]);
    arrangement.close(3, Some(2));
    assert_eq!(arrangement.active(), Some(1));
    assert_eq!(arrangement.panes, [Some(1), None]);
}

#[test]
fn pane_mouse_coordinates_are_relative_to_its_actual_origin() {
    use crossterm::event::{MouseButton, MouseEvent};

    let mut output = Output::new(pty_size((30, 10)));
    output.parser.process(b"\x1b[?1000h\x1b[?1006h");
    let mouse = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 51,
        row: 7,
        modifiers: KeyModifiers::NONE,
    };
    let screen = output.parser.screen();
    assert_eq!(
        mouse_bytes(mouse, screen, (48, 5)).unwrap(),
        b"\x1b[<0;4;3M"
    );
    assert_eq!(mouse_bytes(mouse, screen, (52, 5)), None);
    assert_eq!(mouse_bytes(mouse, screen, (48, 8)), None);
}

#[cfg(windows)]
#[test]
fn conpty_workspace_operations_preserve_live_panes_and_close_only_owned_shell() {
    use crossterm::event::MouseButton;
    use std::sync::mpsc::{Receiver, sync_channel};
    use windows_sys::Win32::{
        Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };

    use super::{
        render,
        workspace::{Catalog, Command as Control, Modal, State, WorkspaceEvent},
    };

    fn finish_job(state: &mut State, manager: &mut Manager, receiver: &Receiver<WorkspaceEvent>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let event = receiver
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("terminal operation finished");
            if let WorkspaceEvent::Job(result) = event {
                assert!(result.is_ok(), "{:#}", result.as_ref().err().unwrap());
                state.complete(manager, result);
                return;
            }
        }
    }

    fn key(state: &mut State, manager: &mut Manager, key: KeyCode) {
        let layout = Layout::new(state.size, &manager.arrangement);
        assert!(!state.key(manager, &layout, KeyEvent::new(key, KeyModifiers::NONE)));
    }

    fn send(state: &mut State, manager: &mut Manager, command: &str) {
        state.paste(
            manager,
            &Layout::new(state.size, &manager.arrangement),
            command.to_owned(),
        );
        assert!(state.notice.is_none(), "{:?}", state.notice);
        key(state, manager, KeyCode::Enter);
    }

    fn witness(session: &Session) -> std::os::windows::io::OwnedHandle {
        use std::os::windows::io::FromRawHandle;
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, session.backend.process_id) };
        assert!(!handle.is_null());
        unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) }
    }

    fn process_state(handle: &std::os::windows::io::OwnedHandle, wait_ms: u32) -> u32 {
        use std::os::windows::io::AsRawHandle;
        unsafe { WaitForSingleObject(handle.as_raw_handle(), wait_ms) }
    }

    let repos = Repos::new();
    let workspace = Workspace {
        root: repos.0.clone(),
    };
    let flags = crate::cli::app().get_matches_from([
        std::ffi::OsStr::new("cnl"),
        std::ffi::OsStr::new("--state-dir"),
        repos.0.join("state").as_os_str(),
    ]);
    let rows: Vec<_> = ["fixture-one", "fixture-two"]
        .into_iter()
        .map(|name| {
            let path = fs::canonicalize(repos.0.join(name)).unwrap();
            Row {
                name: name.into(),
                selector: crate::workspace::user_path(&path).into_owned(),
                path: Some(path),
                shell: Some("cmd".into()),
                ..Row::default()
            }
        })
        .collect();
    let (events, receiver) = sync_channel(128);
    let mut state = State::new(
        (125, 30),
        Catalog::new(&workspace, &flags, Some(&rows)),
        false,
        events,
    );
    let mut manager = Manager::default();

    key(&mut state, &mut manager, KeyCode::F(1));
    finish_job(&mut state, &mut manager, &receiver);
    assert!(matches!(state.modal, Some(Modal::Repositories { .. })));
    key(&mut state, &mut manager, KeyCode::Down);
    key(&mut state, &mut manager, KeyCode::Enter);
    assert!(matches!(state.modal, Some(Modal::Shells { .. })));
    key(&mut state, &mut manager, KeyCode::Enter);
    finish_job(&mut state, &mut manager, &receiver);
    let original = manager.active().unwrap().id;
    let original_process = witness(manager.active().unwrap());
    assert_eq!(
        manager.active().unwrap().cwd,
        *rows[1].path.as_ref().unwrap()
    );
    assert_eq!(manager.active().unwrap().shell, "cmd");
    let cwd = manager.active().unwrap().cwd.clone();
    assert_eq!(
        manager
            .attach(
                "other selector",
                &cwd,
                Some("cmd"),
                pty_size(state.size),
                false
            )
            .unwrap(),
        0,
    );
    send(&mut state, &mut manager, "cd");
    let expected = crate::workspace::user_path(&cwd);
    output_matches(manager.active().unwrap(), &expected, |output| {
        output
            .lines()
            .any(|line| line.trim().eq_ignore_ascii_case(&expected))
    });
    send(&mut state, &mut manager, "set CNL_SPLIT_CHECK=original");

    key(&mut state, &mut manager, KeyCode::F(3));
    finish_job(&mut state, &mut manager, &receiver);
    let split = manager.active().unwrap().id;
    assert_ne!(split, original);
    let size = (124, 28);
    state.size = size;
    let layout = Layout::new(size, &manager.arrangement);
    manager.resize_visible(&layout);
    let first = layout.pane(original).unwrap();
    let second = layout.pane(split).unwrap();
    assert_ne!(first.body.width, second.body.width);
    for pane in [first, second] {
        let entry = manager.session(pane.id).unwrap();
        wait_until(|| {
            entry.output.lock().unwrap().parser.screen().size() == dimensions(pane.size())
        });
    }
    send(
        &mut state,
        &mut manager,
        "if not defined CNL_SPLIT_CHECK echo SPLIT:em^pty",
    );
    output_contains(manager.session(split).unwrap(), "SPLIT:empty");

    let (frame, hits) = render::frame(&manager, &layout, &state);
    let sidebar = layout.sidebar.unwrap();
    let heading = frame.rows[usize::from(sidebar.y)].contents();
    assert!(heading.contains("SESSIONS"));
    assert!(!heading.contains(&rows[1].selector));
    assert_eq!(
        hits.at(sidebar.x, sidebar.y + 1),
        Some(render::Hit::Session(original))
    );
    assert!(!state.mouse(
        &mut manager,
        &layout,
        &hits,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: first.body.x + 2,
            row: first.body.y + 1,
            modifiers: KeyModifiers::NONE,
        },
    ));
    assert_eq!(manager.active().unwrap().id, original);
    send(&mut state, &mut manager, "echo CHECK:[%CNL_SPLIT_CHECK%]");
    output_contains(manager.session(original).unwrap(), "CHECK:[original]");

    state.command(&mut manager, Control::MovePane);
    state.command(&mut manager, Control::MoveDown);
    assert_eq!(manager.arrangement.panes, [Some(split), Some(original)]);
    assert_eq!(
        manager
            .entries
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        [split, original]
    );
    assert_eq!(process_state(&original_process, 0), WAIT_TIMEOUT);

    key(&mut state, &mut manager, KeyCode::F(2));
    send(&mut state, &mut manager, "Kiểm tra 界");
    assert_eq!(manager.active().unwrap().title(), "Kiểm tra 界");
    assert_eq!(manager.active().unwrap().repository, "fixture-two");
    assert_eq!(
        manager.active().unwrap().cwd,
        *rows[1].path.as_ref().unwrap()
    );

    state.command(&mut manager, Control::Duplicate);
    finish_job(&mut state, &mut manager, &receiver);
    let duplicate = manager.active().unwrap().id;
    let duplicate_process = witness(manager.active().unwrap());
    assert_ne!(duplicate, original);
    assert_eq!(
        manager.active().unwrap().cwd,
        *rows[1].path.as_ref().unwrap()
    );
    assert_eq!(manager.active().unwrap().shell, "cmd");
    send(
        &mut state,
        &mut manager,
        "if not defined CNL_SPLIT_CHECK echo DUPE:em^pty",
    );
    output_contains(manager.session(duplicate).unwrap(), "DUPE:empty");

    key(&mut state, &mut manager, KeyCode::F(8));
    assert!(matches!(state.modal, Some(Modal::Confirm(id, Control::Close)) if id == duplicate));
    key(&mut state, &mut manager, KeyCode::Esc);
    assert_eq!(process_state(&duplicate_process, 0), WAIT_TIMEOUT);
    key(&mut state, &mut manager, KeyCode::F(8));
    key(&mut state, &mut manager, KeyCode::Enter);
    finish_job(&mut state, &mut manager, &receiver);
    assert!(manager.session(duplicate).is_none());
    assert_eq!(process_state(&duplicate_process, 2000), WAIT_OBJECT_0);
    assert_eq!(process_state(&original_process, 0), WAIT_TIMEOUT);

    manager.arrangement.activate(original);
    key(&mut state, &mut manager, KeyCode::F(9));
    key(&mut state, &mut manager, KeyCode::Enter);
    finish_job(&mut state, &mut manager, &receiver);
    finish_job(&mut state, &mut manager, &receiver);
    assert_eq!(manager.active().unwrap().id, original);
    assert_eq!(manager.active().unwrap().title(), "Kiểm tra 界");
    assert_eq!(process_state(&original_process, 2000), WAIT_OBJECT_0);
    send(
        &mut state,
        &mut manager,
        "if not defined CNL_SPLIT_CHECK echo RST:em^pty",
    );
    output_contains(manager.session(original).unwrap(), "RST:empty");
}
