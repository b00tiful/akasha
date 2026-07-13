use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{ResolutionEnvironment, ResolveRequest, validate_project};

mod support;

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
    root: PathBuf,
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
    support::write_project_state(
        &project_dir,
        &["events/sessions/2026-07-13.md"],
        &["entities/core.md"],
        &[],
    );

    let request = ResolveRequest {
        root_override: Some(root.clone()),
        project_override: None,
        cwd: repository.join("nested"),
        environment: ResolutionEnvironment::default(),
    };

    Fixture {
        _temp: temp,
        root,
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
    assert_eq!(report.immutable_events, 1);
    assert_eq!(report.projections["index"].sources, 1);
    assert_eq!(report.projections["roadmap"].sources, 0);
    assert_eq!(report.wikilinks, 0);
    assert_eq!(report.note_types["session"].notes, 1);
    assert_eq!(report.note_types["entity"].notes, 1);
}

#[test]
fn validates_full_vault_relative_wikilinks_and_ignores_code() {
    let fixture = fixture("valid-wikilinks");
    let note = fixture.project_dir.join("events/sessions/2026-07-13.md");
    fs::write(
        &note,
        "---\nschema_version: 1\nproject: example\ntype: session\ndate: 2026-07-13\n---\n\n\
         # Session\n\n\
         Worked on [[Projects/example/entities/core|the core]].\n\
         ![[Projects/example/entities/core.md#Details]] [[#Session]]\n\
         `[[missing/inline]]`\n\
         ```md\n[[missing/fenced]]\n```\n",
    )
    .expect("replace session");
    support::write_project_state(
        &fixture.project_dir,
        &["events/sessions/2026-07-13.md"],
        &["entities/core.md"],
        &[],
    );

    let report = validate_project(&fixture.request).expect("validate wikilinks");
    assert_eq!(report.wikilinks, 3);
}

#[test]
fn rejects_a_missing_wikilink_target() {
    let fixture = fixture("missing-wikilink");
    let note = fixture.project_dir.join("events/sessions/2026-07-13.md");
    fs::write(
        &note,
        "---\nschema_version: 1\nproject: example\ntype: session\ndate: 2026-07-13\n---\n\n\
         # Session\n\n[[Projects/example/entities/missing]]\n",
    )
    .expect("replace session");

    let error = validate_project(&fixture.request).expect_err("missing target must fail");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("invalid wikilink"));
    assert!(error.to_string().contains("entities/missing"));
    assert!(
        error
            .to_string()
            .contains(note.to_str().expect("UTF-8 path"))
    );
}

#[test]
fn rejects_unsafe_and_malformed_wikilinks() {
    for (name, link) in [
        ("parent", "[[../outside]]"),
        ("absolute", "[[/outside]]"),
        ("backslash", "[[Projects\\example\\entities\\core]]"),
        ("unterminated", "[[Projects/example/entities/core"),
    ] {
        let fixture = fixture(name);
        fs::write(
            fixture.project_dir.join("events/sessions/2026-07-13.md"),
            format!(
                "---\nschema_version: 1\nproject: example\ntype: session\ndate: 2026-07-13\n---\n\n{link}\n"
            ),
        )
        .expect("replace session");

        let error = validate_project(&fixture.request).expect_err("unsafe link must fail");
        assert_eq!(error.exit_code(), 4);
        assert!(error.to_string().contains("invalid wikilink"));
    }
}

#[cfg(unix)]
#[test]
fn rejects_directory_symlink_and_out_of_root_wikilink_targets() {
    use std::os::unix::fs::symlink;

    let directory = fixture("directory-wikilink");
    fs::create_dir(directory.root.join("Global/directory.md")).expect("create target directory");
    replace_session_link(&directory, "[[Global/directory]]");
    let error = validate_project(&directory.request).expect_err("directory target must fail");
    assert!(error.to_string().contains("not a regular Markdown file"));

    let symlinked = fixture("symlink-wikilink");
    let outside_note = symlinked._temp.path().join("outside.md");
    fs::write(&outside_note, "# Outside\n").expect("write outside note");
    symlink(&outside_note, symlinked.root.join("Global/symlink.md"))
        .expect("create target symlink");
    replace_session_link(&symlinked, "[[Global/symlink]]");
    let error = validate_project(&symlinked.request).expect_err("symlink target must fail");
    assert!(error.to_string().contains("must not be a symbolic link"));

    let escaped = fixture("escaped-wikilink");
    let outside_directory = escaped._temp.path().join("outside");
    fs::create_dir(&outside_directory).expect("create outside directory");
    fs::write(outside_directory.join("note.md"), "# Outside\n").expect("write outside note");
    symlink(&outside_directory, escaped.root.join("Global/external"))
        .expect("create parent symlink");
    replace_session_link(&escaped, "[[Global/external/note]]");
    let error = validate_project(&escaped.request).expect_err("escaped target must fail");
    assert!(error.to_string().contains("escapes data root"));
}

