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
    assert!(app.try_get_matches_from(["cnl", "build", "--all"]).is_ok());
}

#[test]
fn git_mutations_require_one_explicit_scope() {
    for operation in ["clone", "update"] {
        assert!(
            cli::app()
                .try_get_matches_from(["cnl", "repos", operation])
                .is_err()
        );
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
