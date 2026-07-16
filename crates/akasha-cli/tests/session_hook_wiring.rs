use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution/valid-root")
}

#[test]
fn plain_codex_preparation_reports_create_and_writes_nothing() {
    let home = TempDir::new("plain-codex");
    let output = run_prepare(home.path(), "codex", false);

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("plain output is UTF-8");
    assert!(stdout.contains("prepared session hook: codex"));
    assert!(stdout.contains("action: create"));
    assert!(stdout.contains("akasha breadcrumb --optional"));
    assert!(!home.path().join("hooks.json").exists());
}

#[test]
fn json_claude_preparation_preserves_the_existing_settings_file() {
    let home = TempDir::new("json-claude");
    let target = home.path().join("settings.json");
    let original = b"{\"theme\":\"dark\"}\n";
    fs::write(&target, original).expect("seed Claude settings");

    let output = run_prepare(home.path(), "claude", true);

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse hook plan JSON");
    assert_eq!(value["client"], "claude");
    assert_eq!(value["action"], "add-hooks");
    assert_eq!(value["target"], target.to_str().unwrap());
    assert!(value["plan_id"].as_str().unwrap().starts_with("sha256:"));
    assert_eq!(
        fs::read(&target).expect("read unchanged settings"),
        original
    );
}

#[test]
fn malformed_target_uses_stderr_and_exit_five() {
    let home = TempDir::new("malformed");
    let target = home.path().join("hooks.json");
    fs::write(&target, b"[]").expect("seed invalid root shape");

    let output = run_prepare(home.path(), "codex", false);

    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr is UTF-8")
            .contains("configuration root must be a JSON object")
    );
    assert_eq!(fs::read(&target).expect("read preserved target"), b"[]");
}

#[test]
fn plain_apply_and_json_removal_manage_only_the_disposable_codex_home() {
    let home = TempDir::new("apply-remove");
    let apply_plan = run_prepare(home.path(), "codex", true);
    let apply_plan: serde_json::Value =
        serde_json::from_slice(&apply_plan.stdout).expect("parse apply plan");
    let apply_id = apply_plan["plan_id"].as_str().expect("apply plan ID");

    let applied = run_commit(home.path(), "codex", "apply-session-hook", apply_id, false);
    assert!(applied.status.success());
    assert!(applied.stderr.is_empty());
    let stdout = String::from_utf8(applied.stdout).expect("plain output is UTF-8");
    assert!(stdout.contains("applied session hook: codex"));
    assert!(stdout.contains("changed: true"));
    assert!(home.path().join("hooks.json").exists());

    let removal_plan = run_prepare_removal(home.path(), "codex", true);
    let removal_plan: serde_json::Value =
        serde_json::from_slice(&removal_plan.stdout).expect("parse removal plan");
    assert_eq!(removal_plan["operation"], "remove");
    assert_eq!(removal_plan["action"], "remove-managed-file");
    let removal_id = removal_plan["plan_id"].as_str().expect("removal plan ID");
    let removed = run_commit(
        home.path(),
        "codex",
        "remove-session-hook",
        removal_id,
        true,
    );

    assert!(removed.status.success());
    assert!(removed.stderr.is_empty());
    let result: serde_json::Value =
        serde_json::from_slice(&removed.stdout).expect("parse removal result");
    assert_eq!(result["operation"], "remove");
    assert_eq!(result["changed"], true);
    assert!(!home.path().join("hooks.json").exists());
}

#[test]
fn stale_apply_plan_fails_on_stderr_without_overwriting_the_target() {
    let home = TempDir::new("stale-apply");
    let plan = run_prepare(home.path(), "claude", true);
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout).expect("parse apply plan");
    let plan_id = plan["plan_id"].as_str().expect("plan ID");
    let target = home.path().join("settings.json");
    let human = b"{\"theme\":\"light\"}\n";
    fs::write(&target, human).expect("change target after preparation");

    let output = run_commit(home.path(), "claude", "apply-session-hook", plan_id, false);

    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr is UTF-8")
            .contains("plan ID no longer matches")
    );
    assert_eq!(fs::read(&target).expect("read preserved settings"), human);
}

#[test]
#[ignore = "requires an installed Codex CLI and permission to bind a loopback probe server"]
fn active_codex_client_enforces_trust_then_injects_the_breadcrumb() {
    let home = TempDir::new("active-codex");
    let apply_plan = run_prepare(home.path(), "codex", true);
    assert!(apply_plan.status.success());
    let apply_plan: serde_json::Value =
        serde_json::from_slice(&apply_plan.stdout).expect("parse apply plan");
    let apply_id = apply_plan["plan_id"].as_str().expect("apply plan ID");
    let applied = run_commit(home.path(), "codex", "apply-session-hook", apply_id, true);
    assert!(applied.status.success());

    let untrusted = run_active_codex_probe(home.path(), false);
    assert!(
        !untrusted.body.contains("Akasha example"),
        "an untrusted command hook must not reach developer context: {}",
        untrusted.body
    );

    let trusted_automation = run_active_codex_probe(home.path(), true);
    assert!(
        trusted_automation
            .body
            .contains("Akasha example — 1 open task — last handoff 2026-07-13"),
        "the vetted SessionStart hook must reach developer context: {}",
        trusted_automation.body
    );
}

