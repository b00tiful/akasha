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
    fingerprint: String,
}

impl CanonicalNoteEvidence {
    // State consumes exact-content hashes, so do not retain a second archive of note bodies.
    pub(crate) fn new(path: PathBuf, class: NoteClass, source: &[u8]) -> Self {
        Self {
            path,
            class,
            fingerprint: content_fingerprint(source),
        }
    }

    pub(crate) fn replace_source(&mut self, source: &[u8]) {
        self.fingerprint = content_fingerprint(source);
    }
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
    let empty_sources = source_fingerprint(std::iter::empty::<(&str, &str)>());
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
                    .insert(path.clone(), note.fingerprint.clone())
                    .is_some()
                {
                    return Err(format!(
                        "new event {path:?} already exists in the trusted project state"
                    ));
                }
            }
            NoteClass::Event => {}
            NoteClass::Record => roadmap_sources.push((path, note.fingerprint.as_str())),
            NoteClass::Entity => index_sources.push((path, note.fingerprint.as_str())),
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
                current_events.insert(path, note.fingerprint.clone());
            }
            NoteClass::Record => roadmap_sources.push((path, note.fingerprint.as_str())),
            NoteClass::Entity => index_sources.push((path, note.fingerprint.as_str())),
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
    sources: &[(String, &str)],
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