fn replace_session_link(fixture: &Fixture, link: &str) {
    fs::write(
        fixture.project_dir.join("events/sessions/2026-07-13.md"),
        format!(
            "---\nschema_version: 1\nproject: example\ntype: session\ndate: 2026-07-13\n---\n\n{link}\n"
        ),
    )
    .expect("replace session");
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

#[test]
fn rejects_changed_untracked_and_missing_immutable_events() {
    let changed = fixture("changed-event");
    fs::write(
        changed
            .project_dir
            .join("events/sessions/2026-07-13.md"),
        "---\nschema_version: 1\nproject: example\ntype: session\ndate: 2026-07-13\n---\n\n# Changed\n",
    )
    .expect("change event");
    let error = validate_project(&changed.request).expect_err("changed event must fail");
    assert!(
        error
            .to_string()
            .contains("changed since its trusted baseline")
    );

    let untracked = fixture("untracked-event");
    fs::write(
        untracked.project_dir.join("events/sessions/2026-07-14.md"),
        "---\nschema_version: 1\nproject: example\ntype: session\ndate: 2026-07-14\n---\n\n# New\n",
    )
    .expect("add event");
    let error = validate_project(&untracked.request).expect_err("untracked event must fail");
    assert!(error.to_string().contains("has no trusted baseline"));

    let missing = fixture("missing-event");
    fs::remove_file(missing.project_dir.join("events/sessions/2026-07-13.md"))
        .expect("remove event");
    let error = validate_project(&missing.request).expect_err("missing event must fail");
    assert!(error.to_string().contains("tracks missing immutable event"));
}

#[test]
fn rejects_stale_projection_sources_and_changed_projection_bytes() {
    let stale_index = fixture("stale-index-sources");
    fs::write(
        stale_index.project_dir.join("entities/core.md"),
        "---\nschema_version: 1\nentity: core\nkind: subsystem\nstatus: active\nreviewed: 2026-07-13\n---\n\n# Updated core\n",
    )
    .expect("update entity");
    let error = validate_project(&stale_index.request).expect_err("stale index must fail");
    assert!(error.to_string().contains("projection \"index\" is stale"));

    let stale_roadmap = fixture("stale-roadmap-sources");
    fs::write(
        stale_roadmap.project_dir.join("records/tasks/new.md"),
        "---\nschema_version: 1\nproject: example\ntype: task\nstatus: open\n---\n\n# New task\n",
    )
    .expect("add record");
    let error = validate_project(&stale_roadmap.request).expect_err("stale roadmap must fail");
    assert!(
        error
            .to_string()
            .contains("projection \"roadmap\" is stale")
    );

    let changed_output = fixture("changed-index-output");
    fs::write(
        changed_output.project_dir.join("index.md"),
        "# Changed index\n",
    )
    .expect("change index");
    let error =
        validate_project(&changed_output.request).expect_err("changed projection must fail");
    assert!(
        error
            .to_string()
            .contains("projection \"index\" bytes differ")
    );
}

#[test]
fn rejects_missing_and_malformed_project_state() {
    let missing = fixture("missing-state");
    fs::remove_file(missing.project_dir.join(".akasha-state.toml")).expect("remove project state");
    let error = validate_project(&missing.request).expect_err("missing state must fail");
    assert!(error.to_string().contains("required file does not exist"));

    let malformed = fixture("malformed-state");
    fs::write(
        malformed.project_dir.join(".akasha-state.toml"),
        "schema_version = 1\nunknown = true\n",
    )
    .expect("write malformed project state");
    let error = validate_project(&malformed.request).expect_err("malformed state must fail");
    assert!(error.to_string().contains("invalid project state TOML"));
}
