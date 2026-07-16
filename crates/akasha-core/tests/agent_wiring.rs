use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    AgentClient, AgentWiringAction, AgentWiringPatch, AgentWiringRecovery, ResolutionEnvironment,
    ResolveRequest, apply_agent_wiring, prepare_agent_wiring, prepare_agent_wiring_removal,
    remove_agent_wiring,
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution/valid-root")
}

#[test]
fn prepares_a_create_without_writing_the_codex_home() {
    let home = TempDir::new("codex-create");
    let before = directory_entries(home.path());

    let plan = prepare_agent_wiring(
        &request(&fixture_root(), home.path()),
        AgentClient::Codex,
        home.path(),
    )
    .expect("prepare Codex wiring");

    assert_eq!(plan.action, AgentWiringAction::Create);
    assert_eq!(plan.target, home.path().join("AGENTS.md"));
    assert_eq!(plan.current_sha256, None);
    assert_eq!(plan.patch.start, 0);
    assert_eq!(plan.patch.end, 0);
    assert!(plan.patch.replacement.contains("## Akasha project memory"));
    assert!(
        plan.patch
            .replacement
            .contains("akasha-agent-wiring:v1:start")
    );
    assert_eq!(plan.plan_id.len(), 71);
    assert!(plan.plan_id.starts_with("sha256:"));
    assert_eq!(directory_entries(home.path()), before);
    assert!(!plan.target.exists());
}

#[test]
fn appends_to_regular_human_content_with_exact_crlf_preservation() {
    let home = TempDir::new("codex-append");
    let target = home.path().join("AGENTS.md");
    let original = b"# Personal\r\n\r\n- Keep this exact.\r\n";
    fs::write(&target, original).expect("seed personal instructions");

    let plan = prepare_agent_wiring(
        &request(&fixture_root(), home.path()),
        AgentClient::Codex,
        home.path(),
    )
    .expect("prepare composed Codex wiring");
    let result = patched(original, &plan.patch);

    assert_eq!(plan.action, AgentWiringAction::Append);
    assert_eq!(plan.patch.start, original.len());
    assert!(result.starts_with(original));
    assert!(result.windows(2).any(|window| window == b"\r\n"));
    assert!(!has_bare_lf(plan.patch.replacement.as_bytes()));
    assert_eq!(fs::read(&target).expect("read unchanged target"), original);
}

#[test]
fn returns_no_change_for_the_exact_managed_section() {
    let home = TempDir::new("codex-no-change");
    let request = request(&fixture_root(), home.path());
    let initial = prepare_agent_wiring(&request, AgentClient::Codex, home.path())
        .expect("prepare initial wiring");
    fs::write(&initial.target, initial.patch.replacement.as_bytes())
        .expect("seed exact managed target");

    let repeated = prepare_agent_wiring(&request, AgentClient::Codex, home.path())
        .expect("prepare repeated wiring");

    assert_eq!(repeated.action, AgentWiringAction::NoChange);
    assert_eq!(repeated.patch.start, 0);
    assert_eq!(repeated.patch.end, 0);
    assert!(repeated.patch.replacement.is_empty());
    assert_eq!(
        repeated.current_sha256.as_deref(),
        repeated.result_sha256.as_deref()
    );
}

#[test]
fn applies_only_the_exact_prepared_snapshot() {
    let home = TempDir::new("checked-apply");
    let request = request(&fixture_root(), home.path());
    let plan = prepare_agent_wiring(&request, AgentClient::Codex, home.path())
        .expect("prepare Codex wiring");

    let result = apply_agent_wiring(&request, AgentClient::Codex, home.path(), &plan.plan_id)
        .expect("apply exact Codex wiring plan");

    assert!(result.changed);
    assert_eq!(result.recovery, AgentWiringRecovery::None);
    assert_eq!(result.action, AgentWiringAction::Create);
    assert_eq!(
        fs::read(&result.target).expect("read applied target"),
        plan.patch.replacement.as_bytes()
    );

    let stale = apply_agent_wiring(&request, AgentClient::Codex, home.path(), &plan.plan_id)
        .expect_err("an already-consumed create plan must be stale");
    assert_eq!(stale.exit_code(), 5);
    assert!(stale.to_string().contains("plan ID no longer matches"));
}

