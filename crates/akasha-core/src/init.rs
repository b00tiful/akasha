use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::str;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::resolution::{
    CONFIG_SCHEMA_VERSION, POINTER_FILE, ResolutionEnvironment, ResolveError, ResolveRequest,
    RootConfig, canonicalize_directory, load_project_registry, load_root_config, resolve_root,
    validate_slug,
};
use crate::state::{PROJECT_STATE_FILE, content_fingerprint, render_empty_project_state};
use crate::validation::{
    ProjectRegistry, ProjectRegistryEntry, ValidationError, parse_project_registry,
};
use crate::writes::{
    AtomicCreateError, PROJECT_WRITE_LOCK_FILE, create_file_atomically, sync_directory,
};

const MAX_REGISTRY_STAGING_ATTEMPTS: u64 = 128;
const INIT_JOURNAL_SCHEMA_VERSION: u32 = 1;
static NEXT_REGISTRY_STAGE_ID: AtomicU64 = AtomicU64::new(0);

/// Inputs for initializing one repository as a new Akasha project.
#[derive(Debug, Clone)]
pub struct InitRequest {
    pub root_override: Option<PathBuf>,
    pub project: String,
    pub cwd: PathBuf,
    pub environment: ResolutionEnvironment,
}

impl InitRequest {
    /// Build an initialization request from CLI inputs and the current process.
    pub fn from_process(
        root_override: Option<PathBuf>,
        project: String,
    ) -> Result<Self, ResolveError> {
        let request = ResolveRequest::from_process(root_override, Some(project.clone()))?;
        Ok(Self {
            root_override: request.root_override,
            project,
            cwd: request.cwd,
            environment: request.environment,
        })
    }
}

/// The paths and copied-template count produced by successful initialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InitResult {
    pub root: PathBuf,
    pub project: String,
    pub registry: PathBuf,
    pub repository_dir: PathBuf,
    pub project_dir: PathBuf,
    pub state: PathBuf,
    pub pointer: PathBuf,
    pub template_files: usize,
    pub recovery: InitRecovery,
}

/// Recovery work completed before a new initialization transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InitRecovery {
    None,
    Discarded,
    RolledBack,
    Finalized,
}

/// A configuration, validation, conflict, filesystem, or rollback failure during initialization.
#[derive(Debug)]
pub enum InitError {
    Resolution(ResolveError),
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
        original: Option<Box<InitError>>,
        failures: Vec<String>,
    },
}

impl InitError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Resolution(error) => error.exit_code(),
            Self::Validation { .. } => 4,
            Self::Conflict { .. } => 5,
            Self::Creation(error) => error.exit_code(),
            Self::FileSystem { .. } | Self::Cleanup { .. } => 6,
        }
    }
}

impl fmt::Display for InitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolution(error) => write!(formatter, "{error}"),
            Self::Validation { path, message } => {
                write!(
                    formatter,
                    "invalid initialization source at {}: {message}",
                    path.display()
                )
            }
            Self::Conflict { path, message } => {
                write!(
                    formatter,
                    "initialization conflict at {}: {message}",
                    path.display()
                )
            }
            Self::Creation(error) => write!(formatter, "{error}"),
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
                    formatter.write_str("initialization committed, but cleanup failed")?;
                } else {
                    formatter.write_str("initialization failed and rollback was incomplete")?;
                }
                if let Some(original) = original {
                    write!(formatter, ": original error: {original}")?;
                }
                write!(formatter, "; residue: {}", failures.join("; "))
            }
        }
    }
}

impl Error for InitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Resolution(error) => Some(error),
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

impl From<ResolveError> for InitError {
    fn from(error: ResolveError) -> Self {
        Self::Resolution(error)
    }
}

impl From<AtomicCreateError> for InitError {
    fn from(error: AtomicCreateError) -> Self {
        Self::Creation(error)
    }
}

/// Create, register, and link one empty configured project without repository inference.
pub fn initialize_project(request: &InitRequest) -> Result<InitResult, InitError> {
    initialize_project_with_hooks(request, || {}, || {}, |_| {})
}

fn initialize_project_with_hooks(
    request: &InitRequest,
    after_scaffold: impl FnOnce(),
    before_registry_commit: impl FnOnce(),
    publication_hook: impl FnMut(InitPublicationStage),
) -> Result<InitResult, InitError> {
    let target = prepare_target(request)?;
    let _lock = InitLock::acquire(&target.registry)?;
    let (recovery, finalized) = recover_init_locked(&target)?;
    if let Some(journal) = finalized
        && journal.project == request.project
        && journal.repository_dir == target.repository_dir
    {
        return Ok(InitResult {
            root: target.root,
            project: journal.project,
            registry: target.registry,
            repository_dir: journal.repository_dir,
            project_dir: journal.project_dir.clone(),
            state: journal.project_dir.join(PROJECT_STATE_FILE),
            pointer: target.pointer,
            template_files: journal.template_files,
            recovery,
        });
    }
    let prepared = prepare_scaffold(target)?;
    initialize_locked(
        request,
        &prepared,
        recovery,
        after_scaffold,
        before_registry_commit,
        publication_hook,
    )
}

fn prepare_target(request: &InitRequest) -> Result<InitTarget, InitError> {
    validate_slug(&request.project)?;
    let resolution_request = ResolveRequest {
        root_override: request.root_override.clone(),
        project_override: Some(request.project.clone()),
        cwd: request.cwd.clone(),
        environment: request.environment.clone(),
    };
    let (root, _) = resolve_root(&resolution_request)?;
    let config = load_root_config(&root)?;
    let repository_dir = canonicalize_directory(&request.cwd, "repository to initialize")?;

    if repository_dir.to_str().is_none() {
        return Err(InitError::Validation {
            path: repository_dir,
            message: "canonical repository path must be valid UTF-8 for the YAML registry"
                .to_owned(),
        });
    }

    let templates_dir = canonicalize_directory(
        &root.join(&config.folders.templates),
        "root template directory",
    )?;
    ensure_inside_root(&templates_dir, &root, "root template directory")?;
    let projects_dir = canonicalize_directory(
        &root.join(&config.folders.projects),
        "configured projects directory",
    )?;
    ensure_inside_root(&projects_dir, &root, "configured projects directory")?;

    let (registry, _) = load_project_registry(&root, &config)?;
    let pointer = repository_dir.join(POINTER_FILE);
    Ok(InitTarget {
        root,
        registry,
        repository_dir,
        project_dir: projects_dir.join(&request.project),
        pointer,
        projects_dir,
        templates_dir,
        config,
    })
}

