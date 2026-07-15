use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str;

use serde::Serialize;

use crate::note_edit::{
    NoteEditError, NoteEditRecovery, NoteProjectionJournal, complete_note_mutation_journal,
    recover_note_mutation_locked, write_note_projection_mutation_journal,
};
use crate::note_template::{NoteTemplateError, NoteTemplateScope, resolve_note_template};
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

/// Result of one create-only record or entity transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MutableNoteCreationResult {
    pub root: PathBuf,
    pub project: String,
    pub project_dir: PathBuf,
    pub note_type: String,
    pub class: NoteClass,
    pub relative_path: PathBuf,
    pub id: String,
    pub path: PathBuf,
    pub template: PathBuf,
    pub template_scope: NoteTemplateScope,
    pub projection: PathBuf,
    pub projection_changed: bool,
    pub state: PathBuf,
    pub recovery: NoteEditRecovery,
}

/// An input, resolution, validation, conflict, filesystem, or recovery failure.
#[derive(Debug)]
pub enum MutableNoteCreationError {
    Input {
        path: PathBuf,
        message: String,
    },
    Resolve(Box<ResolveError>),
    Project(Box<ProjectValidationError>),
    Template(Box<NoteTemplateError>),
    Mutation(Box<NoteEditError>),
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
        original: Box<MutableNoteCreationError>,
        recovery: Box<NoteEditError>,
    },
}

impl MutableNoteCreationError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Input { .. } => 2,
            Self::Resolve(error) => error.exit_code(),
            Self::Project(error) => error.exit_code(),
            Self::Template(error) => error.exit_code(),
            Self::Mutation(error) => error.exit_code(),
            Self::Validation { .. } => 4,
            Self::Conflict { .. } => 5,
            Self::Creation(error) => error.exit_code(),
            Self::FileSystem { .. } | Self::Recovery { .. } => 6,
        }
    }
}

impl fmt::Display for MutableNoteCreationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input { path, message } => write!(
                formatter,
                "invalid mutable-note input at {}: {message}",
                path.display()
            ),
            Self::Resolve(error) => error.fmt(formatter),
            Self::Project(error) => error.fmt(formatter),
            Self::Template(error) => error.fmt(formatter),
            Self::Mutation(error) => error.fmt(formatter),
            Self::Validation { path, message } => write!(
                formatter,
                "invalid mutable-note creation at {}: {message}",
                path.display()
            ),
            Self::Conflict { path, message } => write!(
                formatter,
                "mutable-note creation conflict at {}: {message}",
                path.display()
            ),
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
                "mutable-note creation failed and automatic recovery also failed: original error: {original}; recovery error: {recovery}"
            ),
        }
    }
}

impl Error for MutableNoteCreationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Resolve(error) => Some(error.as_ref()),
            Self::Project(error) => Some(error.as_ref()),
            Self::Template(error) => Some(error.as_ref()),
            Self::Mutation(error) => Some(error.as_ref()),
            Self::Creation(error) => Some(error),
            Self::FileSystem { source, .. } => Some(source),
            Self::Recovery { original, .. } => Some(original.as_ref()),
            Self::Input { .. } | Self::Validation { .. } | Self::Conflict { .. } => None,
        }
    }
}

impl From<ResolveError> for MutableNoteCreationError {
    fn from(error: ResolveError) -> Self {
        Self::Resolve(Box::new(error))
    }
}

impl From<ProjectValidationError> for MutableNoteCreationError {
    fn from(error: ProjectValidationError) -> Self {
        Self::Project(Box::new(error))
    }
}

impl From<NoteTemplateError> for MutableNoteCreationError {
    fn from(error: NoteTemplateError) -> Self {
        Self::Template(Box::new(error))
    }
}

impl From<NoteEditError> for MutableNoteCreationError {
    fn from(error: NoteEditError) -> Self {
        Self::Mutation(Box::new(error))
    }
}

impl From<AtomicCreateError> for MutableNoteCreationError {
    fn from(error: AtomicCreateError) -> Self {
        Self::Creation(error)
    }
}

