use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::note_edit::{NoteEditError, recover_note_mutation_locked};
use crate::project_validation::{
    ProjectValidationError, canonical_note_paths, validate_project, validate_wikilinks_with_targets,
};
use crate::resolution::{
    NoteClass, ResolveError, ResolveRequest, RootConfig, load_root_config, resolve_project,
};
use crate::state::{
    CanonicalNoteEvidence, PROJECT_STATE_FILE, content_fingerprint, render_updated_project_state,
    validate_project_state,
};
use crate::validation::{parse_leading_frontmatter_bytes, validate_configured_note};
use crate::writes::{
    AtomicCreateError, CheckedReplaceError, ProjectWriteLock, create_file_atomically,
    replace_file_if_unchanged as replace_checked_file,
};

pub const MAX_ONBOARDING_NOTES: usize = 64;
pub const MAX_ONBOARDING_NOTE_CHARS: usize = 65_536;
pub const MAX_ONBOARDING_PROJECTION_CHARS: usize = 131_072;
pub const MAX_ONBOARDING_PROPOSAL_CHARS: usize = 524_288;
pub const MAX_ONBOARDING_EVIDENCE_CLAIMS: usize = 128;
pub const MAX_ONBOARDING_EVIDENCE_SOURCES: usize = 16;
pub const MAX_ONBOARDING_TEMPLATE_CHARS: usize = 24_000;
pub const MAX_ONBOARDING_INVENTORY_ENTRIES: usize = 512;

const ONBOARDING_PROPOSAL_DOMAIN: &[u8] = b"akasha-onboarding-proposal-v1";
const ONBOARDING_PREVIEW_DOMAIN: &[u8] = b"akasha-onboarding-preview-v1";
const COVERAGE_CRITERIA: [&str; 6] = [
    "stack and development environment",
    "durable components and their interfaces",
    "accepted architecture decisions and constraints",
    "workflows, commands, tests, and operational boundaries",
    "current tasks, problems, blockers, and unknowns",
    "index and roadmap links for every proposed canonical note",
];

/// One bounded UTF-8 project template returned to an onboarding agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OnboardingTemplate {
    pub path: PathBuf,
    pub source: String,
}

/// One configured canonical note type available to onboarding proposals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OnboardingNoteType {
    pub class: NoteClass,
    pub folder: PathBuf,
    pub required_fields: Vec<String>,
}

/// One existing canonical note identity, without its source body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OnboardingInventoryEntry {
    pub note_type: String,
    pub path: PathBuf,
}

/// Bounded project-specific instructions for an external onboarding agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OnboardingPreparation {
    pub root: PathBuf,
    pub project: String,
    pub repository_dir: PathBuf,
    pub project_dir: PathBuf,
    pub note_types: BTreeMap<String, OnboardingNoteType>,
    pub templates: Vec<OnboardingTemplate>,
    pub omitted_templates: usize,
    pub template_characters: usize,
    pub existing_notes: Vec<OnboardingInventoryEntry>,
    pub omitted_existing_notes: usize,
    pub coverage_criteria: Vec<String>,
    pub evidence_contract: String,
}

/// What a validated proposal would do to one canonical note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OnboardingNoteAction {
    Create,
    Unchanged,
}

/// One note in a validated onboarding preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OnboardingNotePreview {
    pub note_type: String,
    pub path: PathBuf,
    pub action: OnboardingNoteAction,
    pub evidence_claims: usize,
}

/// A deterministic proposal summary bound to the current validated project snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OnboardingBatchPreview {
    pub root: PathBuf,
    pub project: String,
    pub project_dir: PathBuf,
    pub proposal_id: String,
    pub preview_id: String,
    pub notes: Vec<OnboardingNotePreview>,
    pub index_changed: bool,
    pub roadmap_changed: bool,
    pub state_changed: bool,
}

/// One exact canonical note proposed for create-only onboarding application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedNote {
    pub note_type: String,
    /// Path relative to the configured folder for `note_type`.
    pub path: PathBuf,
    pub source: String,
}

/// A reviewed create-only note batch plus exact accepted projection outputs.
#[derive(Debug, Clone)]
pub struct OnboardingBatchRequest {
    pub resolution: ResolveRequest,
    pub notes: Vec<ProposedNote>,
    pub index: String,
    pub roadmap: String,
}

/// Exact paths affected by a successful onboarding batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OnboardingBatchResult {
    pub root: PathBuf,
    pub project: String,
    pub project_dir: PathBuf,
    pub created_notes: Vec<PathBuf>,
    pub unchanged_notes: Vec<PathBuf>,
    pub updated_projections: Vec<PathBuf>,
    pub state: PathBuf,
}

/// A resolution, validation, conflict, filesystem, or rollback failure.
#[derive(Debug)]
pub enum OnboardingBatchError {
    Resolve(Box<ResolveError>),
    Project(Box<ProjectValidationError>),
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
    Cleanup {
        committed: bool,
        original: Option<Box<OnboardingBatchError>>,
        failures: Vec<String>,
    },
}

impl OnboardingBatchError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Resolve(error) => error.exit_code(),
            Self::Project(error) => error.exit_code(),
            Self::Mutation(error) => error.exit_code(),
            Self::Validation { .. } => 4,
            Self::Conflict { .. } => 5,
            Self::Creation(error) => error.exit_code(),
            Self::FileSystem { .. } | Self::Cleanup { .. } => 6,
        }
    }
}

