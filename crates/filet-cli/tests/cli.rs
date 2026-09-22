use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let t = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(t.path()).unwrap();
        fs::create_dir(root.join("inbox")).unwrap();
        let config = json!({"schemaVersion":1,"sources":{"inbox":{"path":"inbox","ready":{"stableFor":"20ms","retryFor":"2s"},"reconcileEvery":"100ms"}},"rules":[{"id":"archive","on":{"type":"file.ready","source":"inbox"},"actions":[{"move":{"to":"archive"}}]}]});
        fs::write(root.join("filet.yaml"), config.to_string()).unwrap();
        Self { _temp: t, root }
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_filet"));
        cmd.current_dir(&self.root)
            .args(["--json", "--data-dir", "state"]);
        cmd
    }
    fn run(&self, args: &[&str]) -> (bool, Value) {
        let out = self.command().args(args).output().unwrap();
        (
            out.status.success(),
            serde_json::from_slice(&out.stdout)
                .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout))),
        )
    }
}
#[test]
fn cli_plan_apply_history_and_error_envelopes() {
    let f = Fixture::new();
    fs::write(f.root.join("inbox/a.pdf"), b"a").unwrap();
    assert!(f.run(&["check"]).0);
    let (ok, p) = f.run(&["plan", "inbox/a.pdf"]);
    assert!(ok, "{p}");
    assert!(f.root.join("inbox/a.pdf").exists());
    let id = p["data"]["plan"]["planId"].as_str().unwrap();
    assert!(f.run(&["apply", id]).0);
    assert!(f.root.join("archive/a.pdf").exists());
    assert_eq!(f.run(&["apply", id]).1["data"]["reused"], true);
    assert_eq!(f.run(&["history"]).1["data"][0]["status"], "succeeded");
    fs::write(f.root.join("filet.yaml"), "broken").unwrap();
    assert!(f.run(&["history"]).0);
    let (ok, error) = f.run(&["check"]);
    assert!(!ok);
    assert_eq!(error["errors"][0]["code"], "CONFIG_INVALID");
    let (ok, error) = f.run(&["not-a-command"]);
    assert!(!ok);
    assert_eq!(error["formatVersion"], 1);
}
struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn wait(mut check: impl FnMut() -> bool) {
    let start = Instant::now();
    while !check() {
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "condition timed out"
        );
        std::thread::sleep(Duration::from_millis(40));
    }
}
#[test]
fn real_daemon_preserves_baseline_holds_lock_and_rejects_invalid_reload() {
    let f = Fixture::new();
    fs::write(f.root.join("inbox/existing.pdf"), b"old").unwrap();
    let mut cmd = f.command();
    let _daemon = Daemon(
        cmd.arg("daemon")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait(|| {
        let (ok, s) = f.run(&["status"]);
        ok && s["data"]["sources"][0]["status"] == "available"
    });
    let (ok, p) = f.run(&["plan", "inbox/existing.pdf"]);
    assert!(ok);
    let (ok, e) = f.run(&["apply", p["data"]["plan"]["planId"].as_str().unwrap()]);
    assert!(!ok);
    assert_eq!(e["errors"][0]["code"], "INSTANCE_LOCKED");
    fs::write(f.root.join("filet.yaml"), "schemaVersion: 999").unwrap();
    std::thread::sleep(Duration::from_millis(2200));
    fs::write(f.root.join("inbox/new.pdf"), b"new").unwrap();
    wait(|| f.root.join("archive/new.pdf").exists());
    assert!(f.root.join("inbox/existing.pdf").exists());
}
#[test]
fn schema_does_not_require_a_config() {
    let f = Fixture::new();
    fs::remove_file(f.root.join("filet.yaml")).unwrap();
    let (ok, s) = f.run(&["schema"]);
    assert!(ok);
    assert_eq!(s["data"]["configuration"]["title"], "Config");
}
