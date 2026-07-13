use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::resolution::NoteClass;

pub(crate) const PROJECT_STATE_FILE: &str = ".akasha-state.toml";
const PROJECT_STATE_SCHEMA_VERSION: u32 = 1;
const INDEX_PROJECTION: &str = "index";
const ROADMAP_PROJECTION: &str = "roadmap";

#[derive(Debug)]
pub(crate) struct CanonicalNoteEvidence {
    pub(crate) path: PathBuf,
    pub(crate) class: NoteClass,
    pub(crate) source: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct ProjectStateValidation {
    pub(crate) immutable_events: usize,
    pub(crate) projection_sources: BTreeMap<String, usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectState {
    schema_version: u32,
    events: BTreeMap<String, String>,
    projections: BTreeMap<String, ProjectionState>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionState {
    sources: String,
    output: String,
}

pub(crate) fn render_empty_project_state() -> Vec<u8> {
    let empty_sources = source_fingerprint(std::iter::empty::<(&str, &[u8])>());
    let empty_output = content_fingerprint(&[]);
    render_project_state(&ProjectState {
        schema_version: PROJECT_STATE_SCHEMA_VERSION,
        events: BTreeMap::new(),
        projections: BTreeMap::from([
            (
                INDEX_PROJECTION.to_owned(),
                ProjectionState {
                    sources: empty_sources.clone(),
                    output: empty_output.clone(),
                },
            ),
            (
                ROADMAP_PROJECTION.to_owned(),
                ProjectionState {
                    sources: empty_sources,
                    output: empty_output,
                },
            ),
        ]),
    })
}

pub(crate) fn render_updated_project_state(
    current_source: &str,
    project_dir: &Path,
    index: &[u8],
    roadmap: &[u8],
    notes: &[CanonicalNoteEvidence],
    new_events: &BTreeSet<PathBuf>,
) -> Result<Vec<u8>, String> {
    let mut state = parse_project_state(current_source)?;
    let mut index_sources = Vec::new();
    let mut roadmap_sources = Vec::new();

    for note in notes {
        let path = project_relative_path(project_dir, &note.path)?;
        match note.class {
            NoteClass::Event if new_events.contains(&note.path) => {
                if state
                    .events
                    .insert(path.clone(), content_fingerprint(&note.source))
                    .is_some()
                {
                    return Err(format!(
                        "new event {path:?} already exists in the trusted project state"
                    ));
                }
            }
            NoteClass::Event => {}
            NoteClass::Record => roadmap_sources.push((path, note.source.as_slice())),
            NoteClass::Entity => index_sources.push((path, note.source.as_slice())),
        }
    }

    state.projections.insert(
        INDEX_PROJECTION.to_owned(),
        ProjectionState {
            sources: source_fingerprint(
                index_sources
                    .iter()
                    .map(|(path, source)| (path.as_str(), *source)),
            ),
            output: content_fingerprint(index),
        },
    );
    state.projections.insert(
        ROADMAP_PROJECTION.to_owned(),
        ProjectionState {
            sources: source_fingerprint(
                roadmap_sources
                    .iter()
                    .map(|(path, source)| (path.as_str(), *source)),
            ),
            output: content_fingerprint(roadmap),
        },
    );

    let rendered = render_project_state(&state);
    let rendered_text = std::str::from_utf8(&rendered)
        .expect("the deterministic project-state renderer always emits UTF-8");
    validate_project_state(rendered_text, project_dir, index, roadmap, notes)?;
    Ok(rendered)
}

pub(crate) fn validate_project_state(
    source: &str,
    project_dir: &Path,
    index: &[u8],
    roadmap: &[u8],
    notes: &[CanonicalNoteEvidence],
) -> Result<ProjectStateValidation, String> {
    let state = parse_project_state(source)?;
    validate_project_state_contents(&state, project_dir, index, roadmap, notes)
}

fn parse_project_state(source: &str) -> Result<ProjectState, String> {
    let state: ProjectState =
        toml::from_str(source).map_err(|error| format!("invalid project state TOML: {error}"))?;
    if state.schema_version != PROJECT_STATE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported schema_version {} in project state; expected {PROJECT_STATE_SCHEMA_VERSION}",
            state.schema_version
        ));
    }
    validate_projection_names(&state.projections)?;
    for (path, fingerprint) in &state.events {
        validate_state_path(path)?;
        validate_fingerprint(fingerprint, &format!("event {path:?}"))?;
    }
    for (name, projection) in &state.projections {
        validate_fingerprint(&projection.sources, &format!("projection {name:?} sources"))?;
        validate_fingerprint(&projection.output, &format!("projection {name:?} output"))?;
    }
    Ok(state)
}

fn validate_project_state_contents(
    state: &ProjectState,
    project_dir: &Path,
    index: &[u8],
    roadmap: &[u8],
    notes: &[CanonicalNoteEvidence],
) -> Result<ProjectStateValidation, String> {
    let mut current_events = BTreeMap::new();
    let mut index_sources = Vec::new();
    let mut roadmap_sources = Vec::new();
    for note in notes {
        let path = project_relative_path(project_dir, &note.path)?;
        match note.class {
            NoteClass::Event => {
                current_events.insert(path, content_fingerprint(&note.source));
            }
            NoteClass::Record => roadmap_sources.push((path, note.source.as_slice())),
            NoteClass::Entity => index_sources.push((path, note.source.as_slice())),
        }
    }

    for (path, fingerprint) in &current_events {
        match state.events.get(path) {
            Some(expected) if expected == fingerprint => {}
            Some(_) => {
                return Err(format!(
                    "immutable event {path:?} changed since its trusted baseline"
                ));
            }
            None => {
                return Err(format!("immutable event {path:?} has no trusted baseline"));
            }
        }
    }
    if let Some(path) = state
        .events
        .keys()
        .find(|path| !current_events.contains_key(*path))
    {
        return Err(format!(
            "project state tracks missing immutable event {path:?}"
        ));
    }

    let mut projection_sources = BTreeMap::new();
    validate_projection(
        INDEX_PROJECTION,
        &state.projections[INDEX_PROJECTION],
        &index_sources,
        index,
    )?;
    projection_sources.insert(INDEX_PROJECTION.to_owned(), index_sources.len());
    validate_projection(
        ROADMAP_PROJECTION,
        &state.projections[ROADMAP_PROJECTION],
        &roadmap_sources,
        roadmap,
    )?;
    projection_sources.insert(ROADMAP_PROJECTION.to_owned(), roadmap_sources.len());

    Ok(ProjectStateValidation {
        immutable_events: current_events.len(),
        projection_sources,
    })
}