impl fmt::Display for OnboardingBatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolve(error) => error.fmt(formatter),
            Self::Project(error) => error.fmt(formatter),
            Self::Mutation(error) => error.fmt(formatter),
            Self::Validation { path, message } => {
                write!(
                    formatter,
                    "invalid onboarding proposal at {}: {message}",
                    path.display()
                )
            }
            Self::Conflict { path, message } => {
                write!(
                    formatter,
                    "onboarding conflict at {}: {message}",
                    path.display()
                )
            }
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
            Self::Cleanup {
                committed,
                original,
                failures,
            } => {
                if *committed {
                    formatter.write_str("onboarding batch committed, but lock cleanup failed")?;
                } else {
                    formatter.write_str("onboarding batch failed and rollback was incomplete")?;
                }
                if let Some(original) = original {
                    write!(formatter, ": original error: {original}")?;
                }
                write!(formatter, "; residue: {}", failures.join("; "))
            }
        }
    }
}

impl Error for OnboardingBatchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Resolve(error) => Some(error.as_ref()),
            Self::Project(error) => Some(error.as_ref()),
            Self::Mutation(error) => Some(error.as_ref()),
            Self::Creation(error) => Some(error),
            Self::FileSystem { source, .. } => Some(source),
            Self::Cleanup {
                original: Some(error),
                ..
            } => Some(error.as_ref()),
            Self::Validation { .. }
            | Self::Conflict { .. }
            | Self::Cleanup { original: None, .. } => None,
        }
    }
}

impl From<ResolveError> for OnboardingBatchError {
    fn from(error: ResolveError) -> Self {
        Self::Resolve(Box::new(error))
    }
}

impl From<ProjectValidationError> for OnboardingBatchError {
    fn from(error: ProjectValidationError) -> Self {
        Self::Project(Box::new(error))
    }
}

impl From<NoteEditError> for OnboardingBatchError {
    fn from(error: NoteEditError) -> Self {
        Self::Mutation(Box::new(error))
    }
}

impl From<AtomicCreateError> for OnboardingBatchError {
    fn from(error: AtomicCreateError) -> Self {
        Self::Creation(error)
    }
}

/// Return bounded templates, schemas, coverage rules, and note identities for one project.
pub fn prepare_onboarding(
    request: &ResolveRequest,
) -> Result<OnboardingPreparation, OnboardingBatchError> {
    let report = validate_project(request)?;
    let config = load_root_config(&report.root)?;
    let templates_dir = report.project_dir.join(&config.project.templates);
    let (templates, omitted_templates, template_characters) = collect_templates(&templates_dir)?;

    let mut all_existing_notes = Vec::new();
    let mut note_types = BTreeMap::new();
    for (name, note_type) in &config.project.note_types {
        note_types.insert(
            name.clone(),
            OnboardingNoteType {
                class: note_type.class,
                folder: note_type.folder.clone(),
                required_fields: note_type.required_fields.clone(),
            },
        );
        let folder = report.project_dir.join(&note_type.folder);
        for path in canonical_note_paths(&folder)? {
            all_existing_notes.push(OnboardingInventoryEntry {
                note_type: name.clone(),
                path: path
                    .strip_prefix(&folder)
                    .expect("canonical notes are collected below their configured folder")
                    .to_path_buf(),
            });
        }
    }
    all_existing_notes
        .sort_by(|left, right| (&left.note_type, &left.path).cmp(&(&right.note_type, &right.path)));
    let omitted_existing_notes = all_existing_notes
        .len()
        .saturating_sub(MAX_ONBOARDING_INVENTORY_ENTRIES);
    all_existing_notes.truncate(MAX_ONBOARDING_INVENTORY_ENTRIES);

    Ok(OnboardingPreparation {
        root: report.root,
        project: report.project,
        repository_dir: report.repository_dir,
        project_dir: report.project_dir,
        note_types,
        templates,
        omitted_templates,
        template_characters,
        existing_notes: all_existing_notes,
        omitted_existing_notes,
        coverage_criteria: COVERAGE_CRITERIA
            .iter()
            .map(|criterion| (*criterion).to_owned())
            .collect(),
        evidence_contract: "Every proposed note must contain a non-empty top-level frontmatter \
            `evidence` list. Each entry has `kind` (`fact`, `inference`, or `unknown`) and a \
            non-empty `claim`. Facts require one or more `sources`; inferences additionally \
            require a non-empty `rationale`; unknowns require a rationale and no sources. Each \
            source has a normalized repository-relative `path`, the exact whole-file \
            `fingerprint` (`sha256:<64 lowercase hex>`), and either both positive `line_start` \
            and `line_end` or neither."
            .to_owned(),
    })
}

/// Validate a bounded, source-attributed proposal and bind it to the current project snapshot.
pub fn preview_onboarding_batch(
    request: &OnboardingBatchRequest,
) -> Result<OnboardingBatchPreview, OnboardingBatchError> {
    let prepared = prepare_batch(request, None, EvidencePolicy::Required)?;
    Ok(render_preview(request, &prepared))
}

/// Apply a proposal only when it still exactly matches a previously returned preview identifier.
pub fn apply_approved_onboarding_batch(
    request: &OnboardingBatchRequest,
    approved_preview_id: &str,
) -> Result<OnboardingBatchResult, OnboardingBatchError> {
    let resolved = resolve_project(&request.resolution)?;
    let _lock = ProjectWriteLock::acquire(&resolved.project_dir)?;
    recover_note_mutation_locked(&request.resolution, &resolved.project_dir)?;
    let prepared = prepare_batch(
        request,
        Some(&resolved.project_dir),
        EvidencePolicy::Required,
    )?;
    let current_preview = render_preview(request, &prepared);
    if approved_preview_id != current_preview.preview_id {
        return Err(OnboardingBatchError::Conflict {
            path: prepared.state_path.clone(),
            message: "approved preview does not match the current proposal and project state"
                .to_owned(),
        });
    }
    apply_prepared(request, prepared, || {})
}

