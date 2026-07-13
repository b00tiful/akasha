use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use serde::Serialize;

use crate::resolution::{
    CONFIG_SCHEMA_VERSION, POINTER_FILE, ResolutionEnvironment, ResolveError, ResolveRequest,
    canonicalize_directory, relative_to, resolve_project,
};
use crate::writes::{AtomicCreateError, create_file_atomically};

/// Inputs for linking one registered Akasha project to a repository.
#[derive(Debug, Clone)]
pub struct LinkRequest {
    pub root_override: Option<PathBuf>,
    pub project: String,
    pub repository: Option<PathBuf>,
    pub cwd: PathBuf,
    pub environment: ResolutionEnvironment,
}

impl LinkRequest {
    /// Build a link request from CLI inputs and the current process.
    pub fn from_process(
        root_override: Option<PathBuf>,
        project: String,
        repository: Option<PathBuf>,
    ) -> Result<Self, ResolveError> {
        let request = ResolveRequest::from_process(root_override, Some(project.clone()))?;
        Ok(Self {
            root_override: request.root_override,
            project,
            repository,
            cwd: request.cwd,
            environment: request.environment,
        })
    }
}

/// The canonical identity and pointer created by a successful link operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkResult {
    pub root: PathBuf,
    pub project: String,
    pub registry: PathBuf,
    pub repository_dir: PathBuf,
    pub project_dir: PathBuf,
    pub pointer: PathBuf,
}

/// A link configuration, resolution, or exclusive-creation failure.
#[derive(Debug)]
pub enum LinkError {
    Resolution(ResolveError),
    RepositoryMismatch {
        project: String,
        requested: PathBuf,
        registered: PathBuf,
    },
    Creation(AtomicCreateError),
}

impl LinkError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Resolution(error) => error.exit_code(),
            Self::RepositoryMismatch { .. } => 3,
            Self::Creation(error) => error.exit_code(),
        }
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolution(error) => write!(formatter, "{error}"),
            Self::RepositoryMismatch {
                project,
                requested,
                registered,
            } => write!(
                formatter,
                "repository to link {} does not match registry entry {project:?} at {}",
                requested.display(),
                registered.display()
            ),
            Self::Creation(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for LinkError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Resolution(error) => Some(error),
            Self::RepositoryMismatch { .. } => None,
            Self::Creation(error) => Some(error),
        }
    }
}

impl From<ResolveError> for LinkError {
    fn from(error: ResolveError) -> Self {
        Self::Resolution(error)
    }
}

impl From<AtomicCreateError> for LinkError {
    fn from(error: AtomicCreateError) -> Self {
        Self::Creation(error)
    }
}

/// Create a canonical project pointer in an already-registered repository.
pub fn link_project(request: &LinkRequest) -> Result<LinkResult, LinkError> {
    let resolution_request = ResolveRequest {
        root_override: request.root_override.clone(),
        project_override: Some(request.project.clone()),
        cwd: request.cwd.clone(),
        environment: request.environment.clone(),
    };
    let resolved = resolve_project(&resolution_request)?;

    let requested_repository = request.repository.as_deref().unwrap_or(&request.cwd);
    let requested_repository = canonicalize_directory(
        &relative_to(requested_repository, &request.cwd),
        "repository to link",
    )?;
    if requested_repository != resolved.repository_dir {
        return Err(LinkError::RepositoryMismatch {
            project: resolved.project,
            requested: requested_repository,
            registered: resolved.repository_dir,
        });
    }

    let pointer = requested_repository.join(POINTER_FILE);
    let contents = format!(
        "schema_version = {CONFIG_SCHEMA_VERSION}\nproject = \"{}\"\n",
        resolved.project
    );
    create_file_atomically(&pointer, contents.as_bytes())?;

    Ok(LinkResult {
        root: resolved.root,
        project: resolved.project,
        registry: resolved.registry,
        repository_dir: requested_repository,
        project_dir: resolved.project_dir,
        pointer,
    })
}
