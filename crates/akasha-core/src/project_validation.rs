use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str;

use serde::Serialize;

use crate::resolution::{
    NoteClass, ResolveError, ResolveRequest, load_project_registry, load_root_config,
    resolve_project,
};
use crate::state::{CanonicalNoteEvidence, PROJECT_STATE_FILE, validate_project_state};
use crate::validation::{
    ValidationError, parse_leading_frontmatter_bytes, validate_configured_note,
};
use crate::wikilink::parse_wikilinks;

/// Count and class for one configured canonical note type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteTypeValidation {
    pub class: NoteClass,
    pub notes: usize,
}

/// Checked source count for one configured derived projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectionValidation {
    pub sources: usize,
}

/// Deterministic summary returned after a selected project passes validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectValidationReport {
    pub root: PathBuf,
    pub project: String,
    pub project_dir: PathBuf,
    pub repository_dir: PathBuf,
    pub registry: PathBuf,
    pub state: PathBuf,
    pub registry_projects: usize,
    pub canonical_notes: usize,
    pub immutable_events: usize,
    pub projections: BTreeMap<String, ProjectionValidation>,
    pub wikilinks: usize,
    pub note_types: BTreeMap<String, NoteTypeValidation>,
}

/// A project-wide resolution, schema, layout, or filesystem failure.
#[derive(Debug)]
pub enum ProjectValidationError {
    Resolve(Box<ResolveError>),
    InvalidDocument {
        path: PathBuf,
        source: Box<ValidationError>,
    },
    InvalidWikilink {
        path: PathBuf,
        message: String,
    },
    InvalidState {
        path: PathBuf,
        message: String,
    },
    FileSystem {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl ProjectValidationError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Resolve(source) => source.exit_code(),
            Self::InvalidDocument { source, .. } => source.exit_code(),
            Self::InvalidWikilink { .. } | Self::InvalidState { .. } => 4,
            Self::FileSystem { .. } => 6,
        }
    }
}

impl fmt::Display for ProjectValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolve(source) => source.fmt(formatter),
            Self::InvalidDocument { path, source } => {
                write!(
                    formatter,
                    "validation failed at {}: {source}",
                    path.display()
                )
            }
            Self::InvalidWikilink { path, message } => {
                write!(
                    formatter,
                    "validation failed at {}: invalid wikilink: {message}",
                    path.display()
                )
            }
            Self::InvalidState { path, message } => {
                write!(
                    formatter,
                    "validation failed at {}: invalid project state: {message}",
                    path.display()
                )
            }
            Self::FileSystem {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} at {}: {source}",
                path.display()
            ),
        }
    }
}

impl Error for ProjectValidationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Resolve(source) => Some(source.as_ref()),
            Self::InvalidDocument { source, .. } => Some(source.as_ref()),
            Self::InvalidWikilink { .. } | Self::InvalidState { .. } => None,
            Self::FileSystem { source, .. } => Some(source),
        }
    }
}

impl From<ResolveError> for ProjectValidationError {
    fn from(source: ResolveError) -> Self {
        Self::Resolve(Box::new(source))
    }
}

/// Resolve a project, validate its configured layout, and inspect every canonical note.
pub fn validate_project(
    request: &ResolveRequest,
) -> Result<ProjectValidationReport, ProjectValidationError> {
    let resolved = resolve_project(request)?;
    let config = load_root_config(&resolved.root)?;
    let (_, registry) = load_project_registry(&resolved.root, &config)?;

    for path in [
        &config.folders.templates,
        &config.folders.global,
        &config.folders.projects,
        &config.folders.inbox,
    ] {
        require_directory(&resolved.root.join(path), &resolved.root)?;
    }
    require_directory(
        &resolved.project_dir.join(&config.project.templates),
        &resolved.project_dir,
    )?;
    let index = require_file(
        &resolved.project_dir.join(&config.project.index),
        &resolved.project_dir,
    )?;
    let roadmap = require_file(
        &resolved.project_dir.join(&config.project.roadmap),
        &resolved.project_dir,
    )?;
    let state = require_file(
        &resolved.project_dir.join(PROJECT_STATE_FILE),
        &resolved.project_dir,
    )?;

    let mut canonical_notes = 0;
    let mut wikilinks = 0;
    let mut evidence = Vec::new();
    let mut note_types = BTreeMap::new();
    for (name, note_type) in &config.project.note_types {
        let folder = resolved.project_dir.join(&note_type.folder);
        let folder = require_directory(&folder, &resolved.project_dir)?;
        let validated = validate_note_tree(
            &resolved.root,
            &folder,
            &resolved.project,
            name,
            note_type.class,
            &note_type.required_fields,
        )?;
        let note_count = validated.notes.len();
        canonical_notes += note_count;
        wikilinks += validated.wikilinks;
        evidence.extend(validated.notes);
        note_types.insert(
            name.clone(),
            NoteTypeValidation {
                class: note_type.class,
                notes: note_count,
            },
        );
    }

    let index_source = read_validation_file(&index, "read index projection")?;
    let roadmap_source = read_validation_file(&roadmap, "read roadmap projection")?;
    let state_source = read_validation_file(&state, "read project state")?;
    let state_text =
        str::from_utf8(&state_source).map_err(|error| ProjectValidationError::InvalidState {
            path: state.clone(),
            message: format!("project state is not valid UTF-8: {error}"),
        })?;
    let state_validation = validate_project_state(
        state_text,
        &resolved.project_dir,
        &index_source,
        &roadmap_source,
        &evidence,
    )
    .map_err(|message| ProjectValidationError::InvalidState {
        path: state.clone(),
        message,
    })?;
    let projections = state_validation
        .projection_sources
        .into_iter()
        .map(|(name, sources)| (name, ProjectionValidation { sources }))
        .collect();

    Ok(ProjectValidationReport {
        root: resolved.root,
        project: resolved.project,
        project_dir: resolved.project_dir,
        repository_dir: resolved.repository_dir,
        registry: resolved.registry,
        state,
        registry_projects: registry.projects.len(),
        canonical_notes,
        immutable_events: state_validation.immutable_events,
        projections,
        wikilinks,
        note_types,
    })
}

