//! Returned errors from the real setup filesystem path, with thread-scoped faults.

use super::*;
use crate::writes::error_tests::{Fault, Guard, Stage};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
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

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    bytes: Option<Vec<u8>>,
    mode: u32,
    symlink: bool,
}
type Snapshot = BTreeMap<PathBuf, Entry>;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "akasha-setup-errors-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("outside.bin"), b"\0foreign\xff\r\n  ").unwrap();
        Self(path)
    }
    fn root(&self) -> PathBuf {
        self.0.join("memory 世界")
    }
    fn lock(&self) -> PathBuf {
        self.0.join(".memory 世界.akasha-setup.lock")
    }
    fn unlocked(&self) {
        if self.lock().exists() {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(self.lock())
                .unwrap();
            file.try_lock().expect("setup returned with its lock held");
            file.unlock().unwrap();
        }
    }
    fn preserved(&self, plan: &RootSetupPlan) {
        assert_eq!(
            fs::read(self.0.join("outside.bin")).unwrap(),
            b"\0foreign\xff\r\n  "
        );
        if !plan.root.exists() {
            return;
        }
        assert_private(&plan.root, 0o700);
        for (path, entry) in snapshot(&plan.root) {
            if plan.directories.contains(&path) {
                assert!(entry.bytes.is_none() && !entry.symlink, "{path:?}");
                assert_private(&plan.root.join(&path), 0o700);
            } else {
                let expected = if path == Path::new(JOURNAL) {
                    journal_source(plan)
                } else {
                    plan.files
                        .get(&path)
                        .expect("unexpected setup residue")
                        .clone()
                };
                assert_eq!(
                    entry.bytes.as_deref(),
                    Some(expected.as_bytes()),
                    "{path:?}"
                );
                assert_private(&plan.root.join(&path), 0o600);
            }
        }
    }
    fn complete(&self, root: &Path) -> RootSetupPlan {
        let before = snapshot(&self.0);
        let plan = prepare_root_setup(root).unwrap();
        assert_eq!(snapshot(&self.0), before, "review writes nothing");
        let result = apply_root_setup(&plan).unwrap();
        assert_eq!(result.resumed, plan.resuming);
        self.assert_complete(&plan);
        plan
    }
    fn assert_complete(&self, plan: &RootSetupPlan) {
        self.preserved(plan);
        assert_eq!(
            snapshot(&plan.root).len(),
            plan.files.len() + plan.directories.len()
        );
        for (path, source) in &plan.files {
            assert_eq!(fs::read(plan.root.join(path)).unwrap(), source.as_bytes());
        }
        crate::resolution::load_root_config(&plan.root).unwrap();
        let before = snapshot(&self.0);
        assert_eq!(prepare_root_setup(&plan.root).unwrap_err().exit_code(), 5);
        assert_eq!(snapshot(&self.0), before, "completed root is never adopted");
        self.unlocked();
    }
    fn retry_or_preserve_gap(&self, original: &RootSetupPlan) {
        self.unlocked();
        self.preserved(original);
        if original.root.exists() && !original.root.join(JOURNAL).exists() {
            let before = snapshot(&self.0);
            for _ in 0..2 {
                assert_eq!(
                    prepare_root_setup(&original.root).unwrap_err().exit_code(),
                    5
                );
                assert_eq!(apply_root_setup(original).unwrap_err().exit_code(), 5);
                assert_eq!(snapshot(&self.0), before);
            }
            if original.root.join(ROOT_CONFIG_FILE).exists() {
                // The final sync can fail after journal unlink. Inspect the complete image.
                self.assert_complete(original);
            } else {
                assert!(
                    snapshot(&original.root).is_empty(),
                    "reservation/journal gap"
                );
                self.complete(&self.0.join("retry 世界"));
                assert!(snapshot(&original.root).is_empty(), "unowned root retained");
            }
        } else {
            if original.root.exists() {
                let before = snapshot(&self.0);
                assert_eq!(apply_root_setup(original).unwrap_err().exit_code(), 5);
                assert_eq!(snapshot(&self.0), before, "old presence review is stale");
                assert!(crate::resolution::load_root_config(&original.root).is_err());
            }
            self.complete(&original.root);
        }
    }
    fn owned(&self, complete: bool) -> RootSetupPlan {
        let plan = prepare_root_setup(&self.root()).unwrap();
        let fault = if complete {
            Fault::new(
                self.root().join(JOURNAL),
                Stage::Remove,
                io::ErrorKind::Other,
            )
        } else {
            Fault::new(
                self.root().join(
                    plan.files
                        .keys()
                        .find(|path| *path != Path::new(ROOT_CONFIG_FILE))
                        .unwrap(),
                ),
                Stage::Publish,
                io::ErrorKind::Other,
            )
        };
        let guard = Guard::arm(vec![fault]);
        assert_io(apply_root_setup(&plan).unwrap_err(), io::ErrorKind::Other);
        assert_eq!(guard.hits(), [1]);
        drop(guard);
        self.unlocked();
        prepare_root_setup(&self.root()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn mode(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0
    }
}
fn assert_private(path: &Path, expected: u32) {
    #[cfg(unix)]
    assert_eq!(
        mode(&fs::symlink_metadata(path).unwrap()),
        expected,
        "{path:?}"
    );
    #[cfg(not(unix))]
    let _ = (path, expected);
}
// Include directory identity, exact bytes, file type and mode; never follow symlinks.
fn snapshot(root: &Path) -> Snapshot {
    fn visit(root: &Path, directory: &Path, out: &mut Snapshot) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            let bytes = if metadata.file_type().is_symlink() {
                Some(
                    fs::read_link(&path)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                )
            } else if metadata.is_file() {
                Some(fs::read(&path).unwrap())
            } else {
                None
            };
            out.insert(
                path.strip_prefix(root).unwrap().into(),
                Entry {
                    bytes,
                    mode: mode(&metadata),
                    symlink: metadata.file_type().is_symlink(),
                },
            );
            if metadata.is_dir() {
                visit(root, &path, out);
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}
fn assert_io(error: RootSetupError, kind: io::ErrorKind) {
    assert_eq!(error.exit_code(), 6, "{error}");
    assert!(error.to_string().contains("preserve partial files"));
    let source = match error {
        RootSetupError::FileSystem { source, .. }
        | RootSetupError::Write(AtomicCreateError::FileSystem { source, .. }) => source,
        error => panic!("wrong error classification: {error}"),
    };
    assert_eq!(source.kind(), kind);
}

#[test]
fn returned_file_errors_preserve_exact_prefixes_and_allow_only_reviewed_retry() {
    let shape = prepare_root_setup(&Fixture::new().root()).unwrap();
    let mut cases: Vec<_> = std::iter::once(PathBuf::from(JOURNAL))
        .chain(shape.files.keys().cloned())
        .flat_map(|path| WRITE_STAGES.map(|stage| (Some(path.clone()), stage)))
        .collect();
    cases.extend([Stage::Create, Stage::FileSync, Stage::Publish].map(|stage| (None, stage)));
    for kind in KINDS {
        for (relative, stage) in &cases {
            let f = Fixture::new();
            let plan = prepare_root_setup(&f.root()).unwrap();
            let path = relative
                .as_ref()
                .map_or_else(|| f.lock(), |path| f.root().join(path));
            let guard = Guard::arm(vec![Fault::new(&path, *stage, kind)]);
            assert_io(apply_root_setup(&plan).unwrap_err(), kind);
            assert_eq!(guard.hits(), [1], "{relative:?}/{stage:?}/{kind:?}");
            drop(guard);
            f.retry_or_preserve_gap(&plan);
        }
    }
    eprintln!(
        "setup file publication: {} scenarios",
        cases.len() * KINDS.len()
    );
}

#[test]
fn returned_directory_creation_errors_preserve_owned_and_unowned_boundaries() {
    let shape = prepare_root_setup(&Fixture::new().root()).unwrap();
    for kind in KINDS {
        for relative in std::iter::once(PathBuf::new()).chain(shape.directories.iter().cloned()) {
            let f = Fixture::new();
            let plan = prepare_root_setup(&f.root()).unwrap();
            let guard = Guard::arm(vec![Fault::new(
                f.root().join(&relative),
                Stage::Create,
                kind,
            )]);
            assert_io(apply_root_setup(&plan).unwrap_err(), kind);
            assert_eq!(guard.hits(), [1], "{relative:?}/{kind:?}");
            drop(guard);
            f.retry_or_preserve_gap(&plan);
        }
    }
    eprintln!(
        "setup directory creation: {} scenarios",
        (shape.directories.len() + 1) * KINDS.len()
    );
}

fn sync_order(f: &Fixture, plan: &RootSetupPlan) -> Vec<PathBuf> {
    let mut syncs = vec![f.0.clone(), f.0.clone(), f.root()];
    for _ in 0..2 {
        syncs.extend(
            plan.directories
                .iter()
                .rev()
                .map(|path| f.root().join(path)),
        );
        syncs.push(f.root());
        syncs.push(f.root()); // Configuration sync in pass one, post-unlink sync in pass two.
    }
    syncs
}

#[test]
fn every_returned_directory_sync_error_preserves_the_correct_setup_image() {
    let shape = Fixture::new();
    let shape_plan = prepare_root_setup(&shape.root()).unwrap();
    let count = sync_order(&shape, &shape_plan).len();
    for kind in KINDS {
        for index in 0..count {
            let f = Fixture::new();
            let plan = prepare_root_setup(&f.root()).unwrap();
            let syncs = sync_order(&f, &plan);
            let path = &syncs[index];
            let skip = syncs[..index].iter().filter(|prior| *prior == path).count();
            let guard = Guard::arm(vec![
                Fault::new(path, Stage::DirectorySync, kind).skip(skip),
            ]);
            assert_io(apply_root_setup(&plan).unwrap_err(), kind);
            assert_eq!(guard.hits(), [1], "sync {index}/{path:?}/{skip}/{kind:?}");
            drop(guard);
            f.retry_or_preserve_gap(&plan);
        }
    }
    eprintln!(
        "setup publication directory sync: {} scenarios",
        count * KINDS.len()
    );
}

#[test]
fn persistent_resume_sync_errors_cannot_be_hidden_by_existing_exact_files() {
    let shape = prepare_root_setup(&Fixture::new().root()).unwrap();
    for complete in [false, true] {
        for kind in KINDS {
            for relative in std::iter::once(PathBuf::new()).chain(shape.directories.iter().cloned())
            {
                let f = Fixture::new();
                f.owned(complete);
                let guard = Guard::arm(vec![
                    Fault::new(f.root().join(&relative), Stage::DirectorySync, kind).persistent(),
                ]);
                let mut stable = None;
                for _ in 0..2 {
                    let plan = prepare_root_setup(&f.root()).unwrap();
                    assert_io(apply_root_setup(&plan).unwrap_err(), kind);
                    f.unlocked();
                    assert!(f.root().join(JOURNAL).exists());
                    assert!(crate::resolution::load_root_config(&f.root()).is_err());
                    f.preserved(&plan);
                    let now = snapshot(&f.0);
                    if let Some(before) = &stable {
                        assert_eq!(&now, before, "matching files cannot hide failed sync");
                    }
                    stable = Some(now);
                }
                assert_eq!(guard.hits(), [2], "{complete}/{relative:?}/{kind:?}");
                drop(guard);
                f.complete(&f.root());
            }
        }
    }
    eprintln!(
        "setup persistent resume sync: {} scenarios",
        2 * KINDS.len() * (shape.directories.len() + 1)
    );
}

#[test]
fn persistent_journal_removal_errors_retain_authority_and_release_the_lock() {
    for kind in KINDS {
        let f = Fixture::new();
        let original = prepare_root_setup(&f.root()).unwrap();
        let guard = Guard::arm(vec![
            Fault::new(f.root().join(JOURNAL), Stage::Remove, kind).persistent(),
        ]);
        assert_io(apply_root_setup(&original).unwrap_err(), kind);
        let stable = snapshot(&f.0);
        for _ in 0..2 {
            let plan = prepare_root_setup(&f.root()).unwrap();
            assert!(plan.resuming);
            assert_io(apply_root_setup(&plan).unwrap_err(), kind);
            assert_eq!(snapshot(&f.0), stable);
            assert!(crate::resolution::load_root_config(&f.root()).is_err());
            f.unlocked();
        }
        assert_eq!(guard.hits(), [3]);
        drop(guard);
        f.complete(&f.root());
    }
    eprintln!("setup persistent journal removal: 3 scenarios");
}

fn staging_prefix(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        ".{}.akasha-",
        path.file_name().unwrap().to_str().unwrap()
    ))
}
fn residue(root: &Path, prefix: &Path) -> PathBuf {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .as_encoded_bytes()
                .starts_with(prefix.file_name().unwrap().as_encoded_bytes())
        })
        .expect("failed staging cleanup must remain visible")
}

