use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    NOTE_EDIT_JOURNAL_FILE, NoteEditRecovery, NoteTemplateScope, ResolutionEnvironment,
    ResolveRequest, capture_handoff, create_event, recover_pending_note_edit, validate_project,
};
use serde_json::json;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
const EVENT_RELATIVE_PATH: &str = "2026-07-15-goal-integration.md";
const EVENT_ID: &str = "Projects/example/events/sessions/2026-07-15-goal-integration.md";

#[test]
fn creates_exact_template_event_updates_state_and_keeps_project_valid() {
    let fixture = Fixture::new("create");
    let state_before = fixture.state();
    let fields = fixture.fields();

    let result = create_event(
        &fixture.request,
        "session",
        Path::new(EVENT_RELATIVE_PATH),
        &fields,
    )
    .expect("create immutable event");

    assert_eq!(result.id, EVENT_ID);
    assert_eq!(result.template_scope, NoteTemplateScope::Project);
    assert_eq!(result.recovery, NoteEditRecovery::None);
    assert_eq!(fixture.event(), fixture.expected_event());
    assert_ne!(fixture.state(), state_before);
    assert!(!fixture.journal().exists());
    let report = validate_project(&fixture.request).expect("created event keeps project valid");
    assert_eq!(report.immutable_events, 4);
}

#[test]
fn captures_the_configured_handoff_role_through_the_event_transaction() {
    let fixture = Fixture::new("handoff");
    let state_before = fixture.state();
    let result = capture_handoff(
        &fixture.request,
        Path::new("2026-07-16.md"),
        &fixture.fields(),
    )
    .expect("capture configured handoff");

    assert_eq!(result.note_type, "handoff");
    assert_eq!(result.id, "Projects/example/events/handoffs/2026-07-16.md");
    assert_eq!(result.template_scope, NoteTemplateScope::Project);
    assert_eq!(result.recovery, NoteEditRecovery::None);
    let source = fs::read_to_string(&result.path).expect("read captured handoff");
    assert!(source.contains("type: handoff\n"));
    assert_ne!(fixture.state(), state_before);
    assert!(!fixture.journal().exists());
    let report = validate_project(&fixture.request).expect("captured handoff keeps project valid");
    assert_eq!(report.immutable_events, 4);
}

#[test]
fn rejects_existing_paths_non_events_and_incomplete_template_input_without_writes() {
    let fixture = Fixture::new("rejections");
    let state_before = fixture.state();
    let fields = fixture.fields();
    create_event(
        &fixture.request,
        "session",
        Path::new(EVENT_RELATIVE_PATH),
        &fields,
    )
    .expect("create first event");
    let committed_state = fixture.state();

    let existing = create_event(
        &fixture.request,
        "session",
        Path::new(EVENT_RELATIVE_PATH),
        &fields,
    )
    .expect_err("existing immutable event must conflict");
    assert_eq!(existing.exit_code(), 5);
    assert_eq!(fixture.state(), committed_state);

    let non_event = create_event(
        &fixture.request,
        "entity",
        Path::new("new.md"),
        &BTreeMap::new(),
    )
    .expect_err("mutable type must not use event creation");
    assert_eq!(non_event.exit_code(), 2);

    fs::remove_file(fixture.event_path()).expect("remove committed event for isolated input test");
    fs::write(fixture.state_path(), state_before).expect("restore original state");
    let mut missing = fixture.fields();
    missing.remove("date");
    let missing = create_event(
        &fixture.request,
        "session",
        Path::new(EVENT_RELATIVE_PATH),
        &missing,
    )
    .expect_err("missing template field must fail");
    assert_eq!(missing.exit_code(), 2);
    assert!(!fixture.event_path().exists());
    assert!(!fixture.journal().exists());
}

#[test]
fn rejects_unused_fields_unsafe_paths_and_invalid_rendered_frontmatter() {
    let fixture = Fixture::new("invalid-input");
    let state_before = fixture.state();
    let mut unused = fixture.fields();
    unused.insert("typo".to_owned(), "unused".to_owned());
    let unused = create_event(
        &fixture.request,
        "session",
        Path::new(EVENT_RELATIVE_PATH),
        &unused,
    )
    .expect_err("unused field must fail");
    assert_eq!(unused.exit_code(), 2);

    let unsafe_path = create_event(
        &fixture.request,
        "session",
        Path::new("../escape.md"),
        &fixture.fields(),
    )
    .expect_err("parent traversal must fail");
    assert_eq!(unsafe_path.exit_code(), 2);

    let mut invalid = fixture.fields();
    invalid.insert("date".to_owned(), "[invalid".to_owned());
    let invalid = create_event(
        &fixture.request,
        "session",
        Path::new(EVENT_RELATIVE_PATH),
        &invalid,
    )
    .expect_err("invalid rendered frontmatter must fail");
    assert_eq!(invalid.exit_code(), 4);

    assert_eq!(fixture.state(), state_before);
    assert!(!fixture.event_path().exists());
    assert!(!fixture.journal().exists());
}

