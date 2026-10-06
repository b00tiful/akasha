use super::*;
use std::process::Command;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    base: PathBuf,
    root: PathBuf,
    repository: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "akasha-reviewed-init-recovery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let root = base.join("memory 世界");
        let repository = base.join("repository");
        for path in [
            root.join("Meta"),
            root.join("Projects"),
            root.join("Global"),
            root.join("Inbox"),
            root.join("templates/nested"),
            repository.clone(),
        ] {
            fs::create_dir_all(path).unwrap();
        }
        fs::write(
            root.join(ROOT_CONFIG_FILE),
            include_str!("../../../tests/fixtures/resolution/valid-root/akasha.toml"),
        )
        .unwrap();
        fs::write(root.join("Meta/projects.yaml"), "{}\n").unwrap();
        fs::write(
            root.join("templates/nested/binary.md"),
            [0, 255, b'\r', b'\n'],
        )
        .unwrap();
        Self {
            base,
            root,
            repository,
        }
    }

    fn request(&self) -> ResolveRequest {
        ResolveRequest {
            root_override: Some(self.root.clone()),
            project_override: None,
            cwd: self.base.clone(),
            environment: ResolutionEnvironment::default(),
        }
    }

    fn interrupt(&self, stage: &str) {
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "init::recovery_tests::init_recovery_child",
                "--nocapture",
            ])
            .env("AKASHA_TEST_INIT_RECOVERY_ROOT", &self.root)
            .env("AKASHA_TEST_INIT_RECOVERY_REPOSITORY", &self.repository)
            .env("AKASHA_TEST_INIT_RECOVERY_STAGE", stage)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(73), "{stage}");
    }

    fn plan(&self) -> InitRecoveryPlan {
        prepare_init_recovery(&self.request()).unwrap().unwrap()
    }

    fn journal(&self) -> PathBuf {
        init_journal_path(&self.root.join("Meta/projects.yaml"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

// Test-only environment selection; the product has no fault-injection input.
#[test]
fn init_recovery_child() {
    let Some(root) = std::env::var_os("AKASHA_TEST_INIT_RECOVERY_ROOT") else {
        return;
    };
    let stage = std::env::var("AKASHA_TEST_INIT_RECOVERY_STAGE").unwrap();
    let expected = match stage.as_str() {
        "journal" => InitPublicationStage::Journal,
        "directory" => InitPublicationStage::ScaffoldDirectory,
        "file" => InitPublicationStage::ScaffoldFile,
        "pointer" => InitPublicationStage::Pointer,
        "registry" => InitPublicationStage::Registry,
        _ => panic!("invalid fixture stage"),
    };
    let request = InitRequest {
        root_override: Some(root.into()),
        project: "example".into(),
        cwd: std::env::var_os("AKASHA_TEST_INIT_RECOVERY_REPOSITORY")
            .unwrap()
            .into(),
        environment: ResolutionEnvironment::default(),
    };
    initialize_project_with_hooks(
        &request,
        || {},
        || {},
        |published| {
            if published == expected {
                std::process::exit(73);
            }
        },
    )
    .unwrap();
    panic!("fixture did not stop at selected stage");
}

// Include directories, links, file bytes and Unix modes. Never follow a symlink.
fn snapshot(base: &Path) -> BTreeMap<PathBuf, (Vec<u8>, u32)> {
    fn walk(base: &Path, path: &Path, result: &mut BTreeMap<PathBuf, (Vec<u8>, u32)>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode()
        };
        #[cfg(not(unix))]
        let mode = 0;
        let bytes = if metadata.file_type().is_symlink() {
            format!("link:{:?}", fs::read_link(path).unwrap()).into_bytes()
        } else if metadata.is_file() {
            fs::read(path).unwrap()
        } else {
            b"directory".to_vec()
        };
        result.insert(path.strip_prefix(base).unwrap().into(), (bytes, mode));
        if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                walk(base, &entry.unwrap().path(), result);
            }
        }
    }
    let mut result = BTreeMap::new();
    walk(base, base, &mut result);
    result
}