fn prepare_scaffold(target: InitTarget) -> Result<PreparedInit, InitError> {
    let templates = collect_templates(&target.templates_dir)?;
    let scaffold = build_scaffold_plan(&target.config, templates)?;
    Ok(PreparedInit {
        root: target.root,
        registry: target.registry,
        repository_dir: target.repository_dir,
        project_dir: target.project_dir,
        pointer: target.pointer,
        scaffold,
    })
}

fn initialize_locked(
    request: &InitRequest,
    prepared: &PreparedInit,
    recovery: InitRecovery,
    after_scaffold: impl FnOnce(),
    before_registry_commit: impl FnOnce(),
    mut publication_hook: impl FnMut(InitPublicationStage),
) -> Result<InitResult, InitError> {
    let (registry_source, mut registry) = read_registry_snapshot(&prepared.registry)?;
    reject_registered_slug(&registry, &request.project, &prepared.registry)?;
    reject_existing_target(
        &prepared.project_dir,
        "configured project directory already exists",
    )?;
    reject_existing_target(&prepared.pointer, "repository pointer already exists")?;

    registry.projects.insert(
        request.project.clone(),
        ProjectRegistryEntry {
            path: prepared.repository_dir.clone(),
            status: "active".to_owned(),
        },
    );
    let registry_replacement = render_registry(&registry, &prepared.registry)?;
    let pointer_contents = format!(
        "schema_version = {CONFIG_SCHEMA_VERSION}\nproject = \"{}\"\n",
        request.project
    )
    .into_bytes();
    let (journal_path, journal_source) = write_init_journal(
        prepared,
        &request.project,
        &pointer_contents,
        &registry_source,
        &registry_replacement,
    )?;
    publication_hook(InitPublicationStage::Journal);

    let mut created = CreatedPaths::default();
    let mut committed = false;
    let operation = (|| {
        create_scaffold(
            &prepared.project_dir,
            &prepared.scaffold,
            &mut created,
            &mut publication_hook,
        )?;
        after_scaffold();

        create_file_atomically(&prepared.pointer, &pointer_contents)?;
        sync_parent(&prepared.pointer, "sync the initialized repository pointer")?;
        created
            .files
            .push((prepared.pointer.clone(), pointer_contents));
        publication_hook(InitPublicationStage::Pointer);
        before_registry_commit();

        replace_registry_if_unchanged(&prepared.registry, &registry_source, &registry_replacement)?;
        committed = true;
        sync_parent(&prepared.registry, "sync the initialized project registry")?;
        publication_hook(InitPublicationStage::Registry);

        Ok(InitResult {
            root: prepared.root.clone(),
            project: request.project.clone(),
            registry: prepared.registry.clone(),
            repository_dir: prepared.repository_dir.clone(),
            project_dir: prepared.project_dir.clone(),
            state: prepared.project_dir.join(PROJECT_STATE_FILE),
            pointer: prepared.pointer.clone(),
            template_files: prepared.scaffold.template_files,
            recovery,
        })
    })();

    match operation {
        Ok(result) => match complete_init_journal(&journal_path, &journal_source) {
            Ok(()) => Ok(result),
            Err(error) => Err(InitError::Cleanup {
                committed: true,
                original: None,
                failures: vec![error.to_string()],
            }),
        },
        Err(error) if committed => Err(InitError::Cleanup {
            committed: true,
            original: Some(Box::new(error)),
            failures: vec![format!(
                "recovery journal remains at {}",
                journal_path.display()
            )],
        }),
        Err(error) => {
            let mut failures = created.rollback();
            if failures.is_empty()
                && let Err(cleanup) = complete_init_journal(&journal_path, &journal_source)
            {
                failures.push(cleanup.to_string());
            }
            if failures.is_empty() {
                Err(error)
            } else {
                Err(InitError::Cleanup {
                    committed: false,
                    original: Some(Box::new(error)),
                    failures,
                })
            }
        }
    }
}

fn ensure_inside_root(path: &Path, root: &Path, description: &str) -> Result<(), InitError> {
    if path.starts_with(root) {
        Ok(())
    } else {
        Err(InitError::Resolution(ResolveError::Configuration(format!(
            "{description} {} escapes data root {}",
            path.display(),
            root.display()
        ))))
    }
}

fn reject_registered_slug(
    registry: &ProjectRegistry,
    slug: &str,
    path: &Path,
) -> Result<(), InitError> {
    if registry.projects.contains_key(slug) {
        Err(InitError::Conflict {
            path: path.to_path_buf(),
            message: format!("project {slug:?} is already registered"),
        })
    } else {
        Ok(())
    }
}

