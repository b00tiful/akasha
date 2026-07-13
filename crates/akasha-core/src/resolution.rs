use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::validation::{ProjectRegistry, ValidationError, parse_project_registry};

pub(crate) const CONFIG_SCHEMA_VERSION: u32 = 1;
const ROOT_CONFIG_FILE: &str = "akasha.toml";
pub(crate) const POINTER_FILE: &str = ".akasha.toml";

/// Environment values used by project resolution.
///
/// Keeping them explicit makes precedence deterministic and testable without mutating the
/// process environment.
#[derive(Debug, Clone, Default)]
pub struct ResolutionEnvironment {
    pub akasha_root: Option<OsString>,
    pub xdg_config_home: Option<OsString>,
    pub home: Option<OsString>,
}

impl ResolutionEnvironment {
    #[must_use]
    pub fn from_process() -> Self {
        Self {
            akasha_root: env::var_os("AKASHA_ROOT"),
            xdg_config_home: env::var_os("XDG_CONFIG_HOME"),
            home: env::var_os("HOME"),
        }
    }
}

/// Inputs required to resolve one Akasha project.
#[derive(Debug, Clone)]
pub struct ResolveRequest {
    pub root_override: Option<PathBuf>,
    pub project_override: Option<String>,
    pub cwd: PathBuf,
    pub environment: ResolutionEnvironment,
}

