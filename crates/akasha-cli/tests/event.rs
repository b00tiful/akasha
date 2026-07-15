use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn create_event_prints_plain_text_and_publishes_a_valid_event() {
    let fixture = Fixture::new("plain");

    let output = fixture.command(false).output().expect("run create-event");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(
        stdout.starts_with("created event: Projects/example/events/sessions/2026-07-15-cli.md\n")
    );
    assert!(stdout.contains("template scope: project\n"));
    assert!(stdout.ends_with("recovery: none\n"));
    assert_eq!(
        fs::read_to_string(fixture.event()).expect("read created event"),
        expected_event()
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
        .expect("validate event project");
    assert!(validate.status.success());
    assert!(
        String::from_utf8(validate.stdout)
            .expect("validation stdout is UTF-8")
            .contains("immutable events: 4\n")
    );
}

#[test]
fn create_event_json_reports_the_same_identity_and_template() {
    let fixture = Fixture::new("json");

    let output = fixture
        .command(true)
        .output()
        .expect("run JSON create-event");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse event JSON");
    assert_eq!(value["project"], "example");
    assert_eq!(value["note_type"], "session");
    assert_eq!(
        value["id"],
        "Projects/example/events/sessions/2026-07-15-cli.md"
    );
    assert_eq!(value["template_scope"], "project");
    assert_eq!(value["recovery"], "none");
    assert!(
        value["template"]
            .as_str()
            .expect("template path")
            .ends_with("Projects/example/templates/session.md")
    );
}

#[test]
fn create_event_reports_field_and_existing_path_failures_on_stderr() {
    let fixture = Fixture::new("errors");
    let malformed = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            path(&fixture.root),
            "--project",
            "example",
            "create-event",
            "session",
            "2026-07-15-cli.md",
            "--field",
            "date",
        ])
        .current_dir(&fixture.repository)
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run malformed create-event");
    assert_eq!(malformed.status.code(), Some(2));
    assert!(malformed.stdout.is_empty());
    assert!(
        String::from_utf8(malformed.stderr)
            .expect("stderr is UTF-8")
            .contains("NAME=VALUE")
    );

    let first = fixture.command(false).output().expect("create first event");
    assert!(first.status.success());
    let conflict = fixture.command(false).output().expect("rerun create-event");
    assert_eq!(conflict.status.code(), Some(5));
    assert!(conflict.stdout.is_empty());
    assert!(
        String::from_utf8(conflict.stderr)
            .expect("conflict stderr is UTF-8")
            .contains("immutable event destination already exists")
    );
}

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    repository: PathBuf,
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
            root.join("Projects/example/templates/session.md"),
            event_template(),
        )
        .expect("write project event template");
        Self {
            _temp: temp,
            root,
            repository,
        }
    }

    fn command(&self, json: bool) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_akasha"));
        command.args(["--root", path(&self.root), "--project", "example"]);
        if json {
            command.arg("--json");
        }
        command.args([
            "create-event",
            "session",
            "2026-07-15-cli.md",
            "--field",
            "date=2026-07-15",
            "--field",
            "title=CLI session",
            "--field",
            "body=Worked on [[Projects/example/entities/core|the core]].\n\nRelated: [[Projects/example/roadmap|Roadmap]].",
        ]);
        command
            .current_dir(&self.repository)
            .env_remove("AKASHA_ROOT");
        command
    }

    fn event(&self) -> PathBuf {
        self.root
            .join("Projects/example/events/sessions/2026-07-15-cli.md")
    }
}

fn event_template() -> &'static str {
    "---\nschema_version: 1\nproject: {{project}}\ntype: {{type}}\ndate: {{date}}\n---\n\n# {{title}}\n\n{{body}}\n"
}

fn expected_event() -> &'static str {
    "---\nschema_version: 1\nproject: example\ntype: session\ndate: 2026-07-15\n---\n\n# CLI session\n\nWorked on [[Projects/example/entities/core|the core]].\n\nRelated: [[Projects/example/roadmap|Roadmap]].\n"
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
            "akasha-cli-event-{label}-{}-{id}",
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