struct ValidatedNoteTree {
    notes: Vec<CanonicalNoteEvidence>,
    wikilinks: usize,
}

fn validate_note_tree(
    root: &Path,
    directory: &Path,
    project: &str,
    note_type: &str,
    class: NoteClass,
    required_fields: &[String],
) -> Result<ValidatedNoteTree, ProjectValidationError> {
    let paths = canonical_note_paths(directory)?;
    let mut wikilinks = 0;
    let mut notes = Vec::with_capacity(paths.len());
    for path in paths {
        let source = fs::read(&path).map_err(|source| ProjectValidationError::FileSystem {
            operation: "read canonical note",
            path: path.clone(),
            source,
        })?;
        let parsed = parse_leading_frontmatter_bytes(&source).map_err(|source| {
            ProjectValidationError::InvalidDocument {
                path: path.clone(),
                source: Box::new(source),
            }
        })?;
        validate_configured_note(&parsed, project, note_type, required_fields).map_err(
            |source| ProjectValidationError::InvalidDocument {
                path: path.clone(),
                source: Box::new(source),
            },
        )?;
        wikilinks += validate_wikilinks(root, &path, parsed.body)?;
        notes.push(CanonicalNoteEvidence {
            path,
            class,
            source,
        });
    }
    Ok(ValidatedNoteTree { notes, wikilinks })
}

fn read_validation_file(
    path: &Path,
    operation: &'static str,
) -> Result<Vec<u8>, ProjectValidationError> {
    fs::read(path).map_err(|source| ProjectValidationError::FileSystem {
        operation,
        path: path.to_path_buf(),
        source,
    })
}

fn validate_wikilinks(
    root: &Path,
    source_path: &Path,
    body: &str,
) -> Result<usize, ProjectValidationError> {
    validate_wikilinks_with_targets(root, source_path, body, &BTreeSet::new())
}

pub(crate) fn validate_wikilinks_with_targets(
    root: &Path,
    source_path: &Path,
    body: &str,
    proposed_targets: &BTreeSet<PathBuf>,
) -> Result<usize, ProjectValidationError> {
    let links =
        parse_wikilinks(body).map_err(|source| ProjectValidationError::InvalidWikilink {
            path: source_path.to_path_buf(),
            message: source.to_string(),
        })?;
    for link in &links {
        let Some(target) = link.target else {
            continue;
        };
        let relative = wikilink_target_path(target).map_err(|message| {
            ProjectValidationError::InvalidWikilink {
                path: source_path.to_path_buf(),
                message,
            }
        })?;
        let target_path = root.join(relative);
        if proposed_targets.contains(&target_path) {
            continue;
        }
        let metadata = match fs::symlink_metadata(&target_path) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Err(ProjectValidationError::InvalidWikilink {
                    path: source_path.to_path_buf(),
                    message: format!(
                        "target {target:?} does not exist as {}",
                        target_path.display()
                    ),
                });
            }
            Err(source) => {
                return Err(ProjectValidationError::FileSystem {
                    operation: "inspect wikilink target",
                    path: target_path,
                    source,
                });
            }
        };
        if metadata.file_type().is_symlink() {
            return Err(ProjectValidationError::InvalidWikilink {
                path: source_path.to_path_buf(),
                message: format!("target {target:?} must not be a symbolic link"),
            });
        }
        if !metadata.is_file() {
            return Err(ProjectValidationError::InvalidWikilink {
                path: source_path.to_path_buf(),
                message: format!("target {target:?} is not a regular Markdown file"),
            });
        }
        let canonical = fs::canonicalize(&target_path).map_err(|source| {
            ProjectValidationError::FileSystem {
                operation: "resolve wikilink target",
                path: target_path.clone(),
                source,
            }
        })?;
        if !canonical.starts_with(root) {
            return Err(ProjectValidationError::InvalidWikilink {
                path: source_path.to_path_buf(),
                message: format!("target {target:?} escapes data root {}", root.display()),
            });
        }
    }

    Ok(links.len())
}