/// Apply one reviewed, create-only onboarding proposal through the shared core.
///
/// Exact existing note bytes are rerun no-ops. Differing existing bytes conflict. Index and
/// roadmap are the only replaceable human-visible files, and project state is published last.
pub fn apply_onboarding_batch(
    request: &OnboardingBatchRequest,
) -> Result<OnboardingBatchResult, OnboardingBatchError> {
    apply_onboarding_batch_with_hook(request, || {})
}

fn apply_onboarding_batch_with_hook(
    request: &OnboardingBatchRequest,
    after_note_creation: impl FnOnce(),
) -> Result<OnboardingBatchResult, OnboardingBatchError> {
    if request.notes.is_empty() {
        return Err(OnboardingBatchError::Validation {
            path: PathBuf::from("<proposal>"),
            message: "an onboarding batch must contain at least one canonical note".to_owned(),
        });
    }

    let resolved = resolve_project(&request.resolution)?;
    let _lock = ProjectWriteLock::acquire(&resolved.project_dir)?;
    recover_note_mutation_locked(&request.resolution, &resolved.project_dir)?;
    apply_locked(request, &resolved.project_dir, after_note_creation)
}

fn apply_locked(
    request: &OnboardingBatchRequest,
    locked_project_dir: &Path,
    after_note_creation: impl FnOnce(),
) -> Result<OnboardingBatchResult, OnboardingBatchError> {
    let prepared = prepare_batch(request, Some(locked_project_dir), EvidencePolicy::Unchecked)?;
    apply_prepared(request, prepared, after_note_creation)
}

fn apply_prepared(
    request: &OnboardingBatchRequest,
    prepared: PreparedBatch,
    after_note_creation: impl FnOnce(),
) -> Result<OnboardingBatchResult, OnboardingBatchError> {
    let PreparedBatch {
        root,
        project,
        project_dir,
        notes,
        index_path,
        roadmap_path,
        state_path,
        index_before,
        roadmap_before,
        state_before,
        state_after,
    } = prepared;

    let mut transaction = Transaction::default();
    let operation = (|| {
        for note in notes.iter().filter(|note| note.create) {
            create_file_atomically(&note.path, &note.source)?;
            transaction
                .created
                .push((note.path.clone(), note.source.clone()));
        }
        after_note_creation();

        let mut updated_projections = Vec::new();
        if replace_file_if_unchanged(&index_path, &index_before, request.index.as_bytes())? {
            transaction.replaced.push(Replacement {
                path: index_path.clone(),
                before: index_before.clone(),
                after: request.index.as_bytes().to_vec(),
            });
            updated_projections.push(index_path.clone());
        }
        if replace_file_if_unchanged(&roadmap_path, &roadmap_before, request.roadmap.as_bytes())? {
            transaction.replaced.push(Replacement {
                path: roadmap_path.clone(),
                before: roadmap_before.clone(),
                after: request.roadmap.as_bytes().to_vec(),
            });
            updated_projections.push(roadmap_path.clone());
        }
        if replace_file_if_unchanged(&state_path, &state_before, &state_after)? {
            transaction.replaced.push(Replacement {
                path: state_path.clone(),
                before: state_before.clone(),
                after: state_after,
            });
        }

        validate_project(&request.resolution)?;

        Ok(OnboardingBatchResult {
            root,
            project,
            project_dir,
            created_notes: notes
                .iter()
                .filter(|note| note.create)
                .map(|note| note.path.clone())
                .collect(),
            unchanged_notes: notes
                .iter()
                .filter(|note| !note.create)
                .map(|note| note.path.clone())
                .collect(),
            updated_projections,
            state: state_path,
        })
    })();

    match operation {
        Ok(result) => Ok(result),
        Err(error) => {
            let failures = transaction.rollback();
            if failures.is_empty() {
                Err(error)
            } else {
                Err(OnboardingBatchError::Cleanup {
                    committed: false,
                    original: Some(Box::new(error)),
                    failures,
                })
            }
        }
    }
}

