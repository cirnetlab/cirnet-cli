use cirnet_cli::{cli, runtime::verify_ports, workspace::Repo};

#[test]
fn build_scope_is_explicit_and_orders_dependencies() {
    let app = cli::app();
    assert!(app.clone().try_get_matches_from(["cnl", "build"]).is_err());
    assert!(
        app.clone()
            .try_get_matches_from(["cnl", "build", "--all", "--repo", "remote"])
            .is_err()
    );
    assert_eq!(
        Repo::ordered(&[Repo::Remote, Repo::Simulator]),
        [
            Repo::Contracts,
            Repo::Simulator,
            Repo::Hardware,
            Repo::Remote
        ]
    );
    assert!(
        app.clone()
            .try_get_matches_from(["cnl", "build", "--all"])
            .is_ok()
    );
    assert!(
        app.clone()
            .try_get_matches_from(["cnl", "build", "--all", "--reuse-candidate"])
            .is_err()
    );
    assert!(
        app.try_get_matches_from([
            "cnl",
            "build",
            "--all",
            "--reuse-candidate",
            "--verification-evidence",
            "result.json",
            "--verification-sha256",
            &"a".repeat(64),
        ])
        .is_ok()
    );
}

#[test]
fn git_mutations_require_one_explicit_scope() {
    for operation in ["clone", "fetch", "add", "remove"] {
        assert!(
            cli::app()
                .try_get_matches_from(["cnl", "repos", operation])
                .is_err()
        );
    }
    assert!(
        cli::app()
            .try_get_matches_from(["cnl", "repos", "update"])
            .is_ok()
    );
    for operation in ["clone", "update"] {
        assert!(
            cli::app()
                .try_get_matches_from([
                    "cnl",
                    "repos",
                    operation,
                    "hardware",
                    "remote",
                    "--dry-run"
                ])
                .is_ok()
        );
        assert!(
            cli::app()
                .try_get_matches_from(["cnl", "repos", operation, "--all", "--dry-run"])
                .is_ok()
        );
        assert!(
            cli::app()
                .try_get_matches_from(["cnl", "repos", operation, "remote", "--all"])
                .is_err()
        );
    }
}

#[test]
fn binary_resolves_runtime_targets_and_modes_without_starting_services() {
    let workspace = std::env::temp_dir();
    let state = workspace.join(format!("cnl-dry-run-fixture-{}", std::process::id()));
    let run = |args: &[&str]| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_cnl"))
            .arg("--workspace")
            .arg(&workspace)
            .arg("--state-dir")
            .arg(&state)
            .args(["--json"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    for (args, mode, open) in [
        (&["run"][..], "development", false),
        (&["run", "--desktop", "interface"][..], "development", true),
        (
            &["run", "--desktop", "interface", "--mode", "packaged"][..],
            "packaged",
            true,
        ),
        (&["dev", "--mode", "development"][..], "development", false),
    ] {
        let args = args
            .iter()
            .copied()
            .chain(["--dry-run"])
            .collect::<Vec<_>>();
        let report = run(&args);
        assert_eq!(report["mode"], mode);
        assert_eq!(report["target"], "interface");
        assert_eq!(report["openBrowser"], open);
    }
    let preview = run(&["stop", "--desktop", "interface", "--dry-run"]);
    assert_eq!(preview["target"], "interface");
    assert_eq!(preview["ownedInstance"], false);
    assert!(!state.exists());
    assert!(
        cli::app()
            .try_get_matches_from(["cnl", "dev", "--mode", "packaged"])
            .is_err()
    );
}

#[test]
fn binary_help_localizes_options_and_hides_compatibility_commands() {
    let cnl = env!("CARGO_BIN_EXE_cnl");
    let help = |args: &[&str]| {
        let output = std::process::Command::new(cnl).args(args).output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    };
    let root = help(&["--help"]);
    assert!(root.contains("completion"));
    assert!(
        !root
            .lines()
            .any(|line| line.trim_start().starts_with("dev "))
    );
    let vi = help(&["run", "--lang", "vi", "--help"]);
    assert!(vi.contains("Cách dùng"));
    assert!(vi.contains("Chọn chế độ chạy Interface"));
    assert!(vi.contains("Chọn hồ sơ runtime"));
    assert!(vi.contains("Hiển thị trợ giúp"));
    assert!(help(&["--lang=en", "run", "--help"]).contains("Choose the Interface runtime mode"));
}

#[test]
fn binary_localizes_application_diagnostics_without_requiring_a_profile() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_cnl"))
        .arg("--workspace")
        .arg(std::env::temp_dir())
        .args(["--state-dir", "relative-state", "--lang", "vi", "status"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("đường dẫn tuyệt đối"));
}

#[test]
fn vietnamese_help_and_parser_diagnostics_localize_the_cli_grammar() {
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_cnl"))
            .args(args)
            .output()
            .unwrap()
    };
    let help = run(&["--lang", "vi", "help", "env"]);
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("Cách dùng"));
    assert!(help.contains("Hiển thị trợ giúp cho lệnh"));
    assert!(help.contains("[mặc định: windows-x64]"));
    assert!(help.contains("[giá trị hợp lệ: en, vi]"));
    assert!(!help.contains("Print this message"));
    let tui = run(&["tui", "--lang", "vi", "--help"]);
    assert!(tui.status.success());
    let tui = String::from_utf8(tui.stdout).unwrap();
    assert!(tui.contains("--persist-history"));
    assert!(tui.contains("Lưu lịch sử lệnh"));
    for args in [
        &["cnl", "--persist-history"][..],
        &["cnl", "tui", "--persist-history"][..],
    ] {
        let matches = cli::app().try_get_matches_from(args).unwrap();
        assert!(matches.get_flag("persist-history"));
    }

    for (args, expected) in [
        (
            &["run", "--mode", "another-mode"][..],
            "Giá trị không hợp lệ",
        ),
        (&["run", "--port", "80"][..], "Giá trị không hợp lệ"),
        (&["build"][..], "Thiếu đối số bắt buộc"),
        (&["repos"][..], "cần terminal tương tác"),
        (&["unknown-command"][..], "Lệnh con không hợp lệ"),
    ] {
        let output = run(&[&["--lang", "vi"], args].concat());
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains(expected), "{args:?}: {error}");
        if args != ["repos"] {
            assert!(error.contains("--help"));
        }
    }
    let invalid = run(&["--lang", "en", "run", "--mode", "another-mode"]);
    let invalid = String::from_utf8(invalid.stderr).unwrap();
    assert!(invalid.contains("another-mode") && invalid.contains("invalid value"));
}

