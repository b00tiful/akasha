use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::project_validation::{
    ProjectValidationError, canonical_note_paths, validate_project, validate_wikilinks_with_targets,
};
use crate::resolution::{
    NoteClass, ResolveRequest, load_project_registry, load_root_config, resolve_project,
};
use crate::validation::{
    ParsedNote, ValidationError, parse_leading_frontmatter_bytes, validate_configured_note,
    validate_global_configured_note,
};

/// Storage scope for one canonical book identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum LibraryScope {
    Global,
    Project { project: String },
}

/// One canonical note represented as a renderer-neutral book.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryBook {
    /// Stable full vault-relative Markdown path using `/` separators.
    pub id: String,
    pub label: String,
    pub scope: LibraryScope,
    pub note_type: String,
    pub class: NoteClass,
    pub status: Option<String>,
    pub reviewed: Option<String>,
    pub date: Option<String>,
    /// Exact textual fallback for visual inspection.
    pub explanation: String,
}

/// One configured note-type row in a project shelf or the global collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryCategory {
    pub note_type: String,
    pub class: NoteClass,
    pub books: Vec<LibraryBook>,
}

/// One registered project represented as a stable shelf.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryShelf {
    pub project: String,
    pub status: String,
    pub categories: Vec<LibraryCategory>,
}

/// Shared maintained knowledge. Version 1 accepts only configured entity note types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryCollection {
    pub categories: Vec<LibraryCategory>,
}

/// Deterministic read-only projection consumed by future CLI and desktop adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryProjection {
    pub root: PathBuf,
    pub selected_project: String,
    pub global: LibraryCollection,
    pub projects: Vec<LibraryShelf>,
    pub total_books: usize,
}

