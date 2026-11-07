use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/resolution/valid-root");
    Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args(["--root", root.to_str().unwrap(), "--project", "example"])
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn librarian_plain_json_and_classified_failures() {
    let plain = run(&[
        "librarian-prepare",
        "problem",
        "--term",
        "Synthetic open problem",
    ]);
    assert!(
        plain.status.success(),
        "{}",
        String::from_utf8_lossy(&plain.stderr)
    );
    assert!(plain.stderr.is_empty());
    let text = String::from_utf8(plain.stdout).unwrap();
    assert!(text.contains("Librarian evidence") && text.contains("status: open"));
    assert!(!text.contains('\u{1b}'));
    let json = run(&[
        "--json",
        "librarian-prepare",
        "problem",
        "--term",
        "Synthetic open problem",
    ]);
    assert!(json.status.success());
    let packet: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(packet["evidence"].as_array().unwrap().len(), 1);
    assert_eq!(packet["project"], "example");
    for args in [
        vec!["librarian-prepare", ""],
        vec!["librarian-prepare", "problem", "--note", "../../bad.md"],
    ] {
        let failure = run(&args);
        assert_eq!(failure.status.code(), Some(4));
        assert!(failure.stdout.is_empty());
        assert!(!failure.stderr.is_empty());
    }
}

#[test]
fn librarian_actual_binary_checks_citations_and_bounds_input() {
    let dir = std::env::temp_dir().join(format!("akasha-cli-librarian-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let packet = dir.join("packet.json");
    let answer = dir.join("answer.json");
    let output = run(&[
        "--json",
        "librarian-prepare",
        "problem",
        "--term",
        "Synthetic open problem",
    ]);
    assert!(output.status.success());
    fs::write(&packet, &output.stdout).unwrap();
    let args = [
        "--json",
        "librarian-check",
        "--packet",
        packet.to_str().unwrap(),
        "--answer",
        answer.to_str().unwrap(),
    ];
    fs::write(&answer, r#"{"insufficient_evidence":false,"claims":[{"text":"The problem is open.","citations":[1]}]}"#).unwrap();
    let checked = run(&args);
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&checked.stdout).unwrap()["answer"]["claims"]
            [0]["citations"][0],
        1
    );
    for bytes in [
        br#"{"insufficient_evidence":false,"claims":[{"text":"Invented.","citations":[99]}]}"#
            .to_vec(),
        vec![b'x'; 65_537],
    ] {
        fs::write(&answer, bytes).unwrap();
        let rejected = run(&args);
        assert_eq!(rejected.status.code(), Some(4));
        assert!(rejected.stdout.is_empty());
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn librarian_source_cli_outputs_only_checked_passages_and_rejects_claims() {
    let dir = std::env::temp_dir().join(format!("akasha-cli-passages-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let packet = dir.join("packet.json");
    let selection = dir.join("selection.json");
    let prepared = run(&[
        "--json",
        "librarian-prepare",
        "problem",
        "--term",
        "Synthetic open problem",
    ]);
    assert!(prepared.status.success());
    fs::write(&packet, prepared.stdout).unwrap();
    fs::write(&selection, r#"{"selections":[{"citation":1}]}"#).unwrap();
    let args = [
        "librarian-check-sources",
        "--packet",
        packet.to_str().unwrap(),
        "--selection",
        selection.to_str().unwrap(),
    ];
    let plain = run(&args);
    assert!(
        plain.status.success(),
        "{}",
        String::from_utf8_lossy(&plain.stderr)
    );
    let text = String::from_utf8(plain.stdout).unwrap();
    assert!(text.contains("exact text verified; relevance needs review"));
    assert!(
        text.contains("Recorded status: open") && text.contains("  | # Synthetic open problem")
    );
    assert!(!text.contains('\u{1b}'));
    let json = run(&[&["--json"][..], &args].concat());
    assert!(json.status.success());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["kind"], "source_passages");
    assert_eq!(
        value["matches"][0]["id"],
        "Projects/example/records/problems/open.md"
    );
    assert!(value.get("answer").is_none());
    let source = fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/resolution/valid-root/Projects/example/records/problems/open.md",
    ))
    .unwrap();
    let body = source.split_once("\n---\n").unwrap().1;
    assert_eq!(value["matches"][0]["quote"], body);
    assert_eq!(value["matches"][0]["partial_body"], false);
    for bad in [
        r#"{"selections":[{"citation":1,"quote":"invented cause"}]}"#,
        r#"{"selections":[],"claims":[]}"#,
    ] {
        fs::write(&selection, bad).unwrap();
        let rejected = run(&args);
        assert_eq!(rejected.status.code(), Some(4));
        assert!(rejected.stdout.is_empty());
    }
    fs::write(&selection, r#"{"selections":[]}"#).unwrap();
    let empty = run(&args);
    assert!(empty.status.success());
    assert!(
        String::from_utf8(empty.stdout)
            .unwrap()
            .contains("No source passages selected")
    );
    fs::remove_dir_all(dir).unwrap();
}
