use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

const CONFIG_SCHEMA_VERSION: u32 = 1;
const ROOT_CONFIG_FILE: &str = "akasha.toml";
const POINTER_FILE: &str = ".akasha.toml";

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
    pub project_dir: PathBuf,
}

#[derive(Debug)]
pub enum ResolveError {
    Configuration(String),
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
            Self::FileSystem { .. } => 6,
        }
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(message) => formatter.write_str(message),
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

#[derive(Debug, Deserialize)]
struct RootConfig {
    schema_version: u32,
    folders: FolderConfig,
}

#[derive(Debug, Deserialize)]
struct FolderConfig {
    projects: PathBuf,
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
    let root_config_path = root.join(ROOT_CONFIG_FILE);
    let root_config: RootConfig = read_toml(&root_config_path, "data-root configuration")?;
    require_schema_version(root_config.schema_version, &root_config_path)?;
    validate_relative_path(&root_config.folders.projects, "folders.projects")?;

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
        project_dir,
    })
}

fn resolve_root(request: &ResolveRequest) -> Result<(PathBuf, RootSource), ResolveError> {
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

fn relative_to(path: &Path, base: &Path) -> PathBuf {
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

fn validate_slug(slug: &str) -> Result<(), ResolveError> {
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
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ResolveError::Configuration(format!(
            "{field} must be a non-empty relative path without parent traversal"
        )));
    }

    Ok(())
}

fn canonicalize_directory(path: &Path, description: &str) -> Result<PathBuf, ResolveError> {
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
