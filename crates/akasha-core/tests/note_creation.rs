use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    NOTE_EDIT_JOURNAL_FILE, NoteClass, NoteEditRecovery, NoteTemplateScope, ResolutionEnvironment,
    ResolveRequest, create_mutable_note, recover_pending_note_edit, validate_project,
};
use serde_json::json;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
const RECORD_RELATIVE_PATH: &str = "created.md";
const RECORD_ID: &str = "Projects/example/records/tasks/created.md";
const ROADMAP_ID: &str = "Projects/example/roadmap.md";

#[test]
fn creates_record_with_exact_template_projection_and_valid_state() {
    let fixture = Fixture::new("record");
    let state_before = fixture.state();
    let projection = fixture.record_projection();

    let result = fixture.create_record(&projection).expect("create record");

    assert_eq!(result.id, RECORD_ID);
    assert_eq!(result.class, NoteClass::Record);
    assert_eq!(result.template_scope, NoteTemplateScope::Project);
    assert!(result.projection_changed);
    assert_eq!(result.projection, fixture.roadmap_path());
    assert_eq!(result.recovery, NoteEditRecovery::None);
    assert_eq!(fixture.record(), fixture.expected_record());
    assert_eq!(fixture.roadmap(), projection);
    assert_ne!(fixture.state(), state_before);
    assert!(!fixture.journal().exists());
    let report = validate_project(&fixture.request).expect("created record keeps project valid");
    assert_eq!(report.projections["roadmap"].sources, 3);
}

#[test]
fn creates_entity_from_root_template_and_updates_index_projection() {
    let fixture = Fixture::new("entity");
    let index = fixture.entity_projection();
    let fields = BTreeMap::from([
        ("entity".to_owned(), "navigation".to_owned()),
        ("kind".to_owned(), "subsystem".to_owned()),
        ("status".to_owned(), "active".to_owned()),
        ("reviewed".to_owned(), "2026-07-15".to_owned()),
        ("title".to_owned(), "Navigation".to_owned()),
        ("body".to_owned(), "Current navigation truth.".to_owned()),
    ]);

    let result = create_mutable_note(
        &fixture.request,
        "entity",
        Path::new("navigation.md"),
        &fields,
        &index,
    )
    .expect("create entity");

    assert_eq!(result.class, NoteClass::Entity);
    assert_eq!(result.template_scope, NoteTemplateScope::Root);
    assert_eq!(result.projection, fixture.index_path());
    assert_eq!(fixture.index(), index);
    let report = validate_project(&fixture.request).expect("created entity keeps project valid");
    assert_eq!(report.projections["index"].sources, 2);
}

#[test]
fn rejects_existing_event_and_invalid_template_inputs_without_writes() {
    let fixture = Fixture::new("rejections");
    let projection = fixture.record_projection();
    let state_before = fixture.state();

    fixture
        .create_record(&projection)
        .expect("create first record");
    let committed_state = fixture.state();
    let existing = fixture
        .create_record(&projection)
        .expect_err("existing record must conflict");
    assert_eq!(existing.exit_code(), 5);
    assert_eq!(fixture.state(), committed_state);

    fs::remove_file(fixture.record_path()).expect("remove committed record");
    fs::write(fixture.roadmap_path(), fixture.roadmap_before())
        .expect("restore roadmap projection");
    fs::write(fixture.state_path(), state_before).expect("restore original state");
    let event = create_mutable_note(
        &fixture.request,
        "session",
        Path::new("wrong-command.md"),
        &BTreeMap::new(),
        &projection,
    )
    .expect_err("event type must use create-event");
    assert_eq!(event.exit_code(), 2);

    let mut missing = fixture.record_fields();
    missing.remove("status");
    let missing = create_mutable_note(
        &fixture.request,
        "task",
        Path::new(RECORD_RELATIVE_PATH),
        &missing,
        &projection,
    )
    .expect_err("missing template field must fail");
    assert_eq!(missing.exit_code(), 2);
    assert!(!fixture.record_path().exists());
    assert!(!fixture.journal().exists());
}