#[test]
fn reviewed_recovery_only_covers_actual_process_exits_at_every_init_stage() {
    for (stage, outcome) in [
        ("journal", InitRecovery::Discarded),
        ("directory", InitRecovery::RolledBack),
        ("file", InitRecovery::RolledBack),
        ("pointer", InitRecovery::RolledBack),
        ("registry", InitRecovery::Finalized),
    ] {
        let fixture = Fixture::new();
        fixture.interrupt(stage);
        let interrupted = snapshot(&fixture.base);
        let plan = fixture.plan();
        assert_eq!(plan.recovery, outcome, "{stage}");
        assert_eq!(fixture.plan(), plan);
        assert_eq!(
            snapshot(&fixture.base),
            interrupted,
            "preview must write nothing"
        );
        assert_eq!(
            plan.registry_fingerprint,
            content_fingerprint(&fs::read(&plan.registry).unwrap())
        );
        let registry = fs::read(&plan.registry).unwrap();
        // Approval performs recovery alone and does not require templates or selected repository.
        fs::remove_dir_all(fixture.root.join("templates")).unwrap();
        let before = snapshot(&fixture.base);
        let result = apply_init_recovery(&fixture.request(), &plan).unwrap();
        assert_eq!(result.recovery, outcome);
        assert_eq!(result.plan_id, plan.plan_id);
        assert_eq!(fs::read(&plan.registry).unwrap(), registry);
        assert!(!fixture.journal().exists());
        assert_eq!(
            plan.project_dir.exists(),
            outcome == InitRecovery::Finalized
        );
        assert_eq!(
            fixture.repository.join(POINTER_FILE).exists(),
            outcome == InitRecovery::Finalized
        );
        let mut expected = before;
        expected.remove(
            &plan
                .journal_path
                .strip_prefix(&fixture.base)
                .unwrap()
                .to_path_buf(),
        );
        if outcome != InitRecovery::Finalized {
            expected.retain(|path, _| {
                !path.starts_with(plan.project_dir.strip_prefix(&fixture.base).unwrap())
                    && path
                        != fixture
                            .repository
                            .join(POINTER_FILE)
                            .strip_prefix(&fixture.base)
                            .unwrap()
            });
        }
        assert_eq!(
            snapshot(&fixture.base),
            expected,
            "exact recovered snapshot at {stage}"
        );
        assert!(prepare_init_recovery(&fixture.request()).unwrap().is_none());
        let recovered = snapshot(&fixture.base);
        assert_eq!(
            apply_init_recovery(&fixture.request(), &plan)
                .unwrap_err()
                .exit_code(),
            5
        );
        assert_eq!(
            snapshot(&fixture.base),
            recovered,
            "replay must refuse unchanged"
        );
        if outcome == InitRecovery::Finalized {
            // Whole-project validation separately requires the configured root template directory.
            fs::create_dir_all(fixture.root.join("templates/nested")).unwrap();
            fs::write(
                fixture.root.join("templates/nested/binary.md"),
                [0, 255, b'\r', b'\n'],
            )
            .unwrap();
            let mut selected = fixture.request();
            selected.project_override = Some("example".into());
            crate::validate_project(&selected).unwrap();
        }
    }
}

#[test]
fn no_pending_journal_is_an_explicit_no_write_result() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.base);
    assert!(prepare_init_recovery(&fixture.request()).unwrap().is_none());
    assert_eq!(snapshot(&fixture.base), before);
    assert!(
        !fixture
            .root
            .join("Meta/.projects.yaml.akasha-init.lock")
            .exists()
    );
}

