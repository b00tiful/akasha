use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution")
}

fn run(root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args(["--root", root.to_str().unwrap(), "--project", "example"])
        .args(arguments)
        .env_remove("AKASHA_ROOT")
        .env_remove("NO_COLOR")
        .output()
        .unwrap()
}

#[test]
fn link_commands_have_bounded_plain_and_json_results_with_source_coverage() {
    let root = fixtures().join("valid-root");
    let id = "Projects/example/entities/core.md";
    let plain = run(&root, &["backlinks", id, "--limit", "1"]);
    assert!(plain.status.success());
    assert!(plain.stderr.is_empty());
    assert!(!plain.stdout.contains(&0x1b));
    let source = String::from_utf8(plain.stdout).unwrap();
    assert_eq!(
        source,
        format!(
            "Backlinks: {id}\nScanned 7 canonical notes and 2 project projections across all registered projects and global knowledge.\n2 matches; showing 1 (truncated; increase --limit)\nGlobal/entities/rust-pattern.md [canonical]\n"
        )
    );
    for flag in ["--no-color", "--json"] {
        let output = run(&root, &[flag, "backlinks", id, "--limit", "1"]);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert!(!output.stdout.contains(&0x1b));
        if flag == "--json" {
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["note"], id);
            assert_eq!(value["direction"], "incoming");
            assert_eq!(value["scanned_canonical_notes"], 7);
            assert_eq!(value["scanned_projections"], 2);
            assert_eq!(value["total_matches"], 2);
            assert_eq!(value["truncated"], true);
            assert_eq!(value["links"].as_array().unwrap().len(), 1);
            assert_eq!(value["links"][0]["id"], "Global/entities/rust-pattern.md");
        } else {
            assert_eq!(String::from_utf8(output.stdout).unwrap(), source);
        }
    }
    let outgoing = run(
        &root,
        &[
            "--json",
            "links",
            "Projects/example/events/sessions/2026-07-13.md",
        ],
    );
    assert!(outgoing.status.success());
    let value: serde_json::Value = serde_json::from_slice(&outgoing.stdout).unwrap();
    assert_eq!(value["direction"], "outgoing");
    assert_eq!(value["total_matches"], 1);
    assert_eq!(value["links"][0]["id"], id);
    let zero = run(&root, &["links", "Projects/example/index.md"]);
    assert!(zero.status.success());
    assert!(zero.stderr.is_empty());
    assert!(
        String::from_utf8(zero.stdout)
            .unwrap()
            .contains("0 matches; showing 0")
    );
}

#[test]
fn unknown_identities_and_invalid_usage_fail_only_on_stderr() {
    let root = fixtures().join("valid-root");
    for arguments in [
        vec!["backlinks"],
        vec!["links", "NOTE", "--limit", "0"],
        vec!["backlinks", "NOTE", "--limit", "101"],
    ] {
        let output = run(&root, &arguments);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
    for id in [
        "Projects/example/entities/missing.md",
        "Projects/example/entities/core",
        "../outside.md",
        "Meta/AGENTS.md",
    ] {
        let output = run(&root, &["--json", "backlinks", id]);
        assert_eq!(output.status.code(), Some(4));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .starts_with("akasha: ")
        );
    }
    let help = run(&root, &["--help"]);
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("backlinks"));
    assert!(help.contains("links"));
}

#[test]
fn stale_state_and_pending_journal_refuse_without_recovery_or_writes() {
    let fixture = Fixture::new();
    let project = fixture.root.join("Projects/example");
    let index = project.join("index.md");
    fs::write(&index, "external unsaved-to-projection content\n").unwrap();
    let before = snapshot(&fixture.root);
    let output = run(
        &fixture.root,
        &["backlinks", "Projects/example/entities/core.md"],
    );
    assert_eq!(output.status.code(), Some(4));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("project state")
    );
    assert_eq!(snapshot(&fixture.root), before);
    fs::write(
        project.join(akasha_core::NOTE_EDIT_JOURNAL_FILE),
        b"partial-journal",
    )
    .unwrap();
    let before = snapshot(&fixture.root);
    let output = run(
        &fixture.root,
        &["links", "Projects/example/entities/core.md"],
    );
    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("pending note recovery journal")
    );
    assert_eq!(snapshot(&fixture.root), before);
}

#[cfg(unix)]
#[test]
fn unicode_and_control_identities_stay_exact_in_json_and_safe_in_plain_output() {
    let fixture = Fixture::new();
    let id = "Global/entities/двигатель\u{1b}[2J.md";
    fs::write(
        fixture.root.join(id),
        fs::read(fixture.root.join("Global/entities/rust-pattern.md")).unwrap(),
    )
    .unwrap();
    let before = snapshot(&fixture.root);
    let output = run(&fixture.root, &["--json", "links", id]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(!output.stdout.contains(&0x1b));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["note"], id);
    let plain = run(&fixture.root, &["links", id]);
    assert!(plain.status.success());
    assert!(!plain.stdout.contains(&0x1b));
    assert!(
        String::from_utf8(plain.stdout)
            .unwrap()
            .contains("двигатель [2J.md")
    );
    let incoming = run(
        &fixture.root,
        &["--json", "backlinks", "Projects/example/entities/core.md"],
    );
    assert!(incoming.status.success());
    let value: serde_json::Value = serde_json::from_slice(&incoming.stdout).unwrap();
    assert!(
        value["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link["id"] == id)
    );
    assert_eq!(snapshot(&fixture.root), before);
}

#[test]
fn backlinks_default_to_thirty_results_and_expand_only_with_an_explicit_limit() {
    let fixture = Fixture::new();
    let source = fs::read(fixture.root.join("Global/entities/rust-pattern.md")).unwrap();
    for index in 0..35 {
        fs::write(
            fixture
                .root
                .join(format!("Global/entities/source-{index:02}.md")),
            &source,
        )
        .unwrap();
    }
    let before = snapshot(&fixture.root);
    for (limit, count, truncated) in [(None, 30, true), (Some("100"), 37, false)] {
        let mut arguments = vec!["--json", "backlinks", "Projects/example/entities/core.md"];
        if let Some(limit) = limit {
            arguments.extend(["--limit", limit]);
        }
        let output = run(&fixture.root, &arguments);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["total_matches"], 37);
        assert_eq!(value["truncated"], truncated);
        assert_eq!(value["links"].as_array().unwrap().len(), count);
    }
    assert_eq!(snapshot(&fixture.root), before);
}

struct Fixture {
    base: PathBuf,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let base =
            std::env::temp_dir().join(format!("akasha-link-cli-{}-{id}", std::process::id()));
        let root = base.join("valid-root");
        copy_tree(&fixtures().join("valid-root"), &root);
        copy_tree(&fixtures().join("repository"), &base.join("repository"));
        Self { base, root }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn snapshot(directory: &Path) -> Vec<(PathBuf, fs::Permissions, Vec<u8>)> {
    let mut paths = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    let mut result = Vec::new();
    for path in paths {
        let metadata = fs::metadata(&path).unwrap();
        result.push((
            path.clone(),
            metadata.permissions(),
            if metadata.is_file() {
                fs::read(&path).unwrap()
            } else {
                Vec::new()
            },
        ));
        if metadata.is_dir() {
            result.extend(snapshot(&path));
        }
    }
    result
}