/// Create one configured record or entity and explicitly accept its maintained projection.
///
/// The path is relative to the configured note-type folder. Template placeholders use exact
/// `{{name}}` markers. `project` and `type` are supplied by the core; every other marker must be
/// supplied by the caller, and unused caller fields are rejected. `projection_source` is the
/// complete accepted `roadmap.md` source for a record or `index.md` source for an entity.
pub fn create_mutable_note(
    request: &ResolveRequest,
    note_type: &str,
    relative_path: &Path,
    fields: &BTreeMap<String, String>,
    projection_source: &str,
) -> Result<MutableNoteCreationResult, MutableNoteCreationError> {
    let resolved = resolve_project(request)?;
    let _lock = ProjectWriteLock::acquire(&resolved.project_dir)?;
    let recovery = recover_note_mutation_locked(request, &resolved.project_dir)?;
    let report = validate_project(request)?;
    if report.project_dir != resolved.project_dir {
        return Err(MutableNoteCreationError::Conflict {
            path: report.project_dir,
            message: "project resolution changed while acquiring the writer lock".to_owned(),
        });
    }

    let config = load_root_config(&resolved.root)?;
    if config
        .project
        .note_types
        .get(note_type)
        .is_some_and(|configured| configured.class == NoteClass::Event)
    {
        return Err(MutableNoteCreationError::Input {
            path: PathBuf::from(note_type),
            message: format!(
                "configured note type {note_type:?} is immutable; use create-event instead"
            ),
        });
    }
    let template = resolve_note_template(request, note_type)?;
    let configured = &config.project.note_types[note_type];
    let destination = note_destination(&resolved.project_dir, &configured.folder, relative_path)?;
    reject_existing_destination(&destination)?;

    let source = instantiate_template(
        &template.source,
        &template.path,
        &resolved.project,
        note_type,
        fields,
    )?;
    validate_note_source(
        &resolved.root,
        &resolved.project,
        note_type,
        &destination,
        source.as_bytes(),
        &configured.required_fields,
    )?;

    let mut evidence = collect_existing_evidence(&resolved.project_dir, &config)?;
    evidence.push(CanonicalNoteEvidence {
        path: destination.clone(),
        class: template.class,
        source: source.as_bytes().to_vec(),
    });
    evidence.sort_by(|left, right| left.path.cmp(&right.path));

    let index_path = resolved.project_dir.join(&config.project.index);
    let roadmap_path = resolved.project_dir.join(&config.project.roadmap);
    let index_before = read_regular_file(&index_path, "read the current index projection")?;
    let roadmap_before = read_regular_file(&roadmap_path, "read the current roadmap projection")?;
    let (projection_path, projection_before, index_after, roadmap_after) = match template.class {
        NoteClass::Record => (
            &roadmap_path,
            &roadmap_before,
            index_before.as_slice(),
            projection_source.as_bytes(),
        ),
        NoteClass::Entity => (
            &index_path,
            &index_before,
            projection_source.as_bytes(),
            roadmap_before.as_slice(),
        ),
        NoteClass::Event => unreachable!("immutable events were rejected before mutation"),
    };
    let projection_before_text = str::from_utf8(projection_before).map_err(|error| {
        MutableNoteCreationError::Validation {
            path: projection_path.clone(),
            message: format!("current maintained projection is not valid UTF-8: {error}"),
        }
    })?;

    let state_path = resolved.project_dir.join(PROJECT_STATE_FILE);
    let state_before = read_regular_file(&state_path, "read the current project state")?;
    let state_before_text =
        str::from_utf8(&state_before).map_err(|error| MutableNoteCreationError::Validation {
            path: state_path.clone(),
            message: format!("project state is not valid UTF-8: {error}"),
        })?;
    let state_after = render_updated_project_state(
        state_before_text,
        &resolved.project_dir,
        index_after,
        roadmap_after,
        &evidence,
        &BTreeSet::new(),
    )
    .map_err(|message| MutableNoteCreationError::Validation {
        path: state_path.clone(),
        message,
    })?;
    let state_after_text = str::from_utf8(&state_after)
        .expect("the deterministic project-state renderer always emits UTF-8");
    let id = vault_relative_id(&resolved.root, &destination)?;
    let projection_id = vault_relative_id(&resolved.root, projection_path)?;
    let (journal_path, journal_source) = write_note_projection_mutation_journal(
        &resolved.project_dir,
        &NoteProjectionJournal {
            project: &resolved.project,
            id: &id,
            note_after: &source,
            projection_id: &projection_id,
            projection_before: projection_before_text,
            projection_after: projection_source,
            state_before: state_before_text,
            state_after: state_after_text,
        },
    )?;
    let projection_changed = projection_before != projection_source.as_bytes();
    let result_for = |recovery| MutableNoteCreationResult {
        root: resolved.root.clone(),
        project: resolved.project.clone(),
        project_dir: resolved.project_dir.clone(),
        note_type: note_type.to_owned(),
        class: template.class,
        relative_path: relative_path.to_path_buf(),
        id: id.clone(),
        path: destination.clone(),
        template: template.path.clone(),
        template_scope: template.scope,
        projection: projection_path.clone(),
        projection_changed,
        state: state_path.clone(),
        recovery,
    };

    let operation = (|| {
        create_file_atomically(&destination, source.as_bytes())?;
        sync_parent(&destination, "sync the created mutable note")?;
        replace_file_if_unchanged(
            projection_path,
            projection_before,
            projection_source.as_bytes(),
        )
        .map_err(map_checked_replace)?;
        if projection_changed {
            sync_parent(
                projection_path,
                "sync the maintained projection replacement",
            )?;
        }
        replace_file_if_unchanged(&state_path, &state_before, &state_after)
            .map_err(map_checked_replace)?;
        sync_parent(&state_path, "sync the project state replacement")?;
        validate_project(request)?;
        complete_note_mutation_journal(&journal_path, &journal_source, &resolved.project_dir)?;
        Ok(result_for(recovery))
    })();

    match operation {
        Ok(result) => Ok(result),
        Err(original) => match recover_note_mutation_locked(request, &resolved.project_dir) {
            Ok(NoteEditRecovery::Finalized) => Ok(result_for(NoteEditRecovery::Finalized)),
            Ok(_) => Err(original),
            Err(recovery) => Err(MutableNoteCreationError::Recovery {
                original: Box::new(original),
                recovery: Box::new(recovery),
            }),
        },
    }
}

