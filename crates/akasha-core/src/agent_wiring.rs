use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::resolution::{
    ResolveError, ResolveRequest, RootSource, canonicalize_directory, load_root_config,
    resolve_root,
};
use crate::state::content_fingerprint;
use crate::writes::{
    AtomicCreateError, CheckedReplaceError, create_file_atomically, replace_file_if_unchanged,
    sync_directory,
};

const MANAGED_START_TOKEN: &str = "<!-- akasha-agent-wiring:v1:start";
const MANAGED_END: &str = "<!-- akasha-agent-wiring:v1:end -->";
const JOURNAL_SCHEMA_VERSION: u32 = 1;
const LOCK_SUFFIX: &str = ".akasha-agent-wiring.lock";
const JOURNAL_SUFFIX: &str = ".akasha-agent-wiring-journal.json";

/// Agent client whose user-level instruction file is being prepared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// Intent represented by one exact agent-wiring plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentWiringOperation {
    Apply,
    Remove,
}

impl AgentWiringOperation {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Remove => "remove",
        }
    }
}

/// Exact type of checked instruction-file change represented by a preparation plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentWiringAction {
    Create,
    Append,
    RefreshManagedSection,
    RemoveManagedSection,
    RemoveCreatedFile,
    NoChange,
}

/// Exact byte-range replacement needed to realize a prepared wiring plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentWiringPatch {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

/// Read-only, snapshot-bound plan for wiring or removing one agent client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentWiringPlan {
    pub root: PathBuf,
    pub root_source: RootSource,
    pub client: AgentClient,
    pub operation: AgentWiringOperation,
    pub source: PathBuf,
    pub source_sha256: String,
    pub target: PathBuf,
    pub action: AgentWiringAction,
    pub current_sha256: Option<String>,
    pub result_sha256: Option<String>,
    pub plan_id: String,
    pub patch: AgentWiringPatch,
}

/// Recovery performed before an agent-wiring write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentWiringRecovery {
    None,
    Discarded,
    Finalized,
}

/// Result of applying one exact prepared agent-wiring plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentWiringResult {
    pub client: AgentClient,
    pub operation: AgentWiringOperation,
    pub target: PathBuf,
    pub action: AgentWiringAction,
    pub plan_id: String,
    pub changed: bool,
    pub recovery: AgentWiringRecovery,
}

/// Configuration, conflict, or filesystem failure while managing agent wiring.
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
    Recovery {
        original: Box<AgentWiringError>,
        recovery: Box<AgentWiringError>,
    },
}

impl AgentWiringError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Resolution(error) => error.exit_code(),
            Self::Configuration(_) => 3,
            Self::Conflict { .. } => 5,
            Self::FileSystem { .. } | Self::Recovery { .. } => 6,
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
                    "cannot manage agent wiring at {}: {reason}",
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
            Self::Recovery { original, recovery } => write!(
                formatter,
                "agent-wiring operation failed ({original}) and automatic recovery also failed ({recovery})"
            ),
        }
    }
}

impl Error for AgentWiringError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Resolution(error) => Some(error),
            Self::FileSystem { source, .. } => Some(source),
            Self::Recovery { recovery, .. } => Some(recovery.as_ref()),
            Self::Configuration(_) | Self::Conflict { .. } => None,
        }
    }
}

impl From<ResolveError> for AgentWiringError {
    fn from(error: ResolveError) -> Self {
        Self::Resolution(error)
    }
}

/// Prepare an exact, read-only application plan for one agent's user instruction file.
pub fn prepare_agent_wiring(
    request: &ResolveRequest,
    client: AgentClient,
    agent_home: &Path,
) -> Result<AgentWiringPlan, AgentWiringError> {
    prepare_agent_wiring_operation(request, client, agent_home, AgentWiringOperation::Apply)
}

/// Prepare an exact, read-only removal plan for one agent's managed instruction section.
pub fn prepare_agent_wiring_removal(
    request: &ResolveRequest,
    client: AgentClient,
    agent_home: &Path,
) -> Result<AgentWiringPlan, AgentWiringError> {
    prepare_agent_wiring_operation(request, client, agent_home, AgentWiringOperation::Remove)
}

/// Apply one exact prepared agent-wiring plan after recomputing its complete snapshot binding.
pub fn apply_agent_wiring(
    request: &ResolveRequest,
    client: AgentClient,
    agent_home: &Path,
    plan_id: &str,
) -> Result<AgentWiringResult, AgentWiringError> {
    commit_agent_wiring_with_hook(
        request,
        client,
        agent_home,
        AgentWiringOperation::Apply,
        plan_id,
        |_| {},
    )
}

