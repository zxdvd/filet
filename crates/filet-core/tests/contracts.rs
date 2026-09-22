use filet_core::{
    config::Loaded,
    daemon::Engine,
    executor,
    planner::{self, NoScripts},
    snapshot,
    state::{ExecutionLock, Store},
    *,
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, time::Duration};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    config: Loaded,
    store: Store,
}
impl Fixture {
    fn new(actions: Value) -> Self {
        Self::with_rule(
            json!({"id":"first","on":{"type":"file.ready","source":"inbox"},"when":{"extension":"pdf"},"actions":actions}),
        )
    }
    fn with_rule(rule: Value) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir(root.join("inbox")).unwrap();
        let text = json!({"schemaVersion":1,"sources":{"inbox":{"path":"inbox","ready":{"stableFor":"1ms","retryFor":"20ms"},"reconcileEvery":"1ms"}},"rules":[rule]});
        fs::write(root.join("filet.yaml"), serde_json::to_vec(&text).unwrap()).unwrap();
        let config = Loaded::load(&root.join("filet.yaml")).unwrap();
        let store = Store::open(&root.join("state")).unwrap();
        Self {
            _temp: temp,
            root,
            config,
            store,
        }
    }
    fn file(&self) -> PathBuf {
        let p = self.root.join("inbox/报告.PDF");
        fs::write(&p, b"original bytes").unwrap();
        p
    }
    fn plan(&self, p: &std::path::Path) -> Plan {
        let p = planner::plan(&self.config, &NoScripts, p, "manual", None)
            .unwrap()
            .unwrap();
        self.store.save(&p).unwrap();
        p
    }
}
#[test]
fn copy_then_rename_then_move_preserves_file_ref_and_is_idempotent() {
    let f = Fixture::new(
        json!([{"copy":{"to":"backup"}},{"rename":{"name":"renamed.pdf"}},{"move":{"to":"archive"}}]),
    );
    let input = f.file();
    let plan = f.plan(&input);
    assert!(input.exists());
    assert!(!f.root.join("backup").exists());
    executor::apply(&f.store, &f.config, &plan.plan_id).unwrap();
    assert_eq!(
        fs::read(f.root.join("backup/报告.PDF")).unwrap(),
        b"original bytes"
    );
    assert_eq!(
        fs::read(f.root.join("archive/renamed.pdf")).unwrap(),
        b"original bytes"
    );
    assert!(!input.exists());
    assert_eq!(
        executor::apply(&f.store, &f.config, &plan.plan_id).unwrap()["reused"],
        true
    );
}
#[test]
fn changed_content_even_with_same_size_and_mtime_rejects_plan() {
    let f = Fixture::new(json!([{"move":{"to":"archive"}}]));
    let input = f.file();
    let plan = f.plan(&input);
    let mtime = filetime::FileTime::from_last_modification_time(&fs::metadata(&input).unwrap());
    fs::write(&input, b"changed! bytes").unwrap();
    filetime::set_file_mtime(&input, mtime).unwrap();
    assert_eq!(
        executor::apply(&f.store, &f.config, &plan.plan_id)
            .unwrap_err()
            .code,
        "PLAN_STALE"
    );
    assert!(input.exists());
}
#[test]
fn destination_created_after_plan_is_not_overwritten() {
    let f = Fixture::new(json!([{"move":{"to":"archive"}}]));
    let input = f.file();
    let plan = f.plan(&input);
    fs::create_dir(f.root.join("archive")).unwrap();
    fs::write(f.root.join("archive/报告.PDF"), b"other owner").unwrap();
    assert_eq!(
        executor::apply(&f.store, &f.config, &plan.plan_id)
            .unwrap_err()
            .code,
        "TARGET_CONFLICT"
    );
    assert!(input.exists());
    assert_eq!(
        fs::read(f.root.join("archive/报告.PDF")).unwrap(),
        b"other owner"
    );
}
#[test]
fn skipped_move_stops_dependent_actions() {
    let f = Fixture::new(
        json!([{"move":{"to":"archive","onConflict":"skip"}},{"rename":{"name":"bad.pdf"}}]),
    );
    let input = f.file();
    let plan = f.plan(&input);
    fs::create_dir(f.root.join("archive")).unwrap();
    fs::write(f.root.join("archive/报告.PDF"), b"existing").unwrap();
    executor::apply(&f.store, &f.config, &plan.plan_id).unwrap();
    assert!(input.exists());
    assert!(!f.root.join("archive/bad.pdf").exists());
}
#[test]
fn single_executor_lock() {
    let t = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(t.path()).unwrap();
    let _one = ExecutionLock::acquire(&root).unwrap();
    assert!(ExecutionLock::acquire(&root).is_err());
}