struct CapturedRequest {
    body: String,
}

fn run_active_codex_probe(home: &Path, bypass_hook_trust: bool) -> CapturedRequest {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback probe server");
    listener
        .set_nonblocking(true)
        .expect("make probe listener nonblocking");
    let port = listener.local_addr().expect("read probe address").port();
    let server = thread::spawn(move || capture_one_request(listener));

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_akasha"));
    let binary_dir = binary.parent().expect("Akasha binary has a parent");
    let mut path_entries = vec![binary_dir.to_path_buf()];
    path_entries.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let path = std::env::join_paths(path_entries).expect("join probe PATH");
    let provider_url = format!("http://127.0.0.1:{port}/v1");
    let repository = fixture_root().join("../repository");
    let codex = std::env::var_os("AKASHA_CODEX_BIN").unwrap_or_else(|| "codex".into());

    let mut command = Command::new(codex);
    command.args([
        "exec",
        "--ephemeral",
        "--json",
        "--model",
        "akasha-hook-probe",
        "-c",
        "model_provider=\"akasha_probe\"",
        "-c",
        "model_providers.akasha_probe.name=\"Akasha activation probe\"",
        "-c",
        &format!("model_providers.akasha_probe.base_url=\"{provider_url}\""),
        "-c",
        "model_providers.akasha_probe.wire_api=\"responses\"",
        "-c",
        "model_providers.akasha_probe.request_max_retries=0",
        "-c",
        "model_providers.akasha_probe.stream_max_retries=0",
    ]);
    if bypass_hook_trust {
        command.arg("--dangerously-bypass-hook-trust");
    }
    let output = command
        .arg("Return exactly OK.")
        .current_dir(repository)
        .env("CODEX_HOME", home)
        .env("AKASHA_ROOT", fixture_root())
        .env("PATH", path)
        .output()
        .expect("run active Codex hook probe");

    assert!(
        !output.status.success(),
        "the probe provider intentionally refuses generation"
    );
    server
        .join()
        .expect("join probe server")
        .unwrap_or_else(|error| panic!("capture Codex request: {error}"))
}

fn capture_one_request(listener: TcpListener) -> Result<CapturedRequest, String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let (mut stream, _) = loop {
        match listener.accept() {
            Ok(connection) => break connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("Codex did not contact the probe provider within 15 seconds".into());
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(format!("accept probe connection: {error}")),
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("set probe read timeout: {error}"))?;

    let mut request = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 4096];
        let read = stream
            .read(&mut chunk)
            .map_err(|error| format!("read probe request: {error}"))?;
        if read == 0 {
            return Err("Codex closed the probe request before its headers completed".into());
        }
        request.extend_from_slice(&chunk[..read]);
        if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8(request[..header_end].to_vec())
        .map_err(|error| format!("probe headers are not UTF-8: {error}"))?;
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .ok_or_else(|| "probe request has no Content-Length header".to_owned())?;
    while request.len() - header_end < content_length {
        let mut chunk = [0_u8; 4096];
        let read = stream
            .read(&mut chunk)
            .map_err(|error| format!("read probe body: {error}"))?;
        if read == 0 {
            return Err("Codex closed the probe request before its body completed".into());
        }
        request.extend_from_slice(&chunk[..read]);
    }
    let body = String::from_utf8(request[header_end..header_end + content_length].to_vec())
        .map_err(|error| format!("probe body is not UTF-8: {error}"))?;
    let response = concat!(
        "HTTP/1.1 400 Bad Request\r\n",
        "Content-Type: application/json\r\n",
        "Content-Length: 90\r\n",
        "Connection: close\r\n",
        "\r\n",
        "{\"error\":{\"message\":\"intentional Akasha activation probe\",\"type\":\"invalid_request_error\"}}"
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| format!("write probe response: {error}"))?;

    Ok(CapturedRequest { body })
}

fn run_prepare(home: &Path, client: &str, json: bool) -> std::process::Output {
    run_prepare_operation(home, client, json, false)
}

fn run_prepare_removal(home: &Path, client: &str, json: bool) -> std::process::Output {
    run_prepare_operation(home, client, json, true)
}

fn run_prepare_operation(
    home: &Path,
    client: &str,
    json: bool,
    remove: bool,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_akasha"));
    command.args([
        "--root",
        fixture_root().to_str().expect("fixture root is UTF-8"),
        "--no-color",
    ]);
    if json {
        command.arg("--json");
    }
    command.args([
        "prepare-session-hook",
        client,
        "--home",
        home.to_str().expect("agent home is UTF-8"),
    ]);
    if remove {
        command.arg("--remove");
    }
    command
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run session hook preparation")
}

fn run_commit(
    home: &Path,
    client: &str,
    subcommand: &str,
    plan_id: &str,
    json: bool,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_akasha"));
    command.args([
        "--root",
        fixture_root().to_str().expect("fixture root is UTF-8"),
        "--no-color",
    ]);
    if json {
        command.arg("--json");
    }
    command.args([
        subcommand,
        client,
        "--plan-id",
        plan_id,
        "--home",
        home.to_str().expect("agent home is UTF-8"),
    ]);
    command
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run session hook commit")
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "akasha-cli-session-hook-{label}-{}-{id}",
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
