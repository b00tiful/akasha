use std::path::PathBuf;
use std::process::Command;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution")
}

#[test]
fn resolve_prints_plain_text_to_stdout() {
    let fixture = fixtures();
    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            fixture
                .join("valid-root")
                .to_str()
                .expect("fixture path is UTF-8"),
            "resolve",
        ])
        .current_dir(fixture.join("repository/nested"))
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run akasha resolve");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(stdout.contains("project: example"));
    assert!(stdout.contains("project source: pointer"));
}

#[test]
fn resolve_json_is_machine_readable() {
    let fixture = fixtures();
    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            fixture
                .join("valid-root")
                .to_str()
                .expect("fixture path is UTF-8"),
            "--project",
            "example",
            "--json",
            "resolve",
        ])
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run akasha resolve --json");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse resolve JSON");
    assert_eq!(value["project"], "example");
    assert_eq!(value["project_source"], "command-line");
    assert!(value["pointer"].is_null());
}

#[test]
fn resolution_failures_use_stderr_and_exit_code_three() {
    let fixture = fixtures();
    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            fixture
                .join("valid-root")
                .to_str()
                .expect("fixture path is UTF-8"),
            "--project",
            "Not-A-Slug",
            "resolve",
        ])
        .output()
        .expect("run failing resolve");

    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.starts_with("akasha: invalid project slug"));
}