#[test]
fn failed_staging_cleanup_is_preserved_and_never_authorizes_automatic_adoption() {
    let shape = prepare_root_setup(&Fixture::new().root()).unwrap();
    let targets: Vec<_> = std::iter::once(PathBuf::from(JOURNAL))
        .chain(shape.files.keys().cloned())
        .collect();
    for partial in [false, true] {
        for relative in &targets {
            let f = Fixture::new();
            let plan = prepare_root_setup(&f.root()).unwrap();
            let target = f.root().join(relative);
            let prefix = staging_prefix(&target);
            let mut faults = vec![
                Fault::new(&prefix, Stage::Remove, io::ErrorKind::PermissionDenied)
                    .file_name_prefix()
                    .persistent(),
            ];
            if partial {
                faults.push(Fault::new(
                    &target,
                    Stage::PartialWrite,
                    io::ErrorKind::StorageFull,
                ));
            }
            let guard = Guard::arm(faults);
            let error = apply_root_setup(&plan).unwrap_err();
            if partial {
                assert_io(error, io::ErrorKind::StorageFull);
                assert_eq!(guard.hits(), [1, 1]);
            } else {
                assert_eq!(error.exit_code(), 5);
                assert_eq!(guard.hits(), [1]);
            }
            let stage = residue(target.parent().unwrap(), &prefix);
            assert_private(&stage, 0o600);
            let source = if relative == Path::new(JOURNAL) {
                journal_source(&plan)
            } else {
                plan.files[relative].clone()
            };
            assert_eq!(
                fs::read(&stage).unwrap(),
                if partial {
                    &source.as_bytes()[..source.len() / 2]
                } else {
                    source.as_bytes()
                }
            );
            let stable = snapshot(&f.0);
            for _ in 0..2 {
                assert_eq!(prepare_root_setup(&f.root()).unwrap_err().exit_code(), 5);
                assert_eq!(apply_root_setup(&plan).unwrap_err().exit_code(), 5);
                assert_eq!(snapshot(&f.0), stable);
                f.unlocked();
            }
            drop(guard);
            // Model an explicit operator reconciliation after backup/inspection, never product cleanup.
            fs::remove_file(&stage).unwrap();
            sync_directory(stage.parent().unwrap()).unwrap();
            f.retry_or_preserve_gap(&plan);
        }
    }
    eprintln!(
        "setup staging cleanup: {} compound scenarios",
        targets.len() * 2
    );
}