fn reject_existing_target(path: &Path, message: &str) -> Result<(), InitError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(InitError::Conflict {
            path: path.to_path_buf(),
            message: message.to_owned(),
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(InitError::FileSystem {
            operation: "inspect an initialization target",
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[derive(Debug)]
struct InitTarget {
    root: PathBuf,
    registry: PathBuf,
    repository_dir: PathBuf,
    project_dir: PathBuf,
    pointer: PathBuf,
    projects_dir: PathBuf,
    templates_dir: PathBuf,
    config: RootConfig,
}

#[derive(Debug)]
struct PreparedInit {
    root: PathBuf,
    registry: PathBuf,
    repository_dir: PathBuf,
    project_dir: PathBuf,
    pointer: PathBuf,
    scaffold: ScaffoldPlan,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InitJournal {
    schema_version: u32,
    project: String,
    project_dir: PathBuf,
    repository_dir: PathBuf,
    directories: Vec<PathBuf>,
    files: Vec<InitJournalFile>,
    pointer_after: String,
    registry_before: String,
    registry_after: String,
    template_files: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InitJournalFile {
    path: PathBuf,
    after: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitPublicationStage {
    Journal,
    ScaffoldDirectory,
    ScaffoldFile,
    Pointer,
    Registry,
}

#[derive(Debug)]
struct TemplatePlan {
    directories: Vec<PathBuf>,
    files: Vec<(PathBuf, Vec<u8>)>,
}

#[derive(Debug)]
struct ScaffoldPlan {
    directories: Vec<PathBuf>,
    files: Vec<(PathBuf, Vec<u8>)>,
    template_files: usize,
}

fn collect_templates(root: &Path) -> Result<TemplatePlan, InitError> {
    fn visit(
        root: &Path,
        directory: &Path,
        directories: &mut Vec<PathBuf>,
        files: &mut Vec<(PathBuf, Vec<u8>)>,
    ) -> Result<(), InitError> {
        let mut entries = fs::read_dir(directory)
            .map_err(|source| InitError::FileSystem {
                operation: "read the root template directory",
                path: directory.to_path_buf(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| InitError::FileSystem {
                operation: "read a root template entry",
                path: directory.to_path_buf(),
                source,
            })?;
        entries.sort_by_key(fs::DirEntry::file_name);

        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|source| InitError::FileSystem {
                operation: "inspect a root template entry",
                path: path.clone(),
                source,
            })?;
            if metadata.file_type().is_symlink() {
                return Err(InitError::Validation {
                    path,
                    message: "template trees must not contain symlinks".to_owned(),
                });
            }
            let relative = path
                .strip_prefix(root)
                .expect("walked template entries remain below their root")
                .to_path_buf();
            if metadata.is_dir() {
                directories.push(relative);
                visit(root, &path, directories, files)?;
            } else if metadata.is_file() {
                let contents = fs::read(&path).map_err(|source| InitError::FileSystem {
                    operation: "read a root template file",
                    path: path.clone(),
                    source,
                })?;
                files.push((relative, contents));
            } else {
                return Err(InitError::Validation {
                    path,
                    message: "template trees may contain only directories and regular files"
                        .to_owned(),
                });
            }
        }
        Ok(())
    }

    let mut directories = Vec::new();
    let mut files = Vec::new();
    visit(root, root, &mut directories, &mut files)?;
    Ok(TemplatePlan { directories, files })
}

fn build_scaffold_plan(
    config: &RootConfig,
    templates: TemplatePlan,
) -> Result<ScaffoldPlan, InitError> {
    let mut directories = BTreeSet::new();
    insert_directory_with_parents(&mut directories, &config.project.templates);
    for note_type in config.project.note_types.values() {
        insert_directory_with_parents(&mut directories, &note_type.folder);
    }
    for directory in &templates.directories {
        insert_directory_with_parents(&mut directories, &config.project.templates.join(directory));
    }

    let template_files = templates.files.len();
    let mut files = BTreeMap::new();
    insert_scaffold_file(
        &mut files,
        PathBuf::from(PROJECT_STATE_FILE),
        render_empty_project_state(),
    )?;
    insert_scaffold_file(
        &mut files,
        PathBuf::from(PROJECT_WRITE_LOCK_FILE),
        Vec::new(),
    )?;
    insert_scaffold_file(&mut files, config.project.index.clone(), Vec::new())?;
    insert_scaffold_file(&mut files, config.project.roadmap.clone(), Vec::new())?;
    for (relative, contents) in templates.files {
        insert_scaffold_file(
            &mut files,
            config.project.templates.join(relative),
            contents,
        )?;
    }
    for path in files.keys() {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            insert_directory_with_parents(&mut directories, parent);
        }
    }

    for path in files.keys() {
        if directories.contains(path) {
            return Err(InitError::Resolution(ResolveError::Configuration(format!(
                "configured initialization target {} is both a file and directory",
                path.display()
            ))));
        }
    }
    let file_paths = files.keys().collect::<Vec<_>>();
    for (index, path) in file_paths.iter().enumerate() {
        if let Some(other) = file_paths
            .iter()
            .skip(index + 1)
            .find(|other| other.starts_with(path) || path.starts_with(other))
        {
            return Err(InitError::Resolution(ResolveError::Configuration(format!(
                "configured initialization files {} and {} overlap",
                path.display(),
                other.display()
            ))));
        }
    }

    let mut directories = directories.into_iter().collect::<Vec<_>>();
    directories.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });

    Ok(ScaffoldPlan {
        directories,
        files: files.into_iter().collect(),
        template_files,
    })
}

fn insert_directory_with_parents(directories: &mut BTreeSet<PathBuf>, path: &Path) {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        directories.insert(current.clone());
    }
}

fn insert_scaffold_file(
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
    path: PathBuf,
    contents: Vec<u8>,
) -> Result<(), InitError> {
    if files.insert(path.clone(), contents).is_some() {
        Err(InitError::Resolution(ResolveError::Configuration(format!(
            "configured initialization file {} is duplicated",
            path.display()
        ))))
    } else {
        Ok(())
    }
}

fn create_scaffold(
    project_dir: &Path,
    scaffold: &ScaffoldPlan,
    created: &mut CreatedPaths,
    publication_hook: &mut impl FnMut(InitPublicationStage),
) -> Result<(), InitError> {
    create_directory(project_dir)?;
    created.directories.push(project_dir.to_path_buf());
    sync_parent(project_dir, "sync an initialization directory")?;
    publication_hook(InitPublicationStage::ScaffoldDirectory);

    for relative in &scaffold.directories {
        let path = project_dir.join(relative);
        create_directory(&path)?;
        created.directories.push(path.clone());
        sync_parent(&path, "sync an initialization directory")?;
        publication_hook(InitPublicationStage::ScaffoldDirectory);
    }
    for (relative, contents) in &scaffold.files {
        let path = project_dir.join(relative);
        create_file_atomically(&path, contents)?;
        created.files.push((path.clone(), contents.clone()));
        sync_parent(&path, "sync an initialization file")?;
        publication_hook(InitPublicationStage::ScaffoldFile);
    }
    Ok(())
}

fn create_directory(path: &Path) -> Result<(), InitError> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => Err(InitError::Conflict {
            path: path.to_path_buf(),
            message: "directory was created concurrently".to_owned(),
        }),
        Err(source) => Err(InitError::FileSystem {
            operation: "create an initialization directory",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn sync_parent(path: &Path, operation: &'static str) -> Result<(), InitError> {
    let parent = path
        .parent()
        .expect("an initialization target always has a parent");
    sync_directory(parent).map_err(|source| InitError::FileSystem {
        operation,
        path: parent.to_path_buf(),
        source,
    })
}

fn read_registry_snapshot(path: &Path) -> Result<(Vec<u8>, ProjectRegistry), InitError> {
    let source = fs::read(path).map_err(|source| InitError::FileSystem {
        operation: "read the project registry",
        path: path.to_path_buf(),
        source,
    })?;
    let text = str::from_utf8(&source).map_err(|source| {
        InitError::Resolution(ResolveError::Validation {
            path: path.to_path_buf(),
            source: Box::new(ValidationError::InvalidUtf8(source)),
        })
    })?;
    let registry = parse_project_registry(text).map_err(|source| {
        InitError::Resolution(ResolveError::Validation {
            path: path.to_path_buf(),
            source: Box::new(source),
        })
    })?;
    Ok((source, registry))
}

fn write_init_journal(
    prepared: &PreparedInit,
    project: &str,
    pointer_after: &[u8],
    registry_before: &[u8],
    registry_after: &[u8],
) -> Result<(PathBuf, Vec<u8>), InitError> {
    let journal = InitJournal {
        schema_version: INIT_JOURNAL_SCHEMA_VERSION,
        project: project.to_owned(),
        project_dir: prepared.project_dir.clone(),
        repository_dir: prepared.repository_dir.clone(),
        directories: prepared.scaffold.directories.clone(),
        files: prepared
            .scaffold
            .files
            .iter()
            .map(|(path, source)| InitJournalFile {
                path: path.clone(),
                after: content_fingerprint(source),
            })
            .collect(),
        pointer_after: content_fingerprint(pointer_after),
        registry_before: content_fingerprint(registry_before),
        registry_after: content_fingerprint(registry_after),
        template_files: prepared.scaffold.template_files,
    };
    let path = init_journal_path(&prepared.registry);
    let mut source =
        serde_json::to_vec_pretty(&journal).map_err(|error| InitError::Validation {
            path: path.clone(),
            message: format!("could not serialize initialization recovery journal: {error}"),
        })?;
    source.push(b'\n');
    create_file_atomically(&path, &source)?;
    sync_parent(&path, "sync the initialization recovery journal")?;
    Ok((path, source))
}

fn recover_init_locked(
    target: &InitTarget,
) -> Result<(InitRecovery, Option<InitJournal>), InitError> {
    let path = init_journal_path(&target.registry);
    let Some((journal, source)) = read_init_journal(&path)? else {
        return Ok((InitRecovery::None, None));
    };
    validate_init_journal(target, &path, &journal)?;

    let registry = read_regular_init_file(&target.registry, "read the journaled project registry")?;
    let registry_fingerprint = content_fingerprint(&registry);
    let pointer = journal.repository_dir.join(POINTER_FILE);
    let (recovery, finalized) = if registry_fingerprint == journal.registry_before {
        let changed = preflight_uncommitted_init(&path, &journal, &pointer)?;
        rollback_uncommitted_init(&path, &journal, &pointer)?;
        if changed {
            (InitRecovery::RolledBack, None)
        } else {
            (InitRecovery::Discarded, None)
        }
    } else if registry_fingerprint == journal.registry_after {
        verify_committed_init(&path, &journal, &pointer)?;
        (InitRecovery::Finalized, Some(journal))
    } else {
        return Err(InitError::Conflict {
            path,
            message:
                "journaled project registry contains unexpected bytes; automatic recovery refused"
                    .to_owned(),
        });
    };
    complete_init_journal(&init_journal_path(&target.registry), &source)?;
    Ok((recovery, finalized))
}

fn read_init_journal(path: &Path) -> Result<Option<(InitJournal, Vec<u8>)>, InitError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(InitError::FileSystem {
                operation: "inspect the initialization recovery journal",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(InitError::Conflict {
            path: path.to_path_buf(),
            message: "initialization recovery journal is not a regular file".to_owned(),
        });
    }
    let source = fs::read(path).map_err(|source| InitError::FileSystem {
        operation: "read the initialization recovery journal",
        path: path.to_path_buf(),
        source,
    })?;
    let journal =
        serde_json::from_slice::<InitJournal>(&source).map_err(|error| InitError::Validation {
            path: path.to_path_buf(),
            message: format!("invalid initialization recovery journal: {error}"),
        })?;
    Ok(Some((journal, source)))
}

fn validate_init_journal(
    target: &InitTarget,
    path: &Path,
    journal: &InitJournal,
) -> Result<(), InitError> {
    if journal.schema_version != INIT_JOURNAL_SCHEMA_VERSION {
        return Err(InitError::Validation {
            path: path.to_path_buf(),
            message: format!(
                "invalid initialization journal schema_version {}; expected {INIT_JOURNAL_SCHEMA_VERSION}",
                journal.schema_version
            ),
        });
    }
    validate_slug(&journal.project).map_err(|error| InitError::Validation {
        path: path.to_path_buf(),
        message: format!("invalid journaled project identity: {error}"),
    })?;
    let expected_project_dir = target.projects_dir.join(&journal.project);
    if journal.project_dir != expected_project_dir {
        return Err(InitError::Validation {
            path: path.to_path_buf(),
            message: "journaled project directory does not match the configured projects folder and project slug"
                .to_owned(),
        });
    }
    if !journal.repository_dir.is_absolute()
        || journal.repository_dir.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(InitError::Validation {
            path: path.to_path_buf(),
            message: "journaled repository directory must be an absolute normalized path"
                .to_owned(),
        });
    }

    let mut directories = BTreeSet::new();
    for directory in &journal.directories {
        validate_journal_relative_path(path, directory, "directory")?;
        if !directories.insert(directory.clone()) {
            return Err(InitError::Validation {
                path: path.to_path_buf(),
                message: "initialization journal contains a duplicate directory".to_owned(),
            });
        }
    }
    let mut files = BTreeSet::new();
    for file in &journal.files {
        validate_journal_relative_path(path, &file.path, "file")?;
        validate_init_fingerprint(path, &file.after)?;
        if directories.contains(&file.path) || !files.insert(file.path.clone()) {
            return Err(InitError::Validation {
                path: path.to_path_buf(),
                message: "initialization journal contains a duplicate or overlapping file"
                    .to_owned(),
            });
        }
    }
    for file in &files {
        if files
            .iter()
            .chain(directories.iter())
            .any(|other| other != file && other.starts_with(file))
        {
            return Err(InitError::Validation {
                path: path.to_path_buf(),
                message:
                    "initialization journal contains a file that is an ancestor of another target"
                        .to_owned(),
            });
        }
    }
    for fingerprint in [
        &journal.pointer_after,
        &journal.registry_before,
        &journal.registry_after,
    ] {
        validate_init_fingerprint(path, fingerprint)?;
    }
    Ok(())
}

fn validate_journal_relative_path(
    journal_path: &Path,
    path: &Path,
    kind: &str,
) -> Result<(), InitError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(InitError::Validation {
            path: journal_path.to_path_buf(),
            message: format!(
                "journaled initialization {kind} must be a non-empty normalized relative path"
            ),
        });
    }
    Ok(())
}

fn validate_init_fingerprint(path: &Path, fingerprint: &str) -> Result<(), InitError> {
    let valid = fingerprint.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    });
    if valid {
        Ok(())
    } else {
        Err(InitError::Validation {
            path: path.to_path_buf(),
            message: "initialization journal fingerprint must use sha256 followed by 64 lowercase hexadecimal digits"
                .to_owned(),
        })
    }
}

