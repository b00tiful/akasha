use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    NOTE_EDIT_JOURNAL_FILE, NoteEditRecovery, ResolutionEnvironment, ResolveRequest,
    recover_pending_note_edit, update_record, validate_project,
};
use serde_json::json;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
const RECORD_ID: &str = "Projects/example/records/tasks/active.md";
const ENTITY_ID: &str = "Projects/example/entities/core.md";
const EVENT_ID: &str = "Projects/example/events/sessions/2026-07-13.md";
const ROADMAP_ID: &str = "Projects/example/roadmap.md";

#[test]
fn updates_record_and_explicit_roadmap_with_valid_state() {
    let fixture = Fixture::new("update");
    let before = fixture.record();
    let replacement = replacement_record();
    let roadmap = replacement_roadmap();

    let result = update_record(&fixture.request, RECORD_ID, &before, replacement, roadmap)
        .expect("update record lifecycle state");

    assert!(result.changed);
    assert!(result.roadmap_changed);
    assert_eq!(result.note_type, "task");
    assert_eq!(result.id, RECORD_ID);
    assert_eq!(result.recovery, NoteEditRecovery::None);
    assert_eq!(fixture.record(), replacement);
    assert_eq!(fixture.roadmap(), roadmap);
    validate_project(&fixture.request).expect("updated project remains valid");
    assert!(!fixture.journal().exists());

    let no_op = update_record(
        &fixture.request,
        RECORD_ID,
        replacement,
        replacement,
        roadmap,
    )
    .expect("exact lifecycle rerun is a no-op");
    assert!(!no_op.changed);
    assert!(!no_op.roadmap_changed);
}

#[test]
fn rejects_stale_non_record_and_creation_metadata_changes_without_writes() {
    let fixture = Fixture::new("rejections");
    let record_before = fixture.record();
    let roadmap_before = fixture.roadmap();
    let state_before = fixture.state();

    let stale = update_record(
        &fixture.request,
        RECORD_ID,
        "stale source",
        replacement_record(),
        replacement_roadmap(),
    )
    .expect_err("stale expected bytes must conflict");
    assert_eq!(stale.exit_code(), 5);

    for id in [ENTITY_ID, EVENT_ID] {
        let source = fs::read_to_string(fixture.root.join(id)).expect("read non-record note");
        let error = update_record(
            &fixture.request,
            id,
            &source,
            &source,
            replacement_roadmap(),
        )
        .expect_err("only records can use lifecycle update");
        assert_eq!(error.exit_code(), 4);
    }

    let changed_created =
        replacement_record().replace("created: 2026-07-13", "created: 2026-07-16");
    let error = update_record(
        &fixture.request,
        RECORD_ID,
        &record_before,
        &changed_created,
        replacement_roadmap(),
    )
    .expect_err("record creation metadata must be immutable");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("created"));

    assert_eq!(fixture.record(), record_before);
    assert_eq!(fixture.roadmap(), roadmap_before);
    assert_eq!(fixture.state(), state_before);
    assert!(!fixture.journal().exists());
}

#[test]
fn recovers_record_and_roadmap_publication_in_reverse_order() {
    let fixture = Fixture::new("recovery");
    let versions = fixture.successful_versions();

    fixture.restore_before(&versions);
    fixture.write_journal(&versions);
    let discarded = recover_pending_note_edit(&fixture.request).expect("discard unused journal");
    assert_eq!(discarded, NoteEditRecovery::Discarded);

    fs::write(fixture.record_path(), &versions.note_after).expect("seed published record");
    fixture.write_journal(&versions);
    let note_rollback =
        recover_pending_note_edit(&fixture.request).expect("roll back record-only publication");
    assert_eq!(note_rollback, NoteEditRecovery::RolledBack);
    fixture.assert_before(&versions);

    fs::write(fixture.record_path(), &versions.note_after).expect("seed published record");
    fs::write(fixture.roadmap_path(), &versions.roadmap_after).expect("seed published roadmap");
    fixture.write_journal(&versions);
    let projection_rollback = recover_pending_note_edit(&fixture.request)
        .expect("roll back record and roadmap publication");
    assert_eq!(projection_rollback, NoteEditRecovery::RolledBack);
    fixture.assert_before(&versions);
    validate_project(&fixture.request).expect("rolled-back project validates");
}

