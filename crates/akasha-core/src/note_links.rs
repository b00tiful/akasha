//! Bounded inspection of the canonical note graph, including maintained projections.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::library::{build_library_projection, vault_id};
use crate::note_edit::{NoteEditError, inspect_pending_note_edit};
use crate::project_validation::{
    ProjectValidationError, validate_wikilinks_with_targets, wikilink_target_path,
};
use crate::resolution::{ResolveRequest, load_project_registry, load_root_config, resolve_project};
use crate::validation::{ValidationError, optional_frontmatter_body, parse_leading_frontmatter};
use crate::wikilink::parse_wikilinks;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoteLinkDirection {
    Outgoing,
    Incoming,
}

/// Classification does not grant document-read or mutation authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoteLinkKind {
    Canonical,
    Projection,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteLink {
    /// Exact full vault-relative Markdown identity, never a display label.
    pub id: String,
    pub kind: NoteLinkKind,
}

/// Distinct file-level neighbors in deterministic identity order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteLinksResult {
    pub note: String,
    pub direction: NoteLinkDirection,
    pub scanned_canonical_notes: usize,
    pub scanned_projections: usize,
    pub total_matches: usize,
    pub truncated: bool,
    pub links: Vec<NoteLink>,
}

#[derive(Debug)]
pub enum NoteLinksError {
    Validation(Box<ProjectValidationError>),
    Inspection(Box<NoteEditError>),
    PendingJournal(PathBuf),
    StaleSource(String),
}

impl NoteLinksError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Validation(error) => error.exit_code(),
            Self::Inspection(error) => error.exit_code(),
            Self::PendingJournal(_) | Self::StaleSource(_) => 5,
        }
    }
}

impl fmt::Display for NoteLinksError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(error) => error.fmt(formatter),
            Self::Inspection(error) => error.fmt(formatter),
            Self::PendingJournal(path) => write!(
                formatter,
                "pending note recovery journal at {}; reconcile and recover before inspecting links",
                path.display()
            ),
            Self::StaleSource(id) => write!(
                formatter,
                "saved source for {id:?} changed; reload before following links"
            ),
        }
    }
}

impl Error for NoteLinksError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Validation(error) => Some(error),
            Self::Inspection(error) => Some(error),
            Self::PendingJournal(_) | Self::StaleSource(_) => None,
        }
    }
}

impl From<ProjectValidationError> for NoteLinksError {
    fn from(error: ProjectValidationError) -> Self {
        Self::Validation(Box::new(error))
    }
}

