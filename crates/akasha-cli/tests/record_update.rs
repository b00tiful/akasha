use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
const RECORD_ID: &str = "Projects/example/records/tasks/active.md";

#[test]
fn update_record_prints_plain_text_and_publishes_exact_inputs() {
    let fixture = Fixture::new("plain");

    let output = fixture.command(false).output().expect("run update-record");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(stdout.starts_with("updated record: Projects/example/records/tasks/active.md\n"));
    assert!(stdout.contains("type: task\n"));
    assert!(stdout.contains("record changed: yes\n"));
    assert!(stdout.contains("roadmap changed: yes\n"));
    assert!(stdout.ends_with("recovery: none\n"));
    assert_eq!(
        fs::read_to_string(fixture.record()).expect("read updated record"),
        replacement_record()
    );
    assert_eq!(
        fs::read_to_string(fixture.roadmap()).expect("read updated roadmap"),
        replacement_roadmap()
    );

    let validate = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            path(&fixture.root),
            "--project",
            "example",
            "validate",
        ])
        .current_dir(&fixture.repository)
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("validate updated project");
    assert!(validate.status.success());
}

#[test]
fn update_record_json_reports_the_same_identity_and_artifacts() {
    let fixture = Fixture::new("json");

    let output = fixture
        .command(true)
        .output()
        .expect("run JSON update-record");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse update JSON");
    assert_eq!(value["project"], "example");
    assert_eq!(value["note_type"], "task");
    assert_eq!(value["id"], RECORD_ID);
    assert_eq!(value["changed"], true);
    assert_eq!(value["roadmap_changed"], true);
    assert_eq!(value["recovery"], "none");
    assert!(
        value["roadmap"]
            .as_str()
            .expect("roadmap path")
            .ends_with("Projects/example/roadmap.md")
    );
}

#[test]
fn update_record_reports_missing_input_and_stale_source_on_stderr() {
    let fixture = Fixture::new("errors");
    let missing = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            path(&fixture.root),
            "--project",
            "example",
            "update-record",
            RECORD_ID,
            "--expected",
            path(&fixture.expected_input),
            "--replacement",
            "missing.md",
            "--roadmap",
            path(&fixture.roadmap_input),
        ])
        .current_dir(&fixture.repository)
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run update-record with missing replacement");
    assert_eq!(missing.status.code(), Some(6));
    assert!(missing.stdout.is_empty());
    assert!(
        String::from_utf8(missing.stderr)
            .expect("stderr is UTF-8")
            .contains("failed to read replacement record input")
    );

    fs::write(&fixture.expected_input, "stale source\n").expect("seed stale expected input");
    let record_before = fs::read(fixture.record()).expect("read record before stale update");
    let roadmap_before = fs::read(fixture.roadmap()).expect("read roadmap before stale update");
    let stale = fixture
        .command(false)
        .output()
        .expect("run stale update-record");
    assert_eq!(stale.status.code(), Some(5));
    assert!(stale.stdout.is_empty());
    assert!(
        String::from_utf8(stale.stderr)
            .expect("stderr is UTF-8")
            .contains("no longer matches")
    );
    assert_eq!(
        fs::read(fixture.record()).expect("read unchanged record"),
        record_before
    );
    assert_eq!(
        fs::read(fixture.roadmap()).expect("read unchanged roadmap"),
        roadmap_before
    );
}

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    repository: PathBuf,
    expected_input: PathBuf,
    replacement_input: PathBuf,
    roadmap_input: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let temp = TempDir::new(label);
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/resolution/valid-root");
        let root = temp.path().join("valid-root");
        copy_tree(&source, &root);
        let repository = temp.path().join("repository");
        fs::create_dir_all(&repository).expect("create registered repository");
        let expected_input = temp.path().join("record-before.md");
        let replacement_input = temp.path().join("record-after.md");
        let roadmap_input = temp.path().join("roadmap-after.md");
        fs::copy(root.join(RECORD_ID), &expected_input).expect("copy expected record input");
        fs::write(&replacement_input, replacement_record()).expect("write replacement input");
        fs::write(&roadmap_input, replacement_roadmap()).expect("write roadmap input");
        Self {
            _temp: temp,
            root,
            repository,
            expected_input,
            replacement_input,
            roadmap_input,
        }
    }

    fn command(&self, json: bool) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_akasha"));
        command.args(["--root", path(&self.root), "--project", "example"]);
        if json {
            command.arg("--json");
        }
        command.args([
            "update-record",
            RECORD_ID,
            "--expected",
            path(&self.expected_input),
            "--replacement",
            path(&self.replacement_input),
            "--roadmap",
            path(&self.roadmap_input),
        ]);
        command
            .current_dir(&self.repository)
            .env_remove("AKASHA_ROOT");
        command
    }

    fn record(&self) -> PathBuf {
        self.root.join(RECORD_ID)
    }

    fn roadmap(&self) -> PathBuf {
        self.root.join("Projects/example/roadmap.md")
    }
}

fn replacement_record() -> &'static str {
    "---\nschema_version: 1\nproject: example\ntype: task\nstatus: resolved\ncreated: 2026-07-13\nupdated: 2026-07-16\n---\n\n# Synthetic task\n\nUpdated through the CLI.\n"
}

fn replacement_roadmap() -> &'static str {
    "# Example roadmap\n\nSynthetic required projection fixture.\n\n- [[Projects/example/records/tasks/active|Synthetic task]] — resolved\n"
}

fn path(path: &Path) -> &str {
    path.to_str().expect("test path is UTF-8")
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create copied fixture directory");
    let mut entries = fs::read_dir(source)
        .expect("read fixture directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("read fixture entries");
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("read fixture entry type").is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(path, target).expect("copy fixture file");
        }
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "akasha-cli-record-update-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temporary directory");
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
