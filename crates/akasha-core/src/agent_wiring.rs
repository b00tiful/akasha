use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::resolution::{
    ResolveError, ResolveRequest, RootSource, canonicalize_directory, load_root_config,
    resolve_root,
};
use crate::state::content_fingerprint;

const MANAGED_START: &str = "<!-- akasha-agent-wiring:v1:start -->";
const MANAGED_END: &str = "<!-- akasha-agent-wiring:v1:end -->";

/// Agent client whose user-level instruction file is being prepared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentClient {
    Codex,
    Claude,
}

impl AgentClient {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

/// Exact type of checked instruction-file change represented by a preparation plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentWiringAction {
    Create,
    Append,
    RefreshManagedSection,
    NoChange,
}

/// Exact byte-range replacement needed to realize a prepared wiring plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentWiringPatch {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

/// Read-only, snapshot-bound plan for wiring one agent client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentWiringPlan {
    pub root: PathBuf,
    pub root_source: RootSource,
    pub client: AgentClient,
    pub source: PathBuf,
    pub source_sha256: String,
    pub target: PathBuf,
    pub action: AgentWiringAction,
    pub current_sha256: Option<String>,
    pub result_sha256: String,
    pub plan_id: String,
    pub patch: AgentWiringPatch,
}

/// Configuration, conflict, or filesystem failure while preparing agent wiring.
#[derive(Debug)]
pub enum AgentWiringError {
    Resolution(ResolveError),
    Configuration(String),
    Conflict {
        path: PathBuf,
        reason: String,
    },
    FileSystem {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl AgentWiringError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Resolution(error) => error.exit_code(),
            Self::Configuration(_) => 3,
            Self::Conflict { .. } => 5,
            Self::FileSystem { .. } => 6,
        }
    }
}

impl fmt::Display for AgentWiringError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolution(error) => write!(formatter, "{error}"),
            Self::Configuration(message) => write!(formatter, "{message}"),
            Self::Conflict { path, reason } => {
                write!(
                    formatter,
                    "cannot prepare agent wiring at {}: {reason}",
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

impl Error for AgentWiringError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Resolution(error) => Some(error),
            Self::FileSystem { source, .. } => Some(source),
            Self::Configuration(_) | Self::Conflict { .. } => None,
        }
    }
}

impl From<ResolveError> for AgentWiringError {
    fn from(error: ResolveError) -> Self {
        Self::Resolution(error)
    }
}

/// Prepare an exact, read-only change plan for one agent's user instruction file.
pub fn prepare_agent_wiring(
    request: &ResolveRequest,
    client: AgentClient,
    agent_home: &Path,
) -> Result<AgentWiringPlan, AgentWiringError> {
    let (root, root_source) = resolve_root(request)?;
    let config = load_root_config(&root)?;
    let (source_path, source) = load_instruction_source(&root, &config.files.agent_instructions)?;
    let source_sha256 = content_fingerprint(source.as_bytes());
    let agent_home = canonicalize_directory(agent_home, "agent home")?;
    let target = target_path(client, &agent_home)?;
    let current = read_target(client, &agent_home, &target)?;
    let current_bytes = current.as_deref().unwrap_or_default();
    let current_text =
        std::str::from_utf8(current_bytes).map_err(|_| AgentWiringError::Conflict {
            path: target.clone(),
            reason: "the existing instruction file is not UTF-8".to_owned(),
        })?;
    let section = managed_section(client, &source_path, &source, line_ending(current_text))?;
    let (action, patch) = plan_patch(current.as_ref(), current_text, &section, &target)?;
    let result = apply_patch(current_bytes, &patch);
    let current_sha256 = current.as_ref().map(|bytes| content_fingerprint(bytes));
    let result_sha256 = content_fingerprint(&result);
    let plan_id = plan_fingerprint(
        client,
        &source_sha256,
        &target,
        current_sha256.as_deref(),
        &result_sha256,
        &patch,
    )?;

    Ok(AgentWiringPlan {
        root,
        root_source,
        client,
        source: source_path,
        source_sha256,
        target,
        action,
        current_sha256,
        result_sha256,
        plan_id,
        patch,
    })
}