fn note_destination(
    project_dir: &Path,
    configured_folder: &Path,
    relative_path: &Path,
) -> Result<PathBuf, MutableNoteCreationError> {
    validate_relative_markdown_path(relative_path)?;
    let folder = fs::canonicalize(project_dir.join(configured_folder)).map_err(|source| {
        MutableNoteCreationError::FileSystem {
            operation: "resolve the configured note folder",
            path: project_dir.join(configured_folder),
            source,
        }
    })?;
    let requested = folder.join(relative_path);
    let parent = requested
        .parent()
        .expect("a validated relative Markdown path always has a parent");
    let parent = fs::canonicalize(parent).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            MutableNoteCreationError::Input {
                path: parent.to_path_buf(),
                message: "note parent directory does not exist".to_owned(),
            }
        } else {
            MutableNoteCreationError::FileSystem {
                operation: "resolve the note parent directory",
                path: parent.to_path_buf(),
                source,
            }
        }
    })?;
    if !parent.starts_with(&folder) {
        return Err(MutableNoteCreationError::Input {
            path: requested,
            message: format!("note path escapes configured folder {}", folder.display()),
        });
    }
    Ok(parent.join(
        relative_path
            .file_name()
            .expect("a validated relative Markdown path always has a filename"),
    ))
}

fn validate_relative_markdown_path(path: &Path) -> Result<(), MutableNoteCreationError> {
    let Some(text) = path.to_str() else {
        return Err(MutableNoteCreationError::Input {
            path: path.to_path_buf(),
            message: "note paths must be valid UTF-8".to_owned(),
        });
    };
    if text.is_empty()
        || text.starts_with('/')
        || text.contains('\\')
        || text
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || path.extension().and_then(|extension| extension.to_str()) != Some("md")
    {
        return Err(MutableNoteCreationError::Input {
            path: path.to_path_buf(),
            message: "note path must be a normalized relative .md path using / separators"
                .to_owned(),
        });
    }
    Ok(())
}

