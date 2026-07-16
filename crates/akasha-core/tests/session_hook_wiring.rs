use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    AgentClient, ResolutionEnvironment, ResolveRequest, SessionHookWiringAction,
    prepare_session_hook_wiring,
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution/valid-root")
}

fn request() -> ResolveRequest {
    ResolveRequest {
        root_override: Some(fixture_root()),
        project_override: None,
        cwd: fixture_root(),
        environment: ResolutionEnvironment::default(),
    }
}

#[test]
fn absent_codex_target_produces_a_deterministic_create_without_writing() {
    let home = TempDir::new("codex-create");

    let plan = prepare_session_hook_wiring(&request(), AgentClient::Codex, home.path())
        .expect("prepare Codex hook");

    assert_eq!(plan.action, SessionHookWiringAction::Create);
    assert_eq!(plan.patch.start, 0);
    assert_eq!(plan.patch.end, 0);
    assert!(plan.patch.replacement.ends_with('\n'));
    let value: serde_json::Value =
        serde_json::from_str(&plan.patch.replacement).expect("parse prepared hook JSON");
    assert_eq!(
        value["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        "akasha breadcrumb --optional"
    );
    assert!(plan.current_sha256.is_none());
    assert!(plan.plan_id.starts_with("sha256:"));
    assert!(!plan.target.exists());
}

#[test]
fn claude_plan_inserts_hooks_without_changing_existing_bytes() {
    let home = TempDir::new("claude-existing");
    let target = home.path().join("settings.json");
    let original = b"{\n  \"theme\": \"dark\"\n}\n";
    fs::write(&target, original).expect("seed Claude settings");

    let plan = prepare_session_hook_wiring(&request(), AgentClient::Claude, home.path())
        .expect("prepare Claude hook");
    let result = apply_patch(original, &plan);

    assert_eq!(plan.action, SessionHookWiringAction::AddHooks);
    assert_eq!(&result[..plan.patch.start], &original[..plan.patch.start]);
    assert_eq!(
        &result[plan.patch.start + plan.patch.replacement.len()..],
        &original[plan.patch.end..]
    );
    let value: serde_json::Value =
        serde_json::from_slice(&result).expect("prepared result is valid JSON");
    assert_eq!(value["theme"], "dark");
    assert_eq!(
        value["hooks"]["SessionStart"][0]["matcher"],
        "startup|resume|clear|compact"
    );
    assert_eq!(fs::read(&target).expect("read unchanged target"), original);
}

#[test]
fn existing_hook_shapes_choose_the_narrowest_insertion_or_noop() {
    let home = TempDir::new("existing-shapes");
    let target = home.path().join("hooks.json");

    fs::write(&target, br#"{"hooks":{"Stop":[]},"human":true}"#).expect("seed hooks object");
    let add_session = prepare_session_hook_wiring(&request(), AgentClient::Codex, home.path())
        .expect("add SessionStart");
    assert_eq!(add_session.action, SessionHookWiringAction::AddSessionStart);

    fs::write(
        &target,
        br#"{"hooks":{"SessionStart":[{"matcher":"startup","hooks":[{"type":"command","command":"human-hook"}]}]}}"#,
    )
    .expect("seed human SessionStart hook");
    let append = prepare_session_hook_wiring(&request(), AgentClient::Codex, home.path())
        .expect("append SessionStart entry");
    assert_eq!(append.action, SessionHookWiringAction::AppendSessionStart);

    fs::write(&target, apply_patch(&fs::read(&target).unwrap(), &append))
        .expect("seed exact managed entry");
    let no_change = prepare_session_hook_wiring(&request(), AgentClient::Codex, home.path())
        .expect("recognize exact entry");
    assert_eq!(no_change.action, SessionHookWiringAction::NoChange);
    assert!(no_change.patch.replacement.is_empty());
    assert_eq!(no_change.current_sha256, Some(no_change.result_sha256));
}

#[test]
fn malformed_or_modified_managed_hook_state_fails_closed() {
    let home = TempDir::new("conflicts");
    let target = home.path().join("settings.json");

    fs::write(&target, b"{not-json").expect("seed invalid JSON");
    let invalid = prepare_session_hook_wiring(&request(), AgentClient::Claude, home.path())
        .expect_err("invalid JSON must conflict");
    assert_eq!(invalid.exit_code(), 5);
    assert!(invalid.to_string().contains("invalid JSON"));

    let modified = br#"{"hooks":{"SessionStart":[{"matcher":"startup","hooks":[{"type":"command","command":"akasha breadcrumb --optional"}]}]}}"#;
    fs::write(&target, modified).expect("seed modified Akasha entry");
    let conflict = prepare_session_hook_wiring(&request(), AgentClient::Claude, home.path())
        .expect_err("modified managed hook must conflict");
    assert_eq!(conflict.exit_code(), 5);
    assert!(conflict.to_string().contains("differs"));
    assert_eq!(fs::read(&target).expect("read preserved target"), modified);
}

fn apply_patch(current: &[u8], plan: &akasha_core::SessionHookWiringPlan) -> Vec<u8> {
    let mut result = Vec::new();
    result.extend_from_slice(&current[..plan.patch.start]);
    result.extend_from_slice(plan.patch.replacement.as_bytes());
    result.extend_from_slice(&current[plan.patch.end..]);
    result
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "akasha-core-session-hook-{label}-{}-{id}",
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
