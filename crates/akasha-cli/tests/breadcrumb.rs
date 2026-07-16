use std::path::PathBuf;
use std::process::Command;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution")
}

#[test]
fn breadcrumb_prints_one_stable_plain_line() {
    let fixture = fixtures();
    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            fixture
                .join("valid-root")
                .to_str()
                .expect("fixture path is UTF-8"),
            "breadcrumb",
        ])
        .current_dir(fixture.join("repository/nested"))
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run akasha breadcrumb");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(!output.stdout.windows(2).any(|window| window == b"\x1b["));
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.starts_with("Akasha example — 1 open task — "));
    assert!(stdout.contains("last handoff 2026-07-13 ("));
}

#[test]
fn breadcrumb_json_contains_the_equivalent_typed_summary() {
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
            "breadcrumb",
        ])
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run akasha breadcrumb --json");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse breadcrumb JSON");
    assert_eq!(value["project"], "example");
    assert_eq!(value["open_tasks"], 1);
    assert_eq!(value["latest_handoff_date"], "2026-07-13");
    assert!(value["handoff_age_days"].is_i64());
}
