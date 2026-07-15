use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn create_note_prints_plain_text_and_publishes_record_projection_and_state() {
    let fixture = Fixture::new("plain");

    let output = fixture.command(false).output().expect("run create-note");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(stdout.starts_with("created note: Projects/example/records/tasks/cli-created.md\n"));
    assert!(stdout.contains("class: record\n"));
    assert!(stdout.contains("template scope: project\n"));
    assert!(stdout.contains("projection changed: yes\n"));
    assert!(stdout.ends_with("recovery: none\n"));
    assert_eq!(
        fs::read_to_string(fixture.record()).expect("read created record"),
        expected_record()
    );
    assert_eq!(
        fs::read_to_string(fixture.roadmap()).expect("read updated roadmap"),
        fixture.projection_source()
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
        .expect("validate record project");
    assert!(validate.status.success());
}

#[test]
fn create_note_json_reports_the_same_identity_class_and_projection() {
    let fixture = Fixture::new("json");

    let output = fixture
        .command(true)
        .output()
        .expect("run JSON create-note");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse note JSON");
    assert_eq!(value["project"], "example");
    assert_eq!(value["note_type"], "task");
    assert_eq!(value["class"], "record");
    assert_eq!(value["id"], "Projects/example/records/tasks/cli-created.md");
    assert_eq!(value["template_scope"], "project");
    assert_eq!(value["projection_changed"], true);
    assert_eq!(value["recovery"], "none");
    assert!(
        value["projection"]
            .as_str()
            .expect("projection path")
            .ends_with("Projects/example/roadmap.md")
    );
}

#[test]
fn create_note_reports_missing_projection_and_event_class_on_stderr() {
    let fixture = Fixture::new("errors");
    let missing = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            path(&fixture.root),
            "--project",
            "example",
            "create-note",
            "task",
            "cli-created.md",
            "--projection",
            "missing.md",
        ])
        .current_dir(&fixture.repository)
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run create-note with missing projection");
    assert_eq!(missing.status.code(), Some(6));
    assert!(missing.stdout.is_empty());
    assert!(
        String::from_utf8(missing.stderr)
            .expect("stderr is UTF-8")
            .contains("failed to read projection input")
    );

    let event = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            path(&fixture.root),
            "--project",
            "example",
            "create-note",
            "session",
            "wrong-command.md",
            "--projection",
            path(&fixture.projection_input),
        ])
        .current_dir(&fixture.repository)
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run create-note for event type");
    assert_eq!(event.status.code(), Some(2));
    assert!(event.stdout.is_empty());
    assert!(
        String::from_utf8(event.stderr)
            .expect("stderr is UTF-8")
            .contains("use create-event instead")
    );
}

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    repository: PathBuf,
    projection_input: PathBuf,
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
        fs::write(
            root.join("Projects/example/templates/task.md"),
            record_template(),
        )
        .expect("write project record template");
        fs::write(
            root.join("Projects/example/templates/session.md"),
            "unused event template\n",
        )
        .expect("write project event template");
        let projection_input = temp.path().join("roadmap-after.md");
        fs::write(&projection_input, projection_source()).expect("write projection input");
        Self {
            _temp: temp,
            root,
            repository,
            projection_input,
        }
    }

    fn command(&self, json: bool) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_akasha"));
        command.args(["--root", path(&self.root), "--project", "example"]);
        if json {
            command.arg("--json");
        }
        command.args([
            "create-note",
            "task",
            "cli-created.md",
            "--projection",
            path(&self.projection_input),
            "--field",
            "status=open",
            "--field",
            "created=2026-07-15",
            "--field",
            "updated=2026-07-15",
            "--field",
            "title=CLI-created task",
            "--field",
            "body=Tracks [[Projects/example/entities/core|the core]].",
        ]);
        command
            .current_dir(&self.repository)
            .env_remove("AKASHA_ROOT");
        command
    }

    fn record(&self) -> PathBuf {
        self.root
            .join("Projects/example/records/tasks/cli-created.md")
    }

    fn roadmap(&self) -> PathBuf {
        self.root.join("Projects/example/roadmap.md")
    }

    fn projection_source(&self) -> String {
        projection_source().to_owned()
    }
}

fn record_template() -> &'static str {
    "---\nschema_version: 1\nproject: {{project}}\ntype: {{type}}\nstatus: {{status}}\ncreated: {{created}}\nupdated: {{updated}}\n---\n\n# {{title}}\n\n{{body}}\n"
}

fn expected_record() -> &'static str {
    "---\nschema_version: 1\nproject: example\ntype: task\nstatus: open\ncreated: 2026-07-15\nupdated: 2026-07-15\n---\n\n# CLI-created task\n\nTracks [[Projects/example/entities/core|the core]].\n"
}

fn projection_source() -> &'static str {
    "# Example roadmap\n\nSynthetic required projection fixture.\n\n- [[Projects/example/records/tasks/cli-created|CLI-created task]]\n"
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
            "akasha-cli-note-creation-{label}-{}-{id}",
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