#[test]
fn complete_review_tampering_and_busy_lock_refuse_without_mutation() {
    let fixture = Fixture::new();
    fixture.interrupt("pointer");
    let plan = fixture.plan();
    let mut mutations = vec![];
    macro_rules! mutated {
        ($change:expr) => {{
            let mut candidate = plan.clone();
            $change(&mut candidate);
            mutations.push(candidate);
        }};
    }
    mutated!(|p: &mut InitRecoveryPlan| p.root = fixture.base.clone());
    mutated!(|p: &mut InitRecoveryPlan| p.registry = fixture.base.join("outside.yaml"));
    mutated!(|p: &mut InitRecoveryPlan| p.journal_path = fixture.base.join("outside.json"));
    mutated!(|p: &mut InitRecoveryPlan| p.project = "other".into());
    mutated!(|p: &mut InitRecoveryPlan| p.repository_dir = fixture.base.clone());
    mutated!(|p: &mut InitRecoveryPlan| p.repository_present = false);
    mutated!(|p: &mut InitRecoveryPlan| p.project_dir = fixture.base.clone());
    mutated!(|p: &mut InitRecoveryPlan| p.recovery = InitRecovery::Finalized);
    mutated!(|p: &mut InitRecoveryPlan| p.configuration_fingerprint.push('0'));
    mutated!(|p: &mut InitRecoveryPlan| p.journal_fingerprint.push('0'));
    mutated!(|p: &mut InitRecoveryPlan| p.registry_fingerprint.push('0'));
    mutated!(|p: &mut InitRecoveryPlan| p.directories[0].present = false);
    mutated!(|p: &mut InitRecoveryPlan| p.directories[0].path = fixture.base.clone());
    mutated!(|p: &mut InitRecoveryPlan| p.files[0].present = false);
    mutated!(|p: &mut InitRecoveryPlan| p.files[0].path = fixture.base.clone());
    mutated!(|p: &mut InitRecoveryPlan| p.files[0].expected_fingerprint.push('0'));
    mutated!(|p: &mut InitRecoveryPlan| p.plan_id.push('0'));
    mutated!(|p: &mut InitRecoveryPlan| p.files.reverse());
    let before = snapshot(&fixture.base);
    for candidate in mutations {
        assert_eq!(
            apply_init_recovery(&fixture.request(), &candidate)
                .unwrap_err()
                .exit_code(),
            5
        );
        assert_eq!(snapshot(&fixture.base), before);
    }
    let owner = InitLock::acquire(&plan.registry).unwrap();
    assert_eq!(
        apply_init_recovery(&fixture.request(), &plan)
            .unwrap_err()
            .exit_code(),
        5
    );
    assert_eq!(snapshot(&fixture.base), before);
    drop(owner);
    apply_init_recovery(&fixture.request(), &plan).unwrap();
}

#[test]
fn recognized_presence_and_exact_configuration_or_journal_drift_require_fresh_review() {
    for change in ["file", "directory", "repository", "config", "journal"] {
        let fixture = Fixture::new();
        fixture.interrupt("journal");
        let plan = fixture.plan();
        match change {
            "file" => fs::write(
                fixture.repository.join(POINTER_FILE),
                "schema_version = 1\nproject = \"example\"\n",
            )
            .unwrap(),
            "directory" => fs::create_dir(&plan.project_dir).unwrap(),
            "repository" => fs::remove_dir(&fixture.repository).unwrap(),
            "config" => {
                let path = fixture.root.join(ROOT_CONFIG_FILE);
                let text = fs::read_to_string(&path).unwrap();
                fs::write(path, format!("{text}\n# new comment\n")).unwrap();
            }
            "journal" => {
                let text = fs::read_to_string(fixture.journal()).unwrap();
                fs::write(fixture.journal(), format!("{text}\n")).unwrap();
            }
            _ => unreachable!(),
        }
        let before = snapshot(&fixture.base);
        for _ in 0..2 {
            assert_eq!(
                apply_init_recovery(&fixture.request(), &plan)
                    .unwrap_err()
                    .exit_code(),
                5,
                "{change}"
            );
            assert_eq!(snapshot(&fixture.base), before);
        }
        let fresh = fixture.plan();
        assert_ne!(fresh.plan_id, plan.plan_id, "{change}");
        apply_init_recovery(&fixture.request(), &fresh).unwrap();
    }
}

#[test]
fn foreign_bytes_paths_registry_and_invalid_journal_refuse_both_preview_and_apply() {
    for change in ["file", "extra", "registry", "schema", "path"] {
        let fixture = Fixture::new();
        fixture.interrupt("pointer");
        let plan = fixture.plan();
        match change {
            "file" => fs::write(plan.project_dir.join("index.md"), "human 世界\r\n  ").unwrap(),
            "extra" => fs::write(plan.project_dir.join("human.md"), "preserve").unwrap(),
            "registry" => fs::write(&plan.registry, "{}\n# foreign\n").unwrap(),
            "schema" | "path" => {
                let mut value: serde_json::Value =
                    serde_json::from_slice(&fs::read(fixture.journal()).unwrap()).unwrap();
                if change == "schema" {
                    value["schema_version"] = 99.into();
                } else {
                    value["files"][0]["path"] = "../outside.md".into();
                }
                fs::write(fixture.journal(), serde_json::to_vec(&value).unwrap()).unwrap();
            }
            _ => unreachable!(),
        }
        let before = snapshot(&fixture.base);
        for _ in 0..2 {
            let error = prepare_init_recovery(&fixture.request()).unwrap_err();
            assert!(matches!(error.exit_code(), 4 | 5));
            assert!(apply_init_recovery(&fixture.request(), &plan).is_err());
            assert_eq!(snapshot(&fixture.base), before, "{change}");
        }
    }
}