fn source_fingerprint<'a>(sources: impl Iterator<Item = (&'a str, &'a str)>) -> String {
    let source_hashes = sources.collect::<BTreeMap<_, _>>();
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

    // Frozen byte-based source-set algorithm: do not replace this with the compact helper.
    fn byte_source_fingerprint<'a>(sources: impl Iterator<Item = (&'a str, &'a [u8])>) -> String {
        let hashes = sources
            .map(|(path, source)| (path, content_fingerprint(source)))
            .collect::<BTreeMap<_, _>>();
        content_fingerprint(&serde_json::to_vec(&hashes).unwrap())
    }

    fn byte_state(notes: &[(&str, NoteClass, &[u8])], index: &[u8], roadmap: &[u8]) -> Vec<u8> {
        let projection = |class, output| ProjectionState {
            sources: byte_source_fingerprint(
                notes
                    .iter()
                    .filter(|(_, candidate, _)| *candidate == class)
                    .map(|(path, _, source)| (*path, *source)),
            ),
            output: content_fingerprint(output),
        };
        render_project_state(&ProjectState {
            schema_version: 1,
            events: notes
                .iter()
                .filter(|(_, class, _)| *class == NoteClass::Event)
                .map(|(path, _, source)| (path.to_string(), content_fingerprint(source)))
                .collect(),
            projections: BTreeMap::from([
                (
                    INDEX_PROJECTION.to_owned(),
                    projection(NoteClass::Entity, index),
                ),
                (
                    ROADMAP_PROJECTION.to_owned(),
                    projection(NoteClass::Record, roadmap),
                ),
            ]),
        })
    }

    #[test]
    fn compact_evidence_preserves_exact_byte_state_and_ordering() {
        let project = Path::new("project");
        let index = "# Index ☃\r\n".as_bytes();
        let roadmap = b"# Roadmap\n";
        let notes = [
            (
                "renamed/events/雪.md",
                NoteClass::Event,
                b"\x00\xff\r\n".as_slice(),
            ),
            (
                "renamed/entities/z.md",
                NoteClass::Entity,
                "☃\r\n".as_bytes(),
            ),
            ("renamed/records/a.md", NoteClass::Record, b"".as_slice()),
            (
                "renamed/entities/quote\"\t.md",
                NoteClass::Entity,
                b"a\nb\n".as_slice(),
            ),
            ("renamed/events/empty.md", NoteClass::Event, b"".as_slice()),
        ];
        let expected = byte_state(&notes, index, roadmap);
        let evidence = notes
            .iter()
            .rev()
            .map(|(path, class, source)| {
                CanonicalNoteEvidence::new(project.join(path), *class, source)
            })
            .collect::<Vec<_>>();
        let events = evidence
            .iter()
            .filter(|note| note.class == NoteClass::Event)
            .map(|note| note.path.clone())
            .collect();
        let actual = render_updated_project_state(
            std::str::from_utf8(&render_empty_project_state()).unwrap(),
            project,
            index,
            roadmap,
            &evidence,
            &events,
        )
        .unwrap();
        assert_eq!(actual, expected);
        let checked = validate_project_state(
            std::str::from_utf8(&expected).unwrap(),
            project,
            index,
            roadmap,
            &evidence,
        )
        .unwrap();
        assert_eq!(checked.immutable_events, 2);
        assert_eq!(checked.projection_sources[INDEX_PROJECTION], 2);
        assert_eq!(checked.projection_sources[ROADMAP_PROJECTION], 1);
    }

    #[test]
    fn compact_evidence_matches_seeded_byte_source_sets() {
        let mut seed = 0x7db3_291e_u64;
        for count in 0..128 {
            let sources = (0..count)
                .map(|number| {
                    let bytes = (0..number * 3)
                        .map(|_| {
                            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                            (seed >> 32) as u8
                        })
                        .collect::<Vec<_>>();
                    // Repeated identities also preserve the former last-value-wins map behavior.
                    (format!("renamed/雪-{}.md", number % 17), bytes)
                })
                .collect::<Vec<_>>();
            let evidence = sources
                .iter()
                .map(|(path, bytes)| {
                    CanonicalNoteEvidence::new(PathBuf::from(path), NoteClass::Entity, bytes)
                })
                .collect::<Vec<_>>();
            assert_eq!(
                source_fingerprint(
                    sources
                        .iter()
                        .zip(&evidence)
                        .map(|((path, _), note)| { (path.as_str(), note.fingerprint.as_str()) })
                ),
                byte_source_fingerprint(
                    sources
                        .iter()
                        .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
                ),
                "source set of {count} entries",
            );
        }
    }

    #[test]
    fn compact_evidence_retains_original_bytes_and_replaces_candidates_explicitly() {
        let project = Path::new("project");
        let path = "entities/雪.md";
        let mut source = vec![b'a'; 2 * 1024 * 1024];
        let before = byte_state(&[(path, NoteClass::Entity, &source)], b"", b"");
        let mut note = CanonicalNoteEvidence::new(project.join(path), NoteClass::Entity, &source);
        assert_eq!(note.fingerprint.len(), 71);
        assert_eq!(note.fingerprint.capacity(), 71);
        source[1_048_576] = b'b'; // Same length, distinct exact-byte candidate.
        let original = std::str::from_utf8(&before).unwrap();
        validate_project_state(original, project, b"", b"", std::slice::from_ref(&note)).unwrap();
        note.replace_source(&source);
        assert!(
            validate_project_state(original, project, b"", b"", std::slice::from_ref(&note))
                .unwrap_err()
                .contains("canonical sources changed")
        );
        let expected = byte_state(&[(path, NoteClass::Entity, &source)], b"", b"");
        assert_eq!(
            render_updated_project_state(original, project, b"", b"", &[note], &BTreeSet::new(),)
                .unwrap(),
            expected
        );
    }

    #[test]
    fn compact_evidence_preserves_event_and_projection_error_precedence() {
        let project = Path::new("project");
        let notes = [
            ("events/a.md", NoteClass::Event, b"before".as_slice()),
            ("entities/z.md", NoteClass::Entity, b"entity".as_slice()),
        ];
        let state = String::from_utf8(byte_state(&notes, b"index", b"roadmap")).unwrap();
        let mut evidence = notes
            .iter()
            .map(|(path, class, source)| {
                CanonicalNoteEvidence::new(project.join(path), *class, source)
            })
            .collect::<Vec<_>>();
        evidence[0].replace_source(b"after!");
        evidence[1].replace_source(b"ENTITY");
        let error =
            validate_project_state(&state, project, b"changed index", b"roadmap", &evidence)
                .unwrap_err();
        assert_eq!(
            error,
            "immutable event \"events/a.md\" changed since its trusted baseline"
        );
        evidence[0].replace_source(b"before");
        let error =
            validate_project_state(&state, project, b"changed index", b"roadmap", &evidence)
                .unwrap_err();
        assert_eq!(
            error,
            "projection \"index\" is stale because its canonical sources changed"
        );
        evidence[1].replace_source(b"entity");
        let error =
            validate_project_state(&state, project, b"changed index", b"roadmap", &evidence)
                .unwrap_err();
        assert_eq!(
            error,
            "projection \"index\" bytes differ from its trusted baseline"
        );
        evidence.remove(0);
        assert!(
            validate_project_state(&state, project, b"index", b"roadmap", &evidence)
                .unwrap_err()
                .contains("tracks missing immutable event")
        );
        evidence.push(CanonicalNoteEvidence::new(
            project.join("events/new.md"),
            NoteClass::Event,
            b"new",
        ));
        assert!(
            validate_project_state(&state, project, b"index", b"roadmap", &evidence)
                .unwrap_err()
                .contains("has no trusted baseline")
        );
    }
}
