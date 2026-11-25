use super::*;

const RUN_ID: &str = "11111111111111111111111111111111";
const OTHER_RUN_ID: &str = "22222222222222222222222222222222";

struct Fixture {
    state: State,
    instance: Instance,
}

impl Fixture {
    fn new(case: &str) -> Self {
        let dir =
            build::output_dir(&std::env::temp_dir(), &format!("cnl-recovery-{case}")).unwrap();
        fs::create_dir(dir.join("data")).unwrap();
        fs::write(dir.join("data/run.lock"), "").unwrap();
        let ports = [0; 3].map(|_| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap());
        let profile = Profile {
            port: ports[0].local_addr().unwrap().port(),
            hardware_port: ports[1].local_addr().unwrap().port(),
            dev_port: ports[2].local_addr().unwrap().port(),
        };
        let instance = serde_json::from_value(json!({
            "profile":profile, "bundle":dir, "digest":"", "launcher_hash":"",
            "remote_hash":"", "fingerprint":"", "log_dir":dir.join("logs"),
            "supervisor":{"pid":u32::MAX,"created":100},
            "browser_server":null, "dev":false, "target":"interface",
            "native_stopped":false, "native_run_id":RUN_ID
        }))
        .unwrap();
        Self {
            state: State {
                dir,
                name: "fixture".into(),
                profile,
            },
            instance,
        }
    }

    fn save(&self) {
        write_json(&self.state.dir.join("instance.json"), &self.instance).unwrap();
    }

    fn result(&self, result: &str) {
        fs::write(
            self.state.dir.join("data").join(format!("result-{RUN_ID}")),
            result,
        )
        .unwrap();
    }

    fn control(&self, id: &str) {
        fs::write(
            self.state.dir.join("data/supervisor.json"),
            json!({"id":id}).to_string(),
        )
        .unwrap();
    }

    fn pin_bundle(&mut self) {
        let launcher = self.state.dir.join("cirnet-native.ps1");
        let manifest = self.state.dir.join("native-manifest.json");
        fs::write(&launcher, "# pinned fixture launcher").unwrap();
        fs::write(&manifest, "{}").unwrap();
        self.instance.launcher_hash = sha(&launcher).unwrap();
        self.instance.digest = sha(&manifest).unwrap();
    }

    fn reconcile(&self, boot: impl FnOnce() -> Option<u64>) -> Result<()> {
        let _lock = self.state.lock().unwrap();
        reconcile_exited(&self.state, &self.instance, boot)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.state.dir);
    }
}

#[test]
fn stop_reconciles_an_exited_supervisor_with_a_clean_result() {
    let fixture = Fixture::new("stop");
    fixture.save();
    fixture.result("clean");

    let report = stop(&fixture.state, Some(Target::Interface), false).unwrap();
    assert_eq!(report["state"], "stopped");
    assert!(read_instance(&fixture.state).unwrap().is_none());
    assert!(
        fixture
            .state
            .dir
            .join("data")
            .join(format!("result-{RUN_ID}"))
            .is_file()
    );
    assert_eq!(
        stop(&fixture.state, Some(Target::Interface), false).unwrap()["state"],
        "stopped"
    );
}

#[test]
fn run_reconciles_a_clean_result_before_candidate_selection() {
    let fixture = Fixture::new("run");
    fixture.save();
    fixture.result("clean");
    let workspace = Workspace {
        root: fixture.state.dir.clone(),
    };
    let args = crate::cli::app()
        .try_get_matches_from(["cnl", "run", "--desktop", "interface"])
        .unwrap();
    let error = start(
        &workspace,
        &fixture.state,
        args.subcommand_matches("run").unwrap(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("candidate.json"), "{error:#}");
    assert!(read_instance(&fixture.state).unwrap().is_none());
}

#[test]
fn restart_recovers_only_an_interrupted_run_with_matching_evidence() {
    let mut fixture = Fixture::new("restart");
    fixture.pin_bundle();
    fixture.save();
    fixture.control(OTHER_RUN_ID);
    assert!(fixture.reconcile(|| panic!("mismatched run")).is_err());
    fixture.control(RUN_ID);
    assert!(fixture.reconcile(|| None).is_err());
    assert!(fixture.reconcile(|| Some(100)).is_err());
    assert!(read_instance(&fixture.state).unwrap().is_some());
    fixture.reconcile(|| Some(101)).unwrap();
    assert!(read_instance(&fixture.state).unwrap().is_none());
    assert_eq!(native_run_id(&fixture.state).unwrap(), RUN_ID);
}

#[test]
fn failed_or_conflicting_launcher_evidence_keeps_the_record() {
    let mut fixture = Fixture::new("failed");
    fixture.pin_bundle();
    fixture.save();
    fixture.control(RUN_ID);
    for result in ["failed", "clean\n", "clean"] {
        fixture.result(result);
        assert!(
            fixture
                .reconcile(|| panic!("receipt already present"))
                .is_err()
        );
        assert!(read_instance(&fixture.state).unwrap().is_some());
    }
    fs::remove_file(fixture.state.dir.join("data/supervisor.json")).unwrap();
    fs::write(
        fixture
            .state
            .dir
            .join("data")
            .join(format!("stop-{RUN_ID}")),
        "",
    )
    .unwrap();
    assert!(fixture.reconcile(|| panic!("stop still pending")).is_err());
    assert!(read_instance(&fixture.state).unwrap().is_some());
}

#[test]
fn native_lock_and_listeners_protect_live_or_orphaned_instances() {
    let fixture = Fixture::new("ownership");
    fixture.save();
    fixture.result("clean");
    let lock = ownership::open_native_run_lock(&fixture.state.dir.join("data/run.lock")).unwrap();
    assert!(fixture.reconcile(|| panic!("native lock held")).is_err());
    drop(lock);
    let listener =
        TcpListener::bind((Ipv4Addr::LOCALHOST, fixture.instance.profile.hardware_port)).unwrap();
    assert!(
        fixture
            .reconcile(|| panic!("matching clean receipt"))
            .is_err()
    );
    assert!(read_instance(&fixture.state).unwrap().is_some());
    drop(listener);
    fixture
        .reconcile(|| panic!("matching clean receipt"))
        .unwrap();
}