#[test]
fn recovers_unstarted_partial_and_state_only_event_publication() {
    let fixture = Fixture::new("rollback-recovery");
    let versions = fixture.successful_versions();

    fs::remove_file(fixture.event_path()).expect("remove event");
    fs::write(fixture.state_path(), &versions.state_before).expect("restore old state");
    fixture.write_creation_journal(&versions);
    let discarded = recover_pending_note_edit(&fixture.request).expect("discard unused journal");
    assert_eq!(discarded, NoteEditRecovery::Discarded);
    assert!(!fixture.journal().exists());

    fs::write(fixture.event_path(), &versions.event_after).expect("seed published event");
    fixture.write_creation_journal(&versions);
    let partial = recover_pending_note_edit(&fixture.request).expect("rollback partial event");
    assert_eq!(partial, NoteEditRecovery::RolledBack);
    assert!(!fixture.event_path().exists());
    assert_eq!(fixture.state(), versions.state_before);
    validate_project(&fixture.request).expect("event rollback validates");

    fs::write(fixture.state_path(), &versions.state_after).expect("seed state-only publication");
    fixture.write_creation_journal(&versions);
    let state_only =
        recover_pending_note_edit(&fixture.request).expect("rollback state-only event");
    assert_eq!(state_only, NoteEditRecovery::RolledBack);
    assert!(!fixture.event_path().exists());
    assert_eq!(fixture.state(), versions.state_before);
    validate_project(&fixture.request).expect("state-only rollback validates");
}

#[test]
fn finalizes_complete_event_publication_and_preserves_unexpected_bytes() {
    let fixture = Fixture::new("finalize-recovery");
    let versions = fixture.successful_versions();
    fixture.write_creation_journal(&versions);

    let finalized = recover_pending_note_edit(&fixture.request).expect("finalize complete event");
    assert_eq!(finalized, NoteEditRecovery::Finalized);
    assert_eq!(fixture.event(), versions.event_after);
    assert!(!fixture.journal().exists());

    fixture.write_creation_journal(&versions);
    fs::write(fixture.event_path(), "external writer\n").expect("seed unexpected event bytes");
    let error = recover_pending_note_edit(&fixture.request)
        .expect_err("unexpected event bytes must conflict");
    assert_eq!(error.exit_code(), 5);
    assert_eq!(fixture.event(), "external writer\n");
    assert!(fixture.journal().is_file());
}

struct Versions {
    event_after: String,
    state_before: String,
    state_after: String,
}

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    project: PathBuf,
    request: ResolveRequest,
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
        let project = root.join("Projects/example");
        fs::write(project.join("templates/session.md"), event_template())
            .expect("write project event template");
        fs::write(project.join("templates/handoff.md"), event_template())
            .expect("write project handoff template");
        fs::write(root.join("templates/entity.md"), "unused entity template\n")
            .expect("write non-event template");
        let request = ResolveRequest {
            root_override: Some(root.clone()),
            project_override: Some("example".to_owned()),
            cwd: repository,
            environment: ResolutionEnvironment::default(),
        };
        Self {
            _temp: temp,
            root,
            project,
            request,
        }
    }

    fn fields(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("date".to_owned(), "2026-07-15".to_owned()),
            ("title".to_owned(), "Goal integration".to_owned()),
            (
                "body".to_owned(),
                "Worked on [[Projects/example/entities/core|the core]].\n\nRelated: [[Projects/example/roadmap|Roadmap]]."
                    .to_owned(),
            ),
        ])
    }

    fn expected_event(&self) -> String {
        "---\nschema_version: 1\nproject: example\ntype: session\ndate: 2026-07-15\n---\n\n# Goal integration\n\nWorked on [[Projects/example/entities/core|the core]].\n\nRelated: [[Projects/example/roadmap|Roadmap]].\n"
            .to_owned()
    }

    fn event_path(&self) -> PathBuf {
        self.root.join(EVENT_ID)
    }

    fn state_path(&self) -> PathBuf {
        self.project.join(".akasha-state.toml")
    }

    fn journal(&self) -> PathBuf {
        self.project.join(NOTE_EDIT_JOURNAL_FILE)
    }

    fn event(&self) -> String {
        fs::read_to_string(self.event_path()).expect("read created event")
    }

    fn state(&self) -> String {
        fs::read_to_string(self.state_path()).expect("read project state")
    }

    fn successful_versions(&self) -> Versions {
        let state_before = self.state();
        create_event(
            &self.request,
            "session",
            Path::new(EVENT_RELATIVE_PATH),
            &self.fields(),
        )
        .expect("create successful event versions");
        Versions {
            event_after: self.event(),
            state_before,
            state_after: self.state(),
        }
    }

    fn write_creation_journal(&self, versions: &Versions) {
        let source = serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "project": "example",
            "id": EVENT_ID,
            "note_before": null,
            "note_after": versions.event_after,
            "state_before": versions.state_before,
            "state_after": versions.state_after,
        }))
        .expect("serialize event recovery journal");
        fs::write(self.journal(), format!("{source}\n")).expect("write event recovery journal");
    }
}

fn event_template() -> &'static str {
    "---\nschema_version: 1\nproject: {{project}}\ntype: {{type}}\ndate: {{date}}\n---\n\n# {{title}}\n\n{{body}}\n"
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
        let path =
            std::env::temp_dir().join(format!("akasha-event-{label}-{}-{id}", std::process::id()));
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
