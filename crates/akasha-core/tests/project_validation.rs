use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{ResolutionEnvironment, ResolveRequest, validate_project};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "akasha-validation-{name}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create test directory");
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

struct Fixture {
    _temp: TempDir,
    project_dir: PathBuf,
    request: ResolveRequest,
}

fn fixture(name: &str) -> Fixture {
    let temp = TempDir::new(name);
    let root = temp.path().join("root");
    let repository = temp.path().join("repository");
    let project_dir = root.join("Projects/example");

    for directory in [
        root.join("Meta"),
        root.join("templates"),
        root.join("Global"),
        root.join("Inbox"),
        project_dir.join("templates"),
        project_dir.join("events/sessions"),
        project_dir.join("events/handoffs"),
        project_dir.join("records/tasks"),
        project_dir.join("records/problems"),
        project_dir.join("entities"),
        repository.join("nested"),
    ] {
        fs::create_dir_all(directory).expect("create fixture directory");
    }

    fs::write(root.join("akasha.toml"), root_config()).expect("write root config");
    fs::write(
        root.join("Meta/projects.yaml"),
        format!("example:\n  path: {:?}\n  status: active\n", repository),
    )
    .expect("write registry");
    fs::write(
        repository.join(".akasha.toml"),
        "schema_version = 1\nproject = \"example\"\n",
    )
    .expect("write pointer");
    fs::write(project_dir.join("index.md"), "# Index\n").expect("write index");
    fs::write(project_dir.join("roadmap.md"), "# Roadmap\n").expect("write roadmap");
    fs::write(
        project_dir.join("events/sessions/2026-07-13.md"),
        "---\nschema_version: 1\nproject: example\ntype: session\ndate: 2026-07-13\n---\n\n# Session\n",
    )
    .expect("write session");
    fs::write(
        project_dir.join("entities/core.md"),
        "---\nschema_version: 1\nentity: core\nkind: subsystem\nstatus: active\nreviewed: 2026-07-13\n---\n\n# Core\n",
    )
    .expect("write entity");

    let request = ResolveRequest {
        root_override: Some(root.clone()),
        project_override: None,
        cwd: repository.join("nested"),
        environment: ResolutionEnvironment::default(),
    };

    Fixture {
        _temp: temp,
        project_dir,
        request,
    }
}

fn root_config() -> &'static str {
    "schema_version = 1\n\
     \n\
     [files]\n\
     registry = \"Meta/projects.yaml\"\n\
     \n\
     [folders]\n\
     templates = \"templates\"\n\
     global = \"Global\"\n\
     projects = \"Projects\"\n\
     inbox = \"Inbox\"\n\
     \n\
     [context]\n\
     tasks = \"task\"\n\
     problems = \"problem\"\n\
     handoffs = \"handoff\"\n\
     recent_events = [\"session\"]\n\
     open_statuses = [\"open\", \"active\", \"blocked\", \"in-progress\"]\n\
     \n\
     [project]\n\
     templates = \"templates\"\n\
     index = \"index.md\"\n\
     roadmap = \"roadmap.md\"\n\
     \n\
     [project.note_types.session]\n\
     class = \"event\"\n\
     folder = \"events/sessions\"\n\
     required_fields = [\"project\", \"type\", \"date\"]\n\
     \n\
     [project.note_types.handoff]\n\
     class = \"event\"\n\
     folder = \"events/handoffs\"\n\
     required_fields = [\"project\", \"type\", \"date\"]\n\
     \n\
     [project.note_types.task]\n\
     class = \"record\"\n\
     folder = \"records/tasks\"\n\
     required_fields = [\"project\", \"type\", \"status\"]\n\
     \n\
     [project.note_types.problem]\n\
     class = \"record\"\n\
     folder = \"records/problems\"\n\
     required_fields = [\"project\", \"type\", \"status\"]\n\
     \n\
     [project.note_types.entity]\n\
     class = \"entity\"\n\
     folder = \"entities\"\n\
     required_fields = [\"entity\", \"kind\", \"status\", \"reviewed\"]\n"
}

#[test]
fn validates_the_configured_layout_and_canonical_notes() {
    let fixture = fixture("valid");
    let report = validate_project(&fixture.request).expect("validate project");

    assert_eq!(report.project, "example");
    assert_eq!(report.registry_projects, 1);
    assert_eq!(report.canonical_notes, 2);
    assert_eq!(report.note_types["session"].notes, 1);
    assert_eq!(report.note_types["entity"].notes, 1);
}

#[test]
fn rejects_a_missing_required_project_file() {
    let fixture = fixture("missing-roadmap");
    fs::remove_file(fixture.project_dir.join("roadmap.md")).expect("remove roadmap");

    let error = validate_project(&fixture.request).expect_err("missing roadmap must fail");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("required file does not exist"));
}

#[test]
fn rejects_wrong_note_identity() {
    let fixture = fixture("wrong-identity");
    let note = fixture.project_dir.join("events/sessions/2026-07-13.md");
    fs::write(
        &note,
        "---\nschema_version: 1\nproject: another\ntype: session\ndate: 2026-07-13\n---\n",
    )
    .expect("replace session");

    let error = validate_project(&fixture.request).expect_err("invalid note must fail");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("expected \"example\""));
    assert!(
        error
            .to_string()
            .contains(note.to_str().expect("UTF-8 path"))
    );
}

#[test]
fn rejects_missing_configured_note_fields() {
    let fixture = fixture("missing-field");
    fs::write(
        fixture.project_dir.join("events/sessions/2026-07-13.md"),
        "---\nschema_version: 1\nproject: example\ntype: session\n---\n",
    )
    .expect("replace session");

    let error = validate_project(&fixture.request).expect_err("missing field must fail");
    assert_eq!(error.exit_code(), 4);
    assert!(
        error
            .to_string()
            .contains("required field \"date\" is missing")
    );
}

#[test]
fn rejects_non_markdown_files_in_canonical_note_folders() {
    let fixture = fixture("non-markdown");
    fs::write(
        fixture.project_dir.join("entities/opaque.bin"),
        b"not a note",
    )
    .expect("write invalid file");

    let error = validate_project(&fixture.request).expect_err("non-Markdown file must fail");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("only directories and .md files"));
}
