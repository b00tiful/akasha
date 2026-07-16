use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution/valid-root")
}

#[test]
fn plain_codex_preparation_reports_an_exact_create_and_writes_nothing() {
    let home = TempDir::new("plain-codex");
    let output = run(home.path(), "codex", false);

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("plain output is UTF-8");
    assert!(stdout.contains("prepared agent wiring: codex"));
    assert!(stdout.contains("action: create"));
    assert!(stdout.contains("current sha256: absent"));
    assert!(stdout.contains("patch range: 0..0"));
    assert!(stdout.contains("<!-- akasha-agent-wiring:v1:start -->"));
    assert!(stdout.contains("Run `akasha context`"));
    assert!(!home.path().join("AGENTS.md").exists());
}

#[test]
fn json_claude_preparation_preserves_existing_content_and_exposes_the_same_patch() {
    let home = TempDir::new("json-claude");
    let target = home.path().join("CLAUDE.md");
    let original = b"# Human Claude instructions\n";
    fs::write(&target, original).expect("seed Claude instructions");

    let output = run(home.path(), "claude", true);

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse wiring plan JSON");
    assert_eq!(value["client"], "claude");
    assert_eq!(value["action"], "append");
    assert_eq!(value["patch"]["start"], original.len());
    assert_eq!(value["patch"]["end"], original.len());
    assert!(
        value["patch"]["replacement"]
            .as_str()
            .expect("replacement string")
            .contains("@")
    );
    assert_eq!(fs::read(&target).expect("read preserved target"), original);
}

#[test]
fn codex_override_conflict_uses_stderr_and_exit_five() {
    let home = TempDir::new("codex-override");
    let override_path = home.path().join("AGENTS.override.md");
    let original = b"# Human override\n";
    fs::write(&override_path, original).expect("seed override");

    let output = run(home.path(), "codex", false);

    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("error output is UTF-8");
    assert!(stderr.contains("shadows AGENTS.md"));
    assert_eq!(
        fs::read(&override_path).expect("read preserved override"),
        original
    );
}

fn run(home: &Path, client: &str, json: bool) -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_akasha");
    let mut command = Command::new(binary);
    command.args([
        "--root",
        fixture_root().to_str().expect("fixture root is UTF-8"),
        "--no-color",
    ]);
    if json {
        command.arg("--json");
    }
    command.args([
        "prepare-agent-wiring",
        client,
        "--home",
        home.to_str().expect("agent home is UTF-8"),
    ]);
    command
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run agent wiring preparation")
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "akasha-cli-agent-wiring-{label}-{}-{id}",
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
