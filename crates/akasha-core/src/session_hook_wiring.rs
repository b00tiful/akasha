use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agent_wiring::AgentClient;
use crate::resolution::{
    ResolveError, ResolveRequest, RootSource, canonicalize_directory, load_root_config,
    resolve_root,
};
use crate::state::content_fingerprint;
use crate::writes::{
    AtomicCreateError, CheckedReplaceError, create_file_atomically, replace_file_if_unchanged,
    sync_directory,
};

const HOOK_COMMAND: &str = "akasha breadcrumb --optional";
const SESSION_START_MATCHER: &str = "startup|resume|clear|compact";
const JOURNAL_SCHEMA_VERSION: u32 = 1;
const LOCK_SUFFIX: &str = ".akasha-session-hook.lock";
const JOURNAL_SUFFIX: &str = ".akasha-session-hook-journal.json";

/// Intent represented by one exact session-hook wiring plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionHookWiringOperation {
    Apply,
    Remove,
}

/// Exact type of insertion needed to add Akasha's session-start hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionHookWiringAction {
    Create,
    AddHooks,
    AddSessionStart,
    AppendSessionStart,
    RemoveSessionStartEntry,
    RemoveSessionStartKey,
    RemoveHooksKey,
    RemoveManagedFile,
    NoChange,
}

/// Exact byte-range insertion represented by a read-only hook-wiring plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionHookWiringPatch {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

/// Snapshot-bound, read-only plan for one client's user-level session-start hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionHookWiringPlan {
    pub root: PathBuf,
    pub root_source: RootSource,
    pub client: AgentClient,
    pub operation: SessionHookWiringOperation,
    pub target: PathBuf,
    pub action: SessionHookWiringAction,
    pub current_sha256: Option<String>,
    pub result_sha256: Option<String>,
    pub plan_id: String,
    pub patch: SessionHookWiringPatch,
}

/// Recovery performed before a session-hook write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionHookWiringRecovery {
    None,
    Discarded,
    Finalized,
}

/// Result of applying one exact prepared session-hook plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionHookWiringResult {
    pub client: AgentClient,
    pub operation: SessionHookWiringOperation,
    pub target: PathBuf,
    pub action: SessionHookWiringAction,
    pub plan_id: String,
    pub changed: bool,
    pub recovery: SessionHookWiringRecovery,
}

/// Configuration, conflict, or filesystem failure while preparing hook wiring.
#[derive(Debug)]
pub enum SessionHookWiringError {
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
        original: Box<SessionHookWiringError>,
        recovery: Box<SessionHookWiringError>,
    },
}

impl SessionHookWiringError {
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

impl fmt::Display for SessionHookWiringError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolution(error) => error.fmt(formatter),
            Self::Configuration(message) => formatter.write_str(message),
            Self::Conflict { path, reason } => write!(
                formatter,
                "cannot manage session hook wiring at {}: {reason}",
                path.display()
            ),
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
                "session-hook operation failed ({original}) and automatic recovery also failed ({recovery})"
            ),
        }
    }
}

impl Error for SessionHookWiringError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Resolution(error) => Some(error),
            Self::FileSystem { source, .. } => Some(source),
            Self::Recovery { recovery, .. } => Some(recovery.as_ref()),
            Self::Configuration(_) | Self::Conflict { .. } => None,
        }
    }
}

impl From<ResolveError> for SessionHookWiringError {
    fn from(error: ResolveError) -> Self {
        Self::Resolution(error)
    }
}

/// Prepare an exact insertion plan for one client's user-level `SessionStart` configuration.
///
/// This operation is read-only. It validates the existing JSON and preserves every existing byte
/// outside the returned insertion range.
pub fn prepare_session_hook_wiring(
    request: &ResolveRequest,
    client: AgentClient,
    agent_home: &Path,
) -> Result<SessionHookWiringPlan, SessionHookWiringError> {
    prepare_session_hook_operation(
        request,
        client,
        agent_home,
        SessionHookWiringOperation::Apply,
    )
}

/// Prepare an exact, read-only removal plan for one managed `SessionStart` entry.
pub fn prepare_session_hook_removal(
    request: &ResolveRequest,
    client: AgentClient,
    agent_home: &Path,
) -> Result<SessionHookWiringPlan, SessionHookWiringError> {
    prepare_session_hook_operation(
        request,
        client,
        agent_home,
        SessionHookWiringOperation::Remove,
    )
}

/// Apply one exact prepared session-hook plan after recomputing its snapshot binding.
pub fn apply_session_hook_wiring(
    request: &ResolveRequest,
    client: AgentClient,
    agent_home: &Path,
    plan_id: &str,
) -> Result<SessionHookWiringResult, SessionHookWiringError> {
    commit_session_hook_with_hook(
        request,
        client,
        agent_home,
        SessionHookWiringOperation::Apply,
        plan_id,
        |_| {},
    )
}

/// Remove one exact managed session-start entry without rewriting unrelated JSON bytes.
pub fn remove_session_hook_wiring(
    request: &ResolveRequest,
    client: AgentClient,
    agent_home: &Path,
    plan_id: &str,
) -> Result<SessionHookWiringResult, SessionHookWiringError> {
    commit_session_hook_with_hook(
        request,
        client,
        agent_home,
        SessionHookWiringOperation::Remove,
        plan_id,
        |_| {},
    )
}

