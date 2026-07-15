use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    NOTE_EDIT_JOURNAL_FILE, NoteEditRecovery, ResolutionEnvironment, ResolveRequest,
    recover_pending_note_edit, replace_library_document, validate_project,
};
use serde_json::json;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
const ENTITY_ID: &str = "Projects/example/entities/core.md";
const EVENT_ID: &str = "Projects/example/events/sessions/2026-07-13.md";
const GLOBAL_ID: &str = "Global/entities/rust-pattern.md";

#[test]
fn replaces_mutable_note_exactly_and_keeps_project_state_valid() {
    let fixture = Fixture::new("replace");
    let before = fixture.note();
    let replacement = format!(
        "{before}\n## Exact editor corpus\n\n**Bold** [[Global/entities/rust-pattern|pattern]]\n\n```unknown\n{{ untouched }}\n```\n\n> [!note] Callout stays canonical.\n"
    );

    let result = replace_library_document(&fixture.request, ENTITY_ID, &before, &replacement)
        .expect("replace mutable entity");

    assert!(result.changed);
    assert_eq!(result.recovery, NoteEditRecovery::None);
    assert_eq!(fixture.note(), replacement);
    validate_project(&fixture.request).expect("edited project remains valid");
    assert!(!fixture.journal().exists());
    assert!(fixture.project.join(".akasha-write.lock").is_file());

    let no_op = replace_library_document(&fixture.request, ENTITY_ID, &replacement, &replacement)
        .expect("exact source rerun is a no-op");
    assert!(!no_op.changed);
}

#[test]
fn preserves_crlf_whitespace_and_final_newline_exactly() {
    let fixture = Fixture::new("crlf");
    let before = fixture.note();
    let replacement = "---\r\nschema_version: 1\r\nentity: core\r\nkind: subsystem\r\nstatus: active\r\nreviewed: 2026-07-15\r\n---\r\n\r\n# Synthetic entity\r\n\r\n  unknown:: syntax  \r\n";

    replace_library_document(&fixture.request, ENTITY_ID, &before, replacement)
        .expect("replace with exact CRLF source");

    assert_eq!(
        fs::read(fixture.note_path()).expect("read note"),
        replacement.as_bytes()
    );
    validate_project(&fixture.request).expect("CRLF project remains valid");
}

#[test]
fn stale_invalid_immutable_and_global_edits_fail_without_note_mutation() {
    let fixture = Fixture::new("rejections");
    let before = fixture.note();
    let state_before = fixture.state();

    let stale = replace_library_document(&fixture.request, ENTITY_ID, "stale source", &before)
        .expect_err("stale baseline must conflict");
    assert_eq!(stale.exit_code(), 5);

    let invalid = before.replace("status: active\n", "");
    let invalid = replace_library_document(&fixture.request, ENTITY_ID, &before, &invalid)
        .expect_err("missing required field must fail");
    assert_eq!(invalid.exit_code(), 4);

    let event = fs::read_to_string(fixture.root.join(EVENT_ID)).expect("read event");
    let event_error = replace_library_document(
        &fixture.request,
        EVENT_ID,
        &event,
        &format!("{event}\nchanged\n"),
    )
    .expect_err("immutable event must fail");
    assert_eq!(event_error.exit_code(), 4);

    let global = fs::read_to_string(fixture.root.join(GLOBAL_ID)).expect("read global note");
    let global_error = replace_library_document(
        &fixture.request,
        GLOBAL_ID,
        &global,
        &format!("{global}\nchanged\n"),
    )
    .expect_err("global entity is outside first edit scope");
    assert_eq!(global_error.exit_code(), 4);

    assert_eq!(fixture.note(), before);
    assert_eq!(fixture.state(), state_before);
    assert!(!fixture.journal().exists());
}

#[test]
fn active_project_writer_lock_conflicts_without_writes() {
    let fixture = Fixture::new("writer-lock");
    let before = fixture.note();
    let lock = fixture.project.join(".akasha-write.lock");
    fs::write(&lock, []).expect("create writer lock file");
    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock)
        .expect("open writer lock");
    lock_file.try_lock().expect("hold writer lock");

    let error = replace_library_document(
        &fixture.request,
        ENTITY_ID,
        &before,
        &format!("{before}\nblocked\n"),
    )
    .expect_err("active writer must conflict");

    assert_eq!(error.exit_code(), 5);
    assert_eq!(fixture.note(), before);
    assert!(!fixture.journal().exists());
}