fn prepare_batch(
    request: &OnboardingBatchRequest,
    locked_project_dir: Option<&Path>,
    evidence_policy: EvidencePolicy,
) -> Result<PreparedBatch, OnboardingBatchError> {
    if request.notes.is_empty() {
        return Err(OnboardingBatchError::Validation {
            path: PathBuf::from("<proposal>"),
            message: "an onboarding batch must contain at least one canonical note".to_owned(),
        });
    }
    if evidence_policy == EvidencePolicy::Required {
        validate_proposal_bounds(request)?;
    }

    let report = validate_project(&request.resolution)?;
    if locked_project_dir.is_some_and(|locked| report.project_dir != locked) {
        return Err(OnboardingBatchError::Conflict {
            path: report.project_dir,
            message: "project resolution changed while acquiring the onboarding lock".to_owned(),
        });
    }
    let config = load_root_config(&report.root)?;
    let notes = prepare_notes(
        &report.root,
        &report.project,
        &report.project_dir,
        &report.repository_dir,
        &config,
        &request.notes,
        evidence_policy,
    )?;
    let index_path = report.project_dir.join(&config.project.index);
    let roadmap_path = report.project_dir.join(&config.project.roadmap);
    let state_path = report.project_dir.join(PROJECT_STATE_FILE);
    let index_before = read_regular_file(&index_path, "read the current index projection")?;
    let roadmap_before = read_regular_file(&roadmap_path, "read the current roadmap projection")?;
    let state_before = read_regular_file(&state_path, "read the current project state")?;
    let state_text =
        str::from_utf8(&state_before).map_err(|error| OnboardingBatchError::Validation {
            path: state_path.clone(),
            message: format!("project state is not valid UTF-8: {error}"),
        })?;

    let mut evidence = collect_existing_evidence(&report.project_dir, &config)?;
    validate_project_state(
        state_text,
        &report.project_dir,
        &index_before,
        &roadmap_before,
        &evidence,
    )
    .map_err(|message| OnboardingBatchError::Validation {
        path: state_path.clone(),
        message,
    })?;

    let mut new_events = BTreeSet::new();
    for note in &notes {
        if note.create {
            evidence.push(CanonicalNoteEvidence {
                path: note.path.clone(),
                class: note.class,
                source: note.source.clone(),
            });
            if note.class == NoteClass::Event {
                new_events.insert(note.path.clone());
            }
        }
    }
    evidence.sort_by(|left, right| left.path.cmp(&right.path));

    let state_after = render_updated_project_state(
        state_text,
        &report.project_dir,
        request.index.as_bytes(),
        request.roadmap.as_bytes(),
        &evidence,
        &new_events,
    )
    .map_err(|message| OnboardingBatchError::Validation {
        path: state_path.clone(),
        message,
    })?;

    Ok(PreparedBatch {
        root: report.root,
        project: report.project,
        project_dir: report.project_dir,
        notes,
        index_path,
        roadmap_path,
        state_path,
        index_before,
        roadmap_before,
        state_before,
        state_after,
    })
}

fn render_preview(
    request: &OnboardingBatchRequest,
    prepared: &PreparedBatch,
) -> OnboardingBatchPreview {
    let proposal_id = proposal_identifier(request, &prepared.project);
    let preview_id = preview_identifier(&proposal_id, prepared);
    OnboardingBatchPreview {
        root: prepared.root.clone(),
        project: prepared.project.clone(),
        project_dir: prepared.project_dir.clone(),
        proposal_id,
        preview_id,
        notes: prepared
            .notes
            .iter()
            .map(|note| OnboardingNotePreview {
                note_type: note.note_type.clone(),
                path: note.relative_path.clone(),
                action: if note.create {
                    OnboardingNoteAction::Create
                } else {
                    OnboardingNoteAction::Unchanged
                },
                evidence_claims: note.evidence_claims,
            })
            .collect(),
        index_changed: prepared.index_before != request.index.as_bytes(),
        roadmap_changed: prepared.roadmap_before != request.roadmap.as_bytes(),
        state_changed: prepared.state_before != prepared.state_after,
    }
}

fn proposal_identifier(request: &OnboardingBatchRequest, project: &str) -> String {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, ONBOARDING_PROPOSAL_DOMAIN);
    hash_field(&mut hasher, project.as_bytes());
    let mut notes = request.notes.iter().collect::<Vec<_>>();
    notes
        .sort_by(|left, right| (&left.note_type, &left.path).cmp(&(&right.note_type, &right.path)));
    for note in notes {
        hash_field(&mut hasher, note.note_type.as_bytes());
        hash_field(&mut hasher, note.path.to_string_lossy().as_bytes());
        hash_field(&mut hasher, note.source.as_bytes());
    }
    hash_field(&mut hasher, request.index.as_bytes());
    hash_field(&mut hasher, request.roadmap.as_bytes());
    render_digest(hasher.finalize())
}

fn preview_identifier(proposal_id: &str, prepared: &PreparedBatch) -> String {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, ONBOARDING_PREVIEW_DOMAIN);
    hash_field(&mut hasher, proposal_id.as_bytes());
    hash_field(&mut hasher, prepared.root.to_string_lossy().as_bytes());
    hash_field(&mut hasher, prepared.project.as_bytes());
    hash_field(
        &mut hasher,
        prepared.project_dir.to_string_lossy().as_bytes(),
    );
    hash_field(&mut hasher, &prepared.index_before);
    hash_field(&mut hasher, &prepared.roadmap_before);
    hash_field(&mut hasher, &prepared.state_before);
    render_digest(hasher.finalize())
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value);
}

fn render_digest(digest: impl AsRef<[u8]>) -> String {
    let digest = digest.as_ref();
    let mut output = String::with_capacity(71);
    output.push_str("sha256:");
    for byte in digest {
        use fmt::Write;
        write!(output, "{byte:02x}").expect("writing to a string cannot fail");
    }
    output
}

