use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::{fs, io};

use serde::Serialize;

use crate::resolution::{
    CONFIG_SCHEMA_VERSION, POINTER_FILE, ResolutionEnvironment, ResolveError, ResolveRequest,
    canonicalize_directory, relative_to, resolve_project,
};
use crate::state::content_fingerprint;
use crate::writes::{AtomicCreateError, create_file_atomically, sync_directory};

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

/// Read-only review of one exclusive pointer creation. The ID binds all canonical
/// identities and the exact source, not unrelated registry/configuration bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkPlan {
    pub destination: LinkResult,
    pub source: String,
    pub plan_id: String,
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
    InvalidPlan(String),
    StalePlan,
}

impl LinkError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Resolution(error) => error.exit_code(),
            Self::RepositoryMismatch { .. } => 3,
            Self::Creation(error) => error.exit_code(),
            Self::InvalidPlan(_) => 4,
            Self::StalePlan => 5,
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
            Self::InvalidPlan(message) => write!(formatter, "invalid link plan: {message}"),
            Self::StalePlan => write!(
                formatter,
                "link plan changed; prepare and review a fresh plan"
            ),
        }
    }
}

impl Error for LinkError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Resolution(error) => Some(error),
            Self::RepositoryMismatch { .. } | Self::InvalidPlan(_) | Self::StalePlan => None,
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
    let destination = resolve_link(request)?;
    publish_link(destination)
}

/// Resolve and check an absent destination without locks, staging, recovery or writes.
pub fn prepare_project_link(request: &LinkRequest) -> Result<LinkPlan, LinkError> {
    let destination = resolve_link(request)?;
    match fs::symlink_metadata(&destination.pointer) {
        Ok(_) => {
            return Err(AtomicCreateError::Conflict {
                path: destination.pointer,
                source: io::Error::new(io::ErrorKind::AlreadyExists, "pointer already exists"),
            }
            .into());
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(AtomicCreateError::FileSystem {
                operation: "inspect the link destination",
                path: destination.pointer,
                source,
            }
            .into());
        }
    }
    let source = pointer_source(&destination.project);
    // Structured, versioned input avoids ambiguous concatenation and refuses lossy paths.
    let identity = serde_json::to_vec(&("akasha-project-link-v1", &destination, &source))
        .map_err(|error| LinkError::InvalidPlan(error.to_string()))?;
    Ok(LinkPlan {
        destination,
        source,
        plan_id: content_fingerprint(&identity),
    })
}

/// Re-resolve the reviewed identities, then publish only that plan's exact pointer.
/// Exclusive creation remains authoritative if a target appears after revalidation.
/// Registry/configuration edits by external writers are not locked by this operation.
pub fn apply_project_link(request: &LinkRequest, plan: &LinkPlan) -> Result<LinkResult, LinkError> {
    let current = prepare_project_link(request)?;
    if &current != plan {
        return Err(LinkError::StalePlan);
    }
    publish_link(current.destination)
}

fn resolve_link(request: &LinkRequest) -> Result<LinkResult, LinkError> {
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
    Ok(LinkResult {
        root: resolved.root,
        project: resolved.project,
        registry: resolved.registry,
        repository_dir: requested_repository,
        project_dir: resolved.project_dir,
        pointer,
    })
}

fn pointer_source(project: &str) -> String {
    format!("schema_version = {CONFIG_SCHEMA_VERSION}\nproject = \"{project}\"\n")
}

fn publish_link(destination: LinkResult) -> Result<LinkResult, LinkError> {
    create_file_atomically(
        &destination.pointer,
        pointer_source(&destination.project).as_bytes(),
    )?;
    sync_directory(&destination.repository_dir).map_err(|source| {
        LinkError::Creation(AtomicCreateError::FileSystem {
            operation: "sync the linked repository directory",
            path: destination.repository_dir.clone(),
            source,
        })
    })?;
    Ok(destination)
}