#[test]
fn checked_removal_restores_exact_human_crlf_bytes() {
    let home = TempDir::new("checked-remove-append");
    let target = home.path().join("CLAUDE.md");
    let original = b"# Personal Claude guidance\r\n";
    fs::write(&target, original).expect("seed human Claude instructions");
    let request = request(&fixture_root(), home.path());
    let apply_plan = prepare_agent_wiring(&request, AgentClient::Claude, home.path())
        .expect("prepare Claude wiring");
    apply_agent_wiring(
        &request,
        AgentClient::Claude,
        home.path(),
        &apply_plan.plan_id,
    )
    .expect("apply Claude wiring");
    let removal = prepare_agent_wiring_removal(&request, AgentClient::Claude, home.path())
        .expect("prepare Claude removal");

    assert_eq!(removal.action, AgentWiringAction::RemoveManagedSection);
    let result = remove_agent_wiring(&request, AgentClient::Claude, home.path(), &removal.plan_id)
        .expect("remove Claude managed section");

    assert!(result.changed);
    assert_eq!(result.action, AgentWiringAction::RemoveManagedSection);
    assert_eq!(
        fs::read(&target).expect("read restored human file"),
        original
    );
}

#[test]
fn checked_removal_deletes_only_an_exact_akasha_created_file() {
    let home = TempDir::new("checked-remove-create");
    let request = request(&fixture_root(), home.path());
    let apply_plan = prepare_agent_wiring(&request, AgentClient::Codex, home.path())
        .expect("prepare Codex create");
    apply_agent_wiring(
        &request,
        AgentClient::Codex,
        home.path(),
        &apply_plan.plan_id,
    )
    .expect("apply Codex create");
    let removal = prepare_agent_wiring_removal(&request, AgentClient::Codex, home.path())
        .expect("prepare Codex removal");

    assert_eq!(removal.action, AgentWiringAction::RemoveCreatedFile);
    assert_eq!(removal.result_sha256, None);
    remove_agent_wiring(&request, AgentClient::Codex, home.path(), &removal.plan_id)
        .expect("remove exact Akasha-created Codex file");

    assert!(!home.path().join("AGENTS.md").exists());
}

#[test]
fn changed_target_refuses_apply_without_overwriting_human_bytes() {
    let home = TempDir::new("stale-target");
    let request = request(&fixture_root(), home.path());
    let plan = prepare_agent_wiring(&request, AgentClient::Claude, home.path())
        .expect("prepare Claude create");
    let target = home.path().join("CLAUDE.md");
    fs::write(&target, b"# Human arrived after preview\n").expect("change target after preview");

    let error = apply_agent_wiring(&request, AgentClient::Claude, home.path(), &plan.plan_id)
        .expect_err("stale target must conflict");

    assert_eq!(error.exit_code(), 5);
    assert_eq!(
        fs::read(&target).expect("read preserved human target"),
        b"# Human arrived after preview\n"
    );
}

#[test]
fn changed_canonical_source_refuses_the_stale_plan_without_creating_a_target() {
    let temp = TempDir::new("stale-source");
    let root = setup_root(temp.path());
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("create agent home");
    let request = request(&root, &home);
    let plan =
        prepare_agent_wiring(&request, AgentClient::Codex, &home).expect("prepare Codex create");
    fs::write(
        root.join("Meta/AGENTS.md"),
        "## Akasha project memory\n\nRun `akasha context --changed`.\n",
    )
    .expect("change canonical source after preview");

    let error = apply_agent_wiring(&request, AgentClient::Codex, &home, &plan.plan_id)
        .expect_err("stale source must conflict");

    assert_eq!(error.exit_code(), 5);
    assert!(!home.join("AGENTS.md").exists());
}

#[test]
fn refreshes_only_the_managed_codex_section_when_the_source_changes() {
    let temp = TempDir::new("codex-refresh");
    let root = setup_root(temp.path());
    let home = temp.path().join("codex");
    fs::create_dir(&home).expect("create Codex home");
    let request = request(&root, &home);
    let initial =
        prepare_agent_wiring(&request, AgentClient::Codex, &home).expect("prepare initial wiring");
    let prefix = b"# Human\n\n";
    let mut current = prefix.to_vec();
    current.extend_from_slice(initial.patch.replacement.as_bytes());
    fs::write(&initial.target, &current).expect("seed composed target");
    fs::write(
        root.join("Meta/AGENTS.md"),
        "## Akasha project memory\n\nRun `akasha context --fresh`.\n",
    )
    .expect("change canonical instructions");

    let refreshed =
        prepare_agent_wiring(&request, AgentClient::Codex, &home).expect("prepare refresh");
    let result = patched(&current, &refreshed.patch);

    assert_eq!(refreshed.action, AgentWiringAction::RefreshManagedSection);
    assert_eq!(&result[..prefix.len()], prefix);
    assert!(
        String::from_utf8(result)
            .expect("result is UTF-8")
            .contains("akasha context --fresh")
    );
    assert_eq!(
        fs::read(&initial.target).expect("read unchanged target"),
        current
    );
}