fn prepare_session_hook_operation(
    request: &ResolveRequest,
    client: AgentClient,
    agent_home: &Path,
    operation: SessionHookWiringOperation,
) -> Result<SessionHookWiringPlan, SessionHookWiringError> {
    let (root, root_source) = resolve_root(request)?;
    load_root_config(&root)?;
    let agent_home = canonicalize_directory(agent_home, "agent home")?;
    let target = target_path(client, &agent_home);
    let current = read_target(&target)?;
    let (action, patch, remove_file) = match operation {
        SessionHookWiringOperation::Apply => {
            let (action, patch) = plan_apply_patch(&target, current.as_deref())?;
            (action, patch, false)
        }
        SessionHookWiringOperation::Remove => plan_removal_patch(&target, current.as_deref())?,
    };
    let current_bytes = current.as_deref().unwrap_or_default();
    let result = if remove_file {
        None
    } else {
        Some(apply_patch(current_bytes, &patch))
    };
    if let Some(result) = result.as_ref() {
        serde_json::from_slice::<Value>(result).map_err(|error| {
            SessionHookWiringError::Configuration(format!(
                "the prepared session hook result is not valid JSON: {error}"
            ))
        })?;
    }
    let current_sha256 = current.as_ref().map(|bytes| content_fingerprint(bytes));
    let result_sha256 = result.as_ref().map(|bytes| content_fingerprint(bytes));
    let plan_id = plan_fingerprint(
        operation,
        client,
        action,
        &target,
        current_sha256.as_deref(),
        result_sha256.as_deref(),
        &patch,
    )?;

    Ok(SessionHookWiringPlan {
        root,
        root_source,
        client,
        operation,
        target,
        action,
        current_sha256,
        result_sha256,
        plan_id,
        patch,
    })
}

fn target_path(client: AgentClient, agent_home: &Path) -> PathBuf {
    agent_home.join(match client {
        AgentClient::Codex => "hooks.json",
        AgentClient::Claude => "settings.json",
    })
}

fn read_target(target: &Path) -> Result<Option<Vec<u8>>, SessionHookWiringError> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(SessionHookWiringError::FileSystem {
                operation: "inspect client hook configuration",
                path: target.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SessionHookWiringError::Conflict {
            path: target.to_path_buf(),
            reason: "the target exists but is not a regular file".to_owned(),
        });
    }
    fs::read(target)
        .map(Some)
        .map_err(|source| SessionHookWiringError::FileSystem {
            operation: "read client hook configuration",
            path: target.to_path_buf(),
            source,
        })
}

fn plan_apply_patch(
    target: &Path,
    current: Option<&[u8]>,
) -> Result<(SessionHookWiringAction, SessionHookWiringPatch), SessionHookWiringError> {
    let Some(current) = current else {
        let mut replacement =
            serde_json::to_string_pretty(&root_hook_value()).map_err(|error| {
                SessionHookWiringError::Configuration(format!(
                    "could not serialize the session hook configuration: {error}"
                ))
            })?;
        replacement.push('\n');
        return Ok((
            SessionHookWiringAction::Create,
            SessionHookWiringPatch {
                start: 0,
                end: 0,
                replacement,
            },
        ));
    };
    let source = std::str::from_utf8(current).map_err(|_| SessionHookWiringError::Conflict {
        path: target.to_path_buf(),
        reason: "the existing hook configuration is not UTF-8".to_owned(),
    })?;
    let root: Value =
        serde_json::from_str(source).map_err(|error| SessionHookWiringError::Conflict {
            path: target.to_path_buf(),
            reason: format!("the existing hook configuration is invalid JSON: {error}"),
        })?;
    if !root.is_object() {
        return Err(SessionHookWiringError::Conflict {
            path: target.to_path_buf(),
            reason: "the configuration root must be a JSON object".to_owned(),
        });
    }

    let root_layout = parse_object(source, 0).map_err(|reason| json_conflict(target, reason))?;
    let Some(hooks_member) = root_layout.member("hooks") else {
        return Ok((
            SessionHookWiringAction::AddHooks,
            insert_object_member(source, &root_layout, "hooks", &hooks_value()),
        ));
    };
    let hooks = root
        .get("hooks")
        .expect("source span and parsed root agree");
    if !hooks.is_object() {
        return Err(json_conflict(target, "the `hooks` value must be an object"));
    }
    let hooks_layout = parse_object(source, hooks_member.value.start)
        .map_err(|reason| json_conflict(target, reason))?;
    let Some(session_member) = hooks_layout.member("SessionStart") else {
        return Ok((
            SessionHookWiringAction::AddSessionStart,
            insert_object_member(
                source,
                &hooks_layout,
                "SessionStart",
                &Value::Array(vec![managed_hook_entry()]),
            ),
        ));
    };
    let session_start = hooks
        .get("SessionStart")
        .expect("source span and parsed hooks object agree");
    let entries = session_start
        .as_array()
        .ok_or_else(|| json_conflict(target, "the `hooks.SessionStart` value must be an array"))?;
    let layout = parse_array(source, session_member.value.start)
        .map_err(|reason| json_conflict(target, reason))?;
    let managed = managed_hook_entry();
    let exact_count = entries.iter().filter(|entry| **entry == managed).count();
    if exact_count > 1 || managed_command_count(&root) > exact_count {
        return Err(json_conflict(
            target,
            "an existing Akasha breadcrumb hook is duplicated or differs from the managed entry",
        ));
    }
    if exact_count == 1 {
        return Ok((
            SessionHookWiringAction::NoChange,
            SessionHookWiringPatch {
                start: source.len(),
                end: source.len(),
                replacement: String::new(),
            },
        ));
    }

    Ok((
        SessionHookWiringAction::AppendSessionStart,
        insert_array_element(source, &layout, &managed),
    ))
}

