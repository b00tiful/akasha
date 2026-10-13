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
    assert_eq!(value["omissions"].as_array().map(Vec::len), Some(0));
    assert!(value["rendered_chars"].as_u64().expect("character count") <= 16_000);
}

#[test]
fn plain_context_has_lower_wire_overhead_at_equal_fidelity() {
    let fixture = fixtures();
    let root = fixture.join("valid-root");
    let binary = env!("CARGO_BIN_EXE_akasha");

    let plain = Command::new(binary)
        .args([
            "--root",
            root.to_str().expect("fixture path is UTF-8"),
            "--project",
            "example",
            "context",
        ])
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run plain akasha context");
    let json = Command::new(binary)
        .args([
            "--root",
            root.to_str().expect("fixture path is UTF-8"),
            "--project",
            "example",
            "--json",
            "context",
        ])
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run JSON akasha context");

    assert!(plain.status.success());
    assert!(plain.stderr.is_empty());
    assert!(json.status.success());
    assert!(json.stderr.is_empty());

    let plain_text = String::from_utf8(plain.stdout).expect("plain stdout is UTF-8");
    let json_value: serde_json::Value =
        serde_json::from_slice(&json.stdout).expect("parse context JSON");
    let rendered_chars = json_value["rendered_chars"]
        .as_u64()
        .expect("rendered character count") as usize;
    assert_eq!(plain_text.chars().count(), rendered_chars);

    for entry in json_value["entries"].as_array().expect("context entries") {
        let content = entry["content"].as_str().expect("entry content");
        assert!(
            plain_text.contains(content),
            "plain context omitted selected JSON entry content: {content:?}"
        );
    }

    assert!(
        plain_text.len() < json.stdout.len(),
        "plain Markdown should have less wire overhead than equivalent pretty JSON"
    );
}

#[test]
fn explicit_context_budget_preserves_default_and_plain_json_selection() {
    let root = fixtures().join("valid-root");
    let run = |arguments: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_akasha"))
            .args(["--root", root.to_str().unwrap(), "--project", "example"])
            .args(arguments)
            .env_remove("AKASHA_ROOT")
            .output()
            .unwrap()
    };
    let default = run(&["context"]);
    assert!(default.status.success());
    assert_eq!(
        default.stdout,
        run(&["context", "--max-chars", "16000"]).stdout
    );
    let json = run(&["--json", "context"]);
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let full_count = value["rendered_chars"].as_u64().unwrap();
    let budget = (full_count - 1).to_string();
    for bound in [budget.as_str(), "32000"] {
        let plain = run(&["context", "--max-chars", bound]);
        let json = run(&["--json", "context", "--max-chars", bound]);
        assert!(plain.status.success() && json.status.success());
        assert!(plain.stderr.is_empty() && json.stderr.is_empty());
        assert!(!plain.stdout.contains(&0x1b) && !json.stdout.contains(&0x1b));
        let markdown = String::from_utf8(plain.stdout).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
        assert_eq!(value["max_chars"], bound.parse::<u32>().unwrap());
        assert_eq!(value["rendered_chars"], markdown.chars().count());
        assert!(markdown.chars().count() <= bound.parse::<usize>().unwrap());
        assert_eq!(value["truncated"], bound == budget);
        for entry in value["entries"].as_array().unwrap() {
            assert!(markdown.contains(entry["content"].as_str().unwrap()));
        }
    }
}

#[test]
fn invalid_and_tiny_context_budgets_fail_only_on_stderr() {
    let root = fixtures().join("valid-root");
    for (arguments, code) in [
        (vec!["context", "--max-chars", "0"], 2),
        (vec!["context", "--max-chars", "-1"], 2),
        (vec!["context", "--max-chars", "words"], 2),
        (vec!["context", "--max-chars", "4294967296"], 2),
        (vec!["context", "--max-chars"], 2),
        (vec!["context", "--max-chars", "1", "--max-chars", "2"], 2),
        (vec!["context", "--max-chars", "1"], 4),
        (vec!["--json", "context", "--max-chars", "1"], 4),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
            .args(["--root", root.to_str().unwrap(), "--project", "example"])
            .args(&arguments)
            .env_remove("AKASHA_ROOT")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert!(!output.stderr.is_empty(), "{arguments:?}");
    }
}
