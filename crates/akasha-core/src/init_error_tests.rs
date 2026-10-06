//! Returned-I/O acceptance for the real initializer and reviewed recovery.

use super::recovery_tests::snapshot;
use super::*;
use crate::writes::error_tests::{Fault, Guard, Stage};
use std::process::{Command, Stdio};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    base: PathBuf,
    root: PathBuf,
    repository: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "akasha-init-errors-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir(&base).unwrap();
        let root = base.join("memory 世界");
        let defaults = crate::prepare_root_setup(&root).unwrap();
        fs::create_dir(&root).unwrap();
        for path in &defaults.directories {
            fs::create_dir_all(root.join(path)).unwrap();
        }
        for (path, source) in &defaults.files {
            fs::write(root.join(path), source).unwrap();
        }
        let repository = base.join("repository 世界");
        fs::create_dir(&repository).unwrap();
        fs::create_dir_all(root.join("templates/nested/deeper")).unwrap();
        fs::write(
            root.join("templates/nested/deeper/binary.bin"),
            [0, 255, 13, 10],
        )
        .unwrap();
        let fixture = Self {
            base,
            root,
            repository,
        };
        // Avoid selecting lock creation's sync instead of the operation under test.
        drop(InitLock::acquire(&fixture.registry()).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(fixture.registry(), fs::Permissions::from_mode(0o640)).unwrap();
        }
        fixture
    }

    fn registry(&self) -> PathBuf {
        self.root.join("Meta/projects.yaml")
    }
    fn journal(&self) -> PathBuf {
        init_journal_path(&self.registry())
    }
    fn pointer(&self) -> PathBuf {
        self.repository.join(POINTER_FILE)
    }
    fn project(&self) -> PathBuf {
        self.root.join("Projects/example")
    }
    fn request(&self) -> InitRequest {
        InitRequest {
            root_override: Some(self.root.clone()),
            project: "example".into(),
            cwd: self.repository.clone(),
            environment: ResolutionEnvironment::default(),
        }
    }
    fn recovery_request(&self) -> ResolveRequest {
        ResolveRequest {
            root_override: Some(self.root.clone()),
            project_override: None,
            cwd: self.base.clone(),
            environment: ResolutionEnvironment::default(),
        }
    }
    fn recover(&self) -> Result<InitRecoveryResult, InitError> {
        let request = self.recovery_request();
        let plan = prepare_init_recovery(&request)?.expect("pending fixture journal");
        apply_init_recovery(&request, &plan)
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
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(73), "{stage}");
    }
    fn unlocked(&self) {
        drop(InitLock::acquire(&self.registry()).unwrap());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn assert_io(error: &InitError) {
    assert_eq!(error.exit_code(), 6, "{error}");
    assert!(
        error.to_string().contains("synthetic returned I/O failure"),
        "{error}"
    );
}

fn source_kind(error: &InitError) -> io::ErrorKind {
    match error {
        InitError::Creation(AtomicCreateError::FileSystem { source, .. })
        | InitError::FileSystem { source, .. } => source.kind(),
        InitError::Cleanup {
            original: Some(original),
            ..
        } => source_kind(original),
        error => panic!("missing original I/O cause: {error}"),
    }
}

const KINDS: [io::ErrorKind; 3] = [
    io::ErrorKind::StorageFull,
    io::ErrorKind::PermissionDenied,
    io::ErrorKind::Other,
];
const WRITE_STAGES: [Stage; 4] = [
    Stage::Create,
    Stage::PartialWrite,
    Stage::FileSync,
    Stage::Publish,
];

impl Fixture {
    fn validate(&self) {
        let mut request = self.recovery_request();
        request.cwd = self.repository.clone();
        request.project_override = Some("example".into());
        crate::validate_project(&request).unwrap();
        crate::resolve_project(&request).unwrap();
    }

    fn assert_rollback(&self, before: &BTreeMap<PathBuf, (Vec<u8>, u32)>) {
        assert_eq!(snapshot(&self.base), *before);
        self.unlocked();
        assert!(
            prepare_init_recovery(&self.recovery_request())
                .unwrap()
                .is_none()
        );
        let plan = prepare_project_init(&self.request()).unwrap();
        apply_project_init(&self.request(), &plan).unwrap();
        self.validate();
        let after = snapshot(&self.base);
        assert_eq!(
            initialize_project(&self.request()).unwrap_err().exit_code(),
            5
        );
        assert_eq!(snapshot(&self.base), after);
    }
}

#[test]
fn returned_publication_errors_preserve_exact_preimages_and_allow_fresh_review() {
    // Both public write paths exercise the same locked transaction. Each case owns
    // a fresh root, so unexpected scratch, pointer or scaffold residue is observable.
    let shape = Fixture::new();
    let plan = prepare_project_init(&shape.request()).unwrap();
    let files: Vec<_> = plan
        .files
        .iter()
        .map(|(path, bytes)| (path.clone(), bytes.len()))
        .collect();
    for kind in KINDS {
        let reviewed = kind == io::ErrorKind::PermissionDenied;
        let mut cases = vec![(PathBuf::new(), Stage::Create)];
        cases.extend(
            plan.directories
                .iter()
                .map(|path| (path.clone(), Stage::Create)),
        );
        for (path, len) in &files {
            cases.extend(
                WRITE_STAGES
                    .into_iter()
                    .filter(|stage| *stage != Stage::PartialWrite || *len > 1)
                    .map(|stage| (path.clone(), stage)),
            );
        }
        for (relative, stage) in cases {
            let f = Fixture::new();
            let before = snapshot(&f.base);
            let plan = prepare_project_init(&f.request()).unwrap();
            let guard = Guard::arm(vec![Fault::new(f.project().join(&relative), stage, kind)]);
            let error = if reviewed {
                apply_project_init(&f.request(), &plan)
            } else {
                initialize_project(&f.request())
            }
            .unwrap_err();
            assert_io(&error);
            assert_eq!(source_kind(&error), kind);
            assert_eq!(
                guard.hits(),
                [1],
                "{relative:?}/{stage:?}/{kind:?}/{reviewed}"
            );
            drop(guard);
            f.assert_rollback(&before);
        }
        for target in ["journal", "pointer", "registry-stage", "registry"] {
            let stages = if target == "registry" {
                vec![Stage::Permissions, Stage::FileSync, Stage::Publish]
            } else {
                WRITE_STAGES.to_vec()
            };
            for stage in stages {
                let f = Fixture::new();
                let before = snapshot(&f.base);
                let plan = prepare_project_init(&f.request()).unwrap();
                let fault = match target {
                    "journal" => Fault::new(f.journal(), stage, kind),
                    "pointer" => Fault::new(f.pointer(), stage, kind),
                    "registry-stage" => Fault::new(
                        f.registry().with_file_name(".projects.yaml.akasha-init-"),
                        stage,
                        kind,
                    )
                    .file_name_prefix(),
                    "registry" => Fault::new(f.registry(), stage, kind),
                    _ => unreachable!(),
                };
                let guard = Guard::arm(vec![fault]);
                let error = if reviewed {
                    apply_project_init(&f.request(), &plan)
                } else {
                    initialize_project(&f.request())
                }
                .unwrap_err();
                assert_io(&error);
                assert_eq!(source_kind(&error), kind);
                assert_eq!(guard.hits(), [1], "{target}/{stage:?}/{kind:?}/{reviewed}");
                drop(guard);
                f.assert_rollback(&before);
            }
        }
    }
    eprintln!(
        "init matrix publication: {} cases",
        3 * (1
            + plan.directories.len()
            + files
                .iter()
                .map(|(_, len)| if *len > 1 { 4 } else { 3 })
                .sum::<usize>()
            + 15)
    );
}

#[test]
fn every_publication_directory_sync_error_retains_the_correct_commit_image() {
    let shape = Fixture::new();
    let plan = prepare_project_init(&shape.request()).unwrap();
    // Explicit order from the contract: journal, each directory/file, pointer, registry.
    let mut syncs = vec![(PathBuf::from("Meta"), "journal")];
    syncs.push((PathBuf::from("Projects"), "scaffold"));
    syncs.extend(plan.directories.iter().map(|path| {
        (
            PathBuf::from("Projects/example")
                .join(path)
                .parent()
                .unwrap()
                .to_path_buf(),
            "scaffold",
        )
    }));
    syncs.extend(plan.files.iter().map(|(path, _)| {
        (
            PathBuf::from("Projects/example")
                .join(path)
                .parent()
                .unwrap()
                .to_path_buf(),
            "scaffold",
        )
    }));
    syncs.push((PathBuf::from("../repository 世界"), "pointer"));
    syncs.push((PathBuf::from("Meta"), "registry"));
    for kind in KINDS {
        for (index, (relative, boundary)) in syncs.iter().enumerate() {
            let f = Fixture::new();
            let before = snapshot(&f.base);
            let plan = prepare_project_init(&f.request()).unwrap();
            let path = if *boundary == "pointer" {
                f.repository.clone()
            } else {
                f.root.join(relative)
            };
            let skip = syncs[..index]
                .iter()
                .filter(|(prior, _)| prior == relative)
                .count();
            let guard = Guard::arm(vec![
                Fault::new(&path, Stage::DirectorySync, kind).skip(skip),
            ]);
            assert_io(&apply_project_init(&f.request(), &plan).unwrap_err());
            assert_eq!(guard.hits(), [1], "{relative:?}/{boundary}/{skip}/{kind:?}");
            drop(guard);
            f.unlocked();
            match *boundary {
                "journal" => {
                    assert!(!f.project().exists());
                    assert!(!f.pointer().exists());
                    assert_eq!(f.recover().unwrap().recovery, InitRecovery::Discarded);
                    f.assert_rollback(&before);
                }
                "registry" => {
                    let committed = snapshot(&f.base);
                    assert_eq!(f.recover().unwrap().recovery, InitRecovery::Finalized);
                    let mut expected = committed;
                    expected.remove(f.journal().strip_prefix(&f.base).unwrap());
                    assert_eq!(snapshot(&f.base), expected);
                    f.validate();
                }
                _ => f.assert_rollback(&before),
            }
        }
    }
    eprintln!("init matrix publication sync: {} cases", 3 * syncs.len());
}

#[test]
fn returned_rollback_remove_errors_retain_authority_and_retry_exact_artifacts() {
    let shape = Fixture::new();
    let plan = prepare_project_init(&shape.request()).unwrap();
    let targets: Vec<_> = std::iter::once(PathBuf::new())
        .chain(plan.directories.iter().cloned())
        .chain(plan.files.iter().map(|(path, _)| path.clone()))
        .collect();
    for kind in KINDS {
        for target in targets.iter().map(Some).chain(std::iter::once(None)) {
            let f = Fixture::new();
            let before = snapshot(&f.base);
            let target = target.map_or_else(|| f.pointer(), |path| f.project().join(path));
            let guard = Guard::arm(vec![
                Fault::new(f.registry(), Stage::Publish, io::ErrorKind::StorageFull),
                Fault::new(&target, Stage::Remove, kind).persistent(),
            ]);
            let error = initialize_project(&f.request()).unwrap_err();
            assert_io(&error);
            assert!(matches!(
                error,
                InitError::Cleanup {
                    committed: false,
                    original: Some(_),
                    ..
                }
            ));
            assert!(target.exists());
            let journal = fs::read(f.journal()).unwrap();
            f.unlocked();
            for _ in 0..2 {
                assert_io(&f.recover().unwrap_err());
                assert_eq!(fs::read(f.journal()).unwrap(), journal);
                f.unlocked();
            }
            assert_eq!(guard.hits(), [1, 3], "{target:?}/{kind:?}");
            drop(guard);
            f.recover().unwrap();
            f.assert_rollback(&before);
        }
    }
    eprintln!(
        "init matrix rollback remove: {} cases",
        3 * (targets.len() + 1)
    );
}

#[test]
fn returned_rollback_sync_errors_cannot_be_hidden_by_absent_artifacts() {
    for kind in KINDS {
        for parent in ["repository", "projects"] {
            let f = Fixture::new();
            let before = snapshot(&f.base);
            let (path, skip) = if parent == "repository" {
                (f.repository.clone(), 1)
            } else {
                (f.root.join("Projects"), 1)
            };
            let guard = Guard::arm(vec![
                Fault::new(f.registry(), Stage::Publish, io::ErrorKind::StorageFull),
                Fault::new(&path, Stage::DirectorySync, kind)
                    .skip(skip)
                    .persistent(),
            ]);
            assert_io(&initialize_project(&f.request()).unwrap_err());
            assert!(!f.pointer().exists());
            assert!(!f.project().exists());
            let journal = fs::read(f.journal()).unwrap();
            for _ in 0..2 {
                assert_io(&f.recover().unwrap_err());
                assert_eq!(fs::read(f.journal()).unwrap(), journal);
                f.unlocked();
            }
            assert_eq!(guard.hits(), [1, 3]);
            drop(guard);
            assert_eq!(f.recover().unwrap().recovery, InitRecovery::Discarded);
            f.assert_rollback(&before);
        }
    }
    eprintln!("init matrix rollback sync: 6 cases");
}

#[test]
fn persistent_completion_sync_errors_preserve_each_surviving_commit_directory() {
    let shape = Fixture::new();
    let plan = prepare_project_init(&shape.request()).unwrap();
    let directories: Vec<_> = std::iter::once(PathBuf::from("Projects/example"))
        .chain(
            plan.directories
                .iter()
                .map(|path| PathBuf::from("Projects/example").join(path)),
        )
        .chain([PathBuf::from("Meta"), PathBuf::from("Projects")])
        .collect();
    for kind in KINDS {
        for relative in directories.iter().map(Some).chain(std::iter::once(None)) {
            let f = Fixture::new();
            // Return a committed image with authority, without spawning a child for every case.
            let guard = Guard::arm(vec![Fault::new(
                f.journal(),
                Stage::Remove,
                io::ErrorKind::Other,
            )]);
            assert_io(&initialize_project(&f.request()).unwrap_err());
            assert_eq!(guard.hits(), [1]);
            drop(guard);
            let before = snapshot(&f.base);
            let path = relative.map_or_else(|| f.repository.clone(), |path| f.root.join(path));
            let guard = Guard::arm(vec![
                Fault::new(&path, Stage::DirectorySync, kind).persistent(),
            ]);
            for _ in 0..2 {
                assert_io(&f.recover().unwrap_err());
                assert_eq!(snapshot(&f.base), before);
                f.unlocked();
            }
            assert_eq!(guard.hits(), [2], "{path:?}/{kind:?}");
            drop(guard);
            // Named init and reviewed recovery share the completion barrier.
            assert_eq!(
                initialize_project(&f.request()).unwrap().recovery,
                InitRecovery::Finalized
            );
            let mut expected = before;
            expected.remove(f.journal().strip_prefix(&f.base).unwrap());
            assert_eq!(snapshot(&f.base), expected);
            f.validate();
        }
    }
    eprintln!(
        "init matrix completion sync: {} cases",
        3 * (directories.len() + 1)
    );
}

#[test]
fn journal_unlink_and_post_unlink_sync_errors_report_exact_rollback_or_commit_state() {
    for committed in [false, true] {
        for kind in KINDS {
            for stage in [Stage::Remove, Stage::DirectorySync] {
                let f = Fixture::new();
                let before = snapshot(&f.base);
                let fault = if stage == Stage::Remove {
                    Fault::new(f.journal(), stage, kind)
                } else {
                    // Journal publication and (on commit) registry publication precede unlink.
                    Fault::new(f.root.join("Meta"), stage, kind).skip(if committed { 2 } else { 1 })
                };
                let mut faults = vec![fault];
                if !committed {
                    faults.push(Fault::new(
                        f.registry(),
                        Stage::Publish,
                        io::ErrorKind::Other,
                    ));
                }
                let guard = Guard::arm(faults);
                let error = initialize_project(&f.request()).unwrap_err();
                assert_io(&error);
                assert!(
                    matches!(error, InitError::Cleanup { committed: actual, .. } if actual == committed)
                );
                assert_eq!(guard.hits(), if committed { vec![1] } else { vec![1, 1] });
                drop(guard);
                f.unlocked();
                assert_eq!(f.journal().exists(), stage == Stage::Remove);
                if stage == Stage::Remove {
                    assert_eq!(
                        f.recover().unwrap().recovery,
                        if committed {
                            InitRecovery::Finalized
                        } else {
                            InitRecovery::Discarded
                        }
                    );
                } else {
                    assert!(
                        prepare_init_recovery(&f.recovery_request())
                            .unwrap()
                            .is_none()
                    );
                }
                if committed {
                    f.validate();
                    let after = snapshot(&f.base);
                    assert_eq!(initialize_project(&f.request()).unwrap_err().exit_code(), 5);
                    assert_eq!(snapshot(&f.base), after);
                } else {
                    f.assert_rollback(&before);
                }
            }
        }
    }
    eprintln!("init matrix writer cleanup: 12 cases");
}

#[test]
fn recovery_cleanup_errors_preserve_complete_images_and_require_fresh_review() {
    for state in ["discarded", "rolled-back", "finalized"] {
        for kind in KINDS {
            for stage in [Stage::Remove, Stage::DirectorySync] {
                let f = Fixture::new();
                let mut expected = snapshot(&f.base);
                let faults = if state == "finalized" {
                    vec![Fault::new(f.journal(), Stage::Remove, io::ErrorKind::Other)]
                } else {
                    vec![
                        Fault::new(f.registry(), Stage::Publish, io::ErrorKind::Other),
                        Fault::new(
                            if state == "discarded" {
                                f.journal()
                            } else {
                                f.pointer()
                            },
                            Stage::Remove,
                            io::ErrorKind::Other,
                        ),
                    ]
                };
                let initial = Guard::arm(faults);
                assert_io(&initialize_project(&f.request()).unwrap_err());
                assert!(initial.hits().iter().all(|hits| *hits == 1));
                drop(initial);
                if state == "finalized" {
                    expected = snapshot(&f.base);
                }
                let journal_id = f.journal().strip_prefix(&f.base).unwrap().to_path_buf();
                let journal_image = snapshot(&f.base)[&journal_id].clone();
                expected.remove(&journal_id);
                let request = f.recovery_request();
                let plan = prepare_init_recovery(&request).unwrap().unwrap();
                let fault = if stage == Stage::Remove {
                    Fault::new(f.journal(), stage, kind).persistent()
                } else {
                    Fault::new(f.root.join("Meta"), stage, kind).skip(1)
                };
                let guard = Guard::arm(vec![fault]);
                assert_io(&apply_init_recovery(&request, &plan).unwrap_err());
                f.unlocked();
                if stage == Stage::Remove {
                    let mut pending = expected.clone();
                    pending.insert(journal_id.clone(), journal_image);
                    assert_eq!(snapshot(&f.base), pending);
                    if state == "rolled-back" {
                        assert_eq!(
                            apply_init_recovery(&request, &plan)
                                .unwrap_err()
                                .exit_code(),
                            5
                        );
                    }
                    assert_io(&f.recover().unwrap_err());
                    assert_eq!(snapshot(&f.base), pending);
                    assert_eq!(guard.hits(), [2]);
                } else {
                    assert_eq!(snapshot(&f.base), expected);
                    assert_eq!(guard.hits(), [1]);
                    assert!(prepare_init_recovery(&request).unwrap().is_none());
                }
                drop(guard);
                if stage == Stage::Remove {
                    f.recover().unwrap();
                }
                assert_eq!(snapshot(&f.base), expected);
                if state == "finalized" {
                    f.validate();
                } else {
                    f.assert_rollback(&expected);
                }
            }
        }
    }
    eprintln!("init matrix recovery cleanup: 18 cases");
}

#[test]
fn foreign_artifacts_refuse_before_sync_or_cleanup_with_exact_byte_preservation() {
    for committed in [false, true] {
        for target in ["pointer", "scaffold", "registry", "extra", "symlink"] {
            if target == "symlink" && !cfg!(unix) {
                continue;
            }
            let f = Fixture::new();
            f.interrupt(if committed { "registry" } else { "pointer" });
            let request = f.recovery_request();
            let plan = prepare_init_recovery(&request).unwrap().unwrap();
            let path = match target {
                "pointer" => f.pointer(),
                "registry" => f.registry(),
                "scaffold" | "symlink" => f.project().join("templates/nested/deeper/binary.bin"),
                "extra" => f.project().join("human.md"),
                _ => unreachable!(),
            };
            let original = fs::read(&path).ok();
            if target == "registry" {
                let mut bytes = original.clone().unwrap();
                bytes.extend_from_slice(b"# human registry comment\n");
                fs::write(&path, bytes).unwrap();
            } else if target == "symlink" {
                #[cfg(unix)]
                {
                    fs::remove_file(&path).unwrap();
                    std::os::unix::fs::symlink(f.registry(), &path).unwrap();
                }
            } else {
                fs::write(&path, b"external human bytes\r\n  ").unwrap();
            }
            let foreign = snapshot(&f.base);
            let guard = Guard::arm(vec![
                Fault::new(
                    f.root.join("Projects"),
                    Stage::DirectorySync,
                    io::ErrorKind::Other,
                )
                .persistent(),
            ]);
            // Extra files in a committed scaffold are outside init's known-file
            // check; preserve that existing boundary rather than broaden deletion.
            if committed && target == "extra" {
                assert_io(&f.recover().unwrap_err());
            } else {
                for _ in 0..2 {
                    assert_eq!(
                        apply_init_recovery(&request, &plan)
                            .unwrap_err()
                            .exit_code(),
                        5
                    );
                    assert_eq!(initialize_project(&f.request()).unwrap_err().exit_code(), 5);
                    assert_eq!(snapshot(&f.base), foreign);
                }
                assert_eq!(guard.hits(), [0]);
            }
            drop(guard);
            assert_eq!(snapshot(&f.base), foreign);
            fs::remove_file(&path).unwrap();
            if let Some(bytes) = original {
                fs::write(&path, bytes).unwrap();
            }
            // Restore the source's permissions after replacing a symlink; other
            // writes retained the original inode mode.
            #[cfg(unix)]
            if target == "symlink" {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            }
            f.recover().unwrap();
            if committed {
                f.validate();
            }
        }
    }
    eprintln!(
        "init matrix foreign preservation: {} cases",
        if cfg!(unix) { 10 } else { 8 }
    );
}

#[test]
fn pointer_sync_failure_rolls_back_the_published_pointer() {
    let f = Fixture::new();
    let before = super::recovery_tests::snapshot(&f.base);
    let guard = Guard::arm(vec![Fault::new(
        &f.repository,
        Stage::DirectorySync,
        io::ErrorKind::StorageFull,
    )]);
    assert_io(&initialize_project(&f.request()).unwrap_err());
    assert_eq!(guard.hits(), [1]);
    drop(guard);
    assert!(
        !f.pointer().exists(),
        "published pointer must be owned before its sync can fail"
    );
    assert!(!f.journal().exists());
    assert_eq!(super::recovery_tests::snapshot(&f.base), before);
    f.unlocked();
    initialize_project(&f.request()).unwrap();
}

#[test]
fn recovery_retries_failed_pointer_sync_even_after_pointer_removal() {
    let f = Fixture::new();
    let before = super::recovery_tests::snapshot(&f.base);
    f.interrupt("pointer");
    let journal = fs::read(f.journal()).unwrap();
    let guard = Guard::arm(vec![
        Fault::new(&f.repository, Stage::DirectorySync, io::ErrorKind::Other).persistent(),
    ]);
    assert_io(&f.recover().unwrap_err());
    assert!(!f.pointer().exists());
    assert_io(&f.recover().unwrap_err());
    assert_eq!(guard.hits(), [2]);
    assert_eq!(fs::read(f.journal()).unwrap(), journal);
    drop(guard);
    f.unlocked();
    f.recover().unwrap();
    assert_eq!(super::recovery_tests::snapshot(&f.base), before);
}

#[test]
fn committed_recovery_retries_all_artifact_directories_before_journal_removal() {
    let f = Fixture::new();
    f.interrupt("registry");
    let before = super::recovery_tests::snapshot(&f.base);
    let guard = Guard::arm(vec![
        Fault::new(
            f.project().join("templates/nested/deeper"),
            Stage::DirectorySync,
            io::ErrorKind::PermissionDenied,
        )
        .persistent(),
    ]);
    for _ in 0..2 {
        assert_io(&f.recover().unwrap_err());
        assert_eq!(super::recovery_tests::snapshot(&f.base), before);
        f.unlocked();
    }
    assert_eq!(guard.hits(), [2]);
    drop(guard);
    assert_eq!(f.recover().unwrap().recovery, InitRecovery::Finalized);
}
