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
