use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::resolution::{
    NoteClass, ResolveError, ResolveRequest, load_project_registry, load_root_config,
    resolve_project,
};
use crate::validation::{
    ValidationError, parse_leading_frontmatter_bytes, validate_configured_note,
};

/// Count and class for one configured canonical note type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteTypeValidation {
    pub class: NoteClass,
    pub notes: usize,
}

/// Deterministic summary returned after a selected project passes validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectValidationReport {
    pub root: PathBuf,
    pub project: String,
    pub project_dir: PathBuf,
    pub repository_dir: PathBuf,
    pub registry: PathBuf,
    pub registry_projects: usize,
    pub canonical_notes: usize,
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
    for path in [&config.project.index, &config.project.roadmap] {
        require_file(&resolved.project_dir.join(path), &resolved.project_dir)?;
    }

    let mut canonical_notes = 0;
    let mut note_types = BTreeMap::new();
    for (name, note_type) in &config.project.note_types {
        let folder = resolved.project_dir.join(&note_type.folder);
        let folder = require_directory(&folder, &resolved.project_dir)?;
        let notes =
            validate_note_tree(&folder, &resolved.project, name, &note_type.required_fields)?;
        canonical_notes += notes;
        note_types.insert(
            name.clone(),
            NoteTypeValidation {
                class: note_type.class,
                notes,
            },
        );
    }

    Ok(ProjectValidationReport {
        root: resolved.root,
        project: resolved.project,
        project_dir: resolved.project_dir,
        repository_dir: resolved.repository_dir,
        registry: resolved.registry,
        registry_projects: registry.projects.len(),
        canonical_notes,
        note_types,
    })
}

fn validate_note_tree(
    directory: &Path,
    project: &str,
    note_type: &str,
    required_fields: &[String],
) -> Result<usize, ProjectValidationError> {
    let paths = canonical_note_paths(directory)?;
    for path in &paths {
        let source = fs::read(path).map_err(|source| ProjectValidationError::FileSystem {
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
    }
    Ok(paths.len())
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