fn load_instruction_source(
    root: &Path,
    configured: &Path,
) -> Result<(PathBuf, String), AgentWiringError> {
    let candidate = root.join(configured);
    let metadata = match fs::symlink_metadata(&candidate) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(AgentWiringError::Configuration(format!(
                "configured agent instruction source {} does not exist",
                candidate.display()
            )));
        }
        Err(source) => {
            return Err(AgentWiringError::FileSystem {
                operation: "inspect the configured agent instruction source",
                path: candidate,
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AgentWiringError::Configuration(format!(
            "configured agent instruction source {} must be a regular file, not a symlink",
            candidate.display()
        )));
    }

    let path = fs::canonicalize(&candidate).map_err(|source| AgentWiringError::FileSystem {
        operation: "canonicalize the configured agent instruction source",
        path: candidate,
        source,
    })?;
    if !path.starts_with(root) {
        return Err(AgentWiringError::Configuration(format!(
            "configured agent instruction source {} escapes data root {}",
            path.display(),
            root.display()
        )));
    }
    let source = fs::read_to_string(&path).map_err(|source| AgentWiringError::FileSystem {
        operation: "read the configured agent instruction source",
        path: path.clone(),
        source,
    })?;
    if source.trim().is_empty() {
        return Err(AgentWiringError::Configuration(format!(
            "configured agent instruction source {} must not be empty",
            path.display()
        )));
    }
    if source.contains(MANAGED_START) || source.contains(MANAGED_END) {
        return Err(AgentWiringError::Configuration(format!(
            "configured agent instruction source {} contains reserved wiring markers",
            path.display()
        )));
    }
    Ok((path, source))
}

fn target_path(client: AgentClient, home: &Path) -> Result<PathBuf, AgentWiringError> {
    let target = home.join(match client {
        AgentClient::Codex => "AGENTS.md",
        AgentClient::Claude => "CLAUDE.md",
    });
    if target.to_str().is_none() {
        return Err(AgentWiringError::Configuration(
            "agent instruction paths must be valid UTF-8".to_owned(),
        ));
    }
    Ok(target)
}

fn read_target(
    client: AgentClient,
    home: &Path,
    target: &Path,
) -> Result<Option<Vec<u8>>, AgentWiringError> {
    if client == AgentClient::Codex {
        let override_path = home.join("AGENTS.override.md");
        if let Some(contents) = read_regular_if_present(&override_path)?
            && !contents.is_empty()
        {
            return Err(AgentWiringError::Conflict {
                path: override_path,
                reason: "a non-empty global AGENTS.override.md shadows AGENTS.md; resolve that explicit override before wiring Akasha".to_owned(),
            });
        }
    }
    read_regular_if_present(target)
}

fn read_regular_if_present(path: &Path) -> Result<Option<Vec<u8>>, AgentWiringError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(AgentWiringError::FileSystem {
                operation: "inspect an agent instruction file",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AgentWiringError::Conflict {
            path: path.to_path_buf(),
            reason: "the existing path is not a regular file; symlinks and special files require explicit human resolution".to_owned(),
        });
    }
    fs::read(path)
        .map(Some)
        .map_err(|source| AgentWiringError::FileSystem {
            operation: "read an agent instruction file",
            path: path.to_path_buf(),
            source,
        })
}

fn managed_section(
    client: AgentClient,
    source_path: &Path,
    source: &str,
    eol: &str,
) -> Result<String, AgentWiringError> {
    let body = match client {
        AgentClient::Codex => normalize_lines(source)?,
        AgentClient::Claude => {
            let path = source_path.to_str().ok_or_else(|| {
                AgentWiringError::Configuration(
                    "the configured agent instruction source path must be valid UTF-8".to_owned(),
                )
            })?;
            if path.contains(['\r', '\n']) {
                return Err(AgentWiringError::Configuration(
                    "the configured agent instruction source path contains a line break".to_owned(),
                ));
            }
            format!("@{path}\n")
        }
    };
    let body = body.replace('\n', eol);
    Ok(format!("{MANAGED_START}{eol}{body}{MANAGED_END}{eol}"))
}

fn normalize_lines(source: &str) -> Result<String, AgentWiringError> {
    let normalized = source.replace("\r\n", "\n");
    if normalized.contains('\r') {
        return Err(AgentWiringError::Configuration(
            "the configured agent instruction source contains a bare carriage return".to_owned(),
        ));
    }
    Ok(format!("{}\n", normalized.trim_end_matches('\n')))
}