fn reject_existing_destination(path: &Path) -> Result<(), MutableNoteCreationError> {
    match fs::symlink_metadata(path) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(MutableNoteCreationError::Conflict {
            path: path.to_path_buf(),
            message: "record/entity destination already exists".to_owned(),
        }),
        Err(source) => Err(MutableNoteCreationError::FileSystem {
            operation: "inspect the record/entity destination",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn instantiate_template(
    source: &str,
    template_path: &Path,
    project: &str,
    note_type: &str,
    fields: &BTreeMap<String, String>,
) -> Result<String, MutableNoteCreationError> {
    for name in fields.keys() {
        if !valid_placeholder_name(name) {
            return Err(MutableNoteCreationError::Input {
                path: PathBuf::from(name),
                message: "field names must start with a lowercase ASCII letter and contain only lowercase ASCII letters, digits, `_`, or `-`"
                    .to_owned(),
            });
        }
        if name == "project" || name == "type" {
            return Err(MutableNoteCreationError::Input {
                path: PathBuf::from(name),
                message: format!("field {name:?} is supplied by Akasha and cannot be overridden"),
            });
        }
    }

    let mut values = fields.clone();
    values.insert("project".to_owned(), project.to_owned());
    values.insert("type".to_owned(), note_type.to_owned());
    let mut used = BTreeSet::new();
    let mut rendered = String::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(open_offset) = source[cursor..].find("{{") {
        let open = cursor + open_offset;
        rendered.push_str(&source[cursor..open]);
        let marker_start = open + 2;
        let Some(close_offset) = source[marker_start..].find("}}") else {
            rendered.push_str(&source[open..]);
            cursor = source.len();
            break;
        };
        let close = marker_start + close_offset;
        let name = &source[marker_start..close];
        if valid_placeholder_name(name) {
            let value = values
                .get(name)
                .ok_or_else(|| MutableNoteCreationError::Input {
                    path: template_path.to_path_buf(),
                    message: format!("template field {name:?} has no supplied value"),
                })?;
            rendered.push_str(value);
            used.insert(name.to_owned());
        } else {
            rendered.push_str(&source[open..close + 2]);
        }
        cursor = close + 2;
    }
    rendered.push_str(&source[cursor..]);

    if let Some(unused) = fields.keys().find(|name| !used.contains(*name)) {
        return Err(MutableNoteCreationError::Input {
            path: PathBuf::from(unused),
            message: format!(
                "field {unused:?} is not used by template {}",
                template_path.display()
            ),
        });
    }
    Ok(rendered)
}

fn valid_placeholder_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

fn validate_note_source(
    root: &Path,
    project: &str,
    note_type: &str,
    path: &Path,
    source: &[u8],
    required_fields: &[String],
) -> Result<(), MutableNoteCreationError> {
    let parsed = parse_leading_frontmatter_bytes(source).map_err(|error| {
        MutableNoteCreationError::Validation {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    validate_configured_note(&parsed, project, note_type, required_fields).map_err(|error| {
        MutableNoteCreationError::Validation {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    validate_wikilinks_with_targets(
        root,
        path,
        parsed.body,
        &BTreeSet::from([path.to_path_buf()]),
    )?;
    Ok(())
}

fn collect_existing_evidence(
    project_dir: &Path,
    config: &RootConfig,
) -> Result<Vec<CanonicalNoteEvidence>, MutableNoteCreationError> {
    let mut evidence = Vec::new();
    for note_type in config.project.note_types.values() {
        for path in canonical_note_paths(&project_dir.join(&note_type.folder))? {
            let source = read_regular_file(&path, "read canonical note for mutable-note state")?;
            evidence.push(CanonicalNoteEvidence {
                path,
                class: note_type.class,
                source,
            });
        }
    }
    Ok(evidence)
}

fn read_regular_file(
    path: &Path,
    operation: &'static str,
) -> Result<Vec<u8>, MutableNoteCreationError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|source| MutableNoteCreationError::FileSystem {
            operation,
            path: path.to_path_buf(),
            source,
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(MutableNoteCreationError::Conflict {
            path: path.to_path_buf(),
            message: "checked mutable-note target is not a regular file".to_owned(),
        });
    }
    fs::read(path).map_err(|source| MutableNoteCreationError::FileSystem {
        operation,
        path: path.to_path_buf(),
        source,
    })
}

fn vault_relative_id(root: &Path, path: &Path) -> Result<String, MutableNoteCreationError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| MutableNoteCreationError::Validation {
            path: path.to_path_buf(),
            message: format!("mutable-note path is outside data root {}", root.display()),
        })?;
    let text = relative
        .to_str()
        .ok_or_else(|| MutableNoteCreationError::Input {
            path: relative.to_path_buf(),
            message: "mutable-note identity must be valid UTF-8".to_owned(),
        })?;
    Ok(text.replace(std::path::MAIN_SEPARATOR, "/"))
}

fn sync_parent(path: &Path, operation: &'static str) -> Result<(), MutableNoteCreationError> {
    let parent = path
        .parent()
        .expect("resolved mutable-note transaction paths always have a parent");
    sync_directory(parent).map_err(|source| MutableNoteCreationError::FileSystem {
        operation,
        path: parent.to_path_buf(),
        source,
    })
}

fn map_checked_replace(error: CheckedReplaceError) -> MutableNoteCreationError {
    match error {
        CheckedReplaceError::Conflict { path, .. } => MutableNoteCreationError::Conflict {
            path,
            message: "checked mutable-note transaction target changed during creation".to_owned(),
        },
        CheckedReplaceError::FileSystem {
            operation,
            path,
            source,
        } => MutableNoteCreationError::FileSystem {
            operation,
            path,
            source,
        },
    }
}
