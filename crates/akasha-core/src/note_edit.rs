use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::str;

use serde::{Deserialize, Serialize};

use crate::library::build_library_projection;
use crate::project_validation::{
    ProjectValidationError, canonical_note_paths, validate_project, validate_wikilinks_with_targets,
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
    state_before: String,
    state_after: String,
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
    if resolved.project_dir != project_dir || journal.project != resolved.project {
        return Err(NoteEditError::Conflict {
            path: journal_path,
            message: "the recovery journal does not match the resolved project".to_owned(),
        });
    }
    let note_path = checked_journal_note_path(&resolved.root, project_dir, &journal)?;
    let state_path = project_dir.join(PROJECT_STATE_FILE);
    let note = read_optional_regular_file(&note_path, "read the journaled canonical note")?;
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

    let outcome = if note_matches_before && state == state_before {
        NoteEditRecovery::Discarded
    } else if note_matches_after && state == state_after {
        validate_project(request)?;
        NoteEditRecovery::Finalized
    } else if note_matches_after && state == state_before {
        if let Some(note_before) = note_before {
            replace_file_if_unchanged(&note_path, note_after, note_before)
                .map_err(map_checked_replace)?;
        } else {
            remove_created_note_if_unchanged(&note_path, note_after)?;
        }
        sync_replacement_directory(&note_path, "sync recovery of the canonical note")?;
        validate_project(request)?;
        NoteEditRecovery::RolledBack
    } else if note_matches_before && state == state_after {
        replace_file_if_unchanged(&state_path, state_after, state_before)
            .map_err(map_checked_replace)?;
        sync_replacement_directory(&state_path, "sync recovery of project state")?;
        validate_project(request)?;
        NoteEditRecovery::RolledBack
    } else {
        return Err(NoteEditError::Conflict {
            path: journal_path,
            message: "journaled note or project state contains unexpected bytes; automatic recovery refused"
                .to_owned(),
        });
    };

    complete_note_mutation_journal(&journal_path, &journal_source, project_dir)?;
    Ok(outcome)
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

fn collect_candidate_evidence(
    project_dir: &Path,
    replacement_path: &Path,
    replacement_source: &[u8],
    config: &RootConfig,
) -> Result<Vec<CanonicalNoteEvidence>, NoteEditError> {
    let mut evidence = Vec::new();
    for note_type in config.project.note_types.values() {
        for path in canonical_note_paths(&project_dir.join(&note_type.folder))? {
            let source = if path == replacement_path {
                replacement_source.to_vec()
            } else {
                read_regular_file(&path, "read canonical note for edit state")?
            };
            evidence.push(CanonicalNoteEvidence {
                path,
                class: note_type.class,
                source,
            });
        }
    }
    Ok(evidence)
}

fn checked_journal_note_path(
    root: &Path,
    project_dir: &Path,
    journal: &NoteEditJournal,
) -> Result<PathBuf, NoteEditError> {
    let relative = Path::new(&journal.id);
    if relative.extension().and_then(|value| value.to_str()) != Some("md")
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(NoteEditError::Validation {
            path: PathBuf::from(&journal.id),
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

fn read_journal(path: &Path) -> Result<Option<(NoteEditJournal, Vec<u8>)>, NoteEditError> {
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
    let journal: NoteEditJournal =
        serde_json::from_slice(&source).map_err(|error| NoteEditError::Validation {
            path: path.to_path_buf(),
            message: format!("invalid note edit journal JSON: {error}"),
        })?;
    if journal.schema_version != NOTE_EDIT_JOURNAL_SCHEMA_VERSION {
        return Err(NoteEditError::Validation {
            path: path.to_path_buf(),
            message: format!(
                "unsupported note edit journal schema_version {}; expected {NOTE_EDIT_JOURNAL_SCHEMA_VERSION}",
                journal.schema_version
            ),
        });
    }
    Ok(Some((journal, source)))
}

fn render_journal(journal: &NoteEditJournal) -> Result<Vec<u8>, NoteEditError> {
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
        state_before: state_before.to_owned(),
        state_after: state_after.to_owned(),
    };
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
