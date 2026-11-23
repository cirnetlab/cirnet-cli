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
fn git_mutations_require_named_repositories() {
    for operation in ["clone", "update"] {
        assert!(
            cli::app()
                .try_get_matches_from(["cnl", "repos", operation])
                .is_err()
        );
        assert!(
            cli::app()
                .try_get_matches_from(["cnl", "repos", operation, "hardware", "--dry-run"])
                .is_ok()
        );
    }
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
