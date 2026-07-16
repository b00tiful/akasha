use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    NOTE_EDIT_JOURNAL_FILE, NoteEditRecovery, ResolutionEnvironment, ResolveRequest,
    recover_pending_note_edit, update_entity, validate_project,
};
use serde_json::json;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
const ENTITY_ID: &str = "Projects/example/entities/core.md";
const RECORD_ID: &str = "Projects/example/records/tasks/active.md";
const EVENT_ID: &str = "Projects/example/events/sessions/2026-07-13.md";
const INDEX_ID: &str = "Projects/example/index.md";

#[test]
fn updates_entity_and_explicit_index_with_valid_state() {
    let fixture = Fixture::new("update");
    let before = fixture.entity();
    let replacement = replacement_entity();
    let index = replacement_index();

    let result = update_entity(&fixture.request, ENTITY_ID, &before, replacement, index)
        .expect("update maintained entity");

    assert!(result.changed);
    assert!(result.index_changed);
    assert_eq!(result.note_type, "entity");
    assert_eq!(result.id, ENTITY_ID);
    assert_eq!(result.recovery, NoteEditRecovery::None);
    assert_eq!(fixture.entity(), replacement);
    assert_eq!(fixture.index(), index);
    validate_project(&fixture.request).expect("updated project remains valid");
    assert!(!fixture.journal().exists());

    let no_op = update_entity(&fixture.request, ENTITY_ID, replacement, replacement, index)
        .expect("exact entity rerun is a no-op");
    assert!(!no_op.changed);
    assert!(!no_op.index_changed);
}

#[test]
fn rejects_stale_non_entity_and_canonical_name_changes_without_writes() {
    let fixture = Fixture::new("rejections");
    let entity_before = fixture.entity();
    let index_before = fixture.index();
    let state_before = fixture.state();

    let stale = update_entity(
        &fixture.request,
        ENTITY_ID,
        "stale source",
        replacement_entity(),
        replacement_index(),
    )
    .expect_err("stale expected bytes must conflict");
    assert_eq!(stale.exit_code(), 5);

    for id in [RECORD_ID, EVENT_ID] {
        let source = fs::read_to_string(fixture.root.join(id)).expect("read non-entity note");
        let error = update_entity(&fixture.request, id, &source, &source, replacement_index())
            .expect_err("only entities can use maintained entity update");
        assert_eq!(error.exit_code(), 4);
    }

    let renamed = replacement_entity().replace("entity: core", "entity: renamed-core");
    let error = update_entity(
        &fixture.request,
        ENTITY_ID,
        &entity_before,
        &renamed,
        replacement_index(),
    )
    .expect_err("canonical entity name must be immutable in an update");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("rename"));

    assert_eq!(fixture.entity(), entity_before);
    assert_eq!(fixture.index(), index_before);
    assert_eq!(fixture.state(), state_before);
    assert!(!fixture.journal().exists());
}

#[test]
fn recovers_entity_and_index_publication_in_reverse_order() {
    let fixture = Fixture::new("recovery");
    let versions = fixture.successful_versions();

    fixture.restore_before(&versions);
    fixture.write_journal(&versions);
    let discarded = recover_pending_note_edit(&fixture.request).expect("discard unused journal");
    assert_eq!(discarded, NoteEditRecovery::Discarded);

    fs::write(fixture.entity_path(), &versions.note_after).expect("seed published entity");
    fixture.write_journal(&versions);
    let note_rollback =
        recover_pending_note_edit(&fixture.request).expect("roll back entity-only publication");
    assert_eq!(note_rollback, NoteEditRecovery::RolledBack);
    fixture.assert_before(&versions);

    fs::write(fixture.entity_path(), &versions.note_after).expect("seed published entity");
    fs::write(fixture.index_path(), &versions.index_after).expect("seed published index");
    fixture.write_journal(&versions);
    let projection_rollback = recover_pending_note_edit(&fixture.request)
        .expect("roll back entity and index publication");
    assert_eq!(projection_rollback, NoteEditRecovery::RolledBack);
    fixture.assert_before(&versions);
    validate_project(&fixture.request).expect("rolled-back project validates");
}

