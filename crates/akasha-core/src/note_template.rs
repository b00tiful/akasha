use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::resolution::{
    NoteClass, ResolveError, ResolveRequest, load_root_config, resolve_project,
};

/// The configured layer that supplied a note template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoteTemplateScope {
    Project,
    Root,
}

/// An exact configured Markdown template selected for one canonical note type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedNoteTemplate {
    pub note_type: String,
    pub class: NoteClass,
    pub scope: NoteTemplateScope,
    pub path: PathBuf,
    pub source: String,
}

/// A project-resolution, template-selection, validation, or filesystem failure.
#[derive(Debug)]
pub enum NoteTemplateError {
    Resolve(Box<ResolveError>),
    UnknownNoteType {
        note_type: String,
    },
    Missing {
        note_type: String,
        project_path: PathBuf,
        root_path: PathBuf,
    },
    InvalidTemplate {
        path: PathBuf,
        message: String,
    },
    FileSystem {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl NoteTemplateError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Resolve(source) => source.exit_code(),
            Self::UnknownNoteType { .. } => 2,
            Self::Missing { .. } | Self::InvalidTemplate { .. } => 4,
            Self::FileSystem { .. } => 6,
        }
    }
}

impl fmt::Display for NoteTemplateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolve(source) => source.fmt(formatter),
            Self::UnknownNoteType { note_type } => {
                write!(formatter, "unknown configured note type {note_type:?}")
            }
            Self::Missing {
                note_type,
                project_path,
                root_path,
            } => write!(
                formatter,
                "note type {note_type:?} has no project template at {} and no root template at {}",
                project_path.display(),
                root_path.display()
            ),
            Self::InvalidTemplate { path, message } => {
                write!(
                    formatter,
                    "invalid note template at {}: {message}",
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

impl Error for NoteTemplateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Resolve(source) => Some(source.as_ref()),
            Self::FileSystem { source, .. } => Some(source),
            Self::UnknownNoteType { .. } | Self::Missing { .. } | Self::InvalidTemplate { .. } => {
                None
            }
        }
    }
}

impl From<ResolveError> for NoteTemplateError {
    fn from(source: ResolveError) -> Self {
        Self::Resolve(Box::new(source))
    }
}

/// Resolve one configured note template without modifying the data root.
///
/// The selected project's exact `<type>.md` template takes precedence. When it is absent, the
/// data-root template with the same name is used. An existing non-regular, symlinked, or non-UTF-8
/// candidate fails closed instead of silently falling through to the other layer.
pub fn resolve_note_template(
    request: &ResolveRequest,
    note_type: &str,
) -> Result<ResolvedNoteTemplate, NoteTemplateError> {
    let resolved = resolve_project(request)?;
    let config = load_root_config(&resolved.root)?;
    let configured = config.project.note_types.get(note_type).ok_or_else(|| {
        NoteTemplateError::UnknownNoteType {
            note_type: note_type.to_owned(),
        }
    })?;
    let filename = format!("{note_type}.md");
    let project_templates = require_template_directory(
        &resolved.project_dir.join(&config.project.templates),
        &resolved.project_dir,
    )?;
    let root_templates = require_template_directory(
        &resolved.root.join(&config.folders.templates),
        &resolved.root,
    )?;
    let project_path = project_templates.join(&filename);
    let root_path = root_templates.join(filename);

    if let Some(source) = read_template(&project_path)? {
        return Ok(ResolvedNoteTemplate {
            note_type: note_type.to_owned(),
            class: configured.class,
            scope: NoteTemplateScope::Project,
            path: project_path,
            source,
        });
    }
    if let Some(source) = read_template(&root_path)? {
        return Ok(ResolvedNoteTemplate {
            note_type: note_type.to_owned(),
            class: configured.class,
            scope: NoteTemplateScope::Root,
            path: root_path,
            source,
        });
    }

    Err(NoteTemplateError::Missing {
        note_type: note_type.to_owned(),
        project_path,
        root_path,
    })
}

fn require_template_directory(path: &Path, boundary: &Path) -> Result<PathBuf, NoteTemplateError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| NoteTemplateError::FileSystem {
        operation: "inspect note-template directory",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(NoteTemplateError::InvalidTemplate {
            path: path.to_path_buf(),
            message: "template directory must not be a symbolic link".to_owned(),
        });
    }
    if !metadata.is_dir() {
        return Err(NoteTemplateError::InvalidTemplate {
            path: path.to_path_buf(),
            message: "template directory is not a directory".to_owned(),
        });
    }

    let canonical = fs::canonicalize(path).map_err(|source| NoteTemplateError::FileSystem {
        operation: "resolve note-template directory",
        path: path.to_path_buf(),
        source,
    })?;
    if !canonical.starts_with(boundary) || canonical != path {
        return Err(NoteTemplateError::InvalidTemplate {
            path: path.to_path_buf(),
            message: format!(
                "template directory resolves outside its configured boundary {}",
                boundary.display()
            ),
        });
    }
    Ok(canonical)
}

fn read_template(path: &Path) -> Result<Option<String>, NoteTemplateError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(NoteTemplateError::FileSystem {
                operation: "inspect note template",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(NoteTemplateError::InvalidTemplate {
            path: path.to_path_buf(),
            message: "template must not be a symbolic link".to_owned(),
        });
    }
    if !metadata.is_file() {
        return Err(NoteTemplateError::InvalidTemplate {
            path: path.to_path_buf(),
            message: "template is not a regular file".to_owned(),
        });
    }

    let source = fs::read(path).map_err(|source| NoteTemplateError::FileSystem {
        operation: "read note template",
        path: path.to_path_buf(),
        source,
    })?;
    String::from_utf8(source)
        .map(Some)
        .map_err(|source| NoteTemplateError::InvalidTemplate {
            path: path.to_path_buf(),
            message: format!("template is not valid UTF-8: {source}"),
        })
}