#[derive(Debug)]
struct PreparedNote {
    note_type: String,
    relative_path: PathBuf,
    path: PathBuf,
    source: Vec<u8>,
    class: NoteClass,
    create: bool,
    evidence_claims: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvidencePolicy {
    Unchecked,
    Required,
}

#[derive(Debug)]
struct PreparedBatch {
    root: PathBuf,
    project: String,
    project_dir: PathBuf,
    notes: Vec<PreparedNote>,
    index_path: PathBuf,
    roadmap_path: PathBuf,
    state_path: PathBuf,
    index_before: Vec<u8>,
    roadmap_before: Vec<u8>,
    state_before: Vec<u8>,
    state_after: Vec<u8>,
}

fn prepare_notes(
    root: &Path,
    project: &str,
    project_dir: &Path,
    repository_dir: &Path,
    config: &RootConfig,
    proposals: &[ProposedNote],
    evidence_policy: EvidencePolicy,
) -> Result<Vec<PreparedNote>, OnboardingBatchError> {
    let mut destinations = BTreeMap::new();
    for proposal in proposals {
        let note_type = config
            .project
            .note_types
            .get(&proposal.note_type)
            .ok_or_else(|| OnboardingBatchError::Validation {
                path: proposal.path.clone(),
                message: format!("undefined note type {:?}", proposal.note_type),
            })?;
        validate_relative_markdown_path(&proposal.path)?;
        let folder = fs::canonicalize(project_dir.join(&note_type.folder)).map_err(|source| {
            OnboardingBatchError::FileSystem {
                operation: "resolve the configured note folder",
                path: project_dir.join(&note_type.folder),
                source,
            }
        })?;
        let requested = folder.join(&proposal.path);
        let parent = requested
            .parent()
            .expect("a validated relative Markdown path always has a parent");
        let canonical_parent = fs::canonicalize(parent).map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                OnboardingBatchError::Validation {
                    path: parent.to_path_buf(),
                    message: "proposal parent directory does not exist".to_owned(),
                }
            } else {
                OnboardingBatchError::FileSystem {
                    operation: "resolve the proposal parent directory",
                    path: parent.to_path_buf(),
                    source,
                }
            }
        })?;
        if !canonical_parent.starts_with(&folder) {
            return Err(OnboardingBatchError::Validation {
                path: requested,
                message: format!(
                    "proposal path escapes configured note folder {}",
                    folder.display()
                ),
            });
        }
        let destination = canonical_parent.join(
            proposal
                .path
                .file_name()
                .expect("a validated relative Markdown path has a filename"),
        );
        if destinations.insert(destination.clone(), proposal).is_some() {
            return Err(OnboardingBatchError::Validation {
                path: destination,
                message: "proposal contains the same target more than once".to_owned(),
            });
        }
    }

    let proposed_targets = destinations.keys().cloned().collect::<BTreeSet<_>>();
    let mut prepared = Vec::with_capacity(destinations.len());
    for (path, proposal) in destinations {
        let note_type = &config.project.note_types[&proposal.note_type];
        let source = proposal.source.as_bytes().to_vec();
        let parsed = parse_leading_frontmatter_bytes(&source).map_err(|error| {
            OnboardingBatchError::Validation {
                path: path.clone(),
                message: error.to_string(),
            }
        })?;
        validate_configured_note(
            &parsed,
            project,
            &proposal.note_type,
            &note_type.required_fields,
        )
        .map_err(|error| OnboardingBatchError::Validation {
            path: path.clone(),
            message: error.to_string(),
        })?;
        validate_wikilinks_with_targets(root, &path, parsed.body, &proposed_targets)?;
        let evidence_claims = match evidence_policy {
            EvidencePolicy::Unchecked => 0,
            EvidencePolicy::Required => {
                validate_persistent_evidence(&parsed, repository_dir, &path)?
            }
        };

        let create = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(OnboardingBatchError::Conflict {
                    path,
                    message: "proposal target is not a regular file".to_owned(),
                });
            }
            Ok(_) => {
                let current =
                    fs::read(&path).map_err(|source| OnboardingBatchError::FileSystem {
                        operation: "read an existing proposal target",
                        path: path.clone(),
                        source,
                    })?;
                if current != source {
                    return Err(OnboardingBatchError::Conflict {
                        path,
                        message: "existing note differs from the proposed exact bytes".to_owned(),
                    });
                }
                false
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => true,
            Err(source) => {
                return Err(OnboardingBatchError::FileSystem {
                    operation: "inspect a proposal target",
                    path,
                    source,
                });
            }
        };
        prepared.push(PreparedNote {
            note_type: proposal.note_type.clone(),
            relative_path: proposal.path.clone(),
            path,
            source,
            class: note_type.class,
            create,
            evidence_claims,
        });
    }
    Ok(prepared)
}