fn wikilink_target_path(target: &str) -> Result<PathBuf, String> {
    if target.contains('\\') {
        return Err(format!(
            "target {target:?} must use vault-relative / separators"
        ));
    }
    if target.starts_with('/') {
        return Err(format!("target {target:?} must be vault-relative"));
    }

    let parts = target.split('/').collect::<Vec<_>>();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err(format!(
            "target {target:?} contains an empty, current, or parent path component"
        ));
    }

    let mut relative = PathBuf::new();
    for part in parts {
        relative.push(part);
    }
    if !target.ends_with(".md") {
        let file_name = relative
            .file_name()
            .expect("a validated non-empty target has a file name")
            .to_os_string();
        let mut markdown_name = file_name;
        markdown_name.push(".md");
        relative.set_file_name(markdown_name);
    }
    Ok(relative)
}

pub(crate) fn canonical_note_paths(
    directory: &Path,
) -> Result<Vec<PathBuf>, ProjectValidationError> {
    let mut notes = Vec::new();
    collect_note_paths(directory, &mut notes)?;
    notes.sort();
    Ok(notes)
}

fn collect_note_paths(
    directory: &Path,
    notes: &mut Vec<PathBuf>,
) -> Result<(), ProjectValidationError> {
    let entries = fs::read_dir(directory).map_err(|source| ProjectValidationError::FileSystem {
        operation: "read canonical note directory",
        path: directory.to_path_buf(),
        source,
    })?;
    let mut paths = entries
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|source| ProjectValidationError::FileSystem {
                    operation: "read canonical note directory entry",
                    path: directory.to_path_buf(),
                    source,
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();

    for path in paths {
        let metadata =
            fs::symlink_metadata(&path).map_err(|source| ProjectValidationError::FileSystem {
                operation: "inspect canonical note path",
                path: path.clone(),
                source,
            })?;
        if metadata.file_type().is_symlink() {
            return Err(invalid_layout(
                &path,
                "symbolic links are not allowed in canonical note folders",
            ));
        }
        if metadata.is_dir() {
            collect_note_paths(&path, notes)?;
            continue;
        }
        if !metadata.is_file() || path.extension().and_then(|value| value.to_str()) != Some("md") {
            return Err(invalid_layout(
                &path,
                "canonical note folders may contain only directories and .md files",
            ));
        }

        notes.push(path);
    }
    Ok(())
}

fn require_directory(path: &Path, boundary: &Path) -> Result<PathBuf, ProjectValidationError> {
    require_path(path, boundary, true)
}

fn require_file(path: &Path, boundary: &Path) -> Result<PathBuf, ProjectValidationError> {
    require_path(path, boundary, false)
}

fn require_path(
    path: &Path,
    boundary: &Path,
    directory: bool,
) -> Result<PathBuf, ProjectValidationError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            let expected = if directory { "directory" } else { "file" };
            return Err(invalid_layout(
                path,
                &format!("required {expected} does not exist"),
            ));
        }
        Err(source) => {
            return Err(ProjectValidationError::FileSystem {
                operation: "inspect required path",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(invalid_layout(
            path,
            "required layout paths must not be symbolic links",
        ));
    }
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        let expected = if directory { "directory" } else { "file" };
        return Err(invalid_layout(
            path,
            &format!("required path is not a {expected}"),
        ));
    }

    let canonical =
        fs::canonicalize(path).map_err(|source| ProjectValidationError::FileSystem {
            operation: "resolve required path",
            path: path.to_path_buf(),
            source,
        })?;
    if !canonical.starts_with(boundary) {
        return Err(invalid_layout(
            path,
            &format!("required path escapes boundary {}", boundary.display()),
        ));
    }
    Ok(canonical)
}

fn invalid_layout(path: &Path, message: &str) -> ProjectValidationError {
    ProjectValidationError::InvalidDocument {
        path: path.to_path_buf(),
        source: ValidationError::InvalidSchema {
            document: "project layout",
            message: message.to_owned(),
        }
        .into(),
    }
}