fn preflight_uncommitted_init(
    journal_path: &Path,
    journal: &InitJournal,
    pointer: &Path,
) -> Result<bool, InitError> {
    let repository_exists =
        verify_journal_repository(journal_path, &journal.repository_dir, false)?;
    let mut changed = repository_exists
        && inspect_optional_init_file(
            journal_path,
            pointer,
            &journal.pointer_after,
            "repository pointer",
        )?;
    let expected_directories = journal
        .directories
        .iter()
        .map(|path| journal.project_dir.join(path))
        .chain(std::iter::once(journal.project_dir.clone()))
        .collect::<BTreeSet<_>>();
    let expected_files = journal
        .files
        .iter()
        .map(|file| (journal.project_dir.join(&file.path), file.after.as_str()))
        .collect::<BTreeMap<_, _>>();

    let project_metadata = match fs::symlink_metadata(&journal.project_dir) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(changed),
        Err(source) => {
            return Err(InitError::FileSystem {
                operation: "inspect the journaled project directory",
                path: journal.project_dir.clone(),
                source,
            });
        }
    };
    if project_metadata.file_type().is_symlink() || !project_metadata.is_dir() {
        return Err(unexpected_init_bytes(journal_path));
    }
    changed = true;
    preflight_init_tree(
        journal_path,
        &journal.project_dir,
        &expected_directories,
        &expected_files,
    )?;
    Ok(changed)
}

