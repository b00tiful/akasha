use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::str;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

use crate::project_validation::{
    ProjectValidationError, canonical_note_paths, validate_project, validate_wikilinks_with_targets,
};
use crate::resolution::{
    NoteClass, ResolveError, ResolveRequest, RootConfig, load_root_config, resolve_project,
};
use crate::state::{
    CanonicalNoteEvidence, PROJECT_STATE_FILE, render_updated_project_state, validate_project_state,
};
use crate::validation::{parse_leading_frontmatter_bytes, validate_configured_note};
use crate::writes::{AtomicCreateError, create_file_atomically};

const MAX_REPLACEMENT_STAGING_ATTEMPTS: u64 = 128;
static NEXT_REPLACEMENT_STAGE_ID: AtomicU64 = AtomicU64::new(0);

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

impl From<AtomicCreateError> for OnboardingBatchError {
    fn from(error: AtomicCreateError) -> Self {
        Self::Creation(error)
    }
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
    let state_path = resolved.project_dir.join(PROJECT_STATE_FILE);
    let lock = BatchLock::acquire(&state_path)?;
    let result = apply_locked(request, &resolved.project_dir, after_note_creation);

    match lock.release() {
        Ok(()) => result,
        Err((path, source)) => Err(OnboardingBatchError::Cleanup {
            committed: result.is_ok(),
            original: result.err().map(Box::new),
            failures: vec![format!(
                "could not remove onboarding lock {}: {source}",
                path.display()
            )],
        }),
    }
}

fn apply_locked(
    request: &OnboardingBatchRequest,
    locked_project_dir: &Path,
    after_note_creation: impl FnOnce(),
) -> Result<OnboardingBatchResult, OnboardingBatchError> {
    let report = validate_project(&request.resolution)?;
    if report.project_dir != locked_project_dir {
        return Err(OnboardingBatchError::Conflict {
            path: report.project_dir,
            message: "project resolution changed while acquiring the onboarding lock".to_owned(),
        });
    }
    let config = load_root_config(&report.root)?;
    let prepared = prepare_notes(
        &report.root,
        &report.project,
        &report.project_dir,
        &config,
        &request.notes,
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
    for note in &prepared {
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

    let mut transaction = Transaction::default();
    let operation = (|| {
        for note in prepared.iter().filter(|note| note.create) {
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
            root: report.root,
            project: report.project,
            project_dir: report.project_dir,
            created_notes: prepared
                .iter()
                .filter(|note| note.create)
                .map(|note| note.path.clone())
                .collect(),
            unchanged_notes: prepared
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

#[derive(Debug)]
struct PreparedNote {
    path: PathBuf,
    source: Vec<u8>,
    class: NoteClass,
    create: bool,
}

fn prepare_notes(
    root: &Path,
    project: &str,
    project_dir: &Path,
    config: &RootConfig,
    proposals: &[ProposedNote],
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
            path,
            source,
            class: note_type.class,
            create,
        });
    }
    Ok(prepared)
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
    if replacement == expected {
        return Ok(false);
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|source| OnboardingBatchError::FileSystem {
            operation: "inspect a checked replacement target",
            path: path.to_path_buf(),
            source,
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(OnboardingBatchError::Conflict {
            path: path.to_path_buf(),
            message: "checked replacement target is no longer a regular file".to_owned(),
        });
    }
    let current = fs::read(path).map_err(|source| OnboardingBatchError::FileSystem {
        operation: "read a checked replacement target",
        path: path.to_path_buf(),
        source,
    })?;
    if current != expected {
        return Err(OnboardingBatchError::Conflict {
            path: path.to_path_buf(),
            message: "checked replacement target changed during onboarding".to_owned(),
        });
    }

    let stage = create_replacement_stage(path, replacement)?;
    fs::set_permissions(&stage.path, metadata.permissions()).map_err(|source| {
        OnboardingBatchError::FileSystem {
            operation: "preserve checked replacement permissions",
            path: stage.path.clone(),
            source,
        }
    })?;
    File::open(&stage.path)
        .and_then(|file| file.sync_all())
        .map_err(|source| OnboardingBatchError::FileSystem {
            operation: "sync checked replacement permissions",
            path: stage.path.clone(),
            source,
        })?;

    let current = fs::read(path).map_err(|source| OnboardingBatchError::FileSystem {
        operation: "verify a checked replacement target",
        path: path.to_path_buf(),
        source,
    })?;
    if current != expected {
        return Err(OnboardingBatchError::Conflict {
            path: path.to_path_buf(),
            message: "checked replacement target changed during onboarding".to_owned(),
        });
    }
    fs::rename(&stage.path, path).map_err(|source| OnboardingBatchError::FileSystem {
        operation: "publish a checked replacement",
        path: path.to_path_buf(),
        source,
    })?;
    Ok(true)
}

fn create_replacement_stage(
    path: &Path,
    contents: &[u8],
) -> Result<ReplacementStage, OnboardingBatchError> {
    let parent = path
        .parent()
        .expect("a resolved project file always has a parent");
    let file_name = path
        .file_name()
        .expect("a resolved project file always has a filename");
    for _ in 0..MAX_REPLACEMENT_STAGING_ATTEMPTS {
        let id = NEXT_REPLACEMENT_STAGE_ID.fetch_add(1, Ordering::Relaxed);
        let mut stage_name = OsString::from(".");
        stage_name.push(file_name);
        stage_name.push(format!(
            ".akasha-onboard-{}-{id}.replacement",
            std::process::id()
        ));
        let stage = parent.join(stage_name);
        match create_file_atomically(&stage, contents) {
            Ok(()) => return Ok(ReplacementStage { path: stage }),
            Err(AtomicCreateError::Conflict { .. }) => continue,
            Err(error) => return Err(OnboardingBatchError::Creation(error)),
        }
    }
    Err(OnboardingBatchError::FileSystem {
        operation: "create a unique checked replacement stage",
        path: path.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "all replacement staging filename attempts were occupied",
        ),
    })
}

struct ReplacementStage {
    path: PathBuf,
}

impl Drop for ReplacementStage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct BatchLock {
    path: PathBuf,
    released: bool,
}

impl BatchLock {
    fn acquire(state: &Path) -> Result<Self, OnboardingBatchError> {
        let file_name = state
            .file_name()
            .expect("a resolved project state always has a filename");
        let mut lock_name = OsString::from(file_name);
        lock_name.push(".akasha-onboard.lock");
        let path = state
            .parent()
            .expect("a resolved project state always has a parent")
            .join(lock_name);
        create_file_atomically(&path, b"akasha onboarding batch\n")?;
        Ok(Self {
            path,
            released: false,
        })
    }

    fn release(mut self) -> Result<(), (PathBuf, io::Error)> {
        match fs::remove_file(&self.path) {
            Ok(()) => {
                self.released = true;
                Ok(())
            }
            Err(source) => {
                self.released = true;
                Err((self.path.clone(), source))
            }
        }
    }
}

impl Drop for BatchLock {
    fn drop(&mut self) {
        if !self.released {
            let _ = fs::remove_file(&self.path);
        }
    }
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
