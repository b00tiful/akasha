//! Occurrence-bound navigation over exact saved canonical documents.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::library::{
    LibraryDocument, LibraryProjection, build_library_projection, document_from_projection,
    vault_id,
};
use crate::note_links::{NoteLinkKind, NoteLinksError, refuse_pending_journals};
use crate::project_validation::{ProjectValidationError, wikilink_target_path};
use crate::resolution::{ResolveRequest, load_project_registry, load_root_config, resolve_project};
use crate::validation::{ValidationError, parse_leading_frontmatter};
use crate::wikilink::parse_wikilinks;

/// UTF-16 ranges in the exact full source, suitable for DOM/CodeMirror display bindings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryDocumentLink {
    pub from: usize,
    pub to: usize,
    pub label_from: usize,
    pub label_to: usize,
    pub target_id: String,
    pub kind: NoteLinkKind,
}

/// Additive navigation view; the existing `LibraryDocument` wire remains unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkedLibraryDocument {
    #[serde(flatten)]
    pub document: LibraryDocument,
    pub links: Vec<LibraryDocumentLink>,
}

/// Load a projected book with bindings from the shared parser. Never recover or write.
pub fn load_linked_library_document(
    request: &ResolveRequest,
    id: &str,
) -> Result<LinkedLibraryDocument, NoteLinksError> {
    let (projection, requests) = checked_projection(request)?;
    let document = linked_document(&projection, id)?;
    refuse_pending_journals(&requests)?;
    Ok(document)
}

/// Recheck the loaded source and occurrence before loading its canonical target.
/// The caller supplies an occurrence start, never a raw target or alias.
pub fn activate_library_document_link(
    request: &ResolveRequest,
    id: &str,
    expected_source: &str,
    from: usize,
) -> Result<LinkedLibraryDocument, NoteLinksError> {
    let (projection, requests) = checked_projection(request)?;
    let source = linked_document(&projection, id)?;
    if source.document.source != expected_source {
        return Err(NoteLinksError::StaleSource(id.to_owned()));
    }
    let occurrence = source
        .links
        .iter()
        .find(|link| link.from == from)
        .ok_or_else(|| {
            invalid_link(
                id,
                "the requested link occurrence is absent from the saved source",
            )
        })?;
    if occurrence.kind != NoteLinkKind::Canonical {
        return Err(
            invalid_link(id, "the linked target is not a canonical library document").into(),
        );
    }
    let target = linked_document(&projection, &occurrence.target_id)?;
    // Recheck source after reading the target; this is not an atomic filesystem snapshot.
    if document_from_projection(&projection, id)?.source != expected_source {
        return Err(NoteLinksError::StaleSource(id.to_owned()));
    }
    refuse_pending_journals(&requests)?;
    Ok(target)
}

fn checked_projection(
    request: &ResolveRequest,
) -> Result<(LibraryProjection, Vec<ResolveRequest>), NoteLinksError> {
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
    Ok((build_library_projection(request)?, requests))
}

fn linked_document(
    projection: &LibraryProjection,
    id: &str,
) -> Result<LinkedLibraryDocument, NoteLinksError> {
    let document = document_from_projection(projection, id)?;
    let source = &document.source;
    let parsed = parse_leading_frontmatter(source).map_err(|error| {
        ProjectValidationError::InvalidDocument {
            path: projection.root.join(id),
            source: Box::new(error),
        }
    })?;
    let body_offset = source.len() - parsed.body.len();
    let config = load_root_config(&projection.root).map_err(ProjectValidationError::from)?;
    let canonical: BTreeSet<_> = projection
        .global
        .categories
        .iter()
        .chain(
            projection
                .projects
                .iter()
                .flat_map(|shelf| &shelf.categories),
        )
        .flat_map(|category| &category.books)
        .map(|book| book.id.as_str())
        .collect();
    let projections: BTreeSet<_> = projection
        .projects
        .iter()
        .flat_map(|shelf| {
            [&config.project.index, &config.project.roadmap].map(|file| {
                vault_id(
                    &projection.root,
                    &projection
                        .root
                        .join(&config.folders.projects)
                        .join(&shelf.project)
                        .join(file),
                )
            })
        })
        .collect::<Result<_, _>>()?;
    let occurrences =
        parse_wikilinks(parsed.body).map_err(|error| ProjectValidationError::InvalidWikilink {
            path: projection.root.join(id),
            message: error.to_string(),
        })?;
    let mut links = Vec::with_capacity(occurrences.len());
    let mut previous_byte = 0;
    let mut previous_units = 0;
    for occurrence in occurrences {
        let target_id = match occurrence.target {
            Some(target) => {
                let target = wikilink_target_path(target).map_err(|message| {
                    ProjectValidationError::InvalidWikilink {
                        path: projection.root.join(id),
                        message,
                    }
                })?;
                vault_id(&projection.root, &projection.root.join(target))?
            }
            None => id.to_owned(),
        };
        let kind = if canonical.contains(target_id.as_str()) {
            NoteLinkKind::Canonical
        } else if projections.contains(&target_id) {
            NoteLinkKind::Projection
        } else {
            NoteLinkKind::Other
        };
        let mut units = [0; 4];
        for (index, byte) in [
            occurrence.from,
            occurrence.label_from,
            occurrence.label_to,
            occurrence.to,
        ]
        .into_iter()
        .enumerate()
        {
            let byte = body_offset + byte;
            previous_units += source[previous_byte..byte].encode_utf16().count();
            previous_byte = byte;
            units[index] = previous_units;
        }
        links.push(LibraryDocumentLink {
            from: units[0],
            label_from: units[1],
            label_to: units[2],
            to: units[3],
            target_id,
            kind,
        });
    }
    Ok(LinkedLibraryDocument { document, links })
}

fn invalid_link(id: &str, message: &str) -> ProjectValidationError {
    ProjectValidationError::InvalidDocument {
        path: id.into(),
        source: Box::new(ValidationError::InvalidSchema {
            document: "document link activation",
            message: message.to_owned(),
        }),
    }
}
