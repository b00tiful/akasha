use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str;

use serde::Serialize;

use crate::note_edit::{
    NoteEditError, NoteEditRecovery, complete_note_mutation_journal, recover_note_mutation_locked,
    write_note_mutation_journal,
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

/// Result of one create-only immutable event transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventCreationResult {
    pub root: PathBuf,
    pub project: String,
    pub project_dir: PathBuf,
    pub note_type: String,
    pub relative_path: PathBuf,
    pub id: String,
    pub path: PathBuf,
    pub template: PathBuf,
    pub template_scope: NoteTemplateScope,
    pub state: PathBuf,
    pub recovery: NoteEditRecovery,
}

/// An input, resolution, validation, conflict, filesystem, or recovery failure.
#[derive(Debug)]
pub enum EventCreationError {
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
        original: Box<EventCreationError>,
        recovery: Box<NoteEditError>,
    },
}

impl EventCreationError {
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

impl fmt::Display for EventCreationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input { path, message } => {
                write!(
                    formatter,
                    "invalid event input at {}: {message}",
                    path.display()
                )
            }
            Self::Resolve(error) => error.fmt(formatter),
            Self::Project(error) => error.fmt(formatter),
            Self::Template(error) => error.fmt(formatter),
            Self::Mutation(error) => error.fmt(formatter),
            Self::Validation { path, message } => write!(
                formatter,
                "invalid event creation at {}: {message}",
                path.display()
            ),
            Self::Conflict { path, message } => write!(
                formatter,
                "event creation conflict at {}: {message}",
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
                "event creation failed and automatic recovery also failed: original error: {original}; recovery error: {recovery}"
            ),
        }
    }
}

impl Error for EventCreationError {
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

impl From<ResolveError> for EventCreationError {
    fn from(error: ResolveError) -> Self {
        Self::Resolve(Box::new(error))
    }
}

impl From<ProjectValidationError> for EventCreationError {
    fn from(error: ProjectValidationError) -> Self {
        Self::Project(Box::new(error))
    }
}

impl From<NoteTemplateError> for EventCreationError {
    fn from(error: NoteTemplateError) -> Self {
        Self::Template(Box::new(error))
    }
}

impl From<NoteEditError> for EventCreationError {
    fn from(error: NoteEditError) -> Self {
        Self::Mutation(Box::new(error))
    }
}

impl From<AtomicCreateError> for EventCreationError {
    fn from(error: AtomicCreateError) -> Self {
        Self::Creation(error)
    }
}

/// Create one configured event from its exact resolved template.
///
/// The path is relative to the configured note-type folder. Template placeholders use exact
/// `{{name}}` markers. `project` and `type` are supplied by the core; every other marker must be
/// supplied by the caller, and unused caller fields are rejected.
pub fn create_event(
    request: &ResolveRequest,
    note_type: &str,
    relative_path: &Path,
    fields: &BTreeMap<String, String>,
) -> Result<EventCreationResult, EventCreationError> {
    let resolved = resolve_project(request)?;
    let _lock = ProjectWriteLock::acquire(&resolved.project_dir)?;
    let recovery = recover_note_mutation_locked(request, &resolved.project_dir)?;
    let report = validate_project(request)?;
    if report.project_dir != resolved.project_dir {
        return Err(EventCreationError::Conflict {
            path: report.project_dir,
            message: "project resolution changed while acquiring the writer lock".to_owned(),
        });
    }

    let config = load_root_config(&resolved.root)?;
    let template = resolve_note_template(request, note_type)?;
    if template.class != NoteClass::Event {
        return Err(EventCreationError::Input {
            path: PathBuf::from(note_type),
            message: format!("configured note type {note_type:?} is not an immutable event"),
        });
    }
    let configured = &config.project.note_types[note_type];
    let destination = event_destination(&resolved.project_dir, &configured.folder, relative_path)?;
    reject_existing_destination(&destination)?;

    let source = instantiate_template(
        &template.source,
        &template.path,
        &resolved.project,
        note_type,
        fields,
    )?;
    validate_event_source(
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
        class: NoteClass::Event,
        source: source.as_bytes().to_vec(),
    });
    evidence.sort_by(|left, right| left.path.cmp(&right.path));

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
        str::from_utf8(&state_before).map_err(|error| EventCreationError::Validation {
            path: state_path.clone(),
            message: format!("project state is not valid UTF-8: {error}"),
        })?;
    let state_after = render_updated_project_state(
        state_before_text,
        &resolved.project_dir,
        &index,
        &roadmap,
        &evidence,
        &BTreeSet::from([destination.clone()]),
    )
    .map_err(|message| EventCreationError::Validation {
        path: state_path.clone(),
        message,
    })?;
    let state_after_text = str::from_utf8(&state_after)
        .expect("the deterministic project-state renderer always emits UTF-8");
    let id = vault_relative_id(&resolved.root, &destination)?;
    let (journal_path, journal_source) = write_note_mutation_journal(
        &resolved.project_dir,
        &resolved.project,
        &id,
        None,
        &source,
        state_before_text,
        state_after_text,
    )?;
    let result_for = |recovery| EventCreationResult {
        root: resolved.root.clone(),
        project: resolved.project.clone(),
        project_dir: resolved.project_dir.clone(),
        note_type: note_type.to_owned(),
        relative_path: relative_path.to_path_buf(),
        id: id.clone(),
        path: destination.clone(),
        template: template.path.clone(),
        template_scope: template.scope,
        state: state_path.clone(),
        recovery,
    };