fn plan_removal_patch(
    target: &Path,
    current: Option<&[u8]>,
) -> Result<(SessionHookWiringAction, SessionHookWiringPatch, bool), SessionHookWiringError> {
    let Some(current) = current else {
        return Ok((
            SessionHookWiringAction::NoChange,
            SessionHookWiringPatch {
                start: 0,
                end: 0,
                replacement: String::new(),
            },
            true,
        ));
    };
    let source = std::str::from_utf8(current).map_err(|_| SessionHookWiringError::Conflict {
        path: target.to_path_buf(),
        reason: "the existing hook configuration is not UTF-8".to_owned(),
    })?;
    let root: Value =
        serde_json::from_str(source).map_err(|error| SessionHookWiringError::Conflict {
            path: target.to_path_buf(),
            reason: format!("the existing hook configuration is invalid JSON: {error}"),
        })?;
    if !root.is_object() {
        return Err(json_conflict(
            target,
            "the configuration root must be a JSON object",
        ));
    }

    let no_change = || {
        (
            SessionHookWiringAction::NoChange,
            SessionHookWiringPatch {
                start: source.len(),
                end: source.len(),
                replacement: String::new(),
            },
            false,
        )
    };
    let root_layout = parse_object(source, 0).map_err(|reason| json_conflict(target, reason))?;
    let Some(hooks_member) = root_layout.member("hooks") else {
        return Ok(no_change());
    };
    let hooks = root
        .get("hooks")
        .expect("source span and parsed root agree");
    if !hooks.is_object() {
        return Err(json_conflict(target, "the `hooks` value must be an object"));
    }
    let hooks_layout = parse_object(source, hooks_member.value.start)
        .map_err(|reason| json_conflict(target, reason))?;
    let Some(session_member) = hooks_layout.member("SessionStart") else {
        return Ok(no_change());
    };
    let session_start = hooks
        .get("SessionStart")
        .expect("source span and parsed hooks object agree");
    let entries = session_start
        .as_array()
        .ok_or_else(|| json_conflict(target, "the `hooks.SessionStart` value must be an array"))?;
    let layout = parse_array(source, session_member.value.start)
        .map_err(|reason| json_conflict(target, reason))?;
    let managed = managed_hook_entry();
    let exact: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| (*entry == managed).then_some(index))
        .collect();
    if exact.len() > 1 || managed_command_count(&root) > exact.len() {
        return Err(json_conflict(
            target,
            "an existing Akasha breadcrumb hook is duplicated or differs from the managed entry",
        ));
    }
    let Some(index) = exact.first().copied() else {
        return Ok(no_change());
    };

    if entries.len() > 1 {
        return Ok((
            SessionHookWiringAction::RemoveSessionStartEntry,
            remove_array_element(&layout, index),
            false,
        ));
    }
    if hooks_layout.members.len() > 1 {
        let index = member_index(&hooks_layout, "SessionStart")
            .expect("the parsed hooks layout contains SessionStart");
        return Ok((
            SessionHookWiringAction::RemoveSessionStartKey,
            remove_object_member(&hooks_layout, index),
            false,
        ));
    }
    if root_layout.members.len() > 1 {
        let index =
            member_index(&root_layout, "hooks").expect("the parsed root layout contains hooks");
        return Ok((
            SessionHookWiringAction::RemoveHooksKey,
            remove_object_member(&root_layout, index),
            false,
        ));
    }

    Ok((
        SessionHookWiringAction::RemoveManagedFile,
        SessionHookWiringPatch {
            start: 0,
            end: source.len(),
            replacement: String::new(),
        },
        true,
    ))
}

fn root_hook_value() -> Value {
    json!({"hooks": hooks_value()})
}

fn hooks_value() -> Value {
    json!({"SessionStart": [managed_hook_entry()]})
}

fn managed_hook_entry() -> Value {
    json!({
        "matcher": SESSION_START_MATCHER,
        "hooks": [{
            "type": "command",
            "command": HOOK_COMMAND
        }]
    })
}

fn managed_command_count(value: &Value) -> usize {
    match value {
        Value::String(source) => usize::from(source == HOOK_COMMAND),
        Value::Array(values) => values.iter().map(managed_command_count).sum(),
        Value::Object(values) => values.values().map(managed_command_count).sum(),
        Value::Null | Value::Bool(_) | Value::Number(_) => 0,
    }
}

fn insert_object_member(
    source: &str,
    object: &ObjectLayout,
    key: &str,
    value: &Value,
) -> SessionHookWiringPatch {
    let property = format!(
        "{}:{}",
        serde_json::to_string(key).expect("JSON object keys always serialize"),
        serde_json::to_string(value).expect("managed hook values always serialize")
    );
    let (start, replacement) = if let Some(last) = object.members.last() {
        (last.value.end, format!(",{property}"))
    } else {
        (object.open + 1, property)
    };
    debug_assert!(start <= source.len());
    SessionHookWiringPatch {
        start,
        end: start,
        replacement,
    }
}

fn insert_array_element(
    source: &str,
    array: &ArrayLayout,
    value: &Value,
) -> SessionHookWiringPatch {
    let element = serde_json::to_string(value).expect("managed hook values always serialize");
    let (start, replacement) = if let Some(last) = array.elements.last() {
        (last.end, format!(",{element}"))
    } else {
        (array.open + 1, element)
    };
    debug_assert!(start <= source.len());
    SessionHookWiringPatch {
        start,
        end: start,
        replacement,
    }
}

fn member_index(object: &ObjectLayout, key: &str) -> Option<usize> {
    object.members.iter().position(|member| member.key == key)
}

fn remove_object_member(object: &ObjectLayout, index: usize) -> SessionHookWiringPatch {
    let member = &object.members[index];
    let (start, end) = if index > 0 {
        (object.members[index - 1].value.end, member.value.end)
    } else if let Some(next) = object.members.get(1) {
        (member.start, next.start)
    } else {
        (member.start, member.value.end)
    };
    SessionHookWiringPatch {
        start,
        end,
        replacement: String::new(),
    }
}