fn render_project_state(state: &ProjectState) -> Vec<u8> {
    let mut rendered = format!("schema_version = {}\n\n[events]\n", state.schema_version);
    for (path, fingerprint) in &state.events {
        let path = serde_json::to_string(path).expect("state paths always serialize as JSON");
        let fingerprint =
            serde_json::to_string(fingerprint).expect("fingerprints always serialize as JSON");
        writeln!(rendered, "{path} = {fingerprint}")
            .expect("writing deterministic state to a string cannot fail");
    }
    for name in [INDEX_PROJECTION, ROADMAP_PROJECTION] {
        let projection = &state.projections[name];
        let sources = serde_json::to_string(&projection.sources)
            .expect("fingerprints always serialize as JSON");
        let output = serde_json::to_string(&projection.output)
            .expect("fingerprints always serialize as JSON");
        write!(
            rendered,
            "\n[projections.{name}]\nsources = {sources}\noutput = {output}\n"
        )
        .expect("writing deterministic state to a string cannot fail");
    }
    rendered.into_bytes()
}

fn validate_projection_names(
    projections: &BTreeMap<String, ProjectionState>,
) -> Result<(), String> {
    let expected = [INDEX_PROJECTION, ROADMAP_PROJECTION];
    if projections.len() == expected.len()
        && expected.iter().all(|name| projections.contains_key(*name))
    {
        Ok(())
    } else {
        Err("project state must contain exactly the index and roadmap projections".to_owned())
    }
}

fn validate_projection(
    name: &str,
    expected: &ProjectionState,
    sources: &[(String, &[u8])],
    output: &[u8],
) -> Result<(), String> {
    let current_sources = source_fingerprint(
        sources
            .iter()
            .map(|(path, source)| (path.as_str(), *source)),
    );
    if expected.sources != current_sources {
        return Err(format!(
            "projection {name:?} is stale because its canonical sources changed"
        ));
    }
    if expected.output != content_fingerprint(output) {
        return Err(format!(
            "projection {name:?} bytes differ from its trusted baseline"
        ));
    }
    Ok(())
}

fn project_relative_path(project_dir: &Path, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(project_dir).map_err(|_| {
        format!(
            "canonical note {} is outside project directory {}",
            path.display(),
            project_dir.display()
        )
    })?;
    let mut parts = Vec::new();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(format!(
                "canonical note path {} is not a normalized project-relative path",
                path.display()
            ));
        };
        let part = part.to_str().ok_or_else(|| {
            format!(
                "canonical note path {} must be valid UTF-8 for project state",
                path.display()
            )
        })?;
        parts.push(part);
    }
    let relative = parts.join("/");
    validate_state_path(&relative)?;
    Ok(relative)
}

fn validate_state_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        Err(format!(
            "project state path {path:?} must be normalized and project-relative"
        ))
    } else {
        Ok(())
    }
}

fn validate_fingerprint(fingerprint: &str, field: &str) -> Result<(), String> {
    let Some(hex) = fingerprint.strip_prefix("sha256:") else {
        return Err(format!(
            "{field} fingerprint must use the sha256:<lowercase-hex> format"
        ));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "{field} fingerprint must contain exactly 64 lowercase hexadecimal digits"
        ));
    }
    Ok(())
}

fn source_fingerprint<'a>(sources: impl Iterator<Item = (&'a str, &'a [u8])>) -> String {
    let source_hashes = sources
        .map(|(path, source)| (path, content_fingerprint(source)))
        .collect::<BTreeMap<_, _>>();
    let canonical = serde_json::to_vec(&source_hashes)
        .expect("string-keyed fingerprint maps always serialize as JSON");
    content_fingerprint(&canonical)
}

pub(crate) fn content_fingerprint(source: &[u8]) -> String {
    let digest = Sha256::digest(source);
    let mut fingerprint = String::with_capacity(71);
    fingerprint.push_str("sha256:");
    for byte in digest {
        write!(fingerprint, "{byte:02x}").expect("writing to a string cannot fail");
    }
    fingerprint
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_state_is_stable_and_self_validating() {
        let source = String::from_utf8(render_empty_project_state()).expect("state is UTF-8");
        let report = validate_project_state(&source, Path::new("project"), b"", b"", &[])
            .expect("validate empty state");

        assert_eq!(report.immutable_events, 0);
        assert_eq!(report.projection_sources[INDEX_PROJECTION], 0);
        assert_eq!(report.projection_sources[ROADMAP_PROJECTION], 0);
    }

    #[test]
    fn rejects_noncanonical_fingerprints() {
        let source = String::from_utf8(render_empty_project_state())
            .expect("state is UTF-8")
            .replace("sha256:", "SHA256:");
        let error = validate_project_state(&source, Path::new("project"), b"", b"", &[])
            .expect_err("uppercase algorithm must fail");

        assert!(error.contains("sha256:<lowercase-hex>"));
    }
}