fn preflight_init_tree(
    journal_path: &Path,
    directory: &Path,
    expected_directories: &BTreeSet<PathBuf>,
    expected_files: &BTreeMap<PathBuf, &str>,
) -> Result<(), InitError> {
    let entries = fs::read_dir(directory)
        .map_err(|source| InitError::FileSystem {
            operation: "read a journaled initialization directory",
            path: directory.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| InitError::FileSystem {
            operation: "read a journaled initialization entry",
            path: directory.to_path_buf(),
            source,
        })?;
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|source| InitError::FileSystem {
            operation: "inspect a journaled initialization entry",
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(unexpected_init_bytes(journal_path));
        }
        if metadata.is_dir() {
            if !expected_directories.contains(&path) {
                return Err(unexpected_init_bytes(journal_path));
            }
            preflight_init_tree(journal_path, &path, expected_directories, expected_files)?;
        } else if metadata.is_file() {
            let Some(expected) = expected_files.get(&path) else {
                return Err(unexpected_init_bytes(journal_path));
            };
            let current = fs::read(&path).map_err(|source| InitError::FileSystem {
                operation: "read a journaled initialization file",
                path: path.clone(),
                source,
            })?;
            if content_fingerprint(&current) != **expected {
                return Err(unexpected_init_bytes(journal_path));
            }
        } else {
            return Err(unexpected_init_bytes(journal_path));
        }
    }
    Ok(())
}

fn rollback_uncommitted_init(
    journal_path: &Path,
    journal: &InitJournal,
    pointer: &Path,
) -> Result<(), InitError> {
    remove_init_file_if_unchanged(
        journal_path,
        pointer,
        &journal.pointer_after,
        "repository pointer",
    )?;
    for file in journal.files.iter().rev() {
        remove_init_file_if_unchanged(
            journal_path,
            &journal.project_dir.join(&file.path),
            &file.after,
            "project scaffold file",
        )?;
    }
    for directory in journal.directories.iter().rev() {
        remove_init_directory(journal_path, &journal.project_dir.join(directory))?;
    }
    remove_init_directory(journal_path, &journal.project_dir)?;
    sync_directory(
        journal
            .project_dir
            .parent()
            .expect("a journaled project directory always has a parent"),
    )
    .map_err(|source| InitError::FileSystem {
        operation: "sync initialization recovery rollback",
        path: journal
            .project_dir
            .parent()
            .expect("a journaled project directory always has a parent")
            .to_path_buf(),
        source,
    })?;
    Ok(())
}

