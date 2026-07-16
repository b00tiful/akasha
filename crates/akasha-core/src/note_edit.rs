use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::str;

use serde::{Deserialize, Serialize};

use crate::evidence::collect_canonical_evidence;
use crate::library::build_library_projection;
use crate::project_validation::{
    ProjectValidationError, validate_project, validate_wikilinks_with_targets,
};
use crate::resolution::{
    NoteClass, ResolveError, ResolveRequest, RootConfig, load_root_config, resolve_project,
};
use crate::state::{CanonicalNoteEvidence, PROJECT_STATE_FILE, render_updated_project_state};
use crate::validation::{parse_leading_frontmatter_bytes, validate_configured_note};
use crate::writes::{
    AtomicCreateError, CheckedReplaceError, ProjectWriteLock, create_file_atomically,
    replace_file_if_unchanged, sync_directory,
};

pub const NOTE_EDIT_JOURNAL_FILE: &str = ".akasha-edit-journal.json";
const NOTE_EDIT_JOURNAL_SCHEMA_VERSION: u32 = 1;
const NOTE_PROJECTION_JOURNAL_SCHEMA_VERSION: u32 = 2;
const ONBOARDING_BATCH_JOURNAL_SCHEMA_VERSION: u32 = 3;

/// Recovery work performed before a checked note edit or library load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoteEditRecovery {
    None,
    Discarded,
    RolledBack,
    Finalized,
}

/// Result of one exact-source mutable-note replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteEditResult {
    pub root: PathBuf,
    pub project: String,
    pub project_dir: PathBuf,
    pub id: String,
    pub changed: bool,
    pub recovery: NoteEditRecovery,
}

/// Result of one exact-source record update with an explicitly accepted roadmap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordUpdateResult {
    pub root: PathBuf,
    pub project: String,
    pub project_dir: PathBuf,
    pub note_type: String,
    pub id: String,
    pub path: PathBuf,
    pub changed: bool,
    pub roadmap: PathBuf,
    pub roadmap_changed: bool,
    pub state: PathBuf,
    pub recovery: NoteEditRecovery,
}

/// Result of one exact-source entity update with an explicitly accepted index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntityUpdateResult {
    pub root: PathBuf,
    pub project: String,
    pub project_dir: PathBuf,
    pub note_type: String,
    pub id: String,
    pub path: PathBuf,
    pub changed: bool,
    pub index: PathBuf,
    pub index_changed: bool,
    pub state: PathBuf,
    pub recovery: NoteEditRecovery,
}

/// A resolution, validation, optimistic-concurrency, filesystem, or recovery failure.
#[derive(Debug)]
pub enum NoteEditError {
    Resolve(Box<ResolveError>),
    Project(Box<ProjectValidationError>),
    Validation {
        path: PathBuf,
        message: String,
    },
    Conflict {
        path: PathBuf,
        message: String,
    },
    Creation(AtomicCreateError),
    FileSystem {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Recovery {
        original: Box<NoteEditError>,
        recovery: Box<NoteEditError>,
    },
}

impl NoteEditError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Resolve(error) => error.exit_code(),
            Self::Project(error) => error.exit_code(),
            Self::Validation { .. } => 4,
            Self::Conflict { .. } => 5,
            Self::Creation(error) => error.exit_code(),
            Self::FileSystem { .. } | Self::Recovery { .. } => 6,
        }
    }
}

impl fmt::Display for NoteEditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolve(error) => error.fmt(formatter),
            Self::Project(error) => error.fmt(formatter),
            Self::Validation { path, message } => {
                write!(
                    formatter,
                    "invalid note edit at {}: {message}",
                    path.display()
                )
            }
            Self::Conflict { path, message } => {
                write!(
                    formatter,
                    "note edit conflict at {}: {message}",
                    path.display()
                )
            }
            Self::Creation(error) => error.fmt(formatter),
            Self::FileSystem {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} at {}: {source}",
                path.display()
            ),
            Self::Recovery { original, recovery } => write!(
                formatter,
                "note edit failed and automatic recovery also failed: original error: {original}; recovery error: {recovery}"
            ),
        }
    }
}

impl Error for NoteEditError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Resolve(error) => Some(error.as_ref()),
            Self::Project(error) => Some(error.as_ref()),
            Self::Creation(error) => Some(error),
            Self::FileSystem { source, .. } => Some(source),
            Self::Recovery { original, .. } => Some(original.as_ref()),
            Self::Validation { .. } | Self::Conflict { .. } => None,
        }
    }
}

impl From<ResolveError> for NoteEditError {
    fn from(error: ResolveError) -> Self {
        Self::Resolve(Box::new(error))
    }
}

impl From<ProjectValidationError> for NoteEditError {
    fn from(error: ProjectValidationError) -> Self {
        Self::Project(Box::new(error))
    }
}