#[test]
fn human_reports_localize_nested_fields_and_leave_json_language_independent() {
    use std::{fs, path::PathBuf, process::Output};

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir(&self.0);
        }
    }
    let fixture = Fixture(std::env::temp_dir().join(format!("cnl-locales-{}", std::process::id())));
    fs::create_dir(&fixture.0).unwrap();
    let state = fixture.0.join("state");
    let run = |lang, json, args: &[&str]| -> Output {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_cnl"));
        command
            .env_remove("CNL_TUI_REPORT")
            .arg("--workspace")
            .arg(&fixture.0)
            .arg("--state-dir")
            .arg(&state)
            .args(["--lang", lang]);
        if json {
            command.arg("--json");
        }
        command.args(args).output().unwrap()
    };
    for (args, english, vietnamese) in [
        (
            &["doctor"][..],
            "Supported platform",
            "Nền tảng được hỗ trợ",
        ),
        (
            &["env", "check"][..],
            "Hardware RPC port",
            "Cổng Hardware RPC",
        ),
        (&["repos", "status"][..], "Missing", "Thiếu checkout"),
        (
            &["repos", "clone", "cli", "--dry-run"][..],
            "Clone",
            "Sao chép checkout",
        ),
        (
            &["build", "--all", "--dry-run"][..],
            "Build order",
            "Thứ tự bản dựng",
        ),
        (&["run", "--dry-run"][..], "Development", "Phát triển"),
        (&["status"][..], "Stopped", "Đã dừng"),
        (
            &["stop", "--dry-run"][..],
            "Owned instance",
            "Phiên do CLI quản lý",
        ),
        (
            &["verify", "--dry-run"][..],
            "Integration checks",
            "Kiểm thử tích hợp",
        ),
    ] {
        for (lang, expected) in [("en", english), ("vi", vietnamese)] {
            let stdout = String::from_utf8(run(lang, false, args).stdout).unwrap();
            assert!(stdout.contains(expected), "{args:?} ({lang}): {stdout}");
            assert!(!stdout.contains("\"hardware_port\""), "{args:?}: {stdout}");
        }
        let structured = |lang| -> serde_json::Value {
            let stdout = run(lang, true, args).stdout;
            serde_json::from_slice(&stdout).unwrap_or_else(|error| panic!("{args:?}: {error}"))
        };
        let en = structured("en");
        assert_eq!(en, structured("vi"), "{args:?}");
        if args[0] == "run" {
            assert_eq!(en["mode"], "development");
            assert_eq!(en["ports"]["hardware_port"], 50051);
        }
        if args[0] == "repos" && args[1] == "status" {
            assert_eq!(en["repositories"][0]["state"], "missing");
        }
    }
    let transport = |json| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_cnl"));
        command
            .env("CNL_TUI_REPORT", "ReportFixture123")
            .arg("--workspace")
            .arg(&fixture.0)
            .arg("--lang=vi");
        if json {
            command.arg("--json");
        }
        let stdout = command.args(["repos", "status"]).output().unwrap().stdout;
        String::from_utf8(stdout).unwrap()
    };
    let wire = transport(false);
    let report: serde_json::Value = serde_json::from_str(
        wire.strip_prefix("CNL_TUI_REPORT ReportFixture123 ")
            .unwrap(),
    )
    .unwrap();
    let diagnostic = &report["repositories"][0]["detail"]["$cnlDiagnostic"];
    assert!(diagnostic[0].as_str().unwrap().contains("Checkout missing"));
    assert!(diagnostic[1].as_str().unwrap().contains("Thiếu checkout"));
    let direct: serde_json::Value = serde_json::from_str(&transport(true)).unwrap();
    assert_eq!(direct["repositories"][0]["detail"], diagnostic[0]);
    assert!(!state.exists());
}

#[test]
fn a_port_in_use_fails_preflight() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    assert!(verify_ports(&[listener.local_addr().unwrap().port()]).is_err());
}

#[test]
fn lifecycle_supports_explicit_database_consent_and_output() {
    assert!(
        cli::app()
            .try_get_matches_from(["cnl", "run", "--initialize-db", "--json", "--dry-run"])
            .is_ok()
    );
    assert!(
        cli::app()
            .try_get_matches_from(["cnl", "run", "--port", "80"])
            .is_err()
    );
}

#[test]
fn desktop_interface_is_supported_by_both_runtime_commands() {
    for command in ["run", "dev"] {
        let matches = cli::app()
            .try_get_matches_from(["cnl", command, "--desktop", "interface", "--dry-run"])
            .unwrap();
        assert_eq!(
            matches
                .subcommand()
                .unwrap()
                .1
                .get_one::<String>("desktop")
                .map(String::as_str),
            Some("interface")
        );
        assert!(
            cli::app()
                .try_get_matches_from(["cnl", command, "--desktop", "unknown"])
                .is_err()
        );
    }
}