#[test]
fn external_partial_changes_refuse_before_mutation_and_preserve_exact_bytes_and_modes() {
    for mutation in 0..6 {
        let f = Fixture::new();
        let plan = f.owned(true);
        let journal = f.root().join(JOURNAL);
        let registry = f.root().join("Meta/projects.yaml");
        let before_registry = fs::read(&registry).unwrap();
        match mutation {
            0 => fs::write(&registry, b"external\0\xff\r\n  ").unwrap(),
            1 => fs::remove_file(&registry).unwrap(), // Config exists, so a missing supporting file is invalid.
            2 => fs::write(&journal, b"foreign journal").unwrap(),
            3 => fs::remove_file(&journal).unwrap(),
            4 => fs::write(f.root().join("personal.md"), "human 世界  \r\n").unwrap(),
            5 => {
                fs::remove_file(&registry).unwrap();
                fs::create_dir(&registry).unwrap();
            }
            _ => unreachable!(),
        }
        let stable = snapshot(&f.0);
        let guard = Guard::arm(vec![Fault::new(
            &journal,
            Stage::Remove,
            io::ErrorKind::Other,
        )]);
        for _ in 0..2 {
            assert_eq!(prepare_root_setup(&f.root()).unwrap_err().exit_code(), 5);
            assert_eq!(apply_root_setup(&plan).unwrap_err().exit_code(), 5);
            assert_eq!(snapshot(&f.0), stable);
            f.unlocked();
        }
        assert_eq!(guard.hits(), [0], "validation must precede cleanup");
        drop(guard);
        match mutation {
            0 | 1 => fs::write(&registry, before_registry).unwrap(),
            2 | 3 => fs::write(&journal, journal_source(&plan)).unwrap(),
            4 => fs::remove_file(f.root().join("personal.md")).unwrap(),
            5 => {
                fs::remove_dir(&registry).unwrap();
                fs::write(&registry, before_registry).unwrap();
            }
            _ => unreachable!(),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&registry, fs::Permissions::from_mode(0o640)).unwrap();
            fs::set_permissions(f.root().join("Meta"), fs::Permissions::from_mode(0o750)).unwrap();
        }
        let before = snapshot(&f.root());
        let fresh = prepare_root_setup(&f.root()).unwrap();
        apply_root_setup(&fresh).unwrap();
        let mut expected = before;
        expected.remove(Path::new(JOURNAL));
        assert_eq!(
            snapshot(&f.root()),
            expected,
            "matching human-owned modes stay exact"
        );
        crate::resolution::load_root_config(&f.root()).unwrap();
    }
    eprintln!("setup external-byte preservation: 6 scenarios");
}