fn validate_proposal_bounds(request: &OnboardingBatchRequest) -> Result<(), OnboardingBatchError> {
    if request.notes.len() > MAX_ONBOARDING_NOTES {
        return Err(OnboardingBatchError::Validation {
            path: PathBuf::from("<proposal>"),
            message: format!(
                "proposal contains {} notes; the maximum is {MAX_ONBOARDING_NOTES}",
                request.notes.len()
            ),
        });
    }
    for (name, source) in [
        ("index", request.index.as_str()),
        ("roadmap", request.roadmap.as_str()),
    ] {
        let characters = source.chars().count();
        if characters > MAX_ONBOARDING_PROJECTION_CHARS {
            return Err(OnboardingBatchError::Validation {
                path: PathBuf::from(format!("<{name}>")),
                message: format!(
                    "{name} contains {characters} characters; the maximum is \
                     {MAX_ONBOARDING_PROJECTION_CHARS}"
                ),
            });
        }
    }

    let mut total = request.index.chars().count() + request.roadmap.chars().count();
    for note in &request.notes {
        let characters = note.source.chars().count();
        if characters > MAX_ONBOARDING_NOTE_CHARS {
            return Err(OnboardingBatchError::Validation {
                path: note.path.clone(),
                message: format!(
                    "note contains {characters} characters; the maximum is \
                     {MAX_ONBOARDING_NOTE_CHARS}"
                ),
            });
        }
        total = total.saturating_add(characters);
    }
    if total > MAX_ONBOARDING_PROPOSAL_CHARS {
        return Err(OnboardingBatchError::Validation {
            path: PathBuf::from("<proposal>"),
            message: format!(
                "proposal contains {total} source characters; the maximum is \
                 {MAX_ONBOARDING_PROPOSAL_CHARS}"
            ),
        });
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistentEvidenceClaim {
    kind: PersistentEvidenceKind,
    claim: String,
    #[serde(default)]
    sources: Vec<PersistentEvidenceSource>,
    rationale: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PersistentEvidenceKind {
    Fact,
    Inference,
    Unknown,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistentEvidenceSource {
    path: String,
    fingerprint: String,
    line_start: Option<usize>,
    line_end: Option<usize>,
}

fn validate_persistent_evidence(
    parsed: &crate::validation::ParsedNote<'_>,
    repository_dir: &Path,
    note_path: &Path,
) -> Result<usize, OnboardingBatchError> {
    let value = parsed.metadata.get("evidence").cloned().ok_or_else(|| {
        OnboardingBatchError::Validation {
            path: note_path.to_path_buf(),
            message: "onboarding note frontmatter must contain a non-empty `evidence` list"
                .to_owned(),
        }
    })?;
    let claims: Vec<PersistentEvidenceClaim> =
        serde_json::from_value(value).map_err(|error| OnboardingBatchError::Validation {
            path: note_path.to_path_buf(),
            message: format!("invalid onboarding evidence schema: {error}"),
        })?;
    if claims.is_empty() || claims.len() > MAX_ONBOARDING_EVIDENCE_CLAIMS {
        return Err(OnboardingBatchError::Validation {
            path: note_path.to_path_buf(),
            message: format!(
                "onboarding evidence must contain 1..={MAX_ONBOARDING_EVIDENCE_CLAIMS} claims"
            ),
        });
    }

    for evidence in &claims {
        if evidence.claim.trim().is_empty() || evidence.claim.chars().count() > 1_024 {
            return Err(OnboardingBatchError::Validation {
                path: note_path.to_path_buf(),
                message: "evidence claims must contain 1..=1024 characters".to_owned(),
            });
        }
        if evidence.sources.len() > MAX_ONBOARDING_EVIDENCE_SOURCES {
            return Err(OnboardingBatchError::Validation {
                path: note_path.to_path_buf(),
                message: format!(
                    "an evidence claim may contain at most \
                     {MAX_ONBOARDING_EVIDENCE_SOURCES} sources"
                ),
            });
        }
        let rationale = evidence.rationale.as_deref().map(str::trim);
        if rationale.is_some_and(|value| value.is_empty() || value.chars().count() > 2_048) {
            return Err(OnboardingBatchError::Validation {
                path: note_path.to_path_buf(),
                message: "evidence rationale must contain 1..=2048 characters when supplied"
                    .to_owned(),
            });
        }
        match evidence.kind {
            PersistentEvidenceKind::Fact if evidence.sources.is_empty() => {
                return Err(OnboardingBatchError::Validation {
                    path: note_path.to_path_buf(),
                    message: "fact evidence requires at least one repository source".to_owned(),
                });
            }
            PersistentEvidenceKind::Inference
                if evidence.sources.is_empty() || rationale.is_none() =>
            {
                return Err(OnboardingBatchError::Validation {
                    path: note_path.to_path_buf(),
                    message: "inference evidence requires sources and a rationale".to_owned(),
                });
            }
            PersistentEvidenceKind::Unknown
                if !evidence.sources.is_empty() || rationale.is_none() =>
            {
                return Err(OnboardingBatchError::Validation {
                    path: note_path.to_path_buf(),
                    message: "unknown evidence requires a rationale and must not claim sources"
                        .to_owned(),
                });
            }
            PersistentEvidenceKind::Fact
            | PersistentEvidenceKind::Inference
            | PersistentEvidenceKind::Unknown => {}
        }

        let mut seen_sources = BTreeSet::new();
        for source in &evidence.sources {
            let identity = (source.path.as_str(), source.line_start, source.line_end);
            if !seen_sources.insert(identity) {
                return Err(OnboardingBatchError::Validation {
                    path: note_path.to_path_buf(),
                    message: format!("duplicate evidence source {:?}", source.path),
                });
            }
            validate_evidence_source(source, repository_dir, note_path)?;
        }
    }
    Ok(claims.len())
}

fn validate_evidence_source(
    source: &PersistentEvidenceSource,
    repository_dir: &Path,
    note_path: &Path,
) -> Result<(), OnboardingBatchError> {
    validate_normalized_relative_path(&source.path, "evidence source", note_path)?;
    match (source.line_start, source.line_end) {
        (None, None) => {}
        (Some(start), Some(end)) if start > 0 && end >= start => {}
        _ => {
            return Err(OnboardingBatchError::Validation {
                path: note_path.to_path_buf(),
                message: format!(
                    "evidence source {:?} must supply both positive line bounds with end >= start",
                    source.path
                ),
            });
        }
    }

    let path = repository_dir.join(&source.path);
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            OnboardingBatchError::Validation {
                path: note_path.to_path_buf(),
                message: format!("evidence source {:?} does not exist", source.path),
            }
        } else {
            OnboardingBatchError::FileSystem {
                operation: "inspect onboarding evidence source",
                path: path.clone(),
                source: error,
            }
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(OnboardingBatchError::Validation {
            path: note_path.to_path_buf(),
            message: format!(
                "evidence source {:?} must be a regular non-symlink file",
                source.path
            ),
        });
    }
    let canonical = fs::canonicalize(&path).map_err(|error| OnboardingBatchError::FileSystem {
        operation: "resolve onboarding evidence source",
        path: path.clone(),
        source: error,
    })?;
    if !canonical.starts_with(repository_dir) {
        return Err(OnboardingBatchError::Validation {
            path: note_path.to_path_buf(),
            message: format!("evidence source {:?} escapes the repository", source.path),
        });
    }
    let contents = fs::read(&canonical).map_err(|error| OnboardingBatchError::FileSystem {
        operation: "read onboarding evidence source",
        path: canonical,
        source: error,
    })?;
    let actual_fingerprint = content_fingerprint(&contents);
    if source.fingerprint != actual_fingerprint {
        return Err(OnboardingBatchError::Validation {
            path: note_path.to_path_buf(),
            message: format!(
                "evidence source {:?} fingerprint is stale; expected {actual_fingerprint}",
                source.path
            ),
        });
    }
    if let Some(end) = source.line_end {
        let text = str::from_utf8(&contents).map_err(|error| OnboardingBatchError::Validation {
            path: note_path.to_path_buf(),
            message: format!(
                "evidence source {:?} uses line bounds but is not UTF-8: {error}",
                source.path
            ),
        })?;
        let lines = text.lines().count().max(1);
        if end > lines {
            return Err(OnboardingBatchError::Validation {
                path: note_path.to_path_buf(),
                message: format!(
                    "evidence source {:?} ends at line {end}, but the file has {lines} lines",
                    source.path
                ),
            });
        }
    }
    Ok(())
}

fn validate_normalized_relative_path(
    text: &str,
    label: &str,
    error_path: &Path,
) -> Result<(), OnboardingBatchError> {
    if text.is_empty()
        || text.starts_with('/')
        || text.contains('\\')
        || text.chars().count() > 1_024
        || text
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(OnboardingBatchError::Validation {
            path: error_path.to_path_buf(),
            message: format!(
                "{label} path must be a normalized repository-relative path using / separators"
            ),
        });
    }
    Ok(())
}

fn collect_templates(
    templates_dir: &Path,
) -> Result<(Vec<OnboardingTemplate>, usize, usize), OnboardingBatchError> {
    let mut paths = Vec::new();
    collect_template_paths(templates_dir, templates_dir, &mut paths)?;
    paths.sort();

    let mut templates = Vec::new();
    let mut omitted = 0;
    let mut characters: usize = 0;
    for (relative, path) in paths {
        let source = fs::read(&path).map_err(|error| OnboardingBatchError::FileSystem {
            operation: "read onboarding template",
            path: path.clone(),
            source: error,
        })?;
        let source =
            String::from_utf8(source).map_err(|error| OnboardingBatchError::Validation {
                path: path.clone(),
                message: format!("onboarding template is not valid UTF-8: {error}"),
            })?;
        let source_characters = source.chars().count();
        if characters.saturating_add(source_characters) > MAX_ONBOARDING_TEMPLATE_CHARS {
            omitted += 1;
            continue;
        }
        characters += source_characters;
        templates.push(OnboardingTemplate {
            path: relative,
            source,
        });
    }
    Ok((templates, omitted, characters))
}

fn collect_template_paths(
    root: &Path,
    directory: &Path,
    paths: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), OnboardingBatchError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| OnboardingBatchError::FileSystem {
            operation: "read onboarding template directory",
            path: directory.to_path_buf(),
            source: error,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| OnboardingBatchError::FileSystem {
            operation: "read onboarding template entry",
            path: directory.to_path_buf(),
            source: error,
        })?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| OnboardingBatchError::FileSystem {
                operation: "inspect onboarding template entry",
                path: path.clone(),
                source: error,
            })?;
        if file_type.is_symlink() || (!file_type.is_dir() && !file_type.is_file()) {
            return Err(OnboardingBatchError::Validation {
                path,
                message: "template entries must be regular files or directories, not symlinks"
                    .to_owned(),
            });
        }
        if file_type.is_dir() {
            collect_template_paths(root, &path, paths)?;
        } else {
            paths.push((
                path.strip_prefix(root)
                    .expect("template entries are traversed below the template root")
                    .to_path_buf(),
                path,
            ));
        }
    }
    Ok(())
}

