use std::path::PathBuf;
use std::process::Command;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution")
}

#[test]
fn context_prints_bounded_markdown_in_priority_order() {
    let fixture = fixtures();
    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            fixture
                .join("valid-root")
                .to_str()
                .expect("fixture path is UTF-8"),
            "context",
        ])
        .current_dir(fixture.join("repository/nested"))
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run akasha context");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(stdout.starts_with("# Akasha context\n"));
    assert!(stdout.chars().count() <= 16_000);

    let task = stdout.find("## Open task").expect("task section");
    let problem = stdout.find("## Open problem").expect("problem section");
    let roadmap = stdout.find("## Roadmap").expect("roadmap section");
    let index = stdout.find("## Entity index").expect("index section");
    let handoff = stdout.find("## Latest handoff").expect("handoff section");
    let event = stdout.find("## Recent event").expect("event section");
    assert!(task < problem && problem < roadmap && roadmap < index);
    assert!(index < handoff && handoff < event);
}

#[test]
fn context_json_contains_the_equivalent_selected_entries() {
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
            "context",
        ])
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run akasha context --json");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse context JSON");
    assert_eq!(value["project"], "example");
    assert_eq!(value["entries"][0]["section"], "open-task");
    assert_eq!(value["entries"][5]["section"], "recent-event");
    assert_eq!(value["truncated"], false);
    assert!(value["rendered_chars"].as_u64().expect("character count") <= 16_000);
}