    let operation = (|| {
        create_file_atomically(&destination, source.as_bytes())?;
        sync_parent(&destination, "sync the created event")?;
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
            Err(recovery) => Err(EventCreationError::Recovery {
                original: Box::new(original),
                recovery: Box::new(recovery),
            }),
        },
    }
}

fn event_destination(
    project_dir: &Path,
    configured_folder: &Path,
    relative_path: &Path,
) -> Result<PathBuf, EventCreationError> {
    validate_relative_markdown_path(relative_path)?;
    let folder = fs::canonicalize(project_dir.join(configured_folder)).map_err(|source| {
        EventCreationError::FileSystem {
            operation: "resolve the configured event folder",
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
            EventCreationError::Input {
                path: parent.to_path_buf(),
                message: "event parent directory does not exist".to_owned(),
            }
        } else {
            EventCreationError::FileSystem {
                operation: "resolve the event parent directory",
                path: parent.to_path_buf(),
                source,
            }
        }
    })?;
    if !parent.starts_with(&folder) {
        return Err(EventCreationError::Input {
            path: requested,
            message: format!("event path escapes configured folder {}", folder.display()),
        });
    }
    Ok(parent.join(
        relative_path
            .file_name()
            .expect("a validated relative Markdown path always has a filename"),
    ))
}

fn validate_relative_markdown_path(path: &Path) -> Result<(), EventCreationError> {
    let Some(text) = path.to_str() else {
        return Err(EventCreationError::Input {
            path: path.to_path_buf(),
            message: "event paths must be valid UTF-8".to_owned(),
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
        return Err(EventCreationError::Input {
            path: path.to_path_buf(),
            message: "event path must be a normalized relative .md path using / separators"
                .to_owned(),
        });
    }
    Ok(())
}

fn reject_existing_destination(path: &Path) -> Result<(), EventCreationError> {
    match fs::symlink_metadata(path) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(EventCreationError::Conflict {
            path: path.to_path_buf(),
            message: "immutable event destination already exists".to_owned(),
        }),
        Err(source) => Err(EventCreationError::FileSystem {
            operation: "inspect the event destination",
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
) -> Result<String, EventCreationError> {
    for name in fields.keys() {
        if !valid_placeholder_name(name) {
            return Err(EventCreationError::Input {
                path: PathBuf::from(name),
                message: "field names must start with a lowercase ASCII letter and contain only lowercase ASCII letters, digits, `_`, or `-`"
                    .to_owned(),
            });
        }
        if name == "project" || name == "type" {
            return Err(EventCreationError::Input {
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
            let value = values.get(name).ok_or_else(|| EventCreationError::Input {
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
        return Err(EventCreationError::Input {
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

fn validate_event_source(
    root: &Path,
    project: &str,
    note_type: &str,
    path: &Path,
    source: &[u8],
    required_fields: &[String],
) -> Result<(), EventCreationError> {
    let parsed = parse_leading_frontmatter_bytes(source).map_err(|error| {
        EventCreationError::Validation {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    validate_configured_note(&parsed, project, note_type, required_fields).map_err(|error| {
        EventCreationError::Validation {
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
) -> Result<Vec<CanonicalNoteEvidence>, EventCreationError> {
    let mut evidence = Vec::new();
    for note_type in config.project.note_types.values() {
        for path in canonical_note_paths(&project_dir.join(&note_type.folder))? {
            let source = read_regular_file(&path, "read canonical note for event state")?;
            evidence.push(CanonicalNoteEvidence {
                path,
                class: note_type.class,
                source,
            });
        }
    }
    Ok(evidence)
}

fn read_regular_file(path: &Path, operation: &'static str) -> Result<Vec<u8>, EventCreationError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| EventCreationError::FileSystem {
        operation,
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(EventCreationError::Conflict {
            path: path.to_path_buf(),
            message: "checked event target is not a regular file".to_owned(),
        });
    }
    fs::read(path).map_err(|source| EventCreationError::FileSystem {
        operation,
        path: path.to_path_buf(),
        source,
    })
}

fn vault_relative_id(root: &Path, path: &Path) -> Result<String, EventCreationError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| EventCreationError::Validation {
            path: path.to_path_buf(),
            message: format!("event destination is outside data root {}", root.display()),
        })?;
    let text = relative.to_str().ok_or_else(|| EventCreationError::Input {
        path: relative.to_path_buf(),
        message: "event identity must be valid UTF-8".to_owned(),
    })?;
    Ok(text.replace(std::path::MAIN_SEPARATOR, "/"))
}

fn sync_parent(path: &Path, operation: &'static str) -> Result<(), EventCreationError> {
    let parent = path
        .parent()
        .expect("resolved event transaction paths always have a parent");
    sync_directory(parent).map_err(|source| EventCreationError::FileSystem {
        operation,
        path: parent.to_path_buf(),
        source,
    })
}

fn map_checked_replace(error: CheckedReplaceError) -> EventCreationError {
    match error {
        CheckedReplaceError::Conflict { path, .. } => EventCreationError::Conflict {
            path,
            message: "project state changed during event creation".to_owned(),
        },
        CheckedReplaceError::FileSystem {
            operation,
            path,
            source,
        } => EventCreationError::FileSystem {
            operation,
            path,
            source,
        },
    }
}