/// Validate every registered project and project the shared library from canonical notes.
pub fn build_library_projection(
    request: &ResolveRequest,
) -> Result<LibraryProjection, ProjectValidationError> {
    let selected = resolve_project(request)?;
    let config = load_root_config(&selected.root)?;
    let (_, registry) = load_project_registry(&selected.root, &config)?;

    let mut projects = Vec::with_capacity(registry.projects.len());
    let mut total_books = 0;
    for (project, entry) in &registry.projects {
        let project_request = ResolveRequest {
            root_override: Some(selected.root.clone()),
            project_override: Some(project.clone()),
            cwd: request.cwd.clone(),
            environment: request.environment.clone(),
        };
        let report = validate_project(&project_request)?;
        let mut categories = Vec::with_capacity(config.project.note_types.len());
        for (note_type, note_config) in &config.project.note_types {
            let directory = report.project_dir.join(&note_config.folder);
            let mut books = Vec::new();
            for path in canonical_note_paths(&directory)? {
                let source = read_note(&path)?;
                let parsed = parse_note(&path, &source)?;
                validate_configured_note(&parsed, project, note_type, &note_config.required_fields)
                    .map_err(|source| invalid_note(&path, source))?;
                books.push(project_book(
                    &selected.root,
                    &path,
                    &parsed,
                    project,
                    note_type,
                    note_config.class,
                )?);
            }
            total_books += books.len();
            categories.push(LibraryCategory {
                note_type: note_type.clone(),
                class: note_config.class,
                books,
            });
        }
        projects.push(LibraryShelf {
            project: project.clone(),
            status: entry.status.clone(),
            categories,
        });
    }

    let global_directory = selected.root.join(&config.folders.global);
    let mut global_categories = config
        .project
        .note_types
        .iter()
        .filter(|(_, note_type)| note_type.class == NoteClass::Entity)
        .map(|(name, note_type)| {
            (
                name.clone(),
                LibraryCategory {
                    note_type: name.clone(),
                    class: note_type.class,
                    books: Vec::new(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    for path in canonical_note_paths(&global_directory)? {
        let relative = path.strip_prefix(&global_directory).map_err(|_| {
            invalid_library_layout(&path, "global note escapes the configured global folder")
        })?;
        let (note_type, note_config) = config
            .project
            .note_types
            .iter()
            .find(|(_, candidate)| {
                candidate.class == NoteClass::Entity && relative.starts_with(&candidate.folder)
            })
            .ok_or_else(|| {
                invalid_library_layout(
                    &path,
                    "global notes must use a configured entity note-type folder",
                )
            })?;
        let source = read_note(&path)?;
        let parsed = parse_note(&path, &source)?;
        validate_global_configured_note(&parsed, note_type, &note_config.required_fields)
            .map_err(|source| invalid_note(&path, source))?;
        validate_wikilinks_with_targets(&selected.root, &path, parsed.body, &Default::default())?;
        let book = global_book(&selected.root, &path, &parsed, note_type, note_config.class)?;
        global_categories
            .get_mut(note_type)
            .expect("configured global entity category exists")
            .books
            .push(book);
        total_books += 1;
    }

    Ok(LibraryProjection {
        root: selected.root,
        selected_project: selected.project,
        global: LibraryCollection {
            categories: global_categories.into_values().collect(),
        },
        projects,
        total_books,
    })
}

/// Render the exact hierarchy as a keyboard- and screen-reader-friendly Markdown fallback.
#[must_use]
pub fn render_library_markdown(projection: &LibraryProjection) -> String {
    let mut output = format!(
        "# Akasha Library\n\nSelected project: `{}`\n\nBooks: {}\n\n## Global knowledge\n",
        projection.selected_project, projection.total_books
    );
    render_categories(&mut output, &projection.global.categories, 3);
    output.push_str("\n## Project shelves\n");
    for shelf in &projection.projects {
        output.push_str(&format!("\n### `{}` — {}\n", shelf.project, shelf.status));
        render_categories(&mut output, &shelf.categories, 4);
    }
    output
}

fn render_categories(output: &mut String, categories: &[LibraryCategory], heading_level: usize) {
    let heading = "#".repeat(heading_level);
    for category in categories {
        output.push_str(&format!(
            "\n{heading} `{}` ({})\n",
            category.note_type,
            class_name(category.class)
        ));
        if category.books.is_empty() {
            output.push_str("\n- No books\n");
            continue;
        }
        for book in &category.books {
            output.push_str(&format!("\n- `{}` — {}\n", book.id, book.explanation));
        }
    }
}

fn project_book(
    root: &Path,
    path: &Path,
    parsed: &ParsedNote<'_>,
    project: &str,
    note_type: &str,
    class: NoteClass,
) -> Result<LibraryBook, ProjectValidationError> {
    let id = vault_id(root, path)?;
    let label = note_label(path, parsed)?;
    let status = metadata_string(parsed, "status");
    let reviewed = metadata_string(parsed, "reviewed");
    let date = metadata_string(parsed, "date");
    let mut explanation = format!(
        "{label}; project {project}; {note_type} {}; canonical source {id}",
        class_name(class)
    );
    append_lifecycle_explanation(
        &mut explanation,
        status.as_deref(),
        reviewed.as_deref(),
        date.as_deref(),
    );
    Ok(LibraryBook {
        id,
        label,
        scope: LibraryScope::Project {
            project: project.to_owned(),
        },
        note_type: note_type.to_owned(),
        class,
        status,
        reviewed,
        date,
        explanation,
    })
}

fn global_book(
    root: &Path,
    path: &Path,
    parsed: &ParsedNote<'_>,
    note_type: &str,
    class: NoteClass,
) -> Result<LibraryBook, ProjectValidationError> {
    let id = vault_id(root, path)?;
    let label = note_label(path, parsed)?;
    let status = metadata_string(parsed, "status");
    let reviewed = metadata_string(parsed, "reviewed");
    let date = metadata_string(parsed, "date");
    let mut explanation = format!(
        "{label}; global {note_type} {}; canonical source {id}",
        class_name(class)
    );
    append_lifecycle_explanation(
        &mut explanation,
        status.as_deref(),
        reviewed.as_deref(),
        date.as_deref(),
    );
    Ok(LibraryBook {
        id,
        label,
        scope: LibraryScope::Global,
        note_type: note_type.to_owned(),
        class,
        status,
        reviewed,
        date,
        explanation,
    })
}

fn append_lifecycle_explanation(
    explanation: &mut String,
    status: Option<&str>,
    reviewed: Option<&str>,
    date: Option<&str>,
) {
    for (name, value) in [("status", status), ("reviewed", reviewed), ("date", date)] {
        if let Some(value) = value {
            explanation.push_str(&format!("; {name} {value}"));
        }
    }
}

fn read_note(path: &Path) -> Result<Vec<u8>, ProjectValidationError> {
    fs::read(path).map_err(|source| ProjectValidationError::FileSystem {
        operation: "read library note",
        path: path.to_path_buf(),
        source,
    })
}

fn parse_note<'a>(path: &Path, source: &'a [u8]) -> Result<ParsedNote<'a>, ProjectValidationError> {
    parse_leading_frontmatter_bytes(source).map_err(|source| invalid_note(path, source))
}

fn invalid_note(path: &Path, source: ValidationError) -> ProjectValidationError {
    ProjectValidationError::InvalidDocument {
        path: path.to_path_buf(),
        source: Box::new(source),
    }
}

fn invalid_library_layout(path: &Path, message: &str) -> ProjectValidationError {
    invalid_note(
        path,
        ValidationError::InvalidSchema {
            document: "global library layout",
            message: message.to_owned(),
        },
    )
}

fn vault_id(root: &Path, path: &Path) -> Result<String, ProjectValidationError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| invalid_library_layout(path, "canonical note escapes the data root"))?;
    let mut parts = Vec::new();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(invalid_library_layout(
                path,
                "canonical note identity contains a non-normal path component",
            ));
        };
        parts.push(part.to_str().ok_or_else(|| {
            invalid_library_layout(path, "canonical note identity must be valid UTF-8")
        })?);
    }
    Ok(parts.join("/"))
}

fn note_label(path: &Path, parsed: &ParsedNote<'_>) -> Result<String, ProjectValidationError> {
    for field in ["title", "entity"] {
        if let Some(value) = metadata_string(parsed, field)
            && !value.trim().is_empty()
        {
            return Ok(value);
        }
    }
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .ok_or_else(|| invalid_library_layout(path, "book label must be valid UTF-8"))
}

fn metadata_string(parsed: &ParsedNote<'_>, field: &str) -> Option<String> {
    parsed
        .metadata
        .as_object()
        .and_then(|mapping| mapping.get(field))
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

const fn class_name(class: NoteClass) -> &'static str {
    match class {
        NoteClass::Event => "event",
        NoteClass::Record => "record",
        NoteClass::Entity => "entity",
    }
}