#[test]
fn removing_a_recognized_file_invalidates_approval_even_with_the_same_outcome() {
    let fixture = Fixture::new();
    fixture.interrupt("pointer");
    let plan = fixture.plan();
    fs::remove_file(plan.project_dir.join("index.md")).unwrap();
    let before = snapshot(&fixture.base);
    let fresh = fixture.plan();
    assert_eq!(fresh.recovery, plan.recovery);
    assert_ne!(fresh.plan_id, plan.plan_id);
    assert_eq!(
        apply_init_recovery(&fixture.request(), &plan)
            .unwrap_err()
            .exit_code(),
        5
    );
    assert_eq!(snapshot(&fixture.base), before);
    apply_init_recovery(&fixture.request(), &fresh).unwrap();
}

#[test]
fn recovery_uses_configured_registry_and_projects_paths_without_touching_default_paths() {
    let fixture = Fixture::new();
    let configuration = fixture.root.join(ROOT_CONFIG_FILE);
    let source = fs::read_to_string(&configuration)
        .unwrap()
        .replace(
            "registry = \"Meta/projects.yaml\"",
            "registry = \"Registry/project-list.yaml\"",
        )
        .replace(
            "projects = \"Projects\"",
            "projects = \"Scopes/projects 世界\"",
        );
    fs::write(configuration, source).unwrap();
    fs::create_dir(fixture.root.join("Registry")).unwrap();
    fs::rename(
        fixture.root.join("Meta/projects.yaml"),
        fixture.root.join("Registry/project-list.yaml"),
    )
    .unwrap();
    fs::create_dir(fixture.root.join("Scopes")).unwrap();
    fs::rename(
        fixture.root.join("Projects"),
        fixture.root.join("Scopes/projects 世界"),
    )
    .unwrap();
    fs::create_dir(fixture.root.join("Projects")).unwrap();
    let sentinel = fixture.root.join("Projects/human.md");
    fs::write(&sentinel, "unrelated preserved bytes").unwrap();
    fixture.interrupt("pointer");
    let before = snapshot(&fixture.base);
    let plan = fixture.plan();
    assert_eq!(
        plan.registry,
        fixture.root.join("Registry/project-list.yaml")
    );
    assert_eq!(
        plan.project_dir,
        fixture.root.join("Scopes/projects 世界/example")
    );
    assert_eq!(snapshot(&fixture.base), before);
    apply_init_recovery(&fixture.request(), &plan).unwrap();
    assert_eq!(fs::read(sentinel).unwrap(), b"unrelated preserved bytes");
    assert_eq!(fs::read(plan.registry).unwrap(), b"{}\n");
    assert!(!plan.project_dir.exists());
}

#[cfg(unix)]
#[test]
fn symlinks_nonregular_journals_and_non_utf8_identities_fail_closed() {
    use std::os::unix::{ffi::OsStringExt, fs::symlink};
    for change in [
        "journal-symlink",
        "journal-directory",
        "artifact-symlink",
        "identity",
    ] {
        let fixture = Fixture::new();
        fixture.interrupt("pointer");
        let plan = fixture.plan();
        match change {
            "journal-symlink" => {
                fs::rename(fixture.journal(), fixture.base.join("saved.json")).unwrap();
                symlink(fixture.base.join("saved.json"), fixture.journal()).unwrap();
            }
            "journal-directory" => {
                fs::remove_file(fixture.journal()).unwrap();
                fs::create_dir(fixture.journal()).unwrap();
            }
            "artifact-symlink" => {
                let path = plan.project_dir.join("index.md");
                fs::remove_file(&path).unwrap();
                fs::write(fixture.base.join("outside.md"), "human").unwrap();
                symlink(fixture.base.join("outside.md"), path).unwrap();
            }
            "identity" => {
                let invalid = fixture.base.join(OsString::from_vec(vec![b'r', 255]));
                fs::rename(&fixture.root, &invalid).unwrap();
                let request = ResolveRequest {
                    root_override: Some(invalid),
                    ..fixture.request()
                };
                let before = snapshot(&fixture.base);
                assert!(prepare_init_recovery(&request).is_err());
                assert_eq!(snapshot(&fixture.base), before);
                continue;
            }
            _ => unreachable!(),
        }
        let before = snapshot(&fixture.base);
        assert!(prepare_init_recovery(&fixture.request()).is_err());
        assert!(apply_init_recovery(&fixture.request(), &plan).is_err());
        assert_eq!(snapshot(&fixture.base), before);
    }
}