#[test]
fn returned_configuration_errors_allow_validated_init_and_external_agent_handoff() {
    for kind in KINDS {
        let f = Fixture::new();
        let original = prepare_root_setup(&f.root()).unwrap();
        let guard = Guard::arm(vec![Fault::new(
            f.root().join(ROOT_CONFIG_FILE),
            Stage::Publish,
            kind,
        )]);
        assert_io(apply_root_setup(&original).unwrap_err(), kind);
        assert_eq!(guard.hits(), [1]);
        drop(guard);
        f.complete(&f.root());
        let repository = f.0.join("repository 世界");
        fs::create_dir(&repository).unwrap();
        fs::write(
            repository.join("README.md"),
            "# Synthetic offline project\n",
        )
        .unwrap();
        let request = crate::InitRequest {
            root_override: Some(f.root()),
            project: "sample".into(),
            cwd: repository.clone(),
            environment: crate::ResolutionEnvironment::default(),
        };
        let plan = crate::prepare_project_init(&request).unwrap();
        crate::apply_project_init(&request, &plan).unwrap();
        let request = crate::ResolveRequest {
            root_override: Some(f.root()),
            project_override: Some("sample".into()),
            cwd: repository,
            environment: crate::ResolutionEnvironment::default(),
        };
        crate::validate_project(&request).unwrap();
        let before = snapshot(&f.0);
        let handoff = crate::prepare_onboarding_handoff(&request).unwrap();
        assert_eq!(handoff.command, "akasha-onboarding-mcp");
        assert_eq!(
            crate::prepare_onboarding(&request).unwrap().templates.len(),
            7
        );
        assert_eq!(snapshot(&f.0), before, "handoff/preparation writes nothing");
    }
    eprintln!("setup recovered init/handoff: 3 scenarios");
}