fn line_ending(source: &str) -> &'static str {
    let bytes = source.as_bytes();
    let has_crlf = bytes.windows(2).any(|window| window == b"\r\n");
    let has_bare_lf = bytes
        .iter()
        .enumerate()
        .any(|(index, byte)| *byte == b'\n' && (index == 0 || bytes[index - 1] != b'\r'));
    if has_crlf && !has_bare_lf {
        "\r\n"
    } else {
        "\n"
    }
}

fn plan_patch(
    current: Option<&Vec<u8>>,
    current_text: &str,
    section: &str,
    target: &Path,
) -> Result<(AgentWiringAction, AgentWiringPatch), AgentWiringError> {
    let starts = marker_positions(current_text, MANAGED_START);
    let ends = marker_positions(current_text, MANAGED_END);
    if starts.len() != ends.len() || starts.len() > 1 {
        return Err(AgentWiringError::Conflict {
            path: target.to_path_buf(),
            reason: "managed wiring markers are incomplete or duplicated".to_owned(),
        });
    }

    if let (Some(start), Some(end_start)) = (starts.first().copied(), ends.first().copied()) {
        if end_start < start + MANAGED_START.len() {
            return Err(AgentWiringError::Conflict {
                path: target.to_path_buf(),
                reason: "managed wiring markers are out of order".to_owned(),
            });
        }
        let mut end = end_start + MANAGED_END.len();
        if current_text.as_bytes().get(end..end + 2) == Some(b"\r\n") {
            end += 2;
        } else if current_text.as_bytes().get(end) == Some(&b'\n') {
            end += 1;
        }
        if current_text.as_bytes().get(start..end) == Some(section.as_bytes()) {
            return Ok((
                AgentWiringAction::NoChange,
                AgentWiringPatch {
                    start: 0,
                    end: 0,
                    replacement: String::new(),
                },
            ));
        }
        return Ok((
            AgentWiringAction::RefreshManagedSection,
            AgentWiringPatch {
                start,
                end,
                replacement: section.to_owned(),
            },
        ));
    }

    let patch = append_patch(current_text, section);
    Ok((
        if current.is_some() {
            AgentWiringAction::Append
        } else {
            AgentWiringAction::Create
        },
        patch,
    ))
}

fn marker_positions(source: &str, marker: &str) -> Vec<usize> {
    source
        .match_indices(marker)
        .map(|(index, _)| index)
        .collect()
}

fn append_patch(current: &str, section: &str) -> AgentWiringPatch {
    let eol = line_ending(current);
    let replacement = if current.is_empty() || current.ends_with(&format!("{eol}{eol}")) {
        section.to_owned()
    } else if current.ends_with(eol) {
        format!("{eol}{section}")
    } else {
        format!("{eol}{eol}{section}")
    };
    AgentWiringPatch {
        start: current.len(),
        end: current.len(),
        replacement,
    }
}

fn apply_patch(current: &[u8], patch: &AgentWiringPatch) -> Vec<u8> {
    let mut result =
        Vec::with_capacity(current.len() - (patch.end - patch.start) + patch.replacement.len());
    result.extend_from_slice(&current[..patch.start]);
    result.extend_from_slice(patch.replacement.as_bytes());
    result.extend_from_slice(&current[patch.end..]);
    result
}

fn plan_fingerprint(
    client: AgentClient,
    source_sha256: &str,
    target: &Path,
    current_sha256: Option<&str>,
    result_sha256: &str,
    patch: &AgentWiringPatch,
) -> Result<String, AgentWiringError> {
    let target = target.to_str().ok_or_else(|| {
        AgentWiringError::Configuration("agent instruction paths must be valid UTF-8".to_owned())
    })?;
    let binding = format!(
        "agent-wiring-plan-v1\0{}\0{source_sha256}\0{target}\0{}\0{result_sha256}\0{}\0{}\0{}",
        client.as_str(),
        current_sha256.unwrap_or("absent"),
        patch.start,
        patch.end,
        patch.replacement
    );
    Ok(content_fingerprint(binding.as_bytes()))
}
