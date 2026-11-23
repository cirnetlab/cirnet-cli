use super::*;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
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
    let timeout = Instant::now() + Duration::from_secs(10);
    loop {
        let output = session.output.lock().unwrap().parser.screen().contents();
        if output.contains(wanted) {
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
    let first = crate::workspace::canonical(&repos.0.join("fixture-one")).unwrap();
    let second = crate::workspace::canonical(&repos.0.join("fixture-two")).unwrap();
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
    let repo = crate::workspace::canonical(&repos.0.join("fixture-one")).unwrap();
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
    output_contains(&session, if cfg!(windows) { "112" } else { "28 112" });
    assert!(session.send(b"exit 7\r".to_vec()));
    wait_until(|| {
        let output = session.output.lock().unwrap();
        output
            .exit
            .as_ref()
            .is_some_and(|code| code.as_ref().unwrap().exit_code() == 7)
    });
}