impl ResolveRequest {
    /// Build a request from CLI overrides and the current process.
    pub fn from_process(
        root_override: Option<PathBuf>,
        project_override: Option<String>,
    ) -> Result<Self, ResolveError> {
        let cwd = env::current_dir().map_err(|source| ResolveError::FileSystem {
            operation: "read the current directory",
            path: PathBuf::from("."),
            source,
        })?;

        Ok(Self {
            root_override,
            project_override,
            cwd,
            environment: ResolutionEnvironment::from_process(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RootSource {
    CommandLine,
    Environment,
    UserConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectSource {
    CommandLine,
    Pointer,
}

/// Fully validated project identity returned to every Akasha adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedProject {
    pub root: PathBuf,
    pub root_source: RootSource,
    pub project: String,
    pub project_source: ProjectSource,
    pub pointer: Option<PathBuf>,
    pub registry: PathBuf,
    pub repository_dir: PathBuf,
    pub project_dir: PathBuf,
}

#[derive(Debug)]
pub enum ResolveError {
    Configuration(String),
    Validation {
        path: PathBuf,
        source: Box<ValidationError>,
    },
    FileSystem {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl ResolveError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Configuration(_) => 3,
            Self::Validation { source, .. } => source.exit_code(),
            Self::FileSystem { .. } => 6,
        }
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(message) => formatter.write_str(message),
            Self::Validation { path, source } => {
                write!(
                    formatter,
                    "validation failed at {}: {source}",
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

impl std::error::Error for ResolveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Configuration(_) => None,
            Self::Validation { source, .. } => Some(source.as_ref()),
            Self::FileSystem { source, .. } => Some(source),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UserConfig {
    schema_version: u32,
    root: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RootConfig {
    pub(crate) schema_version: u32,
    pub(crate) files: FileConfig,
    pub(crate) folders: FolderConfig,
    pub(crate) context: ContextConfig,
    pub(crate) project: ProjectLayoutConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileConfig {
    pub(crate) registry: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FolderConfig {
    pub(crate) templates: PathBuf,
    pub(crate) global: PathBuf,
    pub(crate) projects: PathBuf,
    pub(crate) inbox: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextConfig {
    pub(crate) tasks: String,
    pub(crate) problems: String,
    pub(crate) handoffs: String,
    pub(crate) recent_events: Vec<String>,
    pub(crate) open_statuses: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectLayoutConfig {
    pub(crate) templates: PathBuf,
    pub(crate) index: PathBuf,
    pub(crate) roadmap: PathBuf,
    pub(crate) note_types: BTreeMap<String, NoteTypeConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoteTypeConfig {
    pub(crate) class: NoteClass,
    pub(crate) folder: PathBuf,
    pub(crate) required_fields: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteClass {
    Event,
    Record,
    Entity,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectPointer {
    schema_version: u32,
    project: String,
}

/// Resolve and validate an Akasha data root and project without guessing either identity.
pub fn resolve_project(request: &ResolveRequest) -> Result<ResolvedProject, ResolveError> {
    let (root, root_source) = resolve_root(request)?;
    let root_config = load_root_config(&root)?;

    let (project, project_source, pointer) = match &request.project_override {
        Some(project) => {
            validate_slug(project)?;
            (project.clone(), ProjectSource::CommandLine, None)
        }
        None => {
            let pointer_path = find_nearest_pointer(&request.cwd)?;
            let pointer_config: ProjectPointer = read_toml(&pointer_path, "project pointer")?;
            require_schema_version(pointer_config.schema_version, &pointer_path)?;
            validate_slug(&pointer_config.project)?;
            (
                pointer_config.project,
                ProjectSource::Pointer,
                Some(pointer_path),
            )
        }
    };

    let (registry_path, registry) = load_project_registry(&root, &root_config)?;
    let registry_entry = registry.projects.get(&project).ok_or_else(|| {
        ResolveError::Configuration(format!(
            "project {project:?} is not registered in {}",
            registry_path.display()
        ))
    })?;
    let repository_dir = resolve_registry_repository(
        &registry_entry.path,
        &registry_path,
        request.environment.home.as_deref(),
    )?;

    if let Some(pointer_path) = &pointer {
        let pointer_repository = canonicalize_directory(
            pointer_path
                .parent()
                .expect("a discovered project pointer always has a parent"),
            "pointer repository",
        )?;
        if pointer_repository != repository_dir {
            return Err(ResolveError::Configuration(format!(
                "project pointer {} resolves to repository {}, but registry entry {project:?} names {}",
                pointer_path.display(),
                pointer_repository.display(),
                repository_dir.display()
            )));
        }
    }

    let project_dir = canonicalize_directory(
        &root.join(&root_config.folders.projects).join(&project),
        "project directory",
    )?;
    if !project_dir.starts_with(&root) {
        return Err(ResolveError::Configuration(format!(
            "project directory {} escapes data root {}",
            project_dir.display(),
            root.display()
        )));
    }

    Ok(ResolvedProject {
        root,
        root_source,
        project,
        project_source,
        pointer,
        registry: registry_path,
        repository_dir,
        project_dir,
    })
}

pub(crate) fn load_root_config(root: &Path) -> Result<RootConfig, ResolveError> {
    let path = root.join(ROOT_CONFIG_FILE);
    let config: RootConfig = read_toml(&path, "data-root configuration")?;
    require_schema_version(config.schema_version, &path)?;
    validate_root_config(&config)?;
    Ok(config)
}

pub(crate) fn load_project_registry(
    root: &Path,
    config: &RootConfig,
) -> Result<(PathBuf, ProjectRegistry), ResolveError> {
    let configured_path = root.join(&config.files.registry);
    let path = canonicalize_file(&configured_path, "project registry")?;
    if !path.starts_with(root) {
        return Err(ResolveError::Configuration(format!(
            "project registry {} escapes data root {}",
            path.display(),
            root.display()
        )));
    }

    let source = fs::read_to_string(&path).map_err(|source| ResolveError::FileSystem {
        operation: "read project registry",
        path: path.clone(),
        source,
    })?;
    let registry = parse_project_registry(&source).map_err(|source| ResolveError::Validation {
        path: path.clone(),
        source: Box::new(source),
    })?;
    Ok((path, registry))
}

fn validate_root_config(config: &RootConfig) -> Result<(), ResolveError> {
    let root_paths = [
        ("files.registry", &config.files.registry),
        ("folders.templates", &config.folders.templates),
        ("folders.global", &config.folders.global),
        ("folders.projects", &config.folders.projects),
        ("folders.inbox", &config.folders.inbox),
        ("project.templates", &config.project.templates),
        ("project.index", &config.project.index),
        ("project.roadmap", &config.project.roadmap),
    ];
    for (field, path) in root_paths {
        validate_relative_path(path, field)?;
    }

    let folder_paths = [
        ("folders.templates", &config.folders.templates),
        ("folders.global", &config.folders.global),
        ("folders.projects", &config.folders.projects),
        ("folders.inbox", &config.folders.inbox),
    ];
    reject_duplicate_paths(&folder_paths)?;

    if config.project.index == config.project.roadmap {
        return Err(ResolveError::Configuration(
            "project.index and project.roadmap must name different files".to_owned(),
        ));
    }
    if config.project.note_types.is_empty() {
        return Err(ResolveError::Configuration(
            "project.note_types must declare at least one canonical note type".to_owned(),
        ));
    }

    let mut note_folders = Vec::new();
    for (name, note_type) in &config.project.note_types {
        if validate_slug(name).is_err() {
            return Err(ResolveError::Configuration(format!(
                "invalid note type {name:?}; use lowercase ASCII letters, digits, and hyphens"
            )));
        }
        validate_relative_path(
            &note_type.folder,
            &format!("project.note_types.{name}.folder"),
        )?;
        if note_type.required_fields.is_empty() {
            return Err(ResolveError::Configuration(format!(
                "project.note_types.{name}.required_fields must not be empty"
            )));
        }
        let mut fields = BTreeSet::new();
        for field in &note_type.required_fields {
            if field.trim().is_empty() || !fields.insert(field) {
                return Err(ResolveError::Configuration(format!(
                    "project.note_types.{name}.required_fields must contain unique non-empty names"
                )));
            }
        }
        note_folders.push((name.as_str(), &note_type.folder));
    }

    for (index, (name, path)) in note_folders.iter().enumerate() {
        if path.starts_with(&config.project.templates) || config.project.templates.starts_with(path)
        {
            return Err(ResolveError::Configuration(format!(
                "project.note_types.{name}.folder must not overlap project.templates"
            )));
        }
        for (other_name, other_path) in note_folders.iter().skip(index + 1) {
            if path.starts_with(other_path) || other_path.starts_with(path) {
                return Err(ResolveError::Configuration(format!(
                    "note type folders {name:?} and {other_name:?} must not overlap"
                )));
            }
        }
    }

    validate_context_config(config)?;

    Ok(())
}

fn validate_context_config(config: &RootConfig) -> Result<(), ResolveError> {
    if config.context.tasks == config.context.problems {
        return Err(ResolveError::Configuration(
            "context.tasks and context.problems must name different note types".to_owned(),
        ));
    }

    for (field, note_type, class, required_field) in [
        (
            "context.tasks",
            config.context.tasks.as_str(),
            NoteClass::Record,
            "status",
        ),
        (
            "context.problems",
            config.context.problems.as_str(),
            NoteClass::Record,
            "status",
        ),
        (
            "context.handoffs",
            config.context.handoffs.as_str(),
            NoteClass::Event,
            "date",
        ),
    ] {
        validate_context_role(config, field, note_type, class, required_field)?;
    }

    if config.context.recent_events.is_empty() {
        return Err(ResolveError::Configuration(
            "context.recent_events must contain at least one note type".to_owned(),
        ));
    }
    let mut recent_events = BTreeSet::new();
    for note_type in &config.context.recent_events {
        if note_type == &config.context.handoffs {
            return Err(ResolveError::Configuration(
                "context.recent_events must not repeat context.handoffs".to_owned(),
            ));
        }
        if !recent_events.insert(note_type) {
            return Err(ResolveError::Configuration(
                "context.recent_events must contain unique note types".to_owned(),
            ));
        }
        validate_context_role(
            config,
            "context.recent_events",
            note_type,
            NoteClass::Event,
            "date",
        )?;
    }

    if config.context.open_statuses.is_empty() {
        return Err(ResolveError::Configuration(
            "context.open_statuses must contain at least one status".to_owned(),
        ));
    }
    let mut open_statuses = BTreeSet::new();
    for status in &config.context.open_statuses {
        if status.trim().is_empty() || !open_statuses.insert(status) {
            return Err(ResolveError::Configuration(
                "context.open_statuses must contain unique non-empty strings".to_owned(),
            ));
        }
    }

    Ok(())
}

fn validate_context_role(
    config: &RootConfig,
    field: &str,
    name: &str,
    expected_class: NoteClass,
    required_field: &str,
) -> Result<(), ResolveError> {
    let note_type = config.project.note_types.get(name).ok_or_else(|| {
        ResolveError::Configuration(format!(
            "{field} names undefined project note type {name:?}"
        ))
    })?;
    if note_type.class != expected_class {
        return Err(ResolveError::Configuration(format!(
            "{field} must name a {:?} note type",
            expected_class
        )));
    }
    if !note_type
        .required_fields
        .iter()
        .any(|candidate| candidate == required_field)
    {
        return Err(ResolveError::Configuration(format!(
            "{field} note type {name:?} must require field {required_field:?}"
        )));
    }
    Ok(())
}

fn reject_duplicate_paths(paths: &[(&str, &PathBuf)]) -> Result<(), ResolveError> {
    for (index, (field, path)) in paths.iter().enumerate() {
        if let Some((other_field, _)) = paths
            .iter()
            .skip(index + 1)
            .find(|(_, other_path)| *other_path == *path)
        {
            return Err(ResolveError::Configuration(format!(
                "{field} and {other_field} must name different paths"
            )));
        }
    }
    Ok(())
}

fn resolve_registry_repository(
    configured: &Path,
    registry_path: &Path,
    home: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, ResolveError> {
    let expanded = match configured.strip_prefix("~") {
        Ok(remainder) => {
            let home = home.ok_or_else(|| {
                ResolveError::Configuration(format!(
                    "registry repository path {} uses ~ but HOME is not set",
                    configured.display()
                ))
            })?;
            PathBuf::from(home).join(remainder)
        }
        Err(_) if configured.is_absolute() => configured.to_path_buf(),
        Err(_) => registry_path
            .parent()
            .expect("a validated registry path always has a parent")
            .join(configured),
    };
    canonicalize_directory(&expanded, "registered repository")
}

pub(crate) fn resolve_root(
    request: &ResolveRequest,
) -> Result<(PathBuf, RootSource), ResolveError> {
    if let Some(root) = &request.root_override {
        return Ok((
            canonicalize_directory(&relative_to(root, &request.cwd), "data root")?,
            RootSource::CommandLine,
        ));
    }

    if let Some(root) = &request.environment.akasha_root {
        if root.is_empty() {
            return Err(ResolveError::Configuration(
                "AKASHA_ROOT is set but empty".to_owned(),
            ));
        }
        return Ok((
            canonicalize_directory(&relative_to(Path::new(root), &request.cwd), "data root")?,
            RootSource::Environment,
        ));
    }

    let config_home = request
        .environment
        .xdg_config_home
        .as_ref()
        .map(PathBuf::from)
        .or_else(|| {
            request
                .environment
                .home
                .as_ref()
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .ok_or_else(|| {
            ResolveError::Configuration(
                "no data root was provided and neither XDG_CONFIG_HOME nor HOME is set".to_owned(),
            )
        })?;
    let config_path = config_home.join("akasha").join("config.toml");
    let config: UserConfig = read_toml(&config_path, "user configuration")?;
    require_schema_version(config.schema_version, &config_path)?;

    let configured_root = if config.root.is_absolute() {
        config.root
    } else {
        config_path
            .parent()
            .expect("Akasha user config always has a parent")
            .join(config.root)
    };

    Ok((
        canonicalize_directory(&configured_root, "data root")?,
        RootSource::UserConfig,
    ))
}

pub(crate) fn relative_to(path: &Path, base: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn find_nearest_pointer(cwd: &Path) -> Result<PathBuf, ResolveError> {
    let mut current = canonicalize_directory(cwd, "working directory")?;

    loop {
        let candidate = current.join(POINTER_FILE);
        match fs::metadata(&candidate) {
            Ok(metadata) if metadata.is_file() => return Ok(candidate),
            Ok(_) => {
                return Err(ResolveError::Configuration(format!(
                    "project pointer {} is not a file",
                    candidate.display()
                )));
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ResolveError::FileSystem {
                    operation: "inspect project pointer",
                    path: candidate,
                    source,
                });
            }
        }

        if !current.pop() {
            return Err(ResolveError::Configuration(format!(
                "no {POINTER_FILE} found from {} upward; pass --project explicitly",
                cwd.display()
            )));
        }
    }
}

fn read_toml<T>(path: &Path, description: &str) -> Result<T, ResolveError>
where
    T: for<'de> Deserialize<'de>,
{
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(ResolveError::Configuration(format!(
                "{description} not found at {}",
                path.display()
            )));
        }
        Err(source) => {
            return Err(ResolveError::FileSystem {
                operation: "read configuration",
                path: path.to_path_buf(),
                source,
            });
        }
    };

    toml::from_str(&content).map_err(|source| {
        ResolveError::Configuration(format!(
            "invalid {description} at {}: {source}",
            path.display()
        ))
    })
}

fn require_schema_version(version: u32, path: &Path) -> Result<(), ResolveError> {
    if version == CONFIG_SCHEMA_VERSION {
        return Ok(());
    }

    Err(ResolveError::Configuration(format!(
        "unsupported schema_version {version} at {}; expected {CONFIG_SCHEMA_VERSION}",
        path.display()
    )))
}

pub(crate) fn validate_slug(slug: &str) -> Result<(), ResolveError> {
    if !slug.is_empty()
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Ok(());
    }

    Err(ResolveError::Configuration(format!(
        "invalid project slug {slug:?}; use lowercase ASCII letters, digits, and hyphens"
    )))
}

fn validate_relative_path(path: &Path, field: &str) -> Result<(), ResolveError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ResolveError::Configuration(format!(
            "{field} must be a non-empty relative path without parent traversal"
        )));
    }

    Ok(())
}

fn canonicalize_file(path: &Path, description: &str) -> Result<PathBuf, ResolveError> {
    let canonical = match fs::canonicalize(path) {
        Ok(path) => path,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(ResolveError::Configuration(format!(
                "{description} does not exist at {}",
                path.display()
            )));
        }
        Err(source) => {
            return Err(ResolveError::FileSystem {
                operation: "resolve file",
                path: path.to_path_buf(),
                source,
            });
        }
    };

    if !canonical.is_file() {
        return Err(ResolveError::Configuration(format!(
            "{description} is not a file at {}",
            canonical.display()
        )));
    }

    Ok(canonical)
}

pub(crate) fn canonicalize_directory(
    path: &Path,
    description: &str,
) -> Result<PathBuf, ResolveError> {
    let canonical = match fs::canonicalize(path) {
        Ok(path) => path,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(ResolveError::Configuration(format!(
                "{description} does not exist at {}",
                path.display()
            )));
        }
        Err(source) => {
            return Err(ResolveError::FileSystem {
                operation: "resolve directory",
                path: path.to_path_buf(),
                source,
            });
        }
    };

    if !canonical.is_dir() {
        return Err(ResolveError::Configuration(format!(
            "{description} is not a directory at {}",
            canonical.display()
        )));
    }

    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::validate_slug;

    #[test]
    fn accepts_specified_slug_alphabet() {
        for slug in ["a", "akasha", "project-2", "2-fast"] {
            assert!(validate_slug(slug).is_ok(), "slug {slug:?} should be valid");
        }
    }

    #[test]
    fn rejects_empty_or_out_of_alphabet_slugs() {
        for slug in ["", "UPPER", "under_score", "with space", "café"] {
            assert!(validate_slug(slug).is_err(), "slug {slug:?} should fail");
        }
    }
}