#[test]
fn recovers_every_exact_partial_record_publication_state() {
    let fixture = Fixture::new("recovery");
    let versions = fixture.successful_versions();

    fixture.seed(&versions, false, false, false);
    fixture.write_creation_journal(&versions);
    assert_eq!(
        recover_pending_note_edit(&fixture.request).expect("discard unused journal"),
        NoteEditRecovery::Discarded
    );

    fixture.seed(&versions, true, false, false);
    fixture.write_creation_journal(&versions);
    assert_eq!(
        recover_pending_note_edit(&fixture.request).expect("rollback note-only publication"),
        NoteEditRecovery::RolledBack
    );
    fixture.assert_before(&versions);

    fixture.seed(&versions, true, true, false);
    fixture.write_creation_journal(&versions);
    assert_eq!(
        recover_pending_note_edit(&fixture.request).expect("rollback note and projection"),
        NoteEditRecovery::RolledBack
    );
    fixture.assert_before(&versions);

    fixture.seed(&versions, false, false, true);
    fixture.write_creation_journal(&versions);
    assert_eq!(
        recover_pending_note_edit(&fixture.request).expect("rollback state-only publication"),
        NoteEditRecovery::RolledBack
    );
    fixture.assert_before(&versions);

    fixture.seed(&versions, true, true, true);
    fixture.write_creation_journal(&versions);
    assert_eq!(
        recover_pending_note_edit(&fixture.request).expect("finalize complete publication"),
        NoteEditRecovery::Finalized
    );
    assert_eq!(fixture.record(), versions.note_after);
    assert_eq!(fixture.roadmap(), versions.projection_after);
    assert_eq!(fixture.state(), versions.state_after);
    assert!(!fixture.journal().exists());
}

#[test]
fn recovery_preserves_unexpected_projection_bytes_and_journal() {
    let fixture = Fixture::new("unexpected-recovery");
    let versions = fixture.successful_versions();
    fixture.write_creation_journal(&versions);
    fs::write(fixture.roadmap_path(), "external projection writer\n")
        .expect("seed unexpected projection bytes");

    let error = recover_pending_note_edit(&fixture.request)
        .expect_err("unexpected projection bytes must conflict");

    assert_eq!(error.exit_code(), 5);
    assert_eq!(fixture.roadmap(), "external projection writer\n");
    assert!(fixture.journal().is_file());
}

struct Versions {
    note_after: String,
    projection_before: String,
    projection_after: String,
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
        fs::write(project.join("templates/task.md"), record_template())
            .expect("write project record template");
        fs::write(
            project.join("templates/session.md"),
            "unused event template\n",
        )
        .expect("write project event template");
        fs::write(root.join("templates/entity.md"), entity_template())
            .expect("write root entity template");
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

    fn create_record(
        &self,
        projection: &str,
    ) -> Result<akasha_core::MutableNoteCreationResult, akasha_core::MutableNoteCreationError> {
        create_mutable_note(
            &self.request,
            "task",
            Path::new(RECORD_RELATIVE_PATH),
            &self.record_fields(),
            projection,
        )
    }

