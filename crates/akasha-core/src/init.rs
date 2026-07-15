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

use crate::resolution::{
    CONFIG_SCHEMA_VERSION, POINTER_FILE, ResolutionEnvironment, ResolveError, ResolveRequest,
    RootConfig, canonicalize_directory, load_project_registry, load_root_config, resolve_root,
    validate_slug,
};
use crate::state::{PROJECT_STATE_FILE, render_empty_project_state};
use crate::validation::{
    ProjectRegistry, ProjectRegistryEntry, ValidationError, parse_project_registry,
};
use crate::writes::{AtomicCreateError, PROJECT_WRITE_LOCK_FILE, create_file_atomically};

const MAX_REGISTRY_STAGING_ATTEMPTS: u64 = 128;
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
    initialize_project_with_hooks(request, || {}, || {})
}

fn initialize_project_with_hooks(
    request: &InitRequest,
    after_scaffold: impl FnOnce(),
    before_registry_commit: impl FnOnce(),
) -> Result<InitResult, InitError> {
    let prepared = prepare(request)?;
    let lock = InitLock::acquire(&prepared.registry)?;
    let result = initialize_locked(request, &prepared, after_scaffold, before_registry_commit);

    match lock.release() {
        Ok(()) => result,
        Err((path, source)) => {
            let committed = result.is_ok();
            let original = result.err().map(Box::new);
            Err(InitError::Cleanup {
                committed,
                original,
                failures: vec![format!(
                    "could not remove initialization lock {}: {source}",
                    path.display()
                )],
            })
        }
    }
}

fn prepare(request: &InitRequest) -> Result<PreparedInit, InitError> {
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

    let project_dir = projects_dir.join(&request.project);
    let pointer = repository_dir.join(POINTER_FILE);
    let (registry, projects) = load_project_registry(&root, &config)?;
    reject_registered_slug(&projects, &request.project, &registry)?;
    reject_existing_target(&project_dir, "configured project directory already exists")?;
    reject_existing_target(&pointer, "repository pointer already exists")?;

    let templates = collect_templates(&templates_dir)?;
    let scaffold = build_scaffold_plan(&config, templates)?;

    Ok(PreparedInit {
        root,
        registry,
        repository_dir,
        project_dir,
        pointer,
        scaffold,
    })
}

fn initialize_locked(
    request: &InitRequest,
    prepared: &PreparedInit,
    after_scaffold: impl FnOnce(),
    before_registry_commit: impl FnOnce(),
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

    let mut created = CreatedPaths::default();
    let operation = (|| {
        create_scaffold(&prepared.project_dir, &prepared.scaffold, &mut created)?;
        after_scaffold();

        create_file_atomically(&prepared.pointer, &pointer_contents)?;
        created
            .files
            .push((prepared.pointer.clone(), pointer_contents));
        before_registry_commit();

        replace_registry_if_unchanged(&prepared.registry, &registry_source, &registry_replacement)?;

        Ok(InitResult {
            root: prepared.root.clone(),
            project: request.project.clone(),
            registry: prepared.registry.clone(),
            repository_dir: prepared.repository_dir.clone(),
            project_dir: prepared.project_dir.clone(),
            state: prepared.project_dir.join(PROJECT_STATE_FILE),
            pointer: prepared.pointer.clone(),
            template_files: prepared.scaffold.template_files,
        })
    })();

    match operation {
        Ok(result) => Ok(result),
        Err(error) => {
            let failures = created.rollback();
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
struct PreparedInit {
    root: PathBuf,
    registry: PathBuf,
    repository_dir: PathBuf,
    project_dir: PathBuf,
    pointer: PathBuf,
    scaffold: ScaffoldPlan,
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
) -> Result<(), InitError> {
    create_directory(project_dir)?;
    created.directories.push(project_dir.to_path_buf());

    for relative in &scaffold.directories {
        let path = project_dir.join(relative);
        create_directory(&path)?;
        created.directories.push(path);
    }
    for (relative, contents) in &scaffold.files {
        let path = project_dir.join(relative);
        create_file_atomically(&path, contents)?;
        created.files.push((path, contents.clone()));
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
    path: PathBuf,
    released: bool,
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
        create_file_atomically(&path, b"akasha init\n")?;
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

impl Drop for InitLock {
    fn drop(&mut self) {
        if !self.released {
            let _ = fs::remove_file(&self.path);
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
                Ok(()) => {}
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