fn verify_committed_init(
    journal_path: &Path,
    journal: &InitJournal,
    pointer: &Path,
) -> Result<(), InitError> {
    verify_journal_repository(journal_path, &journal.repository_dir, true)?;
    require_init_directory(journal_path, &journal.project_dir)?;
    for directory in &journal.directories {
        require_init_directory(journal_path, &journal.project_dir.join(directory))?;
    }
    for file in &journal.files {
        if !inspect_optional_init_file(
            journal_path,
            &journal.project_dir.join(&file.path),
            &file.after,
            "project scaffold file",
        )? {
            return Err(unexpected_init_bytes(journal_path));
        }
    }
    if !inspect_optional_init_file(
        journal_path,
        pointer,
        &journal.pointer_after,
        "repository pointer",
    )? {
        return Err(unexpected_init_bytes(journal_path));
    }
    Ok(())
}

fn verify_journal_repository(
    journal_path: &Path,
    repository: &Path,
    required: bool,
) -> Result<bool, InitError> {
    let metadata = match fs::symlink_metadata(repository) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound && !required => return Ok(false),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(unexpected_init_bytes(journal_path));
        }
        Err(source) => {
            return Err(InitError::FileSystem {
                operation: "inspect the journaled repository directory",
                path: repository.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(unexpected_init_bytes(journal_path));
    }
    let canonical = fs::canonicalize(repository).map_err(|source| InitError::FileSystem {
        operation: "resolve the journaled repository directory",
        path: repository.to_path_buf(),
        source,
    })?;
    if canonical != repository {
        return Err(unexpected_init_bytes(journal_path));
    }
    Ok(true)
}

fn inspect_optional_init_file(
    journal_path: &Path,
    path: &Path,
    expected: &str,
    description: &str,
) -> Result<bool, InitError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(InitError::FileSystem {
                operation: "inspect a journaled initialization file",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(unexpected_init_bytes(journal_path));
    }
    let current = fs::read(path).map_err(|source| InitError::FileSystem {
        operation: "read a journaled initialization file",
        path: path.to_path_buf(),
        source,
    })?;
    if content_fingerprint(&current) != expected {
        return Err(InitError::Conflict {
            path: journal_path.to_path_buf(),
            message: format!(
                "journaled {description} contains unexpected bytes; automatic recovery refused"
            ),
        });
    }
    Ok(true)
}

fn remove_init_file_if_unchanged(
    journal_path: &Path,
    path: &Path,
    expected: &str,
    description: &str,
) -> Result<(), InitError> {
    if !inspect_optional_init_file(journal_path, path, expected, description)? {
        return Ok(());
    }
    fs::remove_file(path).map_err(|source| InitError::FileSystem {
        operation: "remove a journaled initialization file",
        path: path.to_path_buf(),
        source,
    })?;
    if description == "repository pointer" {
        sync_parent(path, "sync initialization pointer recovery")?;
    }
    Ok(())
}

fn require_init_directory(journal_path: &Path, path: &Path) -> Result<(), InitError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            unexpected_init_bytes(journal_path)
        } else {
            InitError::FileSystem {
                operation: "inspect a journaled initialization directory",
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(unexpected_init_bytes(journal_path));
    }
    Ok(())
}