#[test]
fn finalizes_complete_record_update_and_refuses_unexpected_projection_bytes() {
    let fixture = Fixture::new("finalize");
    let versions = fixture.successful_versions();
    fixture.write_journal(&versions);

    let finalized = recover_pending_note_edit(&fixture.request).expect("finalize complete update");
    assert_eq!(finalized, NoteEditRecovery::Finalized);
    assert!(!fixture.journal().exists());
    validate_project(&fixture.request).expect("finalized project validates");

    fixture.restore_before(&versions);
    fixture.write_journal(&versions);
    fs::write(fixture.roadmap_path(), "external roadmap bytes\n").expect("seed unexpected roadmap");
    let error = recover_pending_note_edit(&fixture.request)
        .expect_err("unexpected roadmap bytes must refuse recovery");
    assert_eq!(error.exit_code(), 5);
    assert_eq!(fixture.roadmap(), "external roadmap bytes\n");
    assert!(fixture.journal().is_file());
}

struct Versions {
    note_before: String,
    note_after: String,
    roadmap_before: String,
    roadmap_after: String,
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

    fn record_path(&self) -> PathBuf {
        self.root.join(RECORD_ID)
    }

    fn roadmap_path(&self) -> PathBuf {
        self.root.join(ROADMAP_ID)
    }

    fn state_path(&self) -> PathBuf {
        self.project.join(".akasha-state.toml")
    }

    fn journal(&self) -> PathBuf {
        self.project.join(NOTE_EDIT_JOURNAL_FILE)
    }

    fn record(&self) -> String {
        fs::read_to_string(self.record_path()).expect("read record")
    }

    fn roadmap(&self) -> String {
        fs::read_to_string(self.roadmap_path()).expect("read roadmap")
    }

    fn state(&self) -> String {
        fs::read_to_string(self.state_path()).expect("read state")
    }

    fn successful_versions(&self) -> Versions {
        let note_before = self.record();
        let roadmap_before = self.roadmap();
        let state_before = self.state();
        update_record(
            &self.request,
            RECORD_ID,
            &note_before,
            replacement_record(),
            replacement_roadmap(),
        )
        .expect("create successful update versions");
        Versions {
            note_before,
            note_after: self.record(),
            roadmap_before,
            roadmap_after: self.roadmap(),
            state_before,
            state_after: self.state(),
        }
    }

    fn restore_before(&self, versions: &Versions) {
        fs::write(self.record_path(), &versions.note_before).expect("restore record");
        fs::write(self.roadmap_path(), &versions.roadmap_before).expect("restore roadmap");
        fs::write(self.state_path(), &versions.state_before).expect("restore state");
    }

    fn assert_before(&self, versions: &Versions) {
        assert_eq!(self.record(), versions.note_before);
        assert_eq!(self.roadmap(), versions.roadmap_before);
        assert_eq!(self.state(), versions.state_before);
        assert!(!self.journal().exists());
    }

    fn write_journal(&self, versions: &Versions) {
        let source = serde_json::to_string_pretty(&json!({
            "schema_version": 2,
            "project": "example",
            "id": RECORD_ID,
            "note_before": versions.note_before,
            "note_after": versions.note_after,
            "projection": {
                "id": ROADMAP_ID,
                "before": versions.roadmap_before,
                "after": versions.roadmap_after,
            },
            "state_before": versions.state_before,
            "state_after": versions.state_after,
        }))
        .expect("serialize record update journal");
        fs::write(self.journal(), format!("{source}\n")).expect("write record update journal");
    }
}

fn replacement_record() -> &'static str {
    "---\nschema_version: 1\nproject: example\ntype: task\nstatus: resolved\ncreated: 2026-07-13\nupdated: 2026-07-16\n---\n\n# Synthetic task\n\nLifecycle update complete.\n"
}

fn replacement_roadmap() -> &'static str {
    "# Example roadmap\n\nSynthetic required projection fixture.\n\n- [[Projects/example/records/tasks/active|Synthetic task]] — resolved\n"
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
            "akasha-core-record-update-{label}-{}-{id}",
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
