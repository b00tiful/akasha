use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

#[path = "support/init_recovery.rs"]
mod support;

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    base: PathBuf,
    root: PathBuf,
    repository: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "akasha-cli-init-recovery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).unwrap();
        let root = base.join("memory 世界");
        let plan = akasha_core::prepare_root_setup(&root).unwrap();
        akasha_core::apply_root_setup(&plan).unwrap();
        let repository = base.join("repository 世界");
        fs::create_dir(&repository).unwrap();
        Self {
            base,
            root,
            repository,
        }
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_akasha"))
            .args(["--root", self.root.to_str().unwrap()])
            .args(args)
            .current_dir(&self.repository)
            .env_remove("AKASHA_ROOT")
            .env("XDG_CONFIG_HOME", self.base.join("config"))
            .output()
            .unwrap()
    }
    fn json_review(&self) -> serde_json::Value {
        let output = self.run(&["--json", "recover-init"]);
        assert!(output.status.success(), "{:?}", output);
        assert!(output.stderr.is_empty());
        assert!(!output.stdout.contains(&0x1b));
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(base).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files);
    files
}

#[test]
fn no_pending_recovery_is_explicit_plain_or_json_and_invalid_confirmation_fails_loudly() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.base);
    let plain = fixture.run(&["recover-init"]);
    assert!(plain.status.success());
    assert!(plain.stderr.is_empty());
    assert_eq!(
        plain.stdout,
        b"No pending initialization journal. No files changed.\n"
    );
    assert!(fixture.json_review().is_null());
    for (args, code) in [
        (&["recover-init", "--plan-id", "wrong"][..], 5),
        (&["--project", "other", "recover-init"][..], 3),
    ] {
        let output = fixture.run(args);
        assert_eq!(output.status.code(), Some(code));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
    assert_eq!(snapshot(&fixture.base), before);
    assert!(
        !fixture
            .root
            .join("Meta/.projects.yaml.akasha-init.lock")
            .exists()
    );
}

#[test]
fn cli_preview_cancel_conflict_and_checked_rollback_do_not_initialize_another_project() {
    let fixture = Fixture::new();
    let init = support::stage(&fixture.root, &fixture.repository, "first", false);
    let before = snapshot(&fixture.base);
    let plain = fixture.run(&["recover-init"]);
    assert!(plain.status.success());
    assert!(plain.stderr.is_empty());
    assert!(!plain.stdout.contains(&0x1b));
    let text = String::from_utf8(plain.stdout).unwrap();
    assert!(text.contains("INITIALIZATION RECOVERY REVIEW"));
    assert!(text.contains("Outcome: rolled-back"));
    assert!(text.contains(&format!("{:?}", init.destination.pointer)));
    assert!(text.contains("No new project is initialized"));
    let value = fixture.json_review();
    let id = value["plan_id"].as_str().unwrap();
    assert_eq!(value["project"], "first");
    assert_eq!(value["recovery"], "rolled-back");
    assert_eq!(snapshot(&fixture.base), before);
    let wrong = fixture.run(&["recover-init", "--plan-id", "wrong"]);
    assert_eq!(wrong.status.code(), Some(5));
    assert!(wrong.stdout.is_empty());
    assert_eq!(snapshot(&fixture.base), before);
    let index = init.destination.project_dir.join("index.md");
    fs::write(&index, "external 世界\r\n  ").unwrap();
    let conflicted = snapshot(&fixture.base);
    for args in [vec!["recover-init"], vec!["recover-init", "--plan-id", id]] {
        let output = fixture.run(&args);
        assert_eq!(output.status.code(), Some(5));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("unexpected")
        );
        assert_eq!(snapshot(&fixture.base), conflicted);
    }
    fs::write(&index, []).unwrap();
    let fresh = fixture.json_review();
    let id = fresh["plan_id"].as_str().unwrap();
    let output = fixture.run(&["--json", "recover-init", "--plan-id", id]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["recovery"], "rolled-back");
    assert_eq!(value["plan_id"], id);
    assert!(!init.destination.project_dir.exists());
    assert!(!init.destination.pointer.exists());
    assert_eq!(
        fs::read(&init.destination.registry).unwrap(),
        init.registry_before
    );
    assert!(fixture.json_review().is_null());
    let initialized = fixture.run(&["init", "first"]);
    assert!(initialized.status.success());
    let validated = fixture.run(&["validate"]);
    assert!(validated.status.success());
}

#[test]
fn cli_finalizes_exact_commit_preserves_every_other_file_and_refuses_replay() {
    let fixture = Fixture::new();
    support::stage(&fixture.root, &fixture.repository, "first", true);
    let value = fixture.json_review();
    assert_eq!(value["recovery"], "finalized");
    let id = value["plan_id"].as_str().unwrap();
    let mut expected = snapshot(&fixture.base);
    expected.remove(
        Path::new(value["journal_path"].as_str().unwrap())
            .strip_prefix(&fixture.base)
            .unwrap(),
    );
    let applied = fixture.run(&["recover-init", "--plan-id", id]);
    assert!(applied.status.success());
    assert!(applied.stderr.is_empty());
    assert!(
        String::from_utf8(applied.stdout)
            .unwrap()
            .contains("Outcome: finalized")
    );
    assert_eq!(snapshot(&fixture.base), expected);
    assert!(fixture.run(&["validate"]).status.success());
    let replay = fixture.run(&["recover-init", "--plan-id", id]);
    assert_eq!(replay.status.code(), Some(5));
    assert!(replay.stdout.is_empty());
    assert_eq!(snapshot(&fixture.base), expected);
}
