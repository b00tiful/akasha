use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
const ENTITY_ID: &str = "Projects/example/entities/core.md";

#[test]
fn update_entity_prints_plain_text_and_publishes_exact_inputs() {
    let fixture = Fixture::new("plain");

    let output = fixture.command(false).output().expect("run update-entity");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(stdout.starts_with("updated entity: Projects/example/entities/core.md\n"));
    assert!(stdout.contains("type: entity\n"));
    assert!(stdout.contains("entity changed: yes\n"));
    assert!(stdout.contains("index changed: yes\n"));
    assert!(stdout.ends_with("recovery: none\n"));
    assert_eq!(
        fs::read_to_string(fixture.entity()).expect("read updated entity"),
        replacement_entity()
    );
    assert_eq!(
        fs::read_to_string(fixture.index()).expect("read updated index"),
        replacement_index()
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
fn update_entity_json_reports_the_same_identity_and_artifacts() {
    let fixture = Fixture::new("json");

    let output = fixture
        .command(true)
        .output()
        .expect("run JSON update-entity");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse update JSON");
    assert_eq!(value["project"], "example");
    assert_eq!(value["note_type"], "entity");
    assert_eq!(value["id"], ENTITY_ID);
    assert_eq!(value["changed"], true);
    assert_eq!(value["index_changed"], true);
    assert_eq!(value["recovery"], "none");
    assert!(
        value["index"]
            .as_str()
            .expect("index path")
            .ends_with("Projects/example/index.md")
    );
}

#[test]
fn update_entity_reports_missing_input_and_stale_source_on_stderr() {
    let fixture = Fixture::new("errors");
    let missing = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            path(&fixture.root),
            "--project",
            "example",
            "update-entity",
            ENTITY_ID,
            "--expected",
            path(&fixture.expected_input),
            "--replacement",
            "missing.md",
            "--index",
            path(&fixture.index_input),
        ])
        .current_dir(&fixture.repository)
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run update-entity with missing replacement");
    assert_eq!(missing.status.code(), Some(6));
    assert!(missing.stdout.is_empty());
    assert!(
        String::from_utf8(missing.stderr)
            .expect("stderr is UTF-8")
            .contains("failed to read replacement entity input")
    );

    fs::write(&fixture.expected_input, "stale source\n").expect("seed stale expected input");
    let entity_before = fs::read(fixture.entity()).expect("read entity before stale update");
    let index_before = fs::read(fixture.index()).expect("read index before stale update");
    let stale = fixture
        .command(false)
        .output()
        .expect("run stale update-entity");
    assert_eq!(stale.status.code(), Some(5));
    assert!(stale.stdout.is_empty());
    assert!(
        String::from_utf8(stale.stderr)
            .expect("stderr is UTF-8")
            .contains("no longer matches")
    );
    assert_eq!(
        fs::read(fixture.entity()).expect("read unchanged entity"),
        entity_before
    );
    assert_eq!(
        fs::read(fixture.index()).expect("read unchanged index"),
        index_before
    );
}

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    repository: PathBuf,
    expected_input: PathBuf,
    replacement_input: PathBuf,
    index_input: PathBuf,
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
        let expected_input = temp.path().join("entity-before.md");
        let replacement_input = temp.path().join("entity-after.md");
        let index_input = temp.path().join("index-after.md");
        fs::copy(root.join(ENTITY_ID), &expected_input).expect("copy expected entity input");
        fs::write(&replacement_input, replacement_entity()).expect("write replacement input");
        fs::write(&index_input, replacement_index()).expect("write index input");
        Self {
            _temp: temp,
            root,
            repository,
            expected_input,
            replacement_input,
            index_input,
        }
    }

    fn command(&self, json: bool) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_akasha"));
        command.args(["--root", path(&self.root), "--project", "example"]);
        if json {
            command.arg("--json");
        }
        command.args([
            "update-entity",
            ENTITY_ID,
            "--expected",
            path(&self.expected_input),
            "--replacement",
            path(&self.replacement_input),
            "--index",
            path(&self.index_input),
        ]);
        command
            .current_dir(&self.repository)
            .env_remove("AKASHA_ROOT");
        command
    }

    fn entity(&self) -> PathBuf {
        self.root.join(ENTITY_ID)
    }

    fn index(&self) -> PathBuf {
        self.root.join("Projects/example/index.md")
    }
}

fn replacement_entity() -> &'static str {
    "---\nschema_version: 1\nentity: core\nkind: service\nstatus: deprecated\nreviewed: 2026-07-16\n---\n\n# Synthetic entity\n\nUpdated through the CLI.\n"
}

fn replacement_index() -> &'static str {
    "# Example project\n\nSynthetic resolution fixture for Akasha's core tests.\n\n- [[Projects/example/entities/core|Core]] — deprecated service\n"
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
            "akasha-cli-entity-update-{label}-{}-{id}",
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