fn remove_init_directory(journal_path: &Path, path: &Path) -> Result<(), InitError> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::DirectoryNotEmpty => {
            Err(unexpected_init_bytes(journal_path))
        }
        Err(source) => Err(InitError::FileSystem {
            operation: "remove a journaled initialization directory",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn read_regular_init_file(path: &Path, operation: &'static str) -> Result<Vec<u8>, InitError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| InitError::FileSystem {
        operation,
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(InitError::Conflict {
            path: path.to_path_buf(),
            message: "journaled initialization target is not a regular file".to_owned(),
        });
    }
    fs::read(path).map_err(|source| InitError::FileSystem {
        operation,
        path: path.to_path_buf(),
        source,
    })
}

fn complete_init_journal(path: &Path, expected: &[u8]) -> Result<(), InitError> {
    let current = read_regular_init_file(path, "read the initialization journal for cleanup")?;
    if current != expected {
        return Err(InitError::Conflict {
            path: path.to_path_buf(),
            message: "initialization recovery journal changed before cleanup".to_owned(),
        });
    }
    fs::remove_file(path).map_err(|source| InitError::FileSystem {
        operation: "remove the completed initialization recovery journal",
        path: path.to_path_buf(),
        source,
    })?;
    sync_parent(path, "sync initialization journal cleanup")
}

fn init_journal_path(registry: &Path) -> PathBuf {
    let file_name = registry
        .file_name()
        .expect("a canonical registry always has a filename");
    let mut journal_name = OsString::from(".");
    journal_name.push(file_name);
    journal_name.push(".akasha-init-journal.json");
    registry
        .parent()
        .expect("a canonical registry always has a parent")
        .join(journal_name)
}

fn unexpected_init_bytes(journal_path: &Path) -> InitError {
    InitError::Conflict {
        path: journal_path.to_path_buf(),
        message: "journaled initialization targets contain unexpected paths or bytes; automatic recovery refused"
            .to_owned(),
    }
}

fn render_registry(registry: &ProjectRegistry, path: &Path) -> Result<Vec<u8>, InitError> {
    let mut rendered = String::new();
    for (slug, entry) in &registry.projects {
        let repository = entry.path.to_str().ok_or_else(|| InitError::Validation {
            path: path.to_path_buf(),
            message: format!("registry path for project {slug:?} is not valid UTF-8"),
        })?;
        let repository =
            serde_json::to_string(repository).map_err(|source| InitError::FileSystem {
                operation: "render a registry repository path",
                path: path.to_path_buf(),
                source: io::Error::other(source),
            })?;
        let status =
            serde_json::to_string(&entry.status).map_err(|source| InitError::FileSystem {
                operation: "render a registry status",
                path: path.to_path_buf(),
                source: io::Error::other(source),
            })?;
        rendered.push_str(slug);
        rendered.push_str(":\n  path: ");
        rendered.push_str(&repository);
        rendered.push_str("\n  status: ");
        rendered.push_str(&status);
        rendered.push('\n');
    }

    let reparsed = parse_project_registry(&rendered).map_err(|source| {
        InitError::Resolution(ResolveError::Validation {
            path: path.to_path_buf(),
            source: Box::new(source),
        })
    })?;
    if &reparsed != registry {
        return Err(InitError::FileSystem {
            operation: "verify deterministic registry rendering",
            path: path.to_path_buf(),
            source: io::Error::other("rendered registry changed its typed values"),
        });
    }
    Ok(rendered.into_bytes())
}

fn replace_registry_if_unchanged(
    path: &Path,
    expected: &[u8],
    replacement: &[u8],
) -> Result<(), InitError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| InitError::FileSystem {
        operation: "inspect the project registry before replacement",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(InitError::Conflict {
            path: path.to_path_buf(),
            message: "project registry is no longer the regular file that was preflighted"
                .to_owned(),
        });
    }

    let current = fs::read(path).map_err(|source| InitError::FileSystem {
        operation: "reread the project registry before replacement",
        path: path.to_path_buf(),
        source,
    })?;
    if current != expected {
        return Err(InitError::Conflict {
            path: path.to_path_buf(),
            message: "project registry changed during initialization".to_owned(),
        });
    }

    let stage = create_registry_stage(path, replacement)?;
    fs::set_permissions(&stage.path, metadata.permissions()).map_err(|source| {
        InitError::FileSystem {
            operation: "preserve project registry permissions",
            path: stage.path.clone(),
            source,
        }
    })?;
    File::open(&stage.path)
        .and_then(|file| file.sync_all())
        .map_err(|source| InitError::FileSystem {
            operation: "sync staged project registry permissions",
            path: stage.path.clone(),
            source,
        })?;

    let current = fs::read(path).map_err(|source| InitError::FileSystem {
        operation: "verify the project registry before replacement",
        path: path.to_path_buf(),
        source,
    })?;
    if current != expected {
        return Err(InitError::Conflict {
            path: path.to_path_buf(),
            message: "project registry changed during initialization".to_owned(),
        });
    }

    fs::rename(&stage.path, path).map_err(|source| InitError::FileSystem {
        operation: "atomically publish the project registry",
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

fn create_registry_stage(path: &Path, contents: &[u8]) -> Result<RegistryStage, InitError> {
    let parent = path
        .parent()
        .expect("a canonical registry always has a parent");
    let file_name = path
        .file_name()
        .expect("a canonical registry always has a filename");
    for _ in 0..MAX_REGISTRY_STAGING_ATTEMPTS {
        let id = NEXT_REGISTRY_STAGE_ID.fetch_add(1, Ordering::Relaxed);
        let mut stage_name = OsString::from(".");
        stage_name.push(file_name);
        stage_name.push(format!(
            ".akasha-init-{}-{id}.replacement",
            std::process::id()
        ));
        let stage = parent.join(stage_name);
        match create_file_atomically(&stage, contents) {
            Ok(()) => return Ok(RegistryStage { path: stage }),
            Err(AtomicCreateError::Conflict { .. }) => continue,
            Err(error) => return Err(InitError::Creation(error)),
        }
    }
    Err(InitError::FileSystem {
        operation: "create a unique staged project registry",
        path: path.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "all registry staging filename attempts were occupied",
        ),
    })
}

struct RegistryStage {
    path: PathBuf,
}

impl Drop for RegistryStage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct InitLock {
    _file: File,
}

impl InitLock {
    fn acquire(registry: &Path) -> Result<Self, InitError> {
        let file_name = registry
            .file_name()
            .expect("a canonical registry always has a filename");
        let mut lock_name = OsString::from(".");
        lock_name.push(file_name);
        lock_name.push(".akasha-init.lock");
        let path = registry
            .parent()
            .expect("a canonical registry always has a parent")
            .join(lock_name);
        match create_file_atomically(&path, b"akasha init\n") {
            Ok(()) => sync_parent(&path, "sync the initialization lock file")?,
            Err(error @ AtomicCreateError::Conflict { .. }) => {
                let metadata =
                    fs::symlink_metadata(&path).map_err(|source| InitError::FileSystem {
                        operation: "inspect the initialization lock file",
                        path: path.clone(),
                        source,
                    })?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(InitError::Creation(error));
                }
            }
            Err(error) => return Err(InitError::Creation(error)),
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|source| InitError::FileSystem {
                operation: "open the initialization lock file",
                path: path.clone(),
                source,
            })?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(InitError::Conflict {
                path,
                message: "another Akasha initializer holds the registry lock".to_owned(),
            }),
            Err(TryLockError::Error(source)) => Err(InitError::FileSystem {
                operation: "acquire the initialization registry lock",
                path,
                source,
            }),
        }
    }
}

#[derive(Default)]
struct CreatedPaths {
    files: Vec<(PathBuf, Vec<u8>)>,
    directories: Vec<PathBuf>,
}