#[test]
fn failed_lock_staging_cleanup_retains_sibling_residue_without_reusing_it() {
    for kind in KINDS {
        for published in [false, true] {
            let f = Fixture::new();
            let plan = prepare_root_setup(&f.root()).unwrap();
            let prefix = staging_prefix(&f.lock());
            let mut faults = vec![
                Fault::new(&prefix, Stage::Remove, kind)
                    .file_name_prefix()
                    .persistent(),
            ];
            if !published {
                faults.push(Fault::new(
                    f.lock(),
                    Stage::FileSync,
                    io::ErrorKind::StorageFull,
                ));
            }
            let guard = Guard::arm(faults);
            if published {
                apply_root_setup(&plan).unwrap();
                assert_eq!(guard.hits(), [1]);
            } else {
                assert_io(
                    apply_root_setup(&plan).unwrap_err(),
                    io::ErrorKind::StorageFull,
                );
                assert_eq!(guard.hits(), [1, 1]);
                assert!(!f.root().exists());
            }
            let stage = residue(&f.0, &prefix);
            assert_private(&stage, 0o600);
            assert!(fs::read(&stage).unwrap().is_empty());
            let metadata = fs::symlink_metadata(&stage).unwrap();
            drop(guard);
            if published {
                f.assert_complete(&plan);
            } else {
                f.complete(&f.root());
            }
            assert!(fs::read(&stage).unwrap().is_empty());
            assert_eq!(
                mode(&fs::symlink_metadata(&stage).unwrap()),
                mode(&metadata)
            );
            assert_eq!(
                residue(&f.0, &prefix),
                stage,
                "scratch is never adopted or deleted"
            );
        }
    }
    eprintln!("setup lock staging cleanup: 6 scenarios");
}

