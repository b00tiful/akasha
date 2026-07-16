use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};

use crate::agent_wiring::AgentClient;
use crate::resolution::{
    ResolveError, ResolveRequest, RootSource, canonicalize_directory, load_root_config,
    resolve_root,
};
use crate::state::content_fingerprint;

const HOOK_COMMAND: &str = "akasha breadcrumb --optional";
const SESSION_START_MATCHER: &str = "startup|resume|clear|compact";

/// Exact type of insertion needed to add Akasha's session-start hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionHookWiringAction {
    Create,
    AddHooks,
    AddSessionStart,
    AppendSessionStart,
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
    pub target: PathBuf,
    pub action: SessionHookWiringAction,
    pub current_sha256: Option<String>,
    pub result_sha256: String,
    pub plan_id: String,
    pub patch: SessionHookWiringPatch,
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
}

impl SessionHookWiringError {
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

impl fmt::Display for SessionHookWiringError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolution(error) => error.fmt(formatter),
            Self::Configuration(message) => formatter.write_str(message),
            Self::Conflict { path, reason } => write!(
                formatter,
                "cannot prepare session hook wiring at {}: {reason}",
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
        }
    }
}

impl Error for SessionHookWiringError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Resolution(error) => Some(error),
            Self::FileSystem { source, .. } => Some(source),
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
    let (root, root_source) = resolve_root(request)?;
    load_root_config(&root)?;
    let agent_home = canonicalize_directory(agent_home, "agent home")?;
    let target = agent_home.join(match client {
        AgentClient::Codex => "hooks.json",
        AgentClient::Claude => "settings.json",
    });
    let current = read_target(&target)?;
    let (action, patch) = plan_patch(&target, current.as_deref())?;
    let current_bytes = current.as_deref().unwrap_or_default();
    let result = apply_patch(current_bytes, &patch);
    serde_json::from_slice::<Value>(&result).map_err(|error| {
        SessionHookWiringError::Configuration(format!(
            "the prepared session hook result is not valid JSON: {error}"
        ))
    })?;
    let current_sha256 = current.as_ref().map(|bytes| content_fingerprint(bytes));
    let result_sha256 = content_fingerprint(&result);
    let plan_id = plan_fingerprint(
        client,
        action,
        &target,
        current_sha256.as_deref(),
        &result_sha256,
        &patch,
    )?;

    Ok(SessionHookWiringPlan {
        root,
        root_source,
        client,
        target,
        action,
        current_sha256,
        result_sha256,
        plan_id,
        patch,
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

fn plan_patch(
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
    let related_count = entries
        .iter()
        .filter(|entry| contains_managed_command(entry))
        .count();
    if exact_count > 1 || related_count > exact_count {
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

fn contains_managed_command(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|handlers| {
            handlers
                .iter()
                .any(|handler| handler.get("command").and_then(Value::as_str) == Some(HOOK_COMMAND))
        })
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
    client: AgentClient,
    action: SessionHookWiringAction,
    target: &'a str,
    current_sha256: Option<&'a str>,
    result_sha256: &'a str,
    patch_start: usize,
    patch_end: usize,
    patch_replacement: &'a str,
}

fn plan_fingerprint(
    client: AgentClient,
    action: SessionHookWiringAction,
    target: &Path,
    current_sha256: Option<&str>,
    result_sha256: &str,
    patch: &SessionHookWiringPatch,
) -> Result<String, SessionHookWiringError> {
    let target = target.to_str().ok_or_else(|| {
        SessionHookWiringError::Configuration(
            "client hook configuration paths must be valid UTF-8".to_owned(),
        )
    })?;
    let binding = serde_json::to_vec(&PlanBinding {
        schema: "session-hook-wiring-plan-v1",
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
        members.push(ObjectMember { key, value });
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