impl CreatedPaths {
    fn rollback(&mut self) -> Vec<String> {
        let mut failures = Vec::new();
        for (path, expected) in self.files.iter().rev() {
            match fs::read(path) {
                Ok(current) if current == *expected => {
                    if let Err(source) = fs::remove_file(path) {
                        failures.push(format!("could not remove {}: {source}", path.display()));
                    } else if let Some(parent) = path.parent()
                        && let Err(source) = sync_directory(parent)
                    {
                        failures.push(format!(
                            "could not sync rollback directory {}: {source}",
                            parent.display()
                        ));
                    }
                }
                Ok(_) => failures.push(format!(
                    "did not remove changed created file {}",
                    path.display()
                )),
                Err(source) if source.kind() == io::ErrorKind::NotFound => {}
                Err(source) => failures.push(format!(
                    "could not verify created file {}: {source}",
                    path.display()
                )),
            }
        }
        for path in self.directories.iter().rev() {
            match fs::remove_dir(path) {
                Ok(()) => {
                    if let Some(parent) = path.parent()
                        && let Err(source) = sync_directory(parent)
                    {
                        failures.push(format!(
                            "could not sync rollback directory {}: {source}",
                            parent.display()
                        ));
                    }
                }
                Err(source) if source.kind() == io::ErrorKind::NotFound => {}
                Err(source) => {
                    failures.push(format!("could not remove {}: {source}", path.display()))
                }
            }
        }
        failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn rolls_back_owned_scaffold_when_pointer_appears_concurrently() {
        let fixture = Fixture::new("pointer-race");
        let pointer = fixture.repository.join(POINTER_FILE);
        let request = fixture.request();

        let error = initialize_project_with_hooks(
            &request,
            || fs::write(&pointer, b"external pointer\n").expect("create concurrent pointer"),
            || {},
            |_| {},
        )
        .expect_err("concurrent pointer must fail");

        assert_eq!(error.exit_code(), 5);
        assert_eq!(
            fs::read(&pointer).expect("preserve pointer"),
            b"external pointer\n"
        );
        assert!(!fixture.root.join("Projects/example").exists());
        assert_eq!(
            fs::read_to_string(fixture.root.join("Meta/projects.yaml")).expect("read registry"),
            "{}\n"
        );
    }

    #[test]
    fn rolls_back_when_registry_changes_before_commit() {
        let fixture = Fixture::new("registry-race");
        let registry = fixture.root.join("Meta/projects.yaml");
        let request = fixture.request();

        let error = initialize_project_with_hooks(
            &request,
            || {},
            || {
                fs::write(&registry, "other:\n  path: /tmp\n  status: active\n")
                    .expect("change registry")
            },
            |_| {},
        )
        .expect_err("changed registry must fail");

        assert_eq!(error.exit_code(), 5);
        assert!(!fixture.repository.join(POINTER_FILE).exists());
        assert!(!fixture.root.join("Projects/example").exists());
        assert!(
            fs::read_to_string(registry)
                .expect("read registry")
                .starts_with("other:")
        );
    }

    #[test]
    fn recovers_every_uncommitted_publication_stage_and_releases_the_advisory_lock() {
        for (label, interrupted_stage, expected_recovery) in [
            (
                "journal",
                InitPublicationStage::Journal,
                InitRecovery::Discarded,
            ),
            (
                "directory",
                InitPublicationStage::ScaffoldDirectory,
                InitRecovery::RolledBack,
            ),
            (
                "file",
                InitPublicationStage::ScaffoldFile,
                InitRecovery::RolledBack,
            ),
            (
                "pointer",
                InitPublicationStage::Pointer,
                InitRecovery::RolledBack,
            ),
        ] {
            let fixture = Fixture::new(label);
            let request = fixture.request();
            let interrupted = catch_unwind(AssertUnwindSafe(|| {
                let _ = initialize_project_with_hooks(
                    &request,
                    || {},
                    || {},
                    |stage| assert_ne!(stage, interrupted_stage, "simulated process interruption"),
                );
            }));
            assert!(interrupted.is_err());
            let journal = init_journal_path(&fixture.root.join("Meta/projects.yaml"));
            assert!(journal.is_file());

            let result = initialize_project(&request).expect("recover and initialize project");

            assert_eq!(result.recovery, expected_recovery);
            assert!(!journal.exists());
            assert!(result.project_dir.is_dir());
            assert!(result.pointer.is_file());
            assert!(
                fixture
                    .root
                    .join("Meta/.projects.yaml.akasha-init.lock")
                    .is_file()
            );
        }
    }

    #[test]
    fn finalizes_a_registry_committed_interruption_as_a_successful_exact_rerun() {
        let fixture = Fixture::new("committed");
        let request = fixture.request();
        let interrupted = catch_unwind(AssertUnwindSafe(|| {
            let _ = initialize_project_with_hooks(
                &request,
                || {},
                || {},
                |stage| assert_ne!(stage, InitPublicationStage::Registry, "interrupt"),
            );
        }));
        assert!(interrupted.is_err());

        let result = initialize_project(&request).expect("finalize committed exact rerun");

        assert_eq!(result.recovery, InitRecovery::Finalized);
        assert!(!init_journal_path(&result.registry).exists());
        let registry = fs::read_to_string(result.registry).expect("read recovered registry");
        assert!(registry.starts_with("example:"));
        assert_eq!(registry.matches("example:").count(), 1);
    }

    #[test]
    fn refuses_recovery_when_a_partial_scaffold_contains_unexpected_bytes() {
        let fixture = Fixture::new("unexpected");
        let request = fixture.request();
        let interrupted = catch_unwind(AssertUnwindSafe(|| {
            let _ = initialize_project_with_hooks(
                &request,
                || {},
                || {},
                |stage| assert_ne!(stage, InitPublicationStage::Pointer, "interrupt"),
            );
        }));
        assert!(interrupted.is_err());
        let index = fixture.root.join("Projects/example/index.md");
        fs::write(&index, b"human change\n").expect("change partial scaffold");

        let error = initialize_project(&request).expect_err("unexpected bytes must fail closed");

        assert_eq!(error.exit_code(), 5);
        assert!(error.to_string().contains("unexpected paths or bytes"));
        assert_eq!(
            fs::read(index).expect("preserve changed file"),
            b"human change\n"
        );
        assert!(init_journal_path(&fixture.root.join("Meta/projects.yaml")).is_file());
        assert!(fixture.repository.join(POINTER_FILE).is_file());
    }

    struct Fixture {
        base: PathBuf,
        root: PathBuf,
        repository: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
            let base = std::env::temp_dir().join(format!(
                "akasha-init-unit-{label}-{}-{id}",
                std::process::id()
            ));
            let root = base.join("root");
            let repository = base.join("repository");
            for directory in [
                root.join("Meta"),
                root.join("templates"),
                root.join("Global"),
                root.join("Projects"),
                root.join("Inbox"),
                repository.clone(),
            ] {
                fs::create_dir_all(directory).expect("create fixture directory");
            }
            fs::write(
                root.join("akasha.toml"),
                include_str!("../../../tests/fixtures/resolution/valid-root/akasha.toml"),
            )
            .expect("write root config");
            fs::write(root.join("Meta/projects.yaml"), "{}\n").expect("write registry");
            Self {
                base,
                root,
                repository,
            }
        }

        fn request(&self) -> InitRequest {
            InitRequest {
                root_override: Some(self.root.clone()),
                project: "example".to_owned(),
                cwd: self.repository.clone(),
                environment: ResolutionEnvironment::default(),
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }
}