#[test]
fn recovers_partial_note_publication_by_rolling_back() {
    let fixture = Fixture::new("partial-recovery");
    let versions = fixture.successful_versions();
    fs::write(fixture.note_path(), &versions.note_after).expect("seed published note");
    fs::write(fixture.state_path(), &versions.state_before).expect("seed old state");
    fixture.write_journal(&versions);

    let recovery =
        recover_pending_note_edit(&fixture.request).expect("recover partial note publication");

    assert_eq!(recovery, NoteEditRecovery::RolledBack);
    assert_eq!(fixture.note(), versions.note_before);
    assert_eq!(fixture.state(), versions.state_before);
    assert!(!fixture.journal().exists());
    validate_project(&fixture.request).expect("rolled-back project validates");
}

#[test]
fn discards_an_unstarted_edit_and_rolls_back_state_only_publication() {
    let fixture = Fixture::new("other-recovery-states");
    let versions = fixture.successful_versions();
    fs::write(fixture.note_path(), &versions.note_before).expect("restore old note");
    fs::write(fixture.state_path(), &versions.state_before).expect("restore old state");
    fixture.write_journal(&versions);

    let discarded = recover_pending_note_edit(&fixture.request).expect("discard unstarted edit");
    assert_eq!(discarded, NoteEditRecovery::Discarded);
    assert!(!fixture.journal().exists());

    fs::write(fixture.state_path(), &versions.state_after).expect("seed published state");
    fixture.write_journal(&versions);
    let rolled_back =
        recover_pending_note_edit(&fixture.request).expect("recover state-only publication");

    assert_eq!(rolled_back, NoteEditRecovery::RolledBack);
    assert_eq!(fixture.note(), versions.note_before);
    assert_eq!(fixture.state(), versions.state_before);
    assert!(!fixture.journal().exists());
    validate_project(&fixture.request).expect("state rollback validates");
}

#[test]
fn finalizes_a_fully_published_edit_and_rejects_unexpected_bytes() {
    let fixture = Fixture::new("finalized-recovery");
    let versions = fixture.successful_versions();
    fixture.write_journal(&versions);

    let recovery = recover_pending_note_edit(&fixture.request).expect("finalize completed edit");
    assert_eq!(recovery, NoteEditRecovery::Finalized);
    assert_eq!(fixture.note(), versions.note_after);
    assert!(!fixture.journal().exists());

    fs::write(fixture.note_path(), &versions.note_before).expect("restore old note");
    fs::write(fixture.state_path(), &versions.state_before).expect("restore old state");
    fixture.write_journal(&versions);
    fs::write(fixture.note_path(), "external writer\n").expect("seed unexpected bytes");

    let error = recover_pending_note_edit(&fixture.request)
        .expect_err("unexpected recovery bytes must conflict");
    assert_eq!(error.exit_code(), 5);
    assert_eq!(fixture.note(), "external writer\n");
    assert!(fixture.journal().is_file());
}

struct Versions {
    note_before: String,
    note_after: String,
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

    fn note_path(&self) -> PathBuf {
        self.root.join(ENTITY_ID)
    }

    fn state_path(&self) -> PathBuf {
        self.project.join(".akasha-state.toml")
    }

    fn note(&self) -> String {
        fs::read_to_string(self.note_path()).expect("read entity note")
    }

    fn state(&self) -> String {
        fs::read_to_string(self.state_path()).expect("read project state")
    }

    fn journal(&self) -> PathBuf {
        self.project.join(NOTE_EDIT_JOURNAL_FILE)
    }

    fn successful_versions(&self) -> Versions {
        let note_before = self.note();
        let state_before = self.state();
        let note_after = format!("{note_before}\nRecovered edit.\n");
        replace_library_document(&self.request, ENTITY_ID, &note_before, &note_after)
            .expect("create successful edit versions");
        let state_after = self.state();
        Versions {
            note_before,
            note_after,
            state_before,
            state_after,
        }
    }

    fn write_journal(&self, versions: &Versions) {
        let source = serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "project": "example",
            "id": ENTITY_ID,
            "note_before": versions.note_before,
            "note_after": versions.note_after,
            "state_before": versions.state_before,
            "state_after": versions.state_after,
        }))
        .expect("serialize recovery journal");
        fs::write(self.journal(), format!("{source}\n")).expect("write recovery journal");
    }
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
            "akasha-note-edit-{label}-{}-{id}",
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