#[test]
fn prepares_a_claude_import_without_copying_the_canonical_payload() {
    let home = TempDir::new("claude-import");
    let target = home.path().join("CLAUDE.md");
    let original = b"# Personal Claude guidance\n";
    fs::write(&target, original).expect("seed Claude instructions");

    let plan = prepare_agent_wiring(
        &request(&fixture_root(), home.path()),
        AgentClient::Claude,
        home.path(),
    )
    .expect("prepare Claude wiring");

    assert_eq!(plan.action, AgentWiringAction::Append);
    assert!(
        plan.patch
            .replacement
            .contains(&format!("@{}", plan.source.display()))
    );
    assert!(!plan.patch.replacement.contains("Run `akasha context`"));
    assert_eq!(fs::read(&target).expect("read unchanged target"), original);
}

#[test]
fn refuses_a_nonempty_codex_global_override() {
    let home = TempDir::new("codex-override");
    let override_path = home.path().join("AGENTS.override.md");
    fs::write(&override_path, b"# Temporary override\n").expect("seed override");

    let error = prepare_agent_wiring(
        &request(&fixture_root(), home.path()),
        AgentClient::Codex,
        home.path(),
    )
    .expect_err("a shadowing override must conflict");

    assert_eq!(error.exit_code(), 5);
    assert!(error.to_string().contains("shadows AGENTS.md"));
    assert_eq!(
        fs::read(&override_path).expect("read preserved override"),
        b"# Temporary override\n"
    );
}

#[test]
fn refuses_incomplete_or_duplicate_managed_markers() {
    let home = TempDir::new("bad-markers");
    let target = home.path().join("CLAUDE.md");
    fs::write(&target, b"# Human\n<!-- akasha-agent-wiring:v1:start -->\n")
        .expect("seed incomplete markers");

    let error = prepare_agent_wiring(
        &request(&fixture_root(), home.path()),
        AgentClient::Claude,
        home.path(),
    )
    .expect_err("incomplete markers must conflict");

    assert_eq!(error.exit_code(), 5);
    assert!(error.to_string().contains("incomplete, duplicated"));
}

#[cfg(unix)]
#[test]
fn refuses_symlinked_instruction_targets() {
    use std::os::unix::fs::symlink;

    let home = TempDir::new("target-symlink");
    let human = home.path().join("human.md");
    fs::write(&human, b"# Human\n").expect("seed symlink target");
    symlink(&human, home.path().join("CLAUDE.md")).expect("create instruction symlink");

    let error = prepare_agent_wiring(
        &request(&fixture_root(), home.path()),
        AgentClient::Claude,
        home.path(),
    )
    .expect_err("symlinked target must conflict");

    assert_eq!(error.exit_code(), 5);
    assert!(error.to_string().contains("not a regular file"));
    assert_eq!(
        fs::read(&human).expect("read preserved human file"),
        b"# Human\n"
    );
}

fn request(root: &Path, cwd: &Path) -> ResolveRequest {
    ResolveRequest {
        root_override: Some(root.to_path_buf()),
        project_override: None,
        cwd: cwd.to_path_buf(),
        environment: ResolutionEnvironment::default(),
    }
}

fn patched(current: &[u8], patch: &AgentWiringPatch) -> Vec<u8> {
    let mut result = Vec::new();
    result.extend_from_slice(&current[..patch.start]);
    result.extend_from_slice(patch.replacement.as_bytes());
    result.extend_from_slice(&current[patch.end..]);
    result
}

fn has_bare_lf(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .enumerate()
        .any(|(index, byte)| *byte == b'\n' && (index == 0 || bytes[index - 1] != b'\r'))
}

fn directory_entries(path: &Path) -> Vec<PathBuf> {
    let mut entries = fs::read_dir(path)
        .expect("read directory")
        .map(|entry| entry.expect("read entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn setup_root(base: &Path) -> PathBuf {
    let root = base.join("root");
    fs::create_dir_all(root.join("Meta")).expect("create root metadata directory");
    fs::copy(fixture_root().join("akasha.toml"), root.join("akasha.toml"))
        .expect("copy root configuration");
    fs::copy(
        fixture_root().join("Meta/AGENTS.md"),
        root.join("Meta/AGENTS.md"),
    )
    .expect("copy canonical instructions");
    root
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "akasha-agent-wiring-{label}-{}-{id}",
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