impl From<AtomicCreateError> for NoteEditError {
    fn from(error: AtomicCreateError) -> Self {
        Self::Creation(error)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NoteEditJournal {
    schema_version: u32,
    project: String,
    id: String,
    note_before: Option<String>,
    note_after: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    projection: Option<JournalProjection>,
    state_before: String,
    state_after: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct JournalProjection {
    id: String,
    before: String,
    after: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OnboardingBatchJournal {
    schema_version: u32,
    project: String,
    notes: Vec<JournalCreatedNote>,
    projections: Vec<JournalProjection>,
    state_before: String,
    state_after: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct JournalCreatedNote {
    id: String,
    after: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum NoteMutationJournal {
    Note(NoteEditJournal),
    Onboarding(OnboardingBatchJournal),
}

pub(crate) struct OnboardingJournalNote {
    pub(crate) id: String,
    pub(crate) after: String,
}

pub(crate) struct OnboardingBatchJournalInput<'a> {
    pub(crate) project: &'a str,
    pub(crate) notes: Vec<OnboardingJournalNote>,
    pub(crate) index_id: &'a str,
    pub(crate) index_before: &'a str,
    pub(crate) index_after: &'a str,
    pub(crate) roadmap_id: &'a str,
    pub(crate) roadmap_before: &'a str,
    pub(crate) roadmap_after: &'a str,
    pub(crate) state_before: &'a str,
    pub(crate) state_after: &'a str,
}

/// Replace one selected project's mutable canonical note from an exact loaded baseline.
pub fn replace_library_document(
    request: &ResolveRequest,
    id: &str,
    expected_source: &str,
    replacement_source: &str,
) -> Result<NoteEditResult, NoteEditError> {
    let resolved = resolve_project(request)?;
    let _lock = ProjectWriteLock::acquire(&resolved.project_dir)?;
    let recovery = recover_note_mutation_locked(request, &resolved.project_dir)?;
    let projection = build_library_projection(request)?;
    let book = projection
        .projects
        .iter()
        .find(|shelf| shelf.project == resolved.project)
        .into_iter()
        .flat_map(|shelf| &shelf.categories)
        .flat_map(|category| &category.books)
        .find(|book| book.id == id)
        .ok_or_else(|| NoteEditError::Validation {
            path: PathBuf::from(id),
            message: "only a projected note from the selected project can be edited".to_owned(),
        })?;
    if book.class == NoteClass::Event {
        return Err(NoteEditError::Validation {
            path: PathBuf::from(id),
            message: "immutable event notes cannot be replaced".to_owned(),
        });
    }

    let path = resolved.root.join(id);
    let current = read_regular_file(&path, "read the selected canonical note")?;
    if current != expected_source.as_bytes() {
        return Err(NoteEditError::Conflict {
            path,
            message: "the canonical note no longer matches the source loaded by the editor"
                .to_owned(),
        });
    }
    if replacement_source == expected_source {
        return Ok(NoteEditResult {
            root: resolved.root,
            project: resolved.project,
            project_dir: resolved.project_dir,
            id: id.to_owned(),
            changed: false,
            recovery,
        });
    }

    let config = load_root_config(&resolved.root)?;
    validate_replacement(
        &resolved.root,
        &resolved.project,
        &path,
        &book.note_type,
        replacement_source.as_bytes(),
        &config,
    )?;
    let evidence = collect_candidate_evidence(
        &resolved.project_dir,
        &path,
        replacement_source.as_bytes(),
        &config,
    )?;
    let index = read_regular_file(
        &resolved.project_dir.join(&config.project.index),
        "read the current index projection",
    )?;
    let roadmap = read_regular_file(
        &resolved.project_dir.join(&config.project.roadmap),
        "read the current roadmap projection",
    )?;
    let state_path = resolved.project_dir.join(PROJECT_STATE_FILE);
    let state_before = read_regular_file(&state_path, "read the current project state")?;
    let state_before_text =
        str::from_utf8(&state_before).map_err(|error| NoteEditError::Validation {
            path: state_path.clone(),
            message: format!("project state is not valid UTF-8: {error}"),
        })?;
    let state_after = render_updated_project_state(
        state_before_text,
        &resolved.project_dir,
        &index,
        &roadmap,
        &evidence,
        &BTreeSet::new(),
    )
    .map_err(|message| NoteEditError::Validation {
        path: state_path.clone(),
        message,
    })?;
    let state_after_text = str::from_utf8(&state_after)
        .expect("the deterministic project-state renderer always emits UTF-8");
    let (journal_path, journal_source) = write_note_mutation_journal(
        &resolved.project_dir,
        &resolved.project,
        id,
        Some(expected_source),
        replacement_source,
        state_before_text,
        state_after_text,
    )?;

    let operation = (|| {
        replace_file_if_unchanged(
            &path,
            expected_source.as_bytes(),
            replacement_source.as_bytes(),
        )
        .map_err(map_checked_replace)?;
        sync_replacement_directory(&path, "sync the canonical note replacement")?;
        replace_file_if_unchanged(&state_path, &state_before, &state_after)
            .map_err(map_checked_replace)?;
        sync_replacement_directory(&state_path, "sync the project state replacement")?;
        validate_project(request)?;
        complete_note_mutation_journal(&journal_path, &journal_source, &resolved.project_dir)?;
        Ok(NoteEditResult {
            root: resolved.root.clone(),
            project: resolved.project.clone(),
            project_dir: resolved.project_dir.clone(),
            id: id.to_owned(),
            changed: true,
            recovery,
        })
    })();

    match operation {
        Ok(result) => Ok(result),
        Err(original) => match recover_note_mutation_locked(request, &resolved.project_dir) {
            Ok(NoteEditRecovery::Finalized) => Ok(NoteEditResult {
                root: resolved.root,
                project: resolved.project,
                project_dir: resolved.project_dir,
                id: id.to_owned(),
                changed: true,
                recovery: NoteEditRecovery::Finalized,
            }),
            Ok(_) => Err(original),
            Err(recovery) => Err(NoteEditError::Recovery {
                original: Box::new(original),
                recovery: Box::new(recovery),
            }),
        },
    }
}

/// Update one selected-project record and explicitly accept its complete roadmap projection.
pub fn update_record(
    request: &ResolveRequest,
    id: &str,
    expected_source: &str,
    replacement_source: &str,
    roadmap_source: &str,
) -> Result<RecordUpdateResult, NoteEditError> {
    let resolved = resolve_project(request)?;
    let _lock = ProjectWriteLock::acquire(&resolved.project_dir)?;
    let recovery = recover_note_mutation_locked(request, &resolved.project_dir)?;
    let projection = build_library_projection(request)?;
    let book = projection
        .projects
        .iter()
        .find(|shelf| shelf.project == resolved.project)
        .into_iter()
        .flat_map(|shelf| &shelf.categories)
        .flat_map(|category| &category.books)
        .find(|book| book.id == id)
        .ok_or_else(|| NoteEditError::Validation {
            path: PathBuf::from(id),
            message: "only a projected note from the selected project can be updated".to_owned(),
        })?;
    if book.class != NoteClass::Record {
        return Err(NoteEditError::Validation {
            path: PathBuf::from(id),
            message: "record updates require a configured project record".to_owned(),
        });
    }
    let note_type = book.note_type.clone();

    let path = resolved.root.join(id);
    let current = read_regular_file(&path, "read the selected canonical record")?;
    if current != expected_source.as_bytes() {
        return Err(NoteEditError::Conflict {
            path,
            message: "the canonical record no longer matches the supplied expected source"
                .to_owned(),
        });
    }

    let config = load_root_config(&resolved.root)?;
    validate_replacement(
        &resolved.root,
        &resolved.project,
        &path,
        &note_type,
        replacement_source.as_bytes(),
        &config,
    )?;
    validate_preserved_record_metadata(
        &path,
        expected_source.as_bytes(),
        replacement_source.as_bytes(),
    )?;

    let evidence = collect_candidate_evidence(
        &resolved.project_dir,
        &path,
        replacement_source.as_bytes(),
        &config,
    )?;
    let index_path = resolved.project_dir.join(&config.project.index);
    let roadmap_path = resolved.project_dir.join(&config.project.roadmap);
    let index = read_regular_file(&index_path, "read the current index projection")?;
    let roadmap_before = read_regular_file(&roadmap_path, "read the current roadmap projection")?;
    let roadmap_before_text =
        str::from_utf8(&roadmap_before).map_err(|error| NoteEditError::Validation {
            path: roadmap_path.clone(),
            message: format!("current roadmap projection is not valid UTF-8: {error}"),
        })?;
    let state_path = resolved.project_dir.join(PROJECT_STATE_FILE);
    let note_changed = replacement_source != expected_source;
    let roadmap_changed = roadmap_before != roadmap_source.as_bytes();
    if !note_changed && !roadmap_changed {
        return Ok(RecordUpdateResult {
            root: resolved.root,
            project: resolved.project,
            project_dir: resolved.project_dir,
            note_type,
            id: id.to_owned(),
            path,
            changed: false,
            roadmap: roadmap_path,
            roadmap_changed: false,
            state: state_path,
            recovery,
        });
    }

    let state_before = read_regular_file(&state_path, "read the current project state")?;
    let state_before_text =
        str::from_utf8(&state_before).map_err(|error| NoteEditError::Validation {
            path: state_path.clone(),
            message: format!("project state is not valid UTF-8: {error}"),
        })?;
    let state_after = render_updated_project_state(
        state_before_text,
        &resolved.project_dir,
        &index,
        roadmap_source.as_bytes(),
        &evidence,
        &BTreeSet::new(),
    )
    .map_err(|message| NoteEditError::Validation {
        path: state_path.clone(),
        message,
    })?;
    let state_after_text = str::from_utf8(&state_after)
        .expect("the deterministic project-state renderer always emits UTF-8");
    let roadmap_id = vault_relative_identity(&resolved.root, &roadmap_path)?;
    let (journal_path, journal_source) = write_note_projection_mutation_journal(
        &resolved.project_dir,
        &NoteProjectionJournal {
            project: &resolved.project,
            id,
            note_before: Some(expected_source),
            note_after: replacement_source,
            projection_id: &roadmap_id,
            projection_before: roadmap_before_text,
            projection_after: roadmap_source,
            state_before: state_before_text,
            state_after: state_after_text,
        },
    )?;
    let result_for = |recovery| RecordUpdateResult {
        root: resolved.root.clone(),
        project: resolved.project.clone(),
        project_dir: resolved.project_dir.clone(),
        note_type: note_type.clone(),
        id: id.to_owned(),
        path: path.clone(),
        changed: note_changed,
        roadmap: roadmap_path.clone(),
        roadmap_changed,
        state: state_path.clone(),
        recovery,
    };

    let operation = (|| {
        if note_changed {
            replace_file_if_unchanged(
                &path,
                expected_source.as_bytes(),
                replacement_source.as_bytes(),
            )
            .map_err(map_checked_replace)?;
            sync_replacement_directory(&path, "sync the canonical record replacement")?;
        }
        if roadmap_changed {
            replace_file_if_unchanged(&roadmap_path, &roadmap_before, roadmap_source.as_bytes())
                .map_err(map_checked_replace)?;
            sync_replacement_directory(&roadmap_path, "sync the roadmap replacement")?;
        }
        replace_file_if_unchanged(&state_path, &state_before, &state_after)
            .map_err(map_checked_replace)?;
        sync_replacement_directory(&state_path, "sync the project state replacement")?;
        validate_project(request)?;
        complete_note_mutation_journal(&journal_path, &journal_source, &resolved.project_dir)?;
        Ok(result_for(recovery))
    })();

    match operation {
        Ok(result) => Ok(result),
        Err(original) => match recover_note_mutation_locked(request, &resolved.project_dir) {
            Ok(NoteEditRecovery::Finalized) => Ok(result_for(NoteEditRecovery::Finalized)),
            Ok(_) => Err(original),
            Err(recovery) => Err(NoteEditError::Recovery {
                original: Box::new(original),
                recovery: Box::new(recovery),
            }),
        },
    }
}

/// Update one selected-project entity and explicitly accept its complete index projection.
pub fn update_entity(
    request: &ResolveRequest,
    id: &str,
    expected_source: &str,
    replacement_source: &str,
    index_source: &str,
) -> Result<EntityUpdateResult, NoteEditError> {
    let resolved = resolve_project(request)?;
    let _lock = ProjectWriteLock::acquire(&resolved.project_dir)?;
    let recovery = recover_note_mutation_locked(request, &resolved.project_dir)?;
    let projection = build_library_projection(request)?;
    let book = projection
        .projects
        .iter()
        .find(|shelf| shelf.project == resolved.project)
        .into_iter()
        .flat_map(|shelf| &shelf.categories)
        .flat_map(|category| &category.books)
        .find(|book| book.id == id)
        .ok_or_else(|| NoteEditError::Validation {
            path: PathBuf::from(id),
            message: "only a projected note from the selected project can be updated".to_owned(),
        })?;
    if book.class != NoteClass::Entity {
        return Err(NoteEditError::Validation {
            path: PathBuf::from(id),
            message: "entity updates require a configured project entity".to_owned(),
        });
    }
    let note_type = book.note_type.clone();

    let path = resolved.root.join(id);
    let current = read_regular_file(&path, "read the selected canonical entity")?;
    if current != expected_source.as_bytes() {
        return Err(NoteEditError::Conflict {
            path,
            message: "the canonical entity no longer matches the supplied expected source"
                .to_owned(),
        });
    }

    let config = load_root_config(&resolved.root)?;
    validate_replacement(
        &resolved.root,
        &resolved.project,
        &path,
        &note_type,
        replacement_source.as_bytes(),
        &config,
    )?;
    validate_preserved_entity_identity(
        &path,
        expected_source.as_bytes(),
        replacement_source.as_bytes(),
    )?;

    let evidence = collect_candidate_evidence(
        &resolved.project_dir,
        &path,
        replacement_source.as_bytes(),
        &config,
    )?;
    let index_path = resolved.project_dir.join(&config.project.index);
    let roadmap_path = resolved.project_dir.join(&config.project.roadmap);
    let index_before = read_regular_file(&index_path, "read the current index projection")?;
    let index_before_text =
        str::from_utf8(&index_before).map_err(|error| NoteEditError::Validation {
            path: index_path.clone(),
            message: format!("current index projection is not valid UTF-8: {error}"),
        })?;
    let roadmap = read_regular_file(&roadmap_path, "read the current roadmap projection")?;
    let state_path = resolved.project_dir.join(PROJECT_STATE_FILE);
    let note_changed = replacement_source != expected_source;
    let index_changed = index_before != index_source.as_bytes();
    if !note_changed && !index_changed {
        return Ok(EntityUpdateResult {
            root: resolved.root,
            project: resolved.project,
            project_dir: resolved.project_dir,
            note_type,
            id: id.to_owned(),
            path,
            changed: false,
            index: index_path,
            index_changed: false,
            state: state_path,
            recovery,
        });
    }

    let state_before = read_regular_file(&state_path, "read the current project state")?;
    let state_before_text =
        str::from_utf8(&state_before).map_err(|error| NoteEditError::Validation {
            path: state_path.clone(),
            message: format!("project state is not valid UTF-8: {error}"),
        })?;
    let state_after = render_updated_project_state(
        state_before_text,
        &resolved.project_dir,
        index_source.as_bytes(),
        &roadmap,
        &evidence,
        &BTreeSet::new(),
    )
    .map_err(|message| NoteEditError::Validation {
        path: state_path.clone(),
        message,
    })?;
    let state_after_text = str::from_utf8(&state_after)
        .expect("the deterministic project-state renderer always emits UTF-8");
    let index_id = vault_relative_identity(&resolved.root, &index_path)?;
    let (journal_path, journal_source) = write_note_projection_mutation_journal(
        &resolved.project_dir,
        &NoteProjectionJournal {
            project: &resolved.project,
            id,
            note_before: Some(expected_source),
            note_after: replacement_source,
            projection_id: &index_id,
            projection_before: index_before_text,
            projection_after: index_source,
            state_before: state_before_text,
            state_after: state_after_text,
        },
    )?;
    let result_for = |recovery| EntityUpdateResult {
        root: resolved.root.clone(),
        project: resolved.project.clone(),
        project_dir: resolved.project_dir.clone(),
        note_type: note_type.clone(),
        id: id.to_owned(),
        path: path.clone(),
        changed: note_changed,
        index: index_path.clone(),
        index_changed,
        state: state_path.clone(),
        recovery,
    };

    let operation = (|| {
        if note_changed {
            replace_file_if_unchanged(
                &path,
                expected_source.as_bytes(),
                replacement_source.as_bytes(),
            )
            .map_err(map_checked_replace)?;
            sync_replacement_directory(&path, "sync the canonical entity replacement")?;
        }
        if index_changed {
            replace_file_if_unchanged(&index_path, &index_before, index_source.as_bytes())
                .map_err(map_checked_replace)?;
            sync_replacement_directory(&index_path, "sync the index replacement")?;
        }
        replace_file_if_unchanged(&state_path, &state_before, &state_after)
            .map_err(map_checked_replace)?;
        sync_replacement_directory(&state_path, "sync the project state replacement")?;
        validate_project(request)?;
        complete_note_mutation_journal(&journal_path, &journal_source, &resolved.project_dir)?;
        Ok(result_for(recovery))
    })();

    match operation {
        Ok(result) => Ok(result),
        Err(original) => match recover_note_mutation_locked(request, &resolved.project_dir) {
            Ok(NoteEditRecovery::Finalized) => Ok(result_for(NoteEditRecovery::Finalized)),
            Ok(_) => Err(original),
            Err(recovery) => Err(NoteEditError::Recovery {
                original: Box::new(original),
                recovery: Box::new(recovery),
            }),
        },
    }
}

/// Resolve a pending exact-byte edit journal without requiring the project to validate first.
pub fn recover_pending_note_edit(
    request: &ResolveRequest,
) -> Result<NoteEditRecovery, NoteEditError> {
    let resolved = resolve_project(request)?;
    let journal_path = resolved.project_dir.join(NOTE_EDIT_JOURNAL_FILE);
    match fs::symlink_metadata(&journal_path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(NoteEditRecovery::None),
        Ok(_) => {}
        Err(source) => {
            return Err(NoteEditError::FileSystem {
                operation: "inspect the note edit journal",
                path: journal_path,
                source,
            });
        }
    }
    let _lock = ProjectWriteLock::acquire(&resolved.project_dir)?;
    recover_note_mutation_locked(request, &resolved.project_dir)
}

pub(crate) fn recover_note_mutation_locked(
    request: &ResolveRequest,
    project_dir: &Path,
) -> Result<NoteEditRecovery, NoteEditError> {
    let journal_path = project_dir.join(NOTE_EDIT_JOURNAL_FILE);
    let Some((journal, journal_source)) = read_journal(&journal_path)? else {
        return Ok(NoteEditRecovery::None);
    };
    let resolved = resolve_project(request)?;
    let journal_project = match &journal {
        NoteMutationJournal::Note(journal) => &journal.project,
        NoteMutationJournal::Onboarding(journal) => &journal.project,
    };
    if resolved.project_dir != project_dir || journal_project != &resolved.project {
        return Err(NoteEditError::Conflict {
            path: journal_path,
            message: "the recovery journal does not match the resolved project".to_owned(),
        });
    }
    let outcome = match journal {
        NoteMutationJournal::Note(journal) => recover_note_journal(
            request,
            &resolved.root,
            project_dir,
            &journal_path,
            &journal,
        )?,
        NoteMutationJournal::Onboarding(journal) => recover_onboarding_journal(
            request,
            &resolved.root,
            project_dir,
            &journal_path,
            &journal,
        )?,
    };

    complete_note_mutation_journal(&journal_path, &journal_source, project_dir)?;
    Ok(outcome)
}

fn recover_note_journal(
    request: &ResolveRequest,
    root: &Path,
    project_dir: &Path,
    journal_path: &Path,
    journal: &NoteEditJournal,
) -> Result<NoteEditRecovery, NoteEditError> {
    let note_path = checked_journal_note_path(root, project_dir, &journal.id, journal_path)?;
    let projection_path = journal
        .projection
        .as_ref()
        .map(|projection| {
            checked_journal_projection_path(root, project_dir, &projection.id, journal_path)
        })
        .transpose()?;
    let state_path = project_dir.join(PROJECT_STATE_FILE);
    let note = read_optional_regular_file(&note_path, "read the journaled canonical note")?;
    let projection = projection_path
        .as_ref()
        .map(|path| read_regular_file(path, "read the journaled projection"))
        .transpose()?;
    let state = read_regular_file(&state_path, "read the journaled project state")?;
    let note_before = journal.note_before.as_deref().map(str::as_bytes);
    let note_after = journal.note_after.as_bytes();
    let state_before = journal.state_before.as_bytes();
    let state_after = journal.state_after.as_bytes();
    let note_matches_before = match (note.as_deref(), note_before) {
        (Some(current), Some(expected)) => current == expected,
        (None, None) => true,
        _ => false,
    };
    let note_matches_after = note.as_deref() == Some(note_after);
    let projection_matches_before = match (&projection, &journal.projection) {
        (None, None) => true,
        (Some(current), Some(expected)) => current == expected.before.as_bytes(),
        _ => false,
    };
    let projection_matches_after = match (&projection, &journal.projection) {
        (None, None) => true,
        (Some(current), Some(expected)) => current == expected.after.as_bytes(),
        _ => false,
    };
    let state_matches_before = state == state_before;
    let state_matches_after = state == state_after;

    if (!note_matches_before && !note_matches_after)
        || (!projection_matches_before && !projection_matches_after)
        || (!state_matches_before && !state_matches_after)
    {
        return Err(NoteEditError::Conflict {
            path: journal_path.to_path_buf(),
            message: "journaled note, projection, or project state contains unexpected bytes; automatic recovery refused"
                .to_owned(),
        });
    }

    let outcome = if note_matches_before && projection_matches_before && state_matches_before {
        NoteEditRecovery::Discarded
    } else if note_matches_after && projection_matches_after && state_matches_after {
        validate_project(request)?;
        NoteEditRecovery::Finalized
    } else {
        if !state_matches_before {
            replace_file_if_unchanged(&state_path, state_after, state_before)
                .map_err(map_checked_replace)?;
            sync_replacement_directory(&state_path, "sync recovery of project state")?;
        }
        if !projection_matches_before {
            let expected = journal
                .projection
                .as_ref()
                .expect("only a journaled projection can differ from its pre-image");
            let path = projection_path
                .as_ref()
                .expect("a journaled projection always has a checked path");
            replace_file_if_unchanged(path, expected.after.as_bytes(), expected.before.as_bytes())
                .map_err(map_checked_replace)?;
            sync_replacement_directory(path, "sync recovery of the maintained projection")?;
        }
        if !note_matches_before {
            if let Some(note_before) = note_before {
                replace_file_if_unchanged(&note_path, note_after, note_before)
                    .map_err(map_checked_replace)?;
            } else {
                remove_created_note_if_unchanged(&note_path, note_after)?;
            }
            sync_replacement_directory(&note_path, "sync recovery of the canonical note")?;
        }
        validate_project(request)?;
        NoteEditRecovery::RolledBack
    };
    Ok(outcome)
}

fn recover_onboarding_journal(
    request: &ResolveRequest,
    root: &Path,
    project_dir: &Path,
    journal_path: &Path,
    journal: &OnboardingBatchJournal,
) -> Result<NoteEditRecovery, NoteEditError> {
    if journal.projections.len() != 2 {
        return Err(NoteEditError::Validation {
            path: journal_path.to_path_buf(),
            message: "onboarding recovery journal must contain the index and roadmap projections"
                .to_owned(),
        });
    }

    let mut note_paths = BTreeSet::new();
    let mut notes = Vec::with_capacity(journal.notes.len());
    for note in &journal.notes {
        let path = checked_journal_note_path(root, project_dir, &note.id, journal_path)?;
        if !note_paths.insert(path.clone()) {
            return Err(NoteEditError::Validation {
                path: journal_path.to_path_buf(),
                message: "onboarding recovery journal contains duplicate note identities"
                    .to_owned(),
            });
        }
        let current = read_optional_regular_file(&path, "read a journaled onboarding note")?;
        let before = current.is_none();
        let after = current.as_deref() == Some(note.after.as_bytes());
        if !before && !after {
            return Err(unexpected_onboarding_bytes(journal_path));
        }
        notes.push((path, current, note));
    }

    let mut projection_paths = BTreeSet::new();
    let mut projections = Vec::with_capacity(journal.projections.len());
    for projection in &journal.projections {
        let path =
            checked_journal_projection_path(root, project_dir, &projection.id, journal_path)?;
        if !projection_paths.insert(path.clone()) {
            return Err(NoteEditError::Validation {
                path: journal_path.to_path_buf(),
                message: "onboarding recovery journal contains duplicate projection identities"
                    .to_owned(),
            });
        }
        let current = read_regular_file(&path, "read a journaled onboarding projection")?;
        let before = current == projection.before.as_bytes();
        let after = current == projection.after.as_bytes();
        if !before && !after {
            return Err(unexpected_onboarding_bytes(journal_path));
        }
        projections.push((path, current, projection));
    }

    let state_path = project_dir.join(PROJECT_STATE_FILE);
    let state = read_regular_file(&state_path, "read the journaled project state")?;
    let state_before = journal.state_before.as_bytes();
    let state_after = journal.state_after.as_bytes();
    let state_matches_before = state == state_before;
    let state_matches_after = state == state_after;
    if !state_matches_before && !state_matches_after {
        return Err(unexpected_onboarding_bytes(journal_path));
    }

    let notes_match_before = notes.iter().all(|(_, current, _)| current.is_none());
    let notes_match_after = notes
        .iter()
        .all(|(_, current, note)| current.as_deref() == Some(note.after.as_bytes()));
    let projections_match_before = projections
        .iter()
        .all(|(_, current, projection)| current == projection.before.as_bytes());
    let projections_match_after = projections
        .iter()
        .all(|(_, current, projection)| current == projection.after.as_bytes());

    if notes_match_before && projections_match_before && state_matches_before {
        return Ok(NoteEditRecovery::Discarded);
    }
    if notes_match_after && projections_match_after && state_matches_after {
        validate_project(request)?;
        return Ok(NoteEditRecovery::Finalized);
    }

    if !state_matches_before {
        replace_file_if_unchanged(&state_path, state_after, state_before)
            .map_err(map_checked_replace)?;
        sync_replacement_directory(&state_path, "sync onboarding recovery of project state")?;
    }
    for (path, current, projection) in projections.iter().rev() {
        if current != projection.before.as_bytes() {
            replace_file_if_unchanged(
                path,
                projection.after.as_bytes(),
                projection.before.as_bytes(),
            )
            .map_err(map_checked_replace)?;
            sync_replacement_directory(
                path,
                "sync onboarding recovery of a maintained projection",
            )?;
        }
    }
    for (path, current, note) in notes.iter().rev() {
        if current.is_some() {
            remove_created_note_if_unchanged(path, note.after.as_bytes())?;
            sync_replacement_directory(path, "sync onboarding recovery of a canonical note")?;
        }
    }
    validate_project(request)?;
    Ok(NoteEditRecovery::RolledBack)
}

fn unexpected_onboarding_bytes(journal_path: &Path) -> NoteEditError {
    NoteEditError::Conflict {
        path: journal_path.to_path_buf(),
        message: "journaled onboarding note, projection, or project state contains unexpected bytes; automatic recovery refused"
            .to_owned(),
    }
}

fn validate_replacement(
    root: &Path,
    project: &str,
    path: &Path,
    note_type: &str,
    source: &[u8],
    config: &RootConfig,
) -> Result<(), NoteEditError> {
    let configured =
        config
            .project
            .note_types
            .get(note_type)
            .ok_or_else(|| NoteEditError::Validation {
                path: path.to_path_buf(),
                message: format!("project configuration no longer defines note type {note_type:?}"),
            })?;
    if configured.class == NoteClass::Event {
        return Err(NoteEditError::Validation {
            path: path.to_path_buf(),
            message: "immutable event notes cannot be replaced".to_owned(),
        });
    }
    let parsed =
        parse_leading_frontmatter_bytes(source).map_err(|error| NoteEditError::Validation {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    validate_configured_note(&parsed, project, note_type, &configured.required_fields).map_err(
        |error| NoteEditError::Validation {
            path: path.to_path_buf(),
            message: error.to_string(),
        },
    )?;
    validate_wikilinks_with_targets(root, path, parsed.body, &BTreeSet::new())?;
    Ok(())
}

fn validate_preserved_record_metadata(
    path: &Path,
    expected_source: &[u8],
    replacement_source: &[u8],
) -> Result<(), NoteEditError> {
    let expected = parse_leading_frontmatter_bytes(expected_source).map_err(|error| {
        NoteEditError::Validation {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    let replacement = parse_leading_frontmatter_bytes(replacement_source).map_err(|error| {
        NoteEditError::Validation {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if expected.metadata.get("created") != replacement.metadata.get("created") {
        return Err(NoteEditError::Validation {
            path: path.to_path_buf(),
            message: "record field \"created\" must be preserved across lifecycle updates"
                .to_owned(),
        });
    }
    Ok(())
}

fn validate_preserved_entity_identity(
    path: &Path,
    expected_source: &[u8],
    replacement_source: &[u8],
) -> Result<(), NoteEditError> {
    let expected = parse_leading_frontmatter_bytes(expected_source).map_err(|error| {
        NoteEditError::Validation {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    let replacement = parse_leading_frontmatter_bytes(replacement_source).map_err(|error| {
        NoteEditError::Validation {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if expected.metadata.get("entity") != replacement.metadata.get("entity") {
        return Err(NoteEditError::Validation {
            path: path.to_path_buf(),
            message: "entity field \"entity\" must be preserved across updates; rename is a separate operation"
                .to_owned(),
        });
    }
    Ok(())
}

fn vault_relative_identity(root: &Path, path: &Path) -> Result<String, NoteEditError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| NoteEditError::Validation {
            path: path.to_path_buf(),
            message: "maintained projection is outside the configured data root".to_owned(),
        })?;
    let text = relative.to_str().ok_or_else(|| NoteEditError::Validation {
        path: path.to_path_buf(),
        message: "maintained projection identity is not valid UTF-8".to_owned(),
    })?;
    Ok(text.replace(std::path::MAIN_SEPARATOR, "/"))
}

fn collect_candidate_evidence(
    project_dir: &Path,
    replacement_path: &Path,
    replacement_source: &[u8],
    config: &RootConfig,
) -> Result<Vec<CanonicalNoteEvidence>, NoteEditError> {
    let mut evidence = collect_canonical_evidence(project_dir, config, |path| {
        read_regular_file(path, "read canonical note for edit state")
    })?;
    let candidate = evidence
        .iter_mut()
        .find(|item| item.path == replacement_path)
        .ok_or_else(|| NoteEditError::Conflict {
            path: replacement_path.to_path_buf(),
            message: "the configured canonical note set changed during the update".to_owned(),
        })?;
    candidate.source = replacement_source.to_vec();
    Ok(evidence)
}

fn checked_journal_note_path(
    root: &Path,
    project_dir: &Path,
    id: &str,
    journal_path: &Path,
) -> Result<PathBuf, NoteEditError> {
    let relative = Path::new(id);
    if relative.extension().and_then(|value| value.to_str()) != Some("md")
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(NoteEditError::Validation {
            path: journal_path.to_path_buf(),
            message: "journal identity must be a normalized vault-relative Markdown path"
                .to_owned(),
        });
    }
    let path = root.join(relative);
    let parent = path.parent().ok_or_else(|| NoteEditError::Validation {
        path: path.clone(),
        message: "journal identity must name a project note".to_owned(),
    })?;
    let parent = fs::canonicalize(parent).map_err(|source| NoteEditError::FileSystem {
        operation: "resolve the journaled note directory",
        path: parent.to_path_buf(),
        source,
    })?;
    if !parent.starts_with(project_dir) {
        return Err(NoteEditError::Validation {
            path,
            message: "journaled note is outside the resolved project".to_owned(),
        });
    }
    Ok(parent.join(
        relative
            .file_name()
            .expect("a normalized Markdown path always has a filename"),
    ))
}

fn checked_journal_projection_path(
    root: &Path,
    project_dir: &Path,
    id: &str,
    journal_path: &Path,
) -> Result<PathBuf, NoteEditError> {
    let relative = Path::new(id);
    if relative.extension().and_then(|value| value.to_str()) != Some("md")
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(NoteEditError::Validation {
            path: journal_path.to_path_buf(),
            message:
                "journal projection identity must be a normalized vault-relative Markdown path"
                    .to_owned(),
        });
    }
    let config = load_root_config(root)?;
    let path = root.join(relative);
    let allowed = [
        project_dir.join(&config.project.index),
        project_dir.join(&config.project.roadmap),
    ];
    if !allowed.contains(&path) {
        return Err(NoteEditError::Validation {
            path: journal_path.to_path_buf(),
            message: "journal projection must identify the configured project index or roadmap"
                .to_owned(),
        });
    }
    Ok(path)
}

fn read_journal(path: &Path) -> Result<Option<(NoteMutationJournal, Vec<u8>)>, NoteEditError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(NoteEditError::FileSystem {
                operation: "inspect the note edit journal",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(NoteEditError::Conflict {
            path: path.to_path_buf(),
            message: "the note edit journal is not a regular file".to_owned(),
        });
    }
    let source = fs::read(path).map_err(|source| NoteEditError::FileSystem {
        operation: "read the note edit journal",
        path: path.to_path_buf(),
        source,
    })?;
    let journal: NoteMutationJournal =
        serde_json::from_slice(&source).map_err(|error| NoteEditError::Validation {
            path: path.to_path_buf(),
            message: format!("invalid note edit journal JSON: {error}"),
        })?;
    let valid_schema = match &journal {
        NoteMutationJournal::Note(journal) => match journal.schema_version {
            NOTE_EDIT_JOURNAL_SCHEMA_VERSION => journal.projection.is_none(),
            NOTE_PROJECTION_JOURNAL_SCHEMA_VERSION => journal.projection.is_some(),
            _ => false,
        },
        NoteMutationJournal::Onboarding(journal) => {
            journal.schema_version == ONBOARDING_BATCH_JOURNAL_SCHEMA_VERSION
        }
    };
    if !valid_schema {
        let schema_version = match &journal {
            NoteMutationJournal::Note(journal) => journal.schema_version,
            NoteMutationJournal::Onboarding(journal) => journal.schema_version,
        };
        return Err(NoteEditError::Validation {
            path: path.to_path_buf(),
            message: format!(
                "invalid note edit journal schema_version {schema_version}; expected version {NOTE_EDIT_JOURNAL_SCHEMA_VERSION} without a projection, version {NOTE_PROJECTION_JOURNAL_SCHEMA_VERSION} with one projection, or version {ONBOARDING_BATCH_JOURNAL_SCHEMA_VERSION} for an onboarding batch"
            ),
        });
    }
    Ok(Some((journal, source)))
}

fn render_journal(journal: &impl Serialize) -> Result<Vec<u8>, NoteEditError> {
    let mut source =
        serde_json::to_vec_pretty(journal).map_err(|error| NoteEditError::Validation {
            path: PathBuf::from(NOTE_EDIT_JOURNAL_FILE),
            message: format!("could not serialize note edit journal: {error}"),
        })?;
    source.push(b'\n');
    Ok(source)
}

pub(crate) fn write_note_mutation_journal(
    project_dir: &Path,
    project: &str,
    id: &str,
    note_before: Option<&str>,
    note_after: &str,
    state_before: &str,
    state_after: &str,
) -> Result<(PathBuf, Vec<u8>), NoteEditError> {
    let journal = NoteEditJournal {
        schema_version: NOTE_EDIT_JOURNAL_SCHEMA_VERSION,
        project: project.to_owned(),
        id: id.to_owned(),
        note_before: note_before.map(str::to_owned),
        note_after: note_after.to_owned(),
        projection: None,
        state_before: state_before.to_owned(),
        state_after: state_after.to_owned(),
    };
    write_journal(project_dir, journal)
}

pub(crate) struct NoteProjectionJournal<'a> {
    pub(crate) project: &'a str,
    pub(crate) id: &'a str,
    pub(crate) note_before: Option<&'a str>,
    pub(crate) note_after: &'a str,
    pub(crate) projection_id: &'a str,
    pub(crate) projection_before: &'a str,
    pub(crate) projection_after: &'a str,
    pub(crate) state_before: &'a str,
    pub(crate) state_after: &'a str,
}

pub(crate) fn write_note_projection_mutation_journal(
    project_dir: &Path,
    mutation: &NoteProjectionJournal<'_>,
) -> Result<(PathBuf, Vec<u8>), NoteEditError> {
    let journal = NoteEditJournal {
        schema_version: NOTE_PROJECTION_JOURNAL_SCHEMA_VERSION,
        project: mutation.project.to_owned(),
        id: mutation.id.to_owned(),
        note_before: mutation.note_before.map(str::to_owned),
        note_after: mutation.note_after.to_owned(),
        projection: Some(JournalProjection {
            id: mutation.projection_id.to_owned(),
            before: mutation.projection_before.to_owned(),
            after: mutation.projection_after.to_owned(),
        }),
        state_before: mutation.state_before.to_owned(),
        state_after: mutation.state_after.to_owned(),
    };
    write_journal(project_dir, journal)
}

pub(crate) fn write_onboarding_batch_mutation_journal(
    project_dir: &Path,
    mutation: OnboardingBatchJournalInput<'_>,
) -> Result<(PathBuf, Vec<u8>), NoteEditError> {
    let journal = OnboardingBatchJournal {
        schema_version: ONBOARDING_BATCH_JOURNAL_SCHEMA_VERSION,
        project: mutation.project.to_owned(),
        notes: mutation
            .notes
            .into_iter()
            .map(|note| JournalCreatedNote {
                id: note.id,
                after: note.after,
            })
            .collect(),
        projections: vec![
            JournalProjection {
                id: mutation.index_id.to_owned(),
                before: mutation.index_before.to_owned(),
                after: mutation.index_after.to_owned(),
            },
            JournalProjection {
                id: mutation.roadmap_id.to_owned(),
                before: mutation.roadmap_before.to_owned(),
                after: mutation.roadmap_after.to_owned(),
            },
        ],
        state_before: mutation.state_before.to_owned(),
        state_after: mutation.state_after.to_owned(),
    };
    write_journal(project_dir, journal)
}

fn write_journal(
    project_dir: &Path,
    journal: impl Serialize,
) -> Result<(PathBuf, Vec<u8>), NoteEditError> {
    let source = render_journal(&journal)?;
    let path = project_dir.join(NOTE_EDIT_JOURNAL_FILE);
    create_file_atomically(&path, &source)?;
    sync_project_directory(project_dir, "sync the note mutation journal")?;
    Ok((path, source))
}

fn read_optional_regular_file(
    path: &Path,
    operation: &'static str,
) -> Result<Option<Vec<u8>>, NoteEditError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(NoteEditError::FileSystem {
                operation,
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(NoteEditError::Conflict {
            path: path.to_path_buf(),
            message: "journaled note is not a regular file".to_owned(),
        });
    }
    fs::read(path)
        .map(Some)
        .map_err(|source| NoteEditError::FileSystem {
            operation,
            path: path.to_path_buf(),
            source,
        })
}

fn read_regular_file(path: &Path, operation: &'static str) -> Result<Vec<u8>, NoteEditError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| NoteEditError::FileSystem {
        operation,
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(NoteEditError::Conflict {
            path: path.to_path_buf(),
            message: "checked edit target is not a regular file".to_owned(),
        });
    }
    fs::read(path).map_err(|source| NoteEditError::FileSystem {
        operation,
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn complete_note_mutation_journal(
    path: &Path,
    expected: &[u8],
    project_dir: &Path,
) -> Result<(), NoteEditError> {
    let current = read_regular_file(path, "verify the note edit journal before removal")?;
    if current != expected {
        return Err(NoteEditError::Conflict {
            path: path.to_path_buf(),
            message: "the note edit journal changed before cleanup".to_owned(),
        });
    }
    fs::remove_file(path).map_err(|source| NoteEditError::FileSystem {
        operation: "remove the completed note edit journal",
        path: path.to_path_buf(),
        source,
    })?;
    sync_project_directory(project_dir, "sync removal of the note edit journal")
}

fn remove_created_note_if_unchanged(path: &Path, expected: &[u8]) -> Result<(), NoteEditError> {
    let current = read_regular_file(path, "verify the created note before recovery")?;
    if current != expected {
        return Err(NoteEditError::Conflict {
            path: path.to_path_buf(),
            message: "journaled created note changed before recovery".to_owned(),
        });
    }
    fs::remove_file(path).map_err(|source| NoteEditError::FileSystem {
        operation: "remove the partially published canonical note",
        path: path.to_path_buf(),
        source,
    })
}

fn sync_project_directory(
    project_dir: &Path,
    operation: &'static str,
) -> Result<(), NoteEditError> {
    sync_directory(project_dir).map_err(|source| NoteEditError::FileSystem {
        operation,
        path: project_dir.to_path_buf(),
        source,
    })
}

fn sync_replacement_directory(path: &Path, operation: &'static str) -> Result<(), NoteEditError> {
    let directory = path
        .parent()
        .expect("resolved replacement paths always have a parent");
    sync_directory(directory).map_err(|source| NoteEditError::FileSystem {
        operation,
        path: directory.to_path_buf(),
        source,
    })
}

fn map_checked_replace(error: CheckedReplaceError) -> NoteEditError {
    match error {
        CheckedReplaceError::Conflict { path, .. } => NoteEditError::Conflict {
            path,
            message: "checked replacement target no longer matches the expected bytes".to_owned(),
        },
        CheckedReplaceError::FileSystem {
            operation,
            path,
            source,
        } => NoteEditError::FileSystem {
            operation,
            path,
            source,
        },
    }
}