/// Remove one exact managed section or exact Akasha-created instruction file.
pub fn remove_agent_wiring(
    request: &ResolveRequest,
    client: AgentClient,
    agent_home: &Path,
    plan_id: &str,
) -> Result<AgentWiringResult, AgentWiringError> {
    commit_agent_wiring_with_hook(
        request,
        client,
        agent_home,
        AgentWiringOperation::Remove,
        plan_id,
        |_| {},
    )
}

fn prepare_agent_wiring_operation(
    request: &ResolveRequest,
    client: AgentClient,
    agent_home: &Path,
    operation: AgentWiringOperation,
) -> Result<AgentWiringPlan, AgentWiringError> {
    let (root, root_source) = resolve_root(request)?;
    let config = load_root_config(&root)?;
    let (source_path, source) = load_instruction_source(&root, &config.files.agent_instructions)?;
    let source_sha256 = content_fingerprint(source.as_bytes());
    let agent_home = canonicalize_directory(agent_home, "agent home")?;
    let target = target_path(client, &agent_home)?;
    let current = read_target(client, &agent_home, &target, operation)?;
    let current_bytes = current.as_deref().unwrap_or_default();
    let current_text =
        std::str::from_utf8(current_bytes).map_err(|_| AgentWiringError::Conflict {
            path: target.clone(),
            reason: "the existing instruction file is not UTF-8".to_owned(),
        })?;

    let (action, patch, remove_file) = match operation {
        AgentWiringOperation::Apply => {
            let (action, patch) = plan_apply_patch(
                current.as_ref(),
                current_text,
                client,
                &source_path,
                &source,
                &target,
            )?;
            (action, patch, false)
        }
        AgentWiringOperation::Remove => {
            let (action, patch, remove_file) = plan_removal_patch(current_text, &target)?;
            (action, patch, remove_file)
        }
    };
    let result = if remove_file {
        None
    } else {
        Some(apply_patch(current_bytes, &patch))
    };
    let current_sha256 = current.as_ref().map(|bytes| content_fingerprint(bytes));
    let result_sha256 = result.as_ref().map(|bytes| content_fingerprint(bytes));
    let plan_id = plan_fingerprint(
        operation,
        client,
        action,
        &source_path,
        &source_sha256,
        &target,
        current_sha256.as_deref(),
        result_sha256.as_deref(),
        &patch,
    )?;

    Ok(AgentWiringPlan {
        root,
        root_source,
        client,
        operation,
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
    if source.contains(MANAGED_START_TOKEN) || source.contains(MANAGED_END) {
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
    operation: AgentWiringOperation,
) -> Result<Option<Vec<u8>>, AgentWiringError> {
    if client == AgentClient::Codex && operation == AgentWiringOperation::Apply {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedOrigin {
    Create,
    Append,
}

impl ManagedOrigin {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Append => "append",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ManagedSection {
    start: usize,
    end: usize,
    origin: ManagedOrigin,
    prefix_len: usize,
}

fn plan_apply_patch(
    current: Option<&Vec<u8>>,
    current_text: &str,
    client: AgentClient,
    source_path: &Path,
    source: &str,
    target: &Path,
) -> Result<(AgentWiringAction, AgentWiringPatch), AgentWiringError> {
    if let Some(existing) = parse_managed_section(current_text, target)? {
        let section = managed_section(
            client,
            source_path,
            source,
            line_ending(current_text),
            existing.origin,
            existing.prefix_len,
        )?;
        if current_text.as_bytes().get(existing.start..existing.end) == Some(section.as_bytes()) {
            return Ok((AgentWiringAction::NoChange, empty_patch()));
        }
        return Ok((
            AgentWiringAction::RefreshManagedSection,
            AgentWiringPatch {
                start: existing.start,
                end: existing.end,
                replacement: section,
            },
        ));
    }

    let origin = if current.is_some() {
        ManagedOrigin::Append
    } else {
        ManagedOrigin::Create
    };
    let prefix = if current.is_some() {
        append_prefix(current_text)
    } else {
        String::new()
    };
    let section = managed_section(
        client,
        source_path,
        source,
        line_ending(current_text),
        origin,
        prefix.len(),
    )?;
    let start = current_text.len();
    Ok((
        if current.is_some() {
            AgentWiringAction::Append
        } else {
            AgentWiringAction::Create
        },
        AgentWiringPatch {
            start,
            end: start,
            replacement: format!("{prefix}{section}"),
        },
    ))
}

fn plan_removal_patch(
    current_text: &str,
    target: &Path,
) -> Result<(AgentWiringAction, AgentWiringPatch, bool), AgentWiringError> {
    let Some(section) = parse_managed_section(current_text, target)? else {
        return Ok((AgentWiringAction::NoChange, empty_patch(), false));
    };
    if section.start < section.prefix_len {
        return Err(malformed_marker_conflict(target));
    }
    let start = section.start - section.prefix_len;
    let owned_prefix = &current_text.as_bytes()[start..section.start];
    if !is_owned_separator(owned_prefix)
        || (section.origin == ManagedOrigin::Create && !owned_prefix.is_empty())
    {
        return Err(malformed_marker_conflict(target));
    }
    let patch = AgentWiringPatch {
        start,
        end: section.end,
        replacement: String::new(),
    };
    let result = apply_patch(current_text.as_bytes(), &patch);
    let remove_file = section.origin == ManagedOrigin::Create && result.is_empty();
    Ok((
        if remove_file {
            AgentWiringAction::RemoveCreatedFile
        } else {
            AgentWiringAction::RemoveManagedSection
        },
        patch,
        remove_file,
    ))
}

fn managed_section(
    client: AgentClient,
    source_path: &Path,
    source: &str,
    eol: &str,
    origin: ManagedOrigin,
    prefix_len: usize,
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
    let start = format!(
        "{MANAGED_START_TOKEN} origin={} prefix={prefix_len} -->",
        origin.as_str()
    );
    Ok(format!("{start}{eol}{body}{MANAGED_END}{eol}"))
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

fn append_prefix(current: &str) -> String {
    let eol = line_ending(current);
    if current.is_empty() || current.ends_with(&format!("{eol}{eol}")) {
        String::new()
    } else if current.ends_with(eol) {
        eol.to_owned()
    } else {
        format!("{eol}{eol}")
    }
}

fn parse_managed_section(
    source: &str,
    target: &Path,
) -> Result<Option<ManagedSection>, AgentWiringError> {
    let starts = marker_positions(source, MANAGED_START_TOKEN);
    let ends = marker_positions(source, MANAGED_END);
    if starts.is_empty() && ends.is_empty() {
        return Ok(None);
    }
    if starts.len() != 1 || ends.len() != 1 {
        return Err(malformed_marker_conflict(target));
    }

    let start = starts[0];
    let marker_tail = &source[start..];
    let marker_end = marker_tail
        .find("-->")
        .map(|relative| start + relative + 3)
        .ok_or_else(|| malformed_marker_conflict(target))?;
    let marker = &source[start..marker_end];
    let metadata = marker
        .strip_prefix(MANAGED_START_TOKEN)
        .and_then(|value| value.strip_suffix(" -->"))
        .ok_or_else(|| malformed_marker_conflict(target))?;
    if metadata.contains(['\r', '\n']) {
        return Err(malformed_marker_conflict(target));
    }
    let mut fields = metadata.split_ascii_whitespace();
    let origin = match fields.next() {
        Some("origin=create") => ManagedOrigin::Create,
        Some("origin=append") => ManagedOrigin::Append,
        _ => return Err(malformed_marker_conflict(target)),
    };
    let prefix_len = fields
        .next()
        .and_then(|value| value.strip_prefix("prefix="))
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| matches!(value, 0 | 1 | 2 | 4))
        .ok_or_else(|| malformed_marker_conflict(target))?;
    if fields.next().is_some() || (origin == ManagedOrigin::Create && prefix_len != 0) {
        return Err(malformed_marker_conflict(target));
    }

    let end_start = ends[0];
    if end_start < marker_end {
        return Err(malformed_marker_conflict(target));
    }
    let mut end = end_start + MANAGED_END.len();
    if source.as_bytes().get(end..end + 2) == Some(b"\r\n") {
        end += 2;
    } else if source.as_bytes().get(end) == Some(&b'\n') {
        end += 1;
    }
    Ok(Some(ManagedSection {
        start,
        end,
        origin,
        prefix_len,
    }))
}

fn marker_positions(source: &str, marker: &str) -> Vec<usize> {
    source
        .match_indices(marker)
        .map(|(index, _)| index)
        .collect()
}

fn malformed_marker_conflict(target: &Path) -> AgentWiringError {
    AgentWiringError::Conflict {
        path: target.to_path_buf(),
        reason:
            "managed wiring markers or ownership metadata are incomplete, duplicated, or malformed"
                .to_owned(),
    }
}

fn is_owned_separator(bytes: &[u8]) -> bool {
    matches!(bytes, b"" | b"\n" | b"\n\n" | b"\r\n" | b"\r\n\r\n")
}

fn empty_patch() -> AgentWiringPatch {
    AgentWiringPatch {
        start: 0,
        end: 0,
        replacement: String::new(),
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

#[derive(Serialize)]
struct PlanBinding<'a> {
    schema: &'static str,
    operation: AgentWiringOperation,
    client: AgentClient,
    action: AgentWiringAction,
    source: &'a str,
    source_sha256: &'a str,
    target: &'a str,
    current_sha256: Option<&'a str>,
    result_sha256: Option<&'a str>,
    patch_start: usize,
    patch_end: usize,
    patch_replacement: &'a str,
}

#[allow(clippy::too_many_arguments)]
fn plan_fingerprint(
    operation: AgentWiringOperation,
    client: AgentClient,
    action: AgentWiringAction,
    source: &Path,
    source_sha256: &str,
    target: &Path,
    current_sha256: Option<&str>,
    result_sha256: Option<&str>,
    patch: &AgentWiringPatch,
) -> Result<String, AgentWiringError> {
    let source = source.to_str().ok_or_else(|| {
        AgentWiringError::Configuration(
            "the configured agent instruction source path must be valid UTF-8".to_owned(),
        )
    })?;
    let target = target.to_str().ok_or_else(|| {
        AgentWiringError::Configuration("agent instruction paths must be valid UTF-8".to_owned())
    })?;
    let binding = PlanBinding {
        schema: "agent-wiring-plan-v2",
        operation,
        client,
        action,
        source,
        source_sha256,
        target,
        current_sha256,
        result_sha256,
        patch_start: patch.start,
        patch_end: patch.end,
        patch_replacement: &patch.replacement,
    };
    let binding = serde_json::to_vec(&binding).map_err(|error| {
        AgentWiringError::Configuration(format!(
            "could not serialize the deterministic agent-wiring plan binding: {error}"
        ))
    })?;
    Ok(content_fingerprint(&binding))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationStage {
    Journal,
    Target,
}

fn commit_agent_wiring_with_hook(
    request: &ResolveRequest,
    client: AgentClient,
    agent_home: &Path,
    operation: AgentWiringOperation,
    expected_plan_id: &str,
    mut publication_hook: impl FnMut(PublicationStage),
) -> Result<AgentWiringResult, AgentWiringError> {
    let agent_home = canonicalize_directory(agent_home, "agent home")?;
    let target = target_path(client, &agent_home)?;
    let _lock = AgentWiringLock::acquire(&agent_home, &target)?;
    let recovered = recover_agent_wiring_locked(&agent_home, client, &target)?;
    if let Some(journal) = recovered.finalized.as_ref()
        && journal.operation == operation
        && journal.plan_id == expected_plan_id
    {
        return Ok(result_from_journal(
            journal,
            &target,
            AgentWiringRecovery::Finalized,
        ));
    }

    let plan = prepare_agent_wiring_operation(request, client, &agent_home, operation)?;
    if plan.plan_id != expected_plan_id {
        return Err(AgentWiringError::Conflict {
            path: target,
            reason: format!(
                "prepared plan ID no longer matches the exact source and target snapshots (expected {expected_plan_id}, current {})",
                plan.plan_id
            ),
        });
    }
    if plan.action == AgentWiringAction::NoChange {
        return Ok(AgentWiringResult {
            client,
            operation,
            target: plan.target,
            action: plan.action,
            plan_id: plan.plan_id,
            changed: false,
            recovery: recovered.recovery,
        });
    }

    verify_plan_source(&plan)?;
    let before = read_target(client, &agent_home, &plan.target, operation)?
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| AgentWiringError::Conflict {
                path: plan.target.clone(),
                reason: "the existing instruction file is not UTF-8".to_owned(),
            })
        })
        .transpose()?;
    let current_sha256 = before
        .as_ref()
        .map(|source| content_fingerprint(source.as_bytes()));
    if current_sha256 != plan.current_sha256 {
        return Err(stale_plan_conflict(&plan.target));
    }
    let after = if plan.action == AgentWiringAction::RemoveCreatedFile {
        None
    } else {
        Some(
            String::from_utf8(apply_patch(
                before.as_deref().unwrap_or_default().as_bytes(),
                &plan.patch,
            ))
            .expect("a patch over UTF-8 source with a UTF-8 replacement remains UTF-8"),
        )
    };
    let result_sha256 = after
        .as_ref()
        .map(|source| content_fingerprint(source.as_bytes()));
    if result_sha256 != plan.result_sha256 {
        return Err(stale_plan_conflict(&plan.target));
    }

    let journal = AgentWiringJournal {
        schema_version: JOURNAL_SCHEMA_VERSION,
        client,
        operation,
        action: plan.action,
        target: plan
            .target
            .file_name()
            .expect("agent instruction targets always have a filename")
            .to_string_lossy()
            .into_owned(),
        plan_id: plan.plan_id.clone(),
        before_sha256: before
            .as_ref()
            .map(|source| content_fingerprint(source.as_bytes())),
        after_sha256: after
            .as_ref()
            .map(|source| content_fingerprint(source.as_bytes())),
    };
    let (journal_path, journal_source) = write_journal(&agent_home, &plan.target, &journal)?;
    publication_hook(PublicationStage::Journal);

    let attempt = (|| {
        publish_journaled_target(
            &plan.target,
            before.as_deref().map(str::as_bytes),
            after.as_deref().map(str::as_bytes),
        )?;
        sync_directory(&agent_home).map_err(|source| AgentWiringError::FileSystem {
            operation: "sync the agent instruction directory",
            path: agent_home.clone(),
            source,
        })?;
        publication_hook(PublicationStage::Target);
        complete_journal(&journal_path, &journal_source, &agent_home)?;
        Ok(AgentWiringResult {
            client,
            operation,
            target: plan.target.clone(),
            action: plan.action,
            plan_id: plan.plan_id.clone(),
            changed: true,
            recovery: recovered.recovery,
        })
    })();

    match attempt {
        Ok(result) => Ok(result),
        Err(original) => match recover_agent_wiring_locked(&agent_home, client, &plan.target) {
            Ok(recovery) if recovery.recovery == AgentWiringRecovery::Finalized => {
                let finalized = recovery
                    .finalized
                    .as_ref()
                    .expect("finalized recovery always retains its journal result");
                Ok(result_from_journal(
                    finalized,
                    &plan.target,
                    AgentWiringRecovery::Finalized,
                ))
            }
            Ok(_) => Err(original),
            Err(recovery) => Err(AgentWiringError::Recovery {
                original: Box::new(original),
                recovery: Box::new(recovery),
            }),
        },
    }
}

fn verify_plan_source(plan: &AgentWiringPlan) -> Result<(), AgentWiringError> {
    let metadata =
        fs::symlink_metadata(&plan.source).map_err(|source| AgentWiringError::FileSystem {
            operation: "inspect the configured agent instruction source before application",
            path: plan.source.clone(),
            source,
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(stale_plan_conflict(&plan.source));
    }
    let current = fs::read(&plan.source).map_err(|source| AgentWiringError::FileSystem {
        operation: "verify the configured agent instruction source",
        path: plan.source.clone(),
        source,
    })?;
    if content_fingerprint(&current) != plan.source_sha256 {
        return Err(stale_plan_conflict(&plan.source));
    }
    Ok(())
}

fn stale_plan_conflict(path: &Path) -> AgentWiringError {
    AgentWiringError::Conflict {
        path: path.to_path_buf(),
        reason: "the exact source or target snapshot changed while applying the prepared plan"
            .to_owned(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentWiringJournal {
    schema_version: u32,
    client: AgentClient,
    operation: AgentWiringOperation,
    action: AgentWiringAction,
    target: String,
    plan_id: String,
    before_sha256: Option<String>,
    after_sha256: Option<String>,
}

fn write_journal(
    agent_home: &Path,
    target: &Path,
    journal: &AgentWiringJournal,
) -> Result<(PathBuf, Vec<u8>), AgentWiringError> {
    let path = auxiliary_path(agent_home, target, JOURNAL_SUFFIX)?;
    let mut source = serde_json::to_vec_pretty(journal).map_err(|error| {
        AgentWiringError::Configuration(format!(
            "could not serialize the agent-wiring recovery journal: {error}"
        ))
    })?;
    source.push(b'\n');
    create_file_atomically(&path, &source).map_err(map_atomic_create)?;
    sync_directory(agent_home).map_err(|source| AgentWiringError::FileSystem {
        operation: "sync the agent-wiring recovery journal",
        path: agent_home.to_path_buf(),
        source,
    })?;
    Ok((path, source))
}

fn publish_journaled_target(
    target: &Path,
    before: Option<&[u8]>,
    after: Option<&[u8]>,
) -> Result<(), AgentWiringError> {
    match (before, after) {
        (None, Some(after)) => create_file_atomically(target, after).map_err(map_atomic_create),
        (Some(before), Some(after)) => replace_file_if_unchanged(target, before, after)
            .map(|_| ())
            .map_err(map_checked_replace),
        (Some(before), None) => remove_file_if_unchanged(target, before),
        (None, None) => Err(AgentWiringError::Conflict {
            path: target.to_path_buf(),
            reason: "an agent-wiring journal cannot transition an absent target to absence"
                .to_owned(),
        }),
    }
}

fn remove_file_if_unchanged(path: &Path, expected: &[u8]) -> Result<(), AgentWiringError> {
    let current = read_regular_if_present(path)?.ok_or_else(|| AgentWiringError::Conflict {
        path: path.to_path_buf(),
        reason: "the checked removal target is already absent".to_owned(),
    })?;
    if current != expected {
        return Err(stale_plan_conflict(path));
    }
    fs::remove_file(path).map_err(|source| AgentWiringError::FileSystem {
        operation: "remove an exact Akasha-created instruction file",
        path: path.to_path_buf(),
        source,
    })
}

#[derive(Debug)]
struct RecoveredAgentWiring {
    recovery: AgentWiringRecovery,
    finalized: Option<AgentWiringJournal>,
}

fn recover_agent_wiring_locked(
    agent_home: &Path,
    client: AgentClient,
    target: &Path,
) -> Result<RecoveredAgentWiring, AgentWiringError> {
    let journal_path = auxiliary_path(agent_home, target, JOURNAL_SUFFIX)?;
    let Some((journal, source)) = read_journal(&journal_path)? else {
        return Ok(RecoveredAgentWiring {
            recovery: AgentWiringRecovery::None,
            finalized: None,
        });
    };
    validate_journal(&journal_path, &journal, client, target)?;
    let current = read_regular_if_present(target)?;
    let current_sha256 = current.as_ref().map(|source| content_fingerprint(source));
    let matches_before = current_sha256 == journal.before_sha256;
    let matches_after = current_sha256 == journal.after_sha256;
    let recovery = if matches_before {
        AgentWiringRecovery::Discarded
    } else if matches_after {
        AgentWiringRecovery::Finalized
    } else {
        return Err(AgentWiringError::Conflict {
            path: journal_path,
            reason: "journaled agent instruction target contains unexpected bytes; automatic recovery refused".to_owned(),
        });
    };
    complete_journal(&journal_path, &source, agent_home)?;
    Ok(RecoveredAgentWiring {
        recovery,
        finalized: (recovery == AgentWiringRecovery::Finalized).then_some(journal),
    })
}

fn read_journal(path: &Path) -> Result<Option<(AgentWiringJournal, Vec<u8>)>, AgentWiringError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(AgentWiringError::FileSystem {
                operation: "inspect the agent-wiring recovery journal",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AgentWiringError::Conflict {
            path: path.to_path_buf(),
            reason: "the agent-wiring recovery journal is not a regular file".to_owned(),
        });
    }
    let source = fs::read(path).map_err(|source| AgentWiringError::FileSystem {
        operation: "read the agent-wiring recovery journal",
        path: path.to_path_buf(),
        source,
    })?;
    let journal = serde_json::from_slice(&source).map_err(|error| AgentWiringError::Conflict {
        path: path.to_path_buf(),
        reason: format!("the agent-wiring recovery journal is invalid: {error}"),
    })?;
    Ok(Some((journal, source)))
}

fn validate_journal(
    journal_path: &Path,
    journal: &AgentWiringJournal,
    client: AgentClient,
    target: &Path,
) -> Result<(), AgentWiringError> {
    let target_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            AgentWiringError::Configuration(
                "agent instruction paths must be valid UTF-8".to_owned(),
            )
        })?;
    let operation_matches_action = match journal.operation {
        AgentWiringOperation::Apply => {
            matches!(
                journal.action,
                AgentWiringAction::Create
                    | AgentWiringAction::Append
                    | AgentWiringAction::RefreshManagedSection
            ) && journal.after_sha256.is_some()
        }
        AgentWiringOperation::Remove => match journal.action {
            AgentWiringAction::RemoveManagedSection => {
                journal.before_sha256.is_some() && journal.after_sha256.is_some()
            }
            AgentWiringAction::RemoveCreatedFile => {
                journal.before_sha256.is_some() && journal.after_sha256.is_none()
            }
            _ => false,
        },
    };
    if journal.schema_version != JOURNAL_SCHEMA_VERSION
        || journal.client != client
        || journal.target != target_name
        || journal.before_sha256 == journal.after_sha256
        || !operation_matches_action
        || !valid_plan_id(&journal.plan_id)
        || !journal.before_sha256.as_deref().is_none_or(valid_plan_id)
        || !journal.after_sha256.as_deref().is_none_or(valid_plan_id)
    {
        return Err(AgentWiringError::Conflict {
            path: journal_path.to_path_buf(),
            reason:
                "the agent-wiring recovery journal does not match the selected client transaction"
                    .to_owned(),
        });
    }
    Ok(())
}

fn valid_plan_id(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn complete_journal(
    path: &Path,
    expected: &[u8],
    agent_home: &Path,
) -> Result<(), AgentWiringError> {
    let current = read_regular_if_present(path)?.ok_or_else(|| AgentWiringError::Conflict {
        path: path.to_path_buf(),
        reason: "the agent-wiring recovery journal disappeared before cleanup".to_owned(),
    })?;
    if current != expected {
        return Err(AgentWiringError::Conflict {
            path: path.to_path_buf(),
            reason: "the agent-wiring recovery journal changed before cleanup".to_owned(),
        });
    }
    fs::remove_file(path).map_err(|source| AgentWiringError::FileSystem {
        operation: "remove the completed agent-wiring recovery journal",
        path: path.to_path_buf(),
        source,
    })?;
    sync_directory(agent_home).map_err(|source| AgentWiringError::FileSystem {
        operation: "sync agent-wiring journal cleanup",
        path: agent_home.to_path_buf(),
        source,
    })
}

fn result_from_journal(
    journal: &AgentWiringJournal,
    target: &Path,
    recovery: AgentWiringRecovery,
) -> AgentWiringResult {
    AgentWiringResult {
        client: journal.client,
        operation: journal.operation,
        target: target.to_path_buf(),
        action: journal.action,
        plan_id: journal.plan_id.clone(),
        changed: true,
        recovery,
    }
}

fn auxiliary_path(
    agent_home: &Path,
    target: &Path,
    suffix: &str,
) -> Result<PathBuf, AgentWiringError> {
    let file_name = target.file_name().ok_or_else(|| {
        AgentWiringError::Configuration("agent instruction targets must have a filename".to_owned())
    })?;
    let mut name = OsString::from(".");
    name.push(file_name);
    name.push(suffix);
    Ok(agent_home.join(name))
}

struct AgentWiringLock {
    _file: File,
}

impl AgentWiringLock {
    fn acquire(agent_home: &Path, target: &Path) -> Result<Self, AgentWiringError> {
        let path = auxiliary_path(agent_home, target, LOCK_SUFFIX)?;
        match create_file_atomically(&path, b"") {
            Ok(()) => {
                sync_directory(agent_home).map_err(|source| AgentWiringError::FileSystem {
                    operation: "sync the agent-wiring lock file",
                    path: agent_home.to_path_buf(),
                    source,
                })?
            }
            Err(AtomicCreateError::Conflict { .. }) => {
                let metadata =
                    fs::symlink_metadata(&path).map_err(|source| AgentWiringError::FileSystem {
                        operation: "inspect the agent-wiring lock file",
                        path: path.clone(),
                        source,
                    })?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(AgentWiringError::Conflict {
                        path,
                        reason: "the agent-wiring lock path is not a regular file".to_owned(),
                    });
                }
            }
            Err(error) => return Err(map_atomic_create(error)),
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|source| AgentWiringError::FileSystem {
                operation: "open the agent-wiring lock file",
                path: path.clone(),
                source,
            })?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(AgentWiringError::Conflict {
                path,
                reason: "another Akasha agent-wiring writer holds the lock".to_owned(),
            }),
            Err(TryLockError::Error(source)) => Err(AgentWiringError::FileSystem {
                operation: "acquire the agent-wiring lock",
                path,
                source,
            }),
        }
    }
}

fn map_atomic_create(error: AtomicCreateError) -> AgentWiringError {
    match error {
        AtomicCreateError::Conflict { path, source } => AgentWiringError::Conflict {
            path,
            reason: source.to_string(),
        },
        AtomicCreateError::FileSystem {
            operation,
            path,
            source,
        } => AgentWiringError::FileSystem {
            operation,
            path,
            source,
        },
    }
}

fn map_checked_replace(error: CheckedReplaceError) -> AgentWiringError {
    match error {
        CheckedReplaceError::Conflict { path, source } => AgentWiringError::Conflict {
            path,
            reason: source.to_string(),
        },
        CheckedReplaceError::FileSystem {
            operation,
            path,
            source,
        } => AgentWiringError::FileSystem {
            operation,
            path,
            source,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::resolution::ResolutionEnvironment;

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn recovers_journal_only_and_published_create_interruptions() {
        for (label, stop_after, expected_recovery) in [
            (
                "journal",
                PublicationStage::Journal,
                AgentWiringRecovery::Discarded,
            ),
            (
                "target",
                PublicationStage::Target,
                AgentWiringRecovery::Finalized,
            ),
        ] {
            let fixture = Fixture::new(label);
            let plan = prepare_agent_wiring(&fixture.request, AgentClient::Codex, &fixture.home)
                .expect("prepare create plan");
            let interrupted = catch_unwind(AssertUnwindSafe(|| {
                let _ = commit_agent_wiring_with_hook(
                    &fixture.request,
                    AgentClient::Codex,
                    &fixture.home,
                    AgentWiringOperation::Apply,
                    &plan.plan_id,
                    |stage| {
                        if std::mem::discriminant(&stage) == std::mem::discriminant(&stop_after) {
                            panic!("simulated process interruption");
                        }
                    },
                );
            }));
            assert!(interrupted.is_err());
            let journal_path = auxiliary_path(
                &fixture.home,
                &fixture.home.join("AGENTS.md"),
                JOURNAL_SUFFIX,
            )
            .expect("journal path");
            let journal_source = fs::read_to_string(&journal_path).expect("read pending journal");
            assert!(!journal_source.contains("Akasha project memory"));

            let result = apply_agent_wiring(
                &fixture.request,
                AgentClient::Codex,
                &fixture.home,
                &plan.plan_id,
            )
            .expect("recover and finish exact plan");
            assert_eq!(result.recovery, expected_recovery);
            assert_eq!(fs::read(&result.target).expect("read applied target"), {
                let mut bytes = Vec::new();
                bytes.extend_from_slice(plan.patch.replacement.as_bytes());
                bytes
            });
            assert!(
                !auxiliary_path(&fixture.home, &result.target, JOURNAL_SUFFIX)
                    .expect("journal path")
                    .exists()
            );
        }
    }

    #[test]
    fn recovery_refuses_a_third_target_state() {
        let fixture = Fixture::new("unexpected");
        let plan = prepare_agent_wiring(&fixture.request, AgentClient::Codex, &fixture.home)
            .expect("prepare create plan");
        let interrupted = catch_unwind(AssertUnwindSafe(|| {
            let _ = commit_agent_wiring_with_hook(
                &fixture.request,
                AgentClient::Codex,
                &fixture.home,
                AgentWiringOperation::Apply,
                &plan.plan_id,
                |stage| {
                    if matches!(stage, PublicationStage::Journal) {
                        panic!("simulated process interruption");
                    }
                },
            );
        }));
        assert!(interrupted.is_err());
        fs::write(fixture.home.join("AGENTS.md"), b"unexpected\n").expect("seed unexpected target");

        let error = apply_agent_wiring(
            &fixture.request,
            AgentClient::Codex,
            &fixture.home,
            &plan.plan_id,
        )
        .expect_err("unexpected target state must conflict");
        assert_eq!(error.exit_code(), 5);
        assert!(error.to_string().contains("unexpected bytes"));
        assert_eq!(
            fs::read(fixture.home.join("AGENTS.md")).expect("read preserved unexpected target"),
            b"unexpected\n"
        );
    }

    #[test]
    fn recovers_journal_only_and_published_removal_interruptions() {
        for (label, stop_after, expected_recovery) in [
            (
                "remove-journal",
                PublicationStage::Journal,
                AgentWiringRecovery::Discarded,
            ),
            (
                "remove-target",
                PublicationStage::Target,
                AgentWiringRecovery::Finalized,
            ),
        ] {
            let fixture = Fixture::new(label);
            let apply = prepare_agent_wiring(&fixture.request, AgentClient::Codex, &fixture.home)
                .expect("prepare create plan");
            apply_agent_wiring(
                &fixture.request,
                AgentClient::Codex,
                &fixture.home,
                &apply.plan_id,
            )
            .expect("apply create plan");
            let removal =
                prepare_agent_wiring_removal(&fixture.request, AgentClient::Codex, &fixture.home)
                    .expect("prepare removal plan");
            let interrupted = catch_unwind(AssertUnwindSafe(|| {
                let _ = commit_agent_wiring_with_hook(
                    &fixture.request,
                    AgentClient::Codex,
                    &fixture.home,
                    AgentWiringOperation::Remove,
                    &removal.plan_id,
                    |stage| {
                        if stage == stop_after {
                            panic!("simulated process interruption");
                        }
                    },
                );
            }));
            assert!(interrupted.is_err());

            let result = remove_agent_wiring(
                &fixture.request,
                AgentClient::Codex,
                &fixture.home,
                &removal.plan_id,
            )
            .expect("recover and finish exact removal");
            assert_eq!(result.recovery, expected_recovery);
            assert!(!result.target.exists());
        }
    }

    #[test]
    fn advisory_lock_refuses_a_concurrent_akasha_writer() {
        let fixture = Fixture::new("concurrent-lock");
        let plan = prepare_agent_wiring(&fixture.request, AgentClient::Claude, &fixture.home)
            .expect("prepare Claude plan");
        let target = target_path(AgentClient::Claude, &fixture.home).expect("Claude target");
        let _lock = AgentWiringLock::acquire(&fixture.home, &target).expect("hold wiring lock");

        let error = apply_agent_wiring(
            &fixture.request,
            AgentClient::Claude,
            &fixture.home,
            &plan.plan_id,
        )
        .expect_err("concurrent writer must conflict");

        assert_eq!(error.exit_code(), 5);
        assert!(error.to_string().contains("holds the lock"));
        assert!(!target.exists());
    }

    struct Fixture {
        _base: TempDir,
        home: PathBuf,
        request: ResolveRequest,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let base = TempDir::new(label);
            let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/resolution/valid-root");
            let root = base.path().join("root");
            let home = base.path().join("home");
            fs::create_dir_all(root.join("Meta")).expect("create root metadata");
            fs::create_dir(&home).expect("create agent home");
            fs::copy(fixture_root.join("akasha.toml"), root.join("akasha.toml"))
                .expect("copy root config");
            fs::copy(
                fixture_root.join("Meta/AGENTS.md"),
                root.join("Meta/AGENTS.md"),
            )
            .expect("copy canonical instructions");
            let request = ResolveRequest {
                root_override: Some(root.clone()),
                project_override: None,
                cwd: home.clone(),
                environment: ResolutionEnvironment::default(),
            };
            Self {
                _base: base,
                home,
                request,
            }
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "akasha-agent-wiring-unit-{label}-{}-{id}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create temporary directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