#[test]
fn published_schemas_match_rust_contracts() {
    let current = filet_core::schemas();
    let config: Value =
        serde_json::from_str(include_str!("../../../schemas/config-v1.schema.json")).unwrap();
    let plan: Value =
        serde_json::from_str(include_str!("../../../schemas/action-plan-v1.schema.json")).unwrap();
    assert_eq!(current["configuration"], config);
    assert_eq!(current["actionPlan"], plan);
}

#[test]
fn helper_edits_change_revision_while_loaded_package_stays_frozen() {
    let f = Fixture::new(json!([]));
    fs::create_dir(f.root.join("rules")).unwrap();
    fs::write(
        f.root.join("rules/main.mjs"),
        "export function run(){return []}",
    )
    .unwrap();
    fs::write(f.root.join("rules/helper.mjs"), "export const x=1").unwrap();
    let mut config: Value = serde_json::from_slice(&fs::read(&f.config.path).unwrap()).unwrap();
    config["rules"][0]
        .as_object_mut()
        .unwrap()
        .remove("actions");
    config["rules"][0].as_object_mut().unwrap().remove("when");
    config["rules"][0]["script"] = json!({"file":"rules/main.mjs","apiVersion":1});
    fs::write(&f.config.path, config.to_string()).unwrap();
    let before = Loaded::load(&f.config.path).unwrap();
    fs::write(f.root.join("rules/helper.mjs"), "export const x=2").unwrap();
    let after = Loaded::load(&f.config.path).unwrap();
    assert_ne!(before.revision, after.revision);
    assert_eq!(
        before.packages["first"].modules["helper.mjs"],
        "export const x=1"
    );
}
#[test]
fn completed_move_before_database_ack_is_recovered_without_replay() {
    let f = Fixture::new(json!([{"move":{"to":"archive"}}]));
    let input = f.file();
    let p = f.plan(&input);
    let to = f.root.join("archive/报告.PDF");
    fs::create_dir(to.parent().unwrap()).unwrap();
    let r = Receipt {
        before: p.input_snapshot.clone(),
        after: None,
        temp: None,
        skipped: false,
        output: None,
    };
    f.store.set_status(&p.plan_id, "applying", None).unwrap();
    f.store
        .operation(&p.plan_id, 0, "started", &r, None)
        .unwrap();
    fs::hard_link(&input, &to).unwrap();
    fs::remove_file(&input).unwrap();
    executor::recover(&f.store, &f.config).unwrap();
    assert_eq!(f.store.status(&p.plan_id).unwrap(), "succeeded");
    assert_eq!(fs::read(to).unwrap(), b"original bytes");
}
#[test]
fn incomplete_move_keeps_both_files_for_review() {
    let f = Fixture::new(json!([{"move":{"to":"archive"}}]));
    let input = f.file();
    let p = f.plan(&input);
    let to = f.root.join("archive/报告.PDF");
    fs::create_dir(to.parent().unwrap()).unwrap();
    let r = Receipt {
        before: p.input_snapshot.clone(),
        after: None,
        temp: None,
        skipped: false,
        output: None,
    };
    f.store.set_status(&p.plan_id, "applying", None).unwrap();
    f.store
        .operation(&p.plan_id, 0, "started", &r, None)
        .unwrap();
    fs::hard_link(&input, &to).unwrap();
    executor::recover(&f.store, &f.config).unwrap();
    assert_eq!(f.store.status(&p.plan_id).unwrap(), "needs_review");
    assert!(input.exists() && to.exists());
}
#[test]
fn same_content_from_other_file_is_not_proof_of_ownership() {
    let f = Fixture::new(json!([{"copy":{"to":"backup"}}]));
    let input = f.file();
    let p = f.plan(&input);
    let to = f.root.join("backup/报告.PDF");
    fs::create_dir(to.parent().unwrap()).unwrap();
    fs::write(&to, b"original bytes").unwrap();
    let r = Receipt {
        before: p.input_snapshot.clone(),
        after: None,
        temp: None,
        skipped: false,
        output: None,
    };
    assert!(executor::recover_step(&p.actions[0], &r).unwrap().is_none());
}
#[test]
fn first_nonempty_plan_wins_in_configuration_order() {
    let mut f = Fixture::new(json!([]));
    let mut c: Value = serde_json::from_slice(&fs::read(&f.config.path).unwrap()).unwrap();
    c["rules"].as_array_mut().unwrap().extend([json!({"id":"second","on":{"type":"file.ready","source":"inbox"},"actions":[{"move":{"to":"second"}}]}),json!({"id":"third","on":{"type":"file.ready","source":"inbox"},"actions":[{"move":{"to":"third"}}]})]);
    fs::write(&f.config.path, c.to_string()).unwrap();
    f.config = Loaded::load(&f.config.path).unwrap();
    assert_eq!(f.plan(&f.file()).rule_id, "second");
}
#[test]
fn revision_change_invalidates_a_saved_plan() {
    let f = Fixture::new(json!([{"copy":{"to":"backup"}}]));
    let p = f.plan(&f.file());
    let mut config = f.config.clone();
    config.revision = "new".into();
    assert_eq!(
        executor::apply(&f.store, &config, &p.plan_id)
            .unwrap_err()
            .code,
        "PLAN_STALE"
    );
}
#[test]
fn distinct_identical_files_have_distinct_plan_identities() {
    let f = Fixture::new(json!([{"copy":{"to":"backup"}}]));
    let a = f.file();
    let b = f.root.join("inbox/other.pdf");
    fs::copy(&a, &b).unwrap();
    assert_ne!(f.plan(&a).plan_id, f.plan(&b).plan_id);
}
#[test]
fn extension_and_dotfile_contract() {
    let f = Fixture::new(json!([]));
    for (name, stem, ext) in [
        ("report.PDF", "report", "pdf"),
        (".profile", ".profile", ""),
        ("a.tar.gz", "a.tar", "gz"),
        ("plain", "plain", ""),
    ] {
        let p = f.root.join("inbox").join(name);
        fs::write(&p, b"").unwrap();
        let s = snapshot::take(&p).unwrap();
        let c = snapshot::context(&s, "manual", chrono::Utc::now()).unwrap();
        assert_eq!(c.file.extension, ext);
        assert_eq!(c.file.stem, stem);
    }
}
#[test]
fn strict_yaml_rejects_duplicates_unknown_fields_tags_merges_and_legacy_booleans() {
    let f = Fixture::new(json!([]));
    let valid = "schemaVersion: 1\nsources:\n  inbox:\n    path: inbox\nrules: []\n";
    for text in [
        format!("{valid}schemaVersion: 1\n"),
        format!("{valid}typo: true\n"),
        valid.replace("path: inbox", "path: inbox\n    recursive: yes"),
        valid.replace("path: inbox", "path: !custom inbox"),
        valid.replace("path: inbox", "path: inbox\n    <<: {recursive: true}"),
        valid.replace("path: inbox", "path: inbox\n    path: other"),
    ] {
        fs::write(&f.config.path, text).unwrap();
        assert!(Loaded::load(&f.config.path).is_err());
    }
}
#[test]
fn documented_yaml_mapping_action_parses() {
    let f = Fixture::new(json!([]));
    fs::write(&f.config.path,"schemaVersion: 1\nsources:\n  inbox:\n    path: inbox\nrules:\n  - id: pdf\n    on:\n      type: file.ready\n      source: inbox\n    when:\n      extension: pdf\n    actions:\n      - move:\n          to: archive\n          onConflict: error\n").unwrap();
    assert!(Loaded::load(&f.config.path).is_ok());
}
#[test]
fn initial_baseline_restart_and_missed_events() {
    let f = Fixture::new(json!([{"move":{"to":"archive"}}]));
    let old = f.file();
    let mut engine = Engine::new();
    engine.tick(&f.store, &f.config, &NoScripts).unwrap();
    std::thread::sleep(Duration::from_millis(3));
    engine.tick(&f.store, &f.config, &NoScripts).unwrap();
    assert!(old.exists());
    let new = f.root.join("inbox/new.pdf");
    fs::write(&new, b"new").unwrap();
    let mut restarted = Engine::new();
    restarted.tick(&f.store, &f.config, &NoScripts).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    restarted.tick(&f.store, &f.config, &NoScripts).unwrap();
    assert!(!new.exists());
    assert!(old.exists());
    assert!(f.root.join("archive/new.pdf").exists());
}
#[test]
fn scheduled_scan_reconsiders_unchanged_no_match() {
    let f = Fixture::with_rule(
        json!({"id":"aged","on":{"type":"scan","source":"inbox","every":"1ms"},"when":{"modifiedOlderThan":"30ms"},"actions":[{"move":{"to":"archive"}}]}),
    );
    let input = f.file();
    let mut engine = Engine::new();
    engine.tick(&f.store, &f.config, &NoScripts).unwrap();
    std::thread::sleep(Duration::from_millis(4));
    engine.tick(&f.store, &f.config, &NoScripts).unwrap();
    assert!(input.exists());
    std::thread::sleep(Duration::from_millis(35));
    engine.tick(&f.store, &f.config, &NoScripts).unwrap();
    assert!(!input.exists());
}
#[test]
fn own_outputs_do_not_start_another_rule_chain() {
    let f = Fixture::new(json!([{"rename":{"name":"new.pdf"}}]));
    let input = f.file();
    let p = f.plan(&input);
    executor::apply(&f.store, &f.config, &p.plan_id).unwrap();
    let mut engine = Engine::new();
    for _ in 0..3 {
        engine.tick(&f.store, &f.config, &NoScripts).unwrap();
        std::thread::sleep(Duration::from_millis(3));
    }
    assert_eq!(f.store.history(10).unwrap().as_array().unwrap().len(), 1);
}
#[test]
fn unavailable_source_does_not_erase_observations_or_restart_baseline() {
    let f = Fixture::new(json!([]));
    f.file();
    let mut engine = Engine::new();
    engine.tick(&f.store, &f.config, &NoScripts).unwrap();
    fs::rename(f.root.join("inbox"), f.root.join("offline")).unwrap();
    engine.mark_dirty();
    engine.tick(&f.store, &f.config, &NoScripts).unwrap();
    let count: i64 = f
        .store
        .conn
        .query_row("SELECT count(*) FROM observations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    let initialized: bool = f
        .store
        .conn
        .query_row("SELECT initialized FROM sources", [], |r| r.get(0))
        .unwrap();
    assert!(initialized);
}
#[cfg(unix)]
#[test]
fn symlink_input_and_target_ancestors_are_rejected() {
    let f = Fixture::new(json!([]));
    let input = f.file();
    std::os::unix::fs::symlink(&input, f.root.join("inbox/link.pdf")).unwrap();
    assert!(snapshot::take(&f.root.join("inbox/link.pdf")).is_err());
    std::os::unix::fs::symlink(f.root.join("inbox"), f.root.join("link")).unwrap();
    assert!(paths::expand(&f.root, "link/x").is_err());
}
#[cfg(unix)]
#[test]
fn late_file_argument_tracks_move_and_external_command_runs_once() {
    let f = Fixture::new(
        json!([{"move":{"to":"archive"}},{"exec":{"program":"/bin/sh","args":["-c","printf '%s' \"$1\"; printf x >> count","--",{"pathOf":"input"}],"timeout":"1s"}}]),
    );
    let p = f.plan(&f.file());
    executor::apply(&f.store, &f.config, &p.plan_id).unwrap();
    executor::apply(&f.store, &f.config, &p.plan_id).unwrap();
    assert_eq!(fs::read(f.root.join("count")).unwrap(), b"x");
    let (_, receipt) = f.store.get_operation(&p.plan_id, 1).unwrap().unwrap();
    assert_eq!(
        receipt.output.unwrap()["stdout"],
        f.root.join("archive/报告.PDF").to_str().unwrap()
    );
}
#[cfg(unix)]
#[test]
fn timeout_and_started_exec_are_never_retried() {
    let f = Fixture::new(
        json!([{"exec":{"program":"/bin/sh","args":["-c","printf x >> count; sleep 5"],"env":{"PATH":"/usr/bin:/bin"},"timeout":"30ms"}}]),
    );
    let p = f.plan(&f.file());
    let start = std::time::Instant::now();
    assert!(executor::apply(&f.store, &f.config, &p.plan_id).is_err());
    assert!(start.elapsed() < Duration::from_secs(3));
    assert_eq!(f.store.status(&p.plan_id).unwrap(), "needs_review");
    assert!(executor::apply(&f.store, &f.config, &p.plan_id).is_err());
    assert_eq!(fs::read(f.root.join("count")).unwrap(), b"x");
}
#[cfg(unix)]
#[test]
fn crashed_external_command_is_conservatively_reviewed() {
    let f = Fixture::new(json!([{"exec":{"program":"/bin/echo","args":[]}}]));
    let p = f.plan(&f.file());
    let r = Receipt {
        before: p.input_snapshot.clone(),
        after: None,
        temp: None,
        skipped: false,
        output: None,
    };
    f.store.set_status(&p.plan_id, "applying", None).unwrap();
    f.store
        .operation(&p.plan_id, 0, "started", &r, None)
        .unwrap();
    executor::recover(&f.store, &f.config).unwrap();
    assert_eq!(f.store.status(&p.plan_id).unwrap(), "needs_review");
}
