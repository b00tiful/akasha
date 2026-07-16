use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution/valid-root")
}

#[test]
fn plain_codex_preparation_reports_create_and_writes_nothing() {
    let home = TempDir::new("plain-codex");
    let output = run_prepare(home.path(), "codex", false);

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("plain output is UTF-8");
    assert!(stdout.contains("prepared session hook: codex"));
    assert!(stdout.contains("action: create"));
    assert!(stdout.contains("akasha breadcrumb --optional"));
    assert!(!home.path().join("hooks.json").exists());
}

#[test]
fn json_claude_preparation_preserves_the_existing_settings_file() {
    let home = TempDir::new("json-claude");
    let target = home.path().join("settings.json");
    let original = b"{\"theme\":\"dark\"}\n";
    fs::write(&target, original).expect("seed Claude settings");

    let output = run_prepare(home.path(), "claude", true);

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse hook plan JSON");
    assert_eq!(value["client"], "claude");
    assert_eq!(value["action"], "add-hooks");
    assert_eq!(value["target"], target.to_str().unwrap());
    assert!(value["plan_id"].as_str().unwrap().starts_with("sha256:"));
    assert_eq!(
        fs::read(&target).expect("read unchanged settings"),
        original
    );
}

#[test]
fn malformed_target_uses_stderr_and_exit_five() {
    let home = TempDir::new("malformed");
    let target = home.path().join("hooks.json");
    fs::write(&target, b"[]").expect("seed invalid root shape");

    let output = run_prepare(home.path(), "codex", false);

    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr is UTF-8")
            .contains("configuration root must be a JSON object")
    );
    assert_eq!(fs::read(&target).expect("read preserved target"), b"[]");
}

fn run_prepare(home: &Path, client: &str, json: bool) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_akasha"));
    command.args([
        "--root",
        fixture_root().to_str().expect("fixture root is UTF-8"),
        "--no-color",
    ]);
    if json {
        command.arg("--json");
    }
    command.args([
        "prepare-session-hook",
        client,
        "--home",
        home.to_str().expect("agent home is UTF-8"),
    ]);
    command
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run session hook preparation")
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "akasha-cli-session-hook-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create temporary directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