fn remove_array_element(array: &ArrayLayout, index: usize) -> SessionHookWiringPatch {
    let element = array.elements[index];
    let (start, end) = if index > 0 {
        (array.elements[index - 1].end, element.end)
    } else if let Some(next) = array.elements.get(1) {
        (element.start, next.start)
    } else {
        (element.start, element.end)
    };
    SessionHookWiringPatch {
        start,
        end,
        replacement: String::new(),
    }
}

fn apply_patch(current: &[u8], patch: &SessionHookWiringPatch) -> Vec<u8> {
    let mut result = Vec::with_capacity(current.len() + patch.replacement.len());
    result.extend_from_slice(&current[..patch.start]);
    result.extend_from_slice(patch.replacement.as_bytes());
    result.extend_from_slice(&current[patch.end..]);
    result
}

#[derive(Serialize)]
struct PlanBinding<'a> {
    schema: &'static str,
    operation: SessionHookWiringOperation,
    client: AgentClient,
    action: SessionHookWiringAction,
    target: &'a str,
    current_sha256: Option<&'a str>,
    result_sha256: Option<&'a str>,
    patch_start: usize,
    patch_end: usize,
    patch_replacement: &'a str,
}

fn plan_fingerprint(
    operation: SessionHookWiringOperation,
    client: AgentClient,
    action: SessionHookWiringAction,
    target: &Path,
    current_sha256: Option<&str>,
    result_sha256: Option<&str>,
    patch: &SessionHookWiringPatch,
) -> Result<String, SessionHookWiringError> {
    let target = target.to_str().ok_or_else(|| {
        SessionHookWiringError::Configuration(
            "client hook configuration paths must be valid UTF-8".to_owned(),
        )
    })?;
    let binding = serde_json::to_vec(&PlanBinding {
        schema: "session-hook-wiring-plan-v2",
        operation,
        client,
        action,
        target,
        current_sha256,
        result_sha256,
        patch_start: patch.start,
        patch_end: patch.end,
        patch_replacement: &patch.replacement,
    })
    .map_err(|error| {
        SessionHookWiringError::Configuration(format!(
            "could not serialize the deterministic session-hook plan binding: {error}"
        ))
    })?;
    Ok(content_fingerprint(&binding))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationStage {
    Journal,
    Target,
}

fn commit_session_hook_with_hook(
    request: &ResolveRequest,
    client: AgentClient,
    agent_home: &Path,
    operation: SessionHookWiringOperation,
    expected_plan_id: &str,
    mut publication_hook: impl FnMut(PublicationStage),
) -> Result<SessionHookWiringResult, SessionHookWiringError> {
    let agent_home = canonicalize_directory(agent_home, "agent home")?;
    let target = target_path(client, &agent_home);
    let _lock = SessionHookWiringLock::acquire(&agent_home, &target)?;
    let recovered = recover_session_hook_locked(&agent_home, client, &target)?;
    if let Some(journal) = recovered.finalized.as_ref()
        && journal.operation == operation
        && journal.plan_id == expected_plan_id
    {
        return Ok(result_from_journal(
            journal,
            &target,
            SessionHookWiringRecovery::Finalized,
        ));
    }

    let plan = prepare_session_hook_operation(request, client, &agent_home, operation)?;
    if plan.plan_id != expected_plan_id {
        return Err(SessionHookWiringError::Conflict {
            path: target,
            reason: format!(
                "prepared plan ID no longer matches the exact target snapshot (expected {expected_plan_id}, current {})",
                plan.plan_id
            ),
        });
    }
    if plan.action == SessionHookWiringAction::NoChange {
        return Ok(SessionHookWiringResult {
            client,
            operation,
            target: plan.target,
            action: plan.action,
            plan_id: plan.plan_id,
            changed: false,
            recovery: recovered.recovery,
        });
    }

    let before = read_target(&plan.target)?;
    let current_sha256 = before.as_ref().map(|source| content_fingerprint(source));
    if current_sha256 != plan.current_sha256 {
        return Err(stale_plan_conflict(&plan.target));
    }
    let after = if plan.action == SessionHookWiringAction::RemoveManagedFile {
        None
    } else {
        Some(apply_patch(
            before.as_deref().unwrap_or_default(),
            &plan.patch,
        ))
    };
    if let Some(after) = after.as_ref() {
        serde_json::from_slice::<Value>(after).map_err(|error| {
            SessionHookWiringError::Configuration(format!(
                "the checked session hook result is not valid JSON: {error}"
            ))
        })?;
    }
    let result_sha256 = after.as_ref().map(|source| content_fingerprint(source));
    if result_sha256 != plan.result_sha256 {
        return Err(stale_plan_conflict(&plan.target));
    }

    let journal = SessionHookWiringJournal {
        schema_version: JOURNAL_SCHEMA_VERSION,
        client,
        operation,
        action: plan.action,
        target: plan
            .target
            .file_name()
            .expect("session hook targets always have a filename")
            .to_string_lossy()
            .into_owned(),
        plan_id: plan.plan_id.clone(),
        before_sha256: before.as_ref().map(|source| content_fingerprint(source)),
        after_sha256: after.as_ref().map(|source| content_fingerprint(source)),
    };
    let (journal_path, journal_source) = write_journal(&agent_home, &plan.target, &journal)?;
    publication_hook(PublicationStage::Journal);

    let attempt = (|| {
        publish_journaled_target(&plan.target, before.as_deref(), after.as_deref())?;
        sync_directory(&agent_home).map_err(|source| SessionHookWiringError::FileSystem {
            operation: "sync the session hook configuration directory",
            path: agent_home.clone(),
            source,
        })?;
        publication_hook(PublicationStage::Target);
        complete_journal(&journal_path, &journal_source, &agent_home)?;
        Ok(SessionHookWiringResult {
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
        Err(original) => match recover_session_hook_locked(&agent_home, client, &plan.target) {
            Ok(recovery) if recovery.recovery == SessionHookWiringRecovery::Finalized => {
                let finalized = recovery
                    .finalized
                    .as_ref()
                    .expect("finalized recovery always retains its journal result");
                Ok(result_from_journal(
                    finalized,
                    &plan.target,
                    SessionHookWiringRecovery::Finalized,
                ))
            }
            Ok(_) => Err(original),
            Err(recovery) => Err(SessionHookWiringError::Recovery {
                original: Box::new(original),
                recovery: Box::new(recovery),
            }),
        },
    }
}

fn stale_plan_conflict(path: &Path) -> SessionHookWiringError {
    SessionHookWiringError::Conflict {
        path: path.to_path_buf(),
        reason: "the exact session hook target snapshot changed while applying the prepared plan"
            .to_owned(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionHookWiringJournal {
    schema_version: u32,
    client: AgentClient,
    operation: SessionHookWiringOperation,
    action: SessionHookWiringAction,
    target: String,
    plan_id: String,
    before_sha256: Option<String>,
    after_sha256: Option<String>,
}

fn write_journal(
    agent_home: &Path,
    target: &Path,
    journal: &SessionHookWiringJournal,
) -> Result<(PathBuf, Vec<u8>), SessionHookWiringError> {
    let path = auxiliary_path(agent_home, target, JOURNAL_SUFFIX)?;
    let mut source = serde_json::to_vec_pretty(journal).map_err(|error| {
        SessionHookWiringError::Configuration(format!(
            "could not serialize the session-hook recovery journal: {error}"
        ))
    })?;
    source.push(b'\n');
    create_file_atomically(&path, &source).map_err(map_atomic_create)?;
    sync_directory(agent_home).map_err(|source| SessionHookWiringError::FileSystem {
        operation: "sync the session-hook recovery journal",
        path: agent_home.to_path_buf(),
        source,
    })?;
    Ok((path, source))
}

fn publish_journaled_target(
    target: &Path,
    before: Option<&[u8]>,
    after: Option<&[u8]>,
) -> Result<(), SessionHookWiringError> {
    match (before, after) {
        (None, Some(after)) => create_file_atomically(target, after).map_err(map_atomic_create),
        (Some(before), Some(after)) => replace_file_if_unchanged(target, before, after)
            .map(|_| ())
            .map_err(map_checked_replace),
        (Some(before), None) => remove_file_if_unchanged(target, before),
        (None, None) => Err(SessionHookWiringError::Conflict {
            path: target.to_path_buf(),
            reason: "a session-hook journal cannot transition an absent target to absence"
                .to_owned(),
        }),
    }
}

fn remove_file_if_unchanged(path: &Path, expected: &[u8]) -> Result<(), SessionHookWiringError> {
    let current =
        read_regular_if_present(path)?.ok_or_else(|| SessionHookWiringError::Conflict {
            path: path.to_path_buf(),
            reason: "the checked session-hook removal target is already absent".to_owned(),
        })?;
    if current != expected {
        return Err(stale_plan_conflict(path));
    }
    fs::remove_file(path).map_err(|source| SessionHookWiringError::FileSystem {
        operation: "remove an exact managed-only session-hook file",
        path: path.to_path_buf(),
        source,
    })
}

#[derive(Debug)]
struct RecoveredSessionHookWiring {
    recovery: SessionHookWiringRecovery,
    finalized: Option<SessionHookWiringJournal>,
}

fn recover_session_hook_locked(
    agent_home: &Path,
    client: AgentClient,
    target: &Path,
) -> Result<RecoveredSessionHookWiring, SessionHookWiringError> {
    let journal_path = auxiliary_path(agent_home, target, JOURNAL_SUFFIX)?;
    let Some((journal, source)) = read_journal(&journal_path)? else {
        return Ok(RecoveredSessionHookWiring {
            recovery: SessionHookWiringRecovery::None,
            finalized: None,
        });
    };
    validate_journal(&journal_path, &journal, client, target)?;
    let current = read_regular_if_present(target)?;
    let current_sha256 = current.as_ref().map(|source| content_fingerprint(source));
    let recovery = if current_sha256 == journal.before_sha256 {
        SessionHookWiringRecovery::Discarded
    } else if current_sha256 == journal.after_sha256 {
        SessionHookWiringRecovery::Finalized
    } else {
        return Err(SessionHookWiringError::Conflict {
            path: journal_path,
            reason: "journaled session-hook target contains unexpected bytes; automatic recovery refused"
                .to_owned(),
        });
    };
    complete_journal(&journal_path, &source, agent_home)?;
    Ok(RecoveredSessionHookWiring {
        recovery,
        finalized: (recovery == SessionHookWiringRecovery::Finalized).then_some(journal),
    })
}

fn read_journal(
    path: &Path,
) -> Result<Option<(SessionHookWiringJournal, Vec<u8>)>, SessionHookWiringError> {
    let Some(source) = read_regular_if_present(path)? else {
        return Ok(None);
    };
    let journal =
        serde_json::from_slice(&source).map_err(|error| SessionHookWiringError::Conflict {
            path: path.to_path_buf(),
            reason: format!("the session-hook recovery journal is invalid: {error}"),
        })?;
    Ok(Some((journal, source)))
}

fn validate_journal(
    journal_path: &Path,
    journal: &SessionHookWiringJournal,
    client: AgentClient,
    target: &Path,
) -> Result<(), SessionHookWiringError> {
    let target_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            SessionHookWiringError::Configuration(
                "session hook paths must be valid UTF-8".to_owned(),
            )
        })?;
    let operation_matches_action = match journal.operation {
        SessionHookWiringOperation::Apply => {
            matches!(
                journal.action,
                SessionHookWiringAction::Create
                    | SessionHookWiringAction::AddHooks
                    | SessionHookWiringAction::AddSessionStart
                    | SessionHookWiringAction::AppendSessionStart
            ) && journal.after_sha256.is_some()
        }
        SessionHookWiringOperation::Remove => match journal.action {
            SessionHookWiringAction::RemoveSessionStartEntry
            | SessionHookWiringAction::RemoveSessionStartKey
            | SessionHookWiringAction::RemoveHooksKey => {
                journal.before_sha256.is_some() && journal.after_sha256.is_some()
            }
            SessionHookWiringAction::RemoveManagedFile => {
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
        || !valid_fingerprint(&journal.plan_id)
        || !journal
            .before_sha256
            .as_deref()
            .is_none_or(valid_fingerprint)
        || !journal
            .after_sha256
            .as_deref()
            .is_none_or(valid_fingerprint)
    {
        return Err(SessionHookWiringError::Conflict {
            path: journal_path.to_path_buf(),
            reason:
                "the session-hook recovery journal does not match the selected client transaction"
                    .to_owned(),
        });
    }
    Ok(())
}

fn valid_fingerprint(value: &str) -> bool {
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
) -> Result<(), SessionHookWiringError> {
    let current =
        read_regular_if_present(path)?.ok_or_else(|| SessionHookWiringError::Conflict {
            path: path.to_path_buf(),
            reason: "the session-hook recovery journal disappeared before cleanup".to_owned(),
        })?;
    if current != expected {
        return Err(SessionHookWiringError::Conflict {
            path: path.to_path_buf(),
            reason: "the session-hook recovery journal changed before cleanup".to_owned(),
        });
    }
    fs::remove_file(path).map_err(|source| SessionHookWiringError::FileSystem {
        operation: "remove the completed session-hook recovery journal",
        path: path.to_path_buf(),
        source,
    })?;
    sync_directory(agent_home).map_err(|source| SessionHookWiringError::FileSystem {
        operation: "sync session-hook journal cleanup",
        path: agent_home.to_path_buf(),
        source,
    })
}

fn result_from_journal(
    journal: &SessionHookWiringJournal,
    target: &Path,
    recovery: SessionHookWiringRecovery,
) -> SessionHookWiringResult {
    SessionHookWiringResult {
        client: journal.client,
        operation: journal.operation,
        target: target.to_path_buf(),
        action: journal.action,
        plan_id: journal.plan_id.clone(),
        changed: true,
        recovery,
    }
}

fn read_regular_if_present(path: &Path) -> Result<Option<Vec<u8>>, SessionHookWiringError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(SessionHookWiringError::FileSystem {
                operation: "inspect a session-hook transaction file",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SessionHookWiringError::Conflict {
            path: path.to_path_buf(),
            reason: "the session-hook transaction path is not a regular file".to_owned(),
        });
    }
    fs::read(path)
        .map(Some)
        .map_err(|source| SessionHookWiringError::FileSystem {
            operation: "read a session-hook transaction file",
            path: path.to_path_buf(),
            source,
        })
}

fn auxiliary_path(
    agent_home: &Path,
    target: &Path,
    suffix: &str,
) -> Result<PathBuf, SessionHookWiringError> {
    let file_name = target.file_name().ok_or_else(|| {
        SessionHookWiringError::Configuration(
            "session hook targets must have a filename".to_owned(),
        )
    })?;
    let mut name = OsString::from(".");
    name.push(file_name);
    name.push(suffix);
    Ok(agent_home.join(name))
}

struct SessionHookWiringLock {
    _file: File,
}

impl SessionHookWiringLock {
    fn acquire(agent_home: &Path, target: &Path) -> Result<Self, SessionHookWiringError> {
        let path = auxiliary_path(agent_home, target, LOCK_SUFFIX)?;
        match create_file_atomically(&path, b"") {
            Ok(()) => {
                sync_directory(agent_home).map_err(|source| {
                    SessionHookWiringError::FileSystem {
                        operation: "sync the session-hook lock file",
                        path: agent_home.to_path_buf(),
                        source,
                    }
                })?;
            }
            Err(AtomicCreateError::Conflict { .. }) => {
                let metadata = fs::symlink_metadata(&path).map_err(|source| {
                    SessionHookWiringError::FileSystem {
                        operation: "inspect the session-hook lock file",
                        path: path.clone(),
                        source,
                    }
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(SessionHookWiringError::Conflict {
                        path,
                        reason: "the session-hook lock path is not a regular file".to_owned(),
                    });
                }
            }
            Err(error) => return Err(map_atomic_create(error)),
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|source| SessionHookWiringError::FileSystem {
                operation: "open the session-hook lock file",
                path: path.clone(),
                source,
            })?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(SessionHookWiringError::Conflict {
                path,
                reason: "another Akasha session-hook writer holds the lock".to_owned(),
            }),
            Err(TryLockError::Error(source)) => Err(SessionHookWiringError::FileSystem {
                operation: "acquire the session-hook lock",
                path,
                source,
            }),
        }
    }
}

fn map_atomic_create(error: AtomicCreateError) -> SessionHookWiringError {
    match error {
        AtomicCreateError::Conflict { path, source } => SessionHookWiringError::Conflict {
            path,
            reason: source.to_string(),
        },
        AtomicCreateError::FileSystem {
            operation,
            path,
            source,
        } => SessionHookWiringError::FileSystem {
            operation,
            path,
            source,
        },
    }
}

fn map_checked_replace(error: CheckedReplaceError) -> SessionHookWiringError {
    match error {
        CheckedReplaceError::Conflict { path, source } => SessionHookWiringError::Conflict {
            path,
            reason: source.to_string(),
        },
        CheckedReplaceError::FileSystem {
            operation,
            path,
            source,
        } => SessionHookWiringError::FileSystem {
            operation,
            path,
            source,
        },
    }
}

fn json_conflict(target: &Path, reason: impl Into<String>) -> SessionHookWiringError {
    SessionHookWiringError::Conflict {
        path: target.to_path_buf(),
        reason: reason.into(),
    }
}

#[derive(Debug, Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
}

#[derive(Debug)]
struct ObjectMember {
    start: usize,
    key: String,
    value: Span,
}

#[derive(Debug)]
struct ObjectLayout {
    open: usize,
    members: Vec<ObjectMember>,
}

impl ObjectLayout {
    fn member(&self, key: &str) -> Option<&ObjectMember> {
        self.members.iter().find(|member| member.key == key)
    }
}

#[derive(Debug)]
struct ArrayLayout {
    open: usize,
    elements: Vec<Span>,
}

fn parse_object(source: &str, start: usize) -> Result<ObjectLayout, String> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'{') {
        return Err("the expected JSON object span was not found".to_owned());
    }
    let mut cursor = start + 1;
    let mut members = Vec::new();
    loop {
        skip_whitespace(bytes, &mut cursor);
        if bytes.get(cursor) == Some(&b'}') {
            return Ok(ObjectLayout {
                open: start,
                members,
            });
        }
        let key_start = cursor;
        let key_end = parse_string(bytes, cursor)?;
        let key: String = serde_json::from_str(&source[key_start..key_end])
            .map_err(|error| format!("could not decode a JSON object key: {error}"))?;
        if members
            .iter()
            .any(|member: &ObjectMember| member.key == key)
        {
            return Err(format!("the JSON object contains duplicate key {key:?}"));
        }
        cursor = key_end;
        skip_whitespace(bytes, &mut cursor);
        if bytes.get(cursor) != Some(&b':') {
            return Err("a JSON object key is missing its value separator".to_owned());
        }
        cursor += 1;
        skip_whitespace(bytes, &mut cursor);
        let value = parse_value(bytes, cursor)?;
        cursor = value.end;
        members.push(ObjectMember {
            start: key_start,
            key,
            value,
        });
        skip_whitespace(bytes, &mut cursor);
        match bytes.get(cursor) {
            Some(b',') => cursor += 1,
            Some(b'}') => {
                return Ok(ObjectLayout {
                    open: start,
                    members,
                });
            }
            _ => return Err("a JSON object member has an invalid terminator".to_owned()),
        }
    }
}

fn parse_array(source: &str, start: usize) -> Result<ArrayLayout, String> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'[') {
        return Err("the expected JSON array span was not found".to_owned());
    }
    let mut cursor = start + 1;
    let mut elements = Vec::new();
    loop {
        skip_whitespace(bytes, &mut cursor);
        if bytes.get(cursor) == Some(&b']') {
            return Ok(ArrayLayout {
                open: start,
                elements,
            });
        }
        let value = parse_value(bytes, cursor)?;
        cursor = value.end;
        elements.push(value);
        skip_whitespace(bytes, &mut cursor);
        match bytes.get(cursor) {
            Some(b',') => cursor += 1,
            Some(b']') => {
                return Ok(ArrayLayout {
                    open: start,
                    elements,
                });
            }
            _ => return Err("a JSON array element has an invalid terminator".to_owned()),
        }
    }
}

fn parse_value(bytes: &[u8], start: usize) -> Result<Span, String> {
    let end = match bytes.get(start) {
        Some(b'"') => parse_string(bytes, start)?,
        Some(b'{') => parse_composite(bytes, start, b'{', b'}')?,
        Some(b'[') => parse_composite(bytes, start, b'[', b']')?,
        Some(_) => {
            let mut cursor = start;
            while let Some(byte) = bytes.get(cursor) {
                if byte.is_ascii_whitespace() || matches!(*byte, b',' | b'}' | b']') {
                    break;
                }
                cursor += 1;
            }
            cursor
        }
        None => return Err("a JSON value was unexpectedly missing".to_owned()),
    };
    Ok(Span { start, end })
}

fn parse_string(bytes: &[u8], start: usize) -> Result<usize, String> {
    if bytes.get(start) != Some(&b'"') {
        return Err("a JSON string was expected".to_owned());
    }
    let mut cursor = start + 1;
    while let Some(byte) = bytes.get(cursor) {
        match byte {
            b'\\' => cursor += 2,
            b'"' => return Ok(cursor + 1),
            _ => cursor += 1,
        }
    }
    Err("a JSON string was not terminated".to_owned())
}

fn parse_composite(bytes: &[u8], start: usize, opening: u8, closing: u8) -> Result<usize, String> {
    let mut cursor = start + 1;
    let mut depth = 1usize;
    while let Some(byte) = bytes.get(cursor) {
        if *byte == b'"' {
            cursor = parse_string(bytes, cursor)?;
            continue;
        }
        if *byte == opening {
            depth += 1;
        } else if *byte == closing {
            depth -= 1;
            if depth == 0 {
                return Ok(cursor + 1);
            }
        } else if (*byte == b'{' && opening != b'{') || (*byte == b'[' && opening != b'[') {
            cursor = parse_composite(
                bytes,
                cursor,
                *byte,
                if *byte == b'{' { b'}' } else { b']' },
            )?;
            continue;
        }
        cursor += 1;
    }
    Err("a JSON object or array was not terminated".to_owned())
}

fn skip_whitespace(bytes: &[u8], cursor: &mut usize) {
    while bytes
        .get(*cursor)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        *cursor += 1;
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
    fn recovers_journal_only_and_published_apply_interruptions() {
        for (label, stop_after, expected_recovery) in [
            (
                "apply-journal",
                PublicationStage::Journal,
                SessionHookWiringRecovery::Discarded,
            ),
            (
                "apply-target",
                PublicationStage::Target,
                SessionHookWiringRecovery::Finalized,
            ),
        ] {
            let fixture = Fixture::new(label);
            let plan = prepare_session_hook_wiring(
                &fixture.request,
                AgentClient::Codex,
                fixture.home.path(),
            )
            .expect("prepare hook");
            let interrupted = catch_unwind(AssertUnwindSafe(|| {
                let _ = commit_session_hook_with_hook(
                    &fixture.request,
                    AgentClient::Codex,
                    fixture.home.path(),
                    SessionHookWiringOperation::Apply,
                    &plan.plan_id,
                    |stage| {
                        if stage == stop_after {
                            panic!("simulated process interruption");
                        }
                    },
                );
            }));
            assert!(interrupted.is_err());
            let journal_path = auxiliary_path(
                fixture.home.path(),
                &fixture.home.path().join("hooks.json"),
                JOURNAL_SUFFIX,
            )
            .expect("journal path");
            let journal = fs::read_to_string(&journal_path).expect("read pending journal");
            assert!(!journal.contains(HOOK_COMMAND));

            let result = apply_session_hook_wiring(
                &fixture.request,
                AgentClient::Codex,
                fixture.home.path(),
                &plan.plan_id,
            )
            .expect("recover and apply hook");
            assert_eq!(result.recovery, expected_recovery);
            assert!(result.target.exists());
            assert!(!journal_path.exists());
        }
    }

    #[test]
    fn recovers_journal_only_and_published_removal_interruptions() {
        for (label, stop_after, expected_recovery) in [
            (
                "remove-journal",
                PublicationStage::Journal,
                SessionHookWiringRecovery::Discarded,
            ),
            (
                "remove-target",
                PublicationStage::Target,
                SessionHookWiringRecovery::Finalized,
            ),
        ] {
            let fixture = Fixture::new(label);
            let apply = prepare_session_hook_wiring(
                &fixture.request,
                AgentClient::Claude,
                fixture.home.path(),
            )
            .expect("prepare hook");
            apply_session_hook_wiring(
                &fixture.request,
                AgentClient::Claude,
                fixture.home.path(),
                &apply.plan_id,
            )
            .expect("apply hook");
            let removal = prepare_session_hook_removal(
                &fixture.request,
                AgentClient::Claude,
                fixture.home.path(),
            )
            .expect("prepare removal");
            let interrupted = catch_unwind(AssertUnwindSafe(|| {
                let _ = commit_session_hook_with_hook(
                    &fixture.request,
                    AgentClient::Claude,
                    fixture.home.path(),
                    SessionHookWiringOperation::Remove,
                    &removal.plan_id,
                    |stage| {
                        if stage == stop_after {
                            panic!("simulated process interruption");
                        }
                    },
                );
            }));
            assert!(interrupted.is_err());

            let result = remove_session_hook_wiring(
                &fixture.request,
                AgentClient::Claude,
                fixture.home.path(),
                &removal.plan_id,
            )
            .expect("recover and remove hook");
            assert_eq!(result.recovery, expected_recovery);
            assert!(!result.target.exists());
        }
    }

    #[test]
    fn recovery_refuses_an_unexpected_target_state() {
        let fixture = Fixture::new("unexpected");
        let plan =
            prepare_session_hook_wiring(&fixture.request, AgentClient::Codex, fixture.home.path())
                .expect("prepare hook");
        let interrupted = catch_unwind(AssertUnwindSafe(|| {
            let _ = commit_session_hook_with_hook(
                &fixture.request,
                AgentClient::Codex,
                fixture.home.path(),
                SessionHookWiringOperation::Apply,
                &plan.plan_id,
                |stage| {
                    if stage == PublicationStage::Journal {
                        panic!("simulated process interruption");
                    }
                },
            );
        }));
        assert!(interrupted.is_err());
        let target = fixture.home.path().join("hooks.json");
        fs::write(&target, b"{\"human\":true}\n").expect("seed unexpected target");

        let error = apply_session_hook_wiring(
            &fixture.request,
            AgentClient::Codex,
            fixture.home.path(),
            &plan.plan_id,
        )
        .expect_err("unexpected target must conflict");
        assert_eq!(error.exit_code(), 5);
        assert!(error.to_string().contains("unexpected bytes"));
    }

    #[test]
    fn advisory_lock_refuses_a_concurrent_session_hook_writer() {
        let fixture = Fixture::new("concurrent");
        let target = target_path(AgentClient::Claude, fixture.home.path());
        let plan =
            prepare_session_hook_wiring(&fixture.request, AgentClient::Claude, fixture.home.path())
                .expect("prepare hook");
        let _lock =
            SessionHookWiringLock::acquire(fixture.home.path(), &target).expect("hold lock");

        let error = apply_session_hook_wiring(
            &fixture.request,
            AgentClient::Claude,
            fixture.home.path(),
            &plan.plan_id,
        )
        .expect_err("concurrent writer must conflict");
        assert_eq!(error.exit_code(), 5);
        assert!(error.to_string().contains("holds the lock"));
    }

    struct Fixture {
        home: TempDir,
        request: ResolveRequest,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let home = TempDir::new(label);
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/resolution/valid-root");
            let request = ResolveRequest {
                root_override: Some(root),
                project_override: None,
                cwd: home.path().to_path_buf(),
                environment: ResolutionEnvironment::default(),
            };
            Self { home, request }
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "akasha-core-session-hook-unit-{label}-{}-{id}",
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