fn validate_relative_markdown_path(path: &Path) -> Result<(), OnboardingBatchError> {
    let Some(text) = path.to_str() else {
        return Err(OnboardingBatchError::Validation {
            path: path.to_path_buf(),
            message: "proposal paths must be valid UTF-8".to_owned(),
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
        return Err(OnboardingBatchError::Validation {
            path: path.to_path_buf(),
            message: "proposal path must be a normalized relative .md path using / separators"
                .to_owned(),
        });
    }
    Ok(())
}

fn collect_existing_evidence(
    project_dir: &Path,
    config: &RootConfig,
) -> Result<Vec<CanonicalNoteEvidence>, OnboardingBatchError> {
    let mut evidence = Vec::new();
    for note_type in config.project.note_types.values() {
        let folder = project_dir.join(&note_type.folder);
        for path in canonical_note_paths(&folder)? {
            let source = fs::read(&path).map_err(|source| OnboardingBatchError::FileSystem {
                operation: "read canonical note for onboarding state",
                path: path.clone(),
                source,
            })?;
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
) -> Result<Vec<u8>, OnboardingBatchError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|source| OnboardingBatchError::FileSystem {
            operation,
            path: path.to_path_buf(),
            source,
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(OnboardingBatchError::Conflict {
            path: path.to_path_buf(),
            message: "checked replacement target is not a regular file".to_owned(),
        });
    }
    fs::read(path).map_err(|source| OnboardingBatchError::FileSystem {
        operation,
        path: path.to_path_buf(),
        source,
    })
}

fn replace_file_if_unchanged(
    path: &Path,
    expected: &[u8],
    replacement: &[u8],
) -> Result<bool, OnboardingBatchError> {
    replace_checked_file(path, expected, replacement).map_err(|error| match error {
        CheckedReplaceError::Conflict { path, .. } => OnboardingBatchError::Conflict {
            path,
            message: "checked replacement target changed during onboarding".to_owned(),
        },
        CheckedReplaceError::FileSystem {
            operation,
            path,
            source,
        } => OnboardingBatchError::FileSystem {
            operation,
            path,
            source,
        },
    })
}

#[derive(Default)]
struct Transaction {
    created: Vec<(PathBuf, Vec<u8>)>,
    replaced: Vec<Replacement>,
}

struct Replacement {
    path: PathBuf,
    before: Vec<u8>,
    after: Vec<u8>,
}

impl Transaction {
    fn rollback(&mut self) -> Vec<String> {
        let mut failures = Vec::new();
        for replacement in self.replaced.iter().rev() {
            if let Err(error) = replace_file_if_unchanged(
                &replacement.path,
                &replacement.after,
                &replacement.before,
            ) {
                failures.push(format!(
                    "could not restore {}: {error}",
                    replacement.path.display()
                ));
            }
        }
        for (path, expected) in self.created.iter().rev() {
            match fs::read(path) {
                Ok(current) if current == *expected => {
                    if let Err(source) = fs::remove_file(path) {
                        failures.push(format!("could not remove {}: {source}", path.display()));
                    }
                }
                Ok(_) => failures.push(format!(
                    "did not remove changed created note {}",
                    path.display()
                )),
                Err(source) if source.kind() == io::ErrorKind::NotFound => {}
                Err(source) => failures.push(format!(
                    "could not verify created note {}: {source}",
                    path.display()
                )),
            }
        }
        failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolution::ResolutionEnvironment;
    use crate::state::render_empty_project_state;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn rolls_back_created_notes_when_projection_changes_concurrently() {
        let fixture = Fixture::new("projection-race");
        let request = fixture.request();
        let index = fixture.project.join("index.md");
        let note = fixture.project.join("entities/core.md");

        let error = apply_onboarding_batch_with_hook(&request, || {
            fs::write(&index, "human edit\n").expect("change index concurrently")
        })
        .expect_err("changed index must conflict");

        assert_eq!(error.exit_code(), 5);
        assert!(!note.exists());
        assert_eq!(
            fs::read_to_string(index).expect("preserve edit"),
            "human edit\n"
        );
        assert_eq!(
            fs::read(fixture.project.join(PROJECT_STATE_FILE)).expect("read state"),
            render_empty_project_state()
        );
    }

    struct Fixture {
        base: PathBuf,
        root: PathBuf,
        repository: PathBuf,
        project: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
            let base = std::env::temp_dir().join(format!(
                "akasha-onboarding-unit-{label}-{}-{id}",
                std::process::id()
            ));
            let root = base.join("root");
            let repository = base.join("repository");
            let project = root.join("Projects/example");
            for directory in [
                root.join("Meta"),
                root.join("templates"),
                root.join("Global"),
                root.join("Inbox"),
                project.join("templates"),
                project.join("events/sessions"),
                project.join("events/handoffs"),
                project.join("records/tasks"),
                project.join("records/problems"),
                project.join("entities"),
                repository.clone(),
            ] {
                fs::create_dir_all(directory).expect("create fixture directory");
            }
            fs::write(
                root.join("akasha.toml"),
                include_str!("../../../tests/fixtures/resolution/valid-root/akasha.toml"),
            )
            .expect("write root config");
            fs::write(
                root.join("Meta/projects.yaml"),
                format!("example:\n  path: {:?}\n  status: active\n", repository),
            )
            .expect("write registry");
            fs::write(
                repository.join(".akasha.toml"),
                "schema_version = 1\nproject = \"example\"\n",
            )
            .expect("write pointer");
            fs::write(project.join("index.md"), "").expect("write index");
            fs::write(project.join("roadmap.md"), "").expect("write roadmap");
            fs::write(
                project.join(PROJECT_STATE_FILE),
                render_empty_project_state(),
            )
            .expect("write state");
            Self {
                base,
                root,
                repository,
                project,
            }
        }

        fn request(&self) -> OnboardingBatchRequest {
            OnboardingBatchRequest {
                resolution: ResolveRequest {
                    root_override: Some(self.root.clone()),
                    project_override: None,
                    cwd: self.repository.clone(),
                    environment: ResolutionEnvironment::default(),
                },
                notes: vec![ProposedNote {
                    note_type: "entity".to_owned(),
                    path: PathBuf::from("core.md"),
                    source: "---\nschema_version: 1\nentity: core\nkind: subsystem\nstatus: active\nreviewed: 2026-07-13\n---\n\n# Core\n".to_owned(),
                }],
                index: "# Index\n".to_owned(),
                roadmap: String::new(),
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }
}