#[test]
fn finalizes_complete_entity_update_and_refuses_unexpected_projection_bytes() {
    let fixture = Fixture::new("finalize");
    let versions = fixture.successful_versions();
    fixture.write_journal(&versions);

    let finalized = recover_pending_note_edit(&fixture.request).expect("finalize complete update");
    assert_eq!(finalized, NoteEditRecovery::Finalized);
    assert!(!fixture.journal().exists());
    validate_project(&fixture.request).expect("finalized project validates");

    fixture.restore_before(&versions);
    fixture.write_journal(&versions);
    fs::write(fixture.index_path(), "external index bytes\n").expect("seed unexpected index");
    let error = recover_pending_note_edit(&fixture.request)
        .expect_err("unexpected index bytes must refuse recovery");
    assert_eq!(error.exit_code(), 5);
    assert_eq!(fixture.index(), "external index bytes\n");
    assert!(fixture.journal().is_file());
}

struct Versions {
    note_before: String,
    note_after: String,
    index_before: String,
    index_after: String,
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

    fn entity_path(&self) -> PathBuf {
        self.root.join(ENTITY_ID)
    }

    fn index_path(&self) -> PathBuf {
        self.root.join(INDEX_ID)
    }

    fn state_path(&self) -> PathBuf {
        self.project.join(".akasha-state.toml")
    }

    fn journal(&self) -> PathBuf {
        self.project.join(NOTE_EDIT_JOURNAL_FILE)
    }

    fn entity(&self) -> String {
        fs::read_to_string(self.entity_path()).expect("read entity")
    }

    fn index(&self) -> String {
        fs::read_to_string(self.index_path()).expect("read index")
    }

    fn state(&self) -> String {
        fs::read_to_string(self.state_path()).expect("read state")
    }

    fn successful_versions(&self) -> Versions {
        let note_before = self.entity();
        let index_before = self.index();
        let state_before = self.state();
        update_entity(
            &self.request,
            ENTITY_ID,
            &note_before,
            replacement_entity(),
            replacement_index(),
        )
        .expect("create successful entity update versions");
        Versions {
            note_before,
            note_after: self.entity(),
            index_before,
            index_after: self.index(),
            state_before,
            state_after: self.state(),
        }
    }

    fn restore_before(&self, versions: &Versions) {
        fs::write(self.entity_path(), &versions.note_before).expect("restore entity");
        fs::write(self.index_path(), &versions.index_before).expect("restore index");
        fs::write(self.state_path(), &versions.state_before).expect("restore state");
    }

    fn assert_before(&self, versions: &Versions) {
        assert_eq!(self.entity(), versions.note_before);
        assert_eq!(self.index(), versions.index_before);
        assert_eq!(self.state(), versions.state_before);
        assert!(!self.journal().exists());
    }

    fn write_journal(&self, versions: &Versions) {
        let source = serde_json::to_string_pretty(&json!({
            "schema_version": 2,
            "project": "example",
            "id": ENTITY_ID,
            "note_before": versions.note_before,
            "note_after": versions.note_after,
            "projection": {
                "id": INDEX_ID,
                "before": versions.index_before,
                "after": versions.index_after,
            },
            "state_before": versions.state_before,
            "state_after": versions.state_after,
        }))
        .expect("serialize entity update journal");
        fs::write(self.journal(), format!("{source}\n")).expect("write entity update journal");
    }
}

fn replacement_entity() -> &'static str {
    "---\nschema_version: 1\nentity: core\nkind: service\nstatus: deprecated\nreviewed: 2026-07-16\n---\n\n# Synthetic entity\n\nCurrent understanding updated.\n"
}

fn replacement_index() -> &'static str {
    "# Example project\n\nSynthetic resolution fixture for Akasha's core tests.\n\n- [[Projects/example/entities/core|Core]] — deprecated service\n"
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
            "akasha-core-entity-update-{label}-{}-{id}",
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