#[cfg(unix)]
#[test]
fn symlinks_and_untrusted_scratch_entries_after_errors_refuse_without_following() {
    use std::os::unix::fs::symlink;
    for mutation in 0..4 {
        let f = Fixture::new();
        let plan = f.owned(true);
        let outside = f.0.join("protected.md");
        fs::write(&outside, "外部 bytes  \r\n").unwrap();
        match mutation {
            0 => {
                let target = f.root().join(JOURNAL);
                fs::write(&outside, fs::read(&target).unwrap()).unwrap();
                fs::remove_file(&target).unwrap();
                symlink(&outside, target).unwrap();
            }
            1 => {
                let target = f.root().join(plan.files.keys().next().unwrap());
                fs::write(&outside, fs::read(&target).unwrap()).unwrap();
                fs::remove_file(&target).unwrap();
                symlink(&outside, target).unwrap();
            }
            2 => symlink(&outside, f.root().join(".AGENTS.md.akasha-123-0.tmp")).unwrap(),
            3 => fs::create_dir(f.root().join(".AGENTS.md.akasha-123-0.tmp")).unwrap(),
            _ => unreachable!(),
        }
        let stable = snapshot(&f.0);
        for _ in 0..2 {
            assert_eq!(prepare_root_setup(&f.root()).unwrap_err().exit_code(), 5);
            assert_eq!(apply_root_setup(&plan).unwrap_err().exit_code(), 5);
            assert_eq!(snapshot(&f.0), stable);
            f.unlocked();
        }
    }
    eprintln!("setup hostile partial entries: 4 scenarios");
}