/// Inspect saved sources without recovery, writes, caches or editor-draft state.
/// Coverage is all library books plus every registered project's configured index/roadmap.
/// Results are point-in-time reads; later navigation must still use a checked document load.
pub fn inspect_note_links(
    request: &ResolveRequest,
    id: &str,
    direction: NoteLinkDirection,
    limit: usize,
) -> Result<NoteLinksResult, NoteLinksError> {
    if !(1..=100).contains(&limit) {
        return Err(
            invalid_input(&request.cwd, "link result limit must be between 1 and 100").into(),
        );
    }
    let relative =
        wikilink_target_path(id).map_err(|message| invalid_input(&request.cwd, &message))?;
    if !id.ends_with(".md") || id.contains(['#', '|']) {
        return Err(
            invalid_input(&request.cwd, "note identity must be an exact full .md path").into(),
        );
    }
    let selected = resolve_project(request).map_err(ProjectValidationError::from)?;
    let config = load_root_config(&selected.root).map_err(ProjectValidationError::from)?;
    let (_, registry) =
        load_project_registry(&selected.root, &config).map_err(ProjectValidationError::from)?;
    let requests: Vec<_> = registry
        .projects
        .keys()
        .map(|project| ResolveRequest {
            root_override: Some(selected.root.clone()),
            project_override: Some(project.clone()),
            cwd: request.cwd.clone(),
            environment: request.environment.clone(),
        })
        .collect();
    refuse_pending_journals(&requests)?;
    let projection = build_library_projection(request)?;
    let mut sources = BTreeMap::new();
    for category in projection.global.categories.iter().chain(
        projection
            .projects
            .iter()
            .flat_map(|shelf| &shelf.categories),
    ) {
        for book in &category.books {
            sources.insert(book.id.clone(), NoteLinkKind::Canonical);
        }
    }
    let scanned_canonical_notes = sources.len();
    for project in registry.projects.keys() {
        let directory = selected.root.join(&config.folders.projects).join(project);
        for file in [&config.project.index, &config.project.roadmap] {
            sources.insert(
                vault_id(&selected.root, &directory.join(file))?,
                NoteLinkKind::Projection,
            );
        }
    }
    if !sources.contains_key(id) || vault_id(&selected.root, &selected.root.join(relative))? != id {
        return Err(invalid_input(
            &selected.root,
            &format!("note {id:?} is not a canonical note or configured project projection"),
        )
        .into());
    }
    let scanned_projections = sources.len() - scanned_canonical_notes;
    let mut matches = BTreeSet::new();
    for (source_id, kind) in &sources {
        let path = selected.root.join(source_id);
        let source = read_checked_source(&selected.root, &path)?;
        let body = if *kind == NoteLinkKind::Canonical {
            parse_leading_frontmatter(&source).map(|parsed| parsed.body)
        } else {
            optional_frontmatter_body(&source)
        }
        .map_err(|error| ProjectValidationError::InvalidDocument {
            path: path.clone(),
            source: Box::new(error),
        })?;
        validate_wikilinks_with_targets(&selected.root, &path, body, &BTreeSet::new())?;
        let links =
            parse_wikilinks(body).map_err(|error| ProjectValidationError::InvalidWikilink {
                path: path.clone(),
                message: error.to_string(),
            })?;
        for link in links {
            let target_id = match link.target {
                Some(target) => {
                    let target = wikilink_target_path(target).map_err(|message| {
                        ProjectValidationError::InvalidWikilink {
                            path: path.clone(),
                            message,
                        }
                    })?;
                    vault_id(&selected.root, &selected.root.join(target))?
                }
                None => source_id.clone(),
            };
            match direction {
                NoteLinkDirection::Outgoing if source_id == id => {
                    matches.insert(target_id);
                }
                NoteLinkDirection::Incoming if target_id == id => {
                    matches.insert(source_id.clone());
                }
                _ => {}
            }
        }
    }
    refuse_pending_journals(&requests)?;
    let total_matches = matches.len();
    let links = matches
        .into_iter()
        .take(limit)
        .map(|id| NoteLink {
            kind: sources.get(&id).copied().unwrap_or(NoteLinkKind::Other),
            id,
        })
        .collect();
    Ok(NoteLinksResult {
        note: id.to_owned(),
        direction,
        scanned_canonical_notes,
        scanned_projections,
        total_matches,
        truncated: total_matches > limit,
        links,
    })
}

pub(crate) fn refuse_pending_journals(requests: &[ResolveRequest]) -> Result<(), NoteLinksError> {
    for request in requests {
        let inspection = inspect_pending_note_edit(request)
            .map_err(|error| NoteLinksError::Inspection(Box::new(error)))?;
        if inspection.pending {
            return Err(NoteLinksError::PendingJournal(inspection.journal_path));
        }
    }
    Ok(())
}

pub(crate) fn read_checked_source(
    root: &Path,
    path: &Path,
) -> Result<String, ProjectValidationError> {
    let io_error = |operation, source| ProjectValidationError::FileSystem {
        operation,
        path: path.to_path_buf(),
        source,
    };
    let metadata =
        fs::symlink_metadata(path).map_err(|error| io_error("inspect graph source", error))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || fs::canonicalize(path).map_err(|error| io_error("resolve graph source", error))? != path
        || !path.starts_with(root)
    {
        return Err(invalid_input(
            path,
            "graph source must be a regular non-symlink file at its exact root-contained identity",
        ));
    }
    let bytes = fs::read(path).map_err(|error| io_error("read graph source", error))?;
    String::from_utf8(bytes).map_err(|error| ProjectValidationError::InvalidDocument {
        path: path.to_path_buf(),
        source: Box::new(ValidationError::InvalidUtf8(error.utf8_error())),
    })
}

fn invalid_input(path: &Path, message: &str) -> ProjectValidationError {
    ProjectValidationError::InvalidDocument {
        path: path.to_path_buf(),
        source: Box::new(ValidationError::InvalidSchema {
            document: "note link inspection",
            message: message.to_owned(),
        }),
    }
}
