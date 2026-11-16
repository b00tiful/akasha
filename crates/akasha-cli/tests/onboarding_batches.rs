use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn discovery_batch_cli_preserves_default_output_and_classifies_invalid_batches() {
    let base = std::env::temp_dir().join(format!(
        "akasha-cli-onboarding-batches-{}",
        std::process::id()
    ));
    fs::create_dir(&base).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let root = base.join("valid-root");
    copy(&fixture.join("resolution/valid-root"), &root);
    copy(
        &fixture.join("local-onboarding/relay-service"),
        &base.join("repository"),
    );
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_akasha"))
            .args(["--root", root.to_str().unwrap(), "--project", "example"])
            .args(args)
            .output()
            .unwrap()
    };
    let default = run(&["--json", "local-onboarding-discover", "--directory", "."]);
    assert!(default.status.success());
    assert!(default.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&default.stdout).unwrap();
    assert!(value.get("batch").is_none());
    let batch = run(&[
        "--json",
        "local-onboarding-discover",
        "--directory",
        ".",
        "--batch",
        "2",
    ]);
    assert!(batch.status.success());
    assert!(batch.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&batch.stdout).unwrap();
    assert_eq!(value["batch"]["current"], 2);
    assert_eq!(
        value["batch"]["packets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_array().unwrap().len())
            .sum::<usize>(),
        18
    );
    assert!(serde_json::to_vec(&value["packet"]).unwrap().len() <= 16_000);
    let plain = run(&[
        "local-onboarding-discover",
        "--directory",
        ".",
        "--batch",
        "2",
    ]);
    assert!(plain.status.success());
    assert!(plain.stderr.is_empty());
    let text = String::from_utf8(plain.stdout).unwrap();
    assert!(text.contains("Batch 2/3"));
    assert!(text.contains("not review or publication"));
    assert!(!text.contains('\u{1b}'));
    for index in ["0", "4", "129"] {
        let error = run(&[
            "--json",
            "local-onboarding-discover",
            "--directory",
            ".",
            "--batch",
            index,
        ]);
        assert_eq!(error.status.code(), Some(4));
        assert!(error.stdout.is_empty());
        assert!(!error.stderr.is_empty());
    }
    fs::remove_dir_all(base).unwrap();
}
