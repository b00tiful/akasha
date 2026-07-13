use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution")
}

#[test]
fn validate_prints_a_compact_plain_text_report() {
    let fixture = fixtures();
    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            fixture
                .join("valid-root")
                .to_str()
                .expect("fixture path is UTF-8"),
            "validate",
        ])
        .current_dir(fixture.join("repository/nested"))
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run akasha validate");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(stdout.contains("valid: example"));
    assert!(stdout.contains("canonical notes: 6"));
    assert!(stdout.contains("immutable events: 3"));
    assert!(stdout.contains("projection: index — 1 sources"));
    assert!(stdout.contains("projection: roadmap — 2 sources"));
    assert!(stdout.contains("wikilinks: 1"));
    assert!(stdout.contains("note type: session (event) — 1"));
}

#[test]
fn validate_json_is_machine_readable() {
    let fixture = fixtures();
    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            fixture
                .join("valid-root")
                .to_str()
                .expect("fixture path is UTF-8"),
            "--project",
            "example",
            "--json",
            "validate",
        ])
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run akasha validate --json");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse validate JSON");
    assert_eq!(value["project"], "example");
    assert_eq!(value["canonical_notes"], 6);
    assert_eq!(value["immutable_events"], 3);
    assert_eq!(value["projections"]["index"]["sources"], 1);
    assert_eq!(value["projections"]["roadmap"]["sources"], 2);
    assert_eq!(value["wikilinks"], 1);
    assert_eq!(value["note_types"]["entity"]["class"], "entity");
}

#[test]
fn validation_failures_use_stderr_and_exit_code_four() {
    let temp = TempDir::copy_of(&fixtures());
    fs::remove_file(temp.path().join("valid-root/Projects/example/roadmap.md"))
        .expect("remove copied roadmap");

    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            temp.path()
                .join("valid-root")
                .to_str()
                .expect("fixture path is UTF-8"),
            "--project",
            "example",
            "validate",
        ])
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run failing validate");

    assert_eq!(output.status.code(), Some(4));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.starts_with("akasha: validation failed"));
    assert!(stderr.contains("required file does not exist"));

    let temp = TempDir::copy_of(&fixtures());
    fs::write(
        temp.path()
            .join("valid-root/Projects/example/events/sessions/2026-07-13.md"),
        "---\nschema_version: 1\nproject: example\ntype: session\ndate: 2026-07-13\n---\n\n\
         [[Projects/example/entities/missing]]\n",
    )
    .expect("replace session with broken link");
    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            temp.path()
                .join("valid-root")
                .to_str()
                .expect("fixture path is UTF-8"),
            "--project",
            "example",
            "validate",
        ])
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run failing wikilink validation");

    assert_eq!(output.status.code(), Some(4));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.starts_with("akasha: validation failed"));
    assert!(stderr.contains("invalid wikilink"));
    assert!(stderr.contains("entities/missing"));
}

struct TempDir(PathBuf);

impl TempDir {
    fn copy_of(source: &Path) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("akasha-cli-validation-{}-{id}", std::process::id()));
        copy_directory(source, &path);
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

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create copied fixture directory");
    for entry in fs::read_dir(source).expect("read fixture directory") {
        let entry = entry.expect("read fixture entry");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type().expect("read fixture type").is_dir() {
            copy_directory(&source_path, &destination_path);
        } else {
            fs::copy(source_path, destination_path).expect("copy fixture file");
        }
    }
}