    fn record_fields(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("status".to_owned(), "open".to_owned()),
            ("created".to_owned(), "2026-07-15".to_owned()),
            ("updated".to_owned(), "2026-07-15".to_owned()),
            ("title".to_owned(), "Created task".to_owned()),
            (
                "body".to_owned(),
                "Tracks [[Projects/example/entities/core|the core]].".to_owned(),
            ),
        ])
    }

    fn expected_record(&self) -> String {
        "---\nschema_version: 1\nproject: example\ntype: task\nstatus: open\ncreated: 2026-07-15\nupdated: 2026-07-15\n---\n\n# Created task\n\nTracks [[Projects/example/entities/core|the core]].\n"
            .to_owned()
    }

    fn record_projection(&self) -> String {
        format!(
            "{}\n- [[{RECORD_ID}|Created task]]\n",
            self.roadmap_before().trim_end()
        )
    }

    fn entity_projection(&self) -> String {
        format!(
            "{}\n- [[Projects/example/entities/navigation|Navigation]]\n",
            self.index().trim_end()
        )
    }

    fn successful_versions(&self) -> Versions {
        let projection_before = self.roadmap();
        let state_before = self.state();
        let projection_after = self.record_projection();
        self.create_record(&projection_after)
            .expect("create successful record versions");
        Versions {
            note_after: self.record(),
            projection_before,
            projection_after,
            state_before,
            state_after: self.state(),
        }
    }

    fn seed(
        &self,
        versions: &Versions,
        note_after: bool,
        projection_after: bool,
        state_after: bool,
    ) {
        if self.record_path().exists() {
            fs::remove_file(self.record_path()).expect("remove record while seeding recovery");
        }
        if note_after {
            fs::write(self.record_path(), &versions.note_after).expect("seed created note");
        }
        fs::write(
            self.roadmap_path(),
            if projection_after {
                &versions.projection_after
            } else {
                &versions.projection_before
            },
        )
        .expect("seed projection");
        fs::write(
            self.state_path(),
            if state_after {
                &versions.state_after
            } else {
                &versions.state_before
            },
        )
        .expect("seed state");
    }

    fn assert_before(&self, versions: &Versions) {
        assert!(!self.record_path().exists());
        assert_eq!(self.roadmap(), versions.projection_before);
        assert_eq!(self.state(), versions.state_before);
        assert!(!self.journal().exists());
        validate_project(&self.request).expect("rolled-back project validates");
    }

    fn write_creation_journal(&self, versions: &Versions) {
        let source = serde_json::to_string_pretty(&json!({
            "schema_version": 2,
            "project": "example",
            "id": RECORD_ID,
            "note_before": null,
            "note_after": versions.note_after,
            "projection": {
                "id": ROADMAP_ID,
                "before": versions.projection_before,
                "after": versions.projection_after,
            },
            "state_before": versions.state_before,
            "state_after": versions.state_after,
        }))
        .expect("serialize record recovery journal");
        fs::write(self.journal(), format!("{source}\n")).expect("write record recovery journal");
    }

    fn record_path(&self) -> PathBuf {
        self.root.join(RECORD_ID)
    }

    fn roadmap_path(&self) -> PathBuf {
        self.project.join("roadmap.md")
    }

    fn index_path(&self) -> PathBuf {
        self.project.join("index.md")
    }

    fn state_path(&self) -> PathBuf {
        self.project.join(".akasha-state.toml")
    }

    fn journal(&self) -> PathBuf {
        self.project.join(NOTE_EDIT_JOURNAL_FILE)
    }

    fn record(&self) -> String {
        fs::read_to_string(self.record_path()).expect("read created record")
    }

    fn roadmap(&self) -> String {
        fs::read_to_string(self.roadmap_path()).expect("read roadmap")
    }

    fn roadmap_before(&self) -> String {
        "# Example roadmap\n\nSynthetic required projection fixture.\n".to_owned()
    }

    fn index(&self) -> String {
        fs::read_to_string(self.index_path()).expect("read index")
    }

    fn state(&self) -> String {
        fs::read_to_string(self.state_path()).expect("read project state")
    }
}

fn record_template() -> &'static str {
    "---\nschema_version: 1\nproject: {{project}}\ntype: {{type}}\nstatus: {{status}}\ncreated: {{created}}\nupdated: {{updated}}\n---\n\n# {{title}}\n\n{{body}}\n"
}

fn entity_template() -> &'static str {
    "---\nschema_version: 1\nentity: {{entity}}\nkind: {{kind}}\nstatus: {{status}}\nreviewed: {{reviewed}}\n---\n\n# {{title}}\n\n{{body}}\n"
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
            "akasha-note-creation-{label}-{}-{id}",
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
