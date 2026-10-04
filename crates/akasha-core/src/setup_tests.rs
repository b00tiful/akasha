use super::*;
use crate::{
    InitRequest, ResolutionEnvironment, ResolveRequest, initialize_project, prepare_onboarding,
    prepare_onboarding_handoff, validate_project,
};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "akasha-setup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn root(&self) -> PathBuf {
        self.0.join("memory 世界")
    }
    fn request(&self) -> ResolveRequest {
        ResolveRequest {
            root_override: Some(self.root()),
            project_override: Some("sample".into()),
            cwd: self.0.join("repository"),
            environment: ResolutionEnvironment::default(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            out.insert(
                path.strip_prefix(root).unwrap().into(),
                if path.is_file() {
                    fs::read(&path).unwrap()
                } else {
                    vec![]
                },
            );
            if entry.file_type().unwrap().is_dir() {
                visit(root, &path, out);
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

#[test]
fn empty_start_is_reviewed_private_and_ready_for_every_template_and_onboarding() {
    let f = Fixture::new();
    let before = snapshot(&f.0);
    let plan = prepare_root_setup(&f.root()).unwrap();
    assert_eq!(plan, prepare_root_setup(&f.root()).unwrap());
    assert_eq!(snapshot(&f.0), before);
    let result = apply_root_setup(&plan).unwrap();
    assert!(!result.resumed);
    for (path, source) in &plan.files {
        assert_eq!(fs::read(f.root().join(path)).unwrap(), source.as_bytes());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(f.root()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for path in plan.files.keys() {
            assert_eq!(
                fs::metadata(f.root().join(path))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        for path in &plan.directories {
            assert_eq!(
                fs::metadata(f.root().join(path))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }
    fs::create_dir(f.0.join("repository")).unwrap();
    initialize_project(&InitRequest {
        root_override: Some(f.root()),
        project: "sample".into(),
        cwd: f.0.join("repository"),
        environment: ResolutionEnvironment::default(),
    })
    .unwrap();
    validate_project(&f.request()).unwrap();
    let before = snapshot(&f.0);
    let prepared = prepare_onboarding(&f.request()).unwrap();
    assert_eq!(prepared.templates.len(), 7);
    for name in prepared.note_types.keys() {
        crate::resolve_note_template(&f.request(), name).unwrap();
    }
    let handoff = prepare_onboarding_handoff(&f.request()).unwrap();
    assert_eq!(
        handoff.args,
        vec![
            "--root",
            f.root().to_str().unwrap(),
            "--project",
            "sample",
            "--repo",
            f.0.join("repository").to_str().unwrap()
        ]
    );
    assert!(handoff.agent_request.contains("human approval"));
    assert!(handoff.next_step.contains("No agent connection"));
    assert_eq!(snapshot(&f.0), before);
    assert_eq!(prepare_root_setup(&f.root()).unwrap_err().exit_code(), 5);
    assert_eq!(snapshot(&f.0), before);
}

#[test]
fn tampered_stale_and_unowned_roots_refuse_without_writes() {
    let f = Fixture::new();
    let plan = prepare_root_setup(&f.root()).unwrap();
    for mutation in 0..5 {
        let mut altered = plan.clone();
        match mutation {
            0 => altered.plan_id.push('x'),
            1 => {
                altered.files.insert("outside".into(), "bad".into());
            }
            2 => altered.directories.push("../outside".into()),
            3 => altered.present.push("akasha.toml".into()),
            _ => altered.resuming = true,
        }
        assert_eq!(apply_root_setup(&altered).unwrap_err().exit_code(), 5);
        assert!(snapshot(&f.0).is_empty());
    }
    fs::create_dir(f.root()).unwrap();
    let before = snapshot(&f.0);
    assert_eq!(apply_root_setup(&plan).unwrap_err().exit_code(), 5);
    assert_eq!(snapshot(&f.0), before);
    fs::write(f.root().join("personal.md"), "human source").unwrap();
    let before = snapshot(&f.0);
    assert_eq!(prepare_root_setup(&f.root()).unwrap_err().exit_code(), 5);
    assert_eq!(snapshot(&f.0), before);
    assert!(prepare_root_setup(&f.0.join("missing/root")).is_err());
    assert!(prepare_root_setup(&f.0.join("../outside")).is_err());
}

#[test]
fn changed_partial_state_and_busy_lock_refuse() {
    let f = Fixture::new();
    let plan = prepare_root_setup(&f.root()).unwrap();
    let lock_path = f.0.join(".memory 世界.akasha-setup.lock");
    let lock = acquire_lock(&lock_path).unwrap();
    let before = snapshot(&f.0);
    assert_eq!(apply_root_setup(&plan).unwrap_err().exit_code(), 5);
    assert_eq!(snapshot(&f.0), before);
    drop(lock);
    private_directory(&f.root()).unwrap();
    create_file_atomically(f.root().join(JOURNAL), journal_source(&plan).as_bytes()).unwrap();
    let partial = prepare_root_setup(&f.root()).unwrap();
    fs::create_dir(f.root().join("Meta")).unwrap();
    let before = snapshot(&f.0);
    assert_eq!(apply_root_setup(&partial).unwrap_err().exit_code(), 5);
    assert_eq!(snapshot(&f.0), before);
    fs::write(f.root().join("Meta/projects.yaml"), "external").unwrap();
    let before = snapshot(&f.0);
    for _ in 0..2 {
        assert_eq!(prepare_root_setup(&f.root()).unwrap_err().exit_code(), 5);
    }
    assert_eq!(snapshot(&f.0), before);
    fs::remove_file(f.root().join("Meta/projects.yaml")).unwrap();
    fs::write(f.root().join("unexpected"), "external").unwrap();
    assert_eq!(prepare_root_setup(&f.root()).unwrap_err().exit_code(), 5);
}

#[cfg(unix)]
#[test]
fn symlinks_and_non_utf8_destinations_refuse_without_following() {
    use std::os::unix::{ffi::OsStringExt, fs::symlink};
    let f = Fixture::new();
    symlink("absent", f.root()).unwrap();
    assert_eq!(prepare_root_setup(&f.root()).unwrap_err().exit_code(), 5);
    fs::remove_file(f.root()).unwrap();
    let plan = prepare_root_setup(&f.root()).unwrap();
    symlink("absent", f.0.join(".memory 世界.akasha-setup.lock")).unwrap();
    assert_eq!(apply_root_setup(&plan).unwrap_err().exit_code(), 5);
    assert!(!f.root().exists());
    private_directory(&f.root()).unwrap();
    create_file_atomically(f.root().join(JOURNAL), journal_source(&plan).as_bytes()).unwrap();
    symlink(&f.0, f.root().join("Meta")).unwrap();
    assert_eq!(prepare_root_setup(&f.root()).unwrap_err().exit_code(), 5);
    let invalid = f.0.join(std::ffi::OsString::from_vec(vec![0xff]));
    assert_eq!(prepare_root_setup(&invalid).unwrap_err().exit_code(), 3);
}

pub(super) fn checkpoint(stage: &str) {
    if std::env::var("AKASHA_TEST_SETUP_STAGE").as_deref() == Ok(stage) {
        std::process::exit(73);
    }
}
#[test]
fn setup_crash_child() {
    let Some(path) = std::env::var_os("AKASHA_TEST_SETUP_ROOT") else {
        return;
    };
    let plan = prepare_root_setup(Path::new(&path)).unwrap();
    apply_root_setup(&plan).unwrap();
    panic!("child did not stop at requested boundary");
}
#[test]
fn real_process_exits_resume_exact_partial_trees_and_preserve_unowned_gap() {
    for stage in [
        "reserved",
        "journal",
        "directories",
        "Meta/AGENTS.md",
        "Meta/projects.yaml",
        "templates/entity.md",
        "templates/session.md",
        "configured",
        "unlinked",
    ] {
        let f = Fixture::new();
        let original = prepare_root_setup(&f.root()).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "setup::tests::setup_crash_child", "--nocapture"])
            .env("AKASHA_TEST_SETUP_ROOT", f.root())
            .env("AKASHA_TEST_SETUP_STAGE", stage)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(73), "{stage}");
        let lock = acquire_lock(&f.0.join(".memory 世界.akasha-setup.lock")).unwrap();
        drop(lock);
        if stage == "reserved" || stage == "unlinked" {
            let before = snapshot(&f.0);
            assert_eq!(prepare_root_setup(&f.root()).unwrap_err().exit_code(), 5);
            assert_eq!(snapshot(&f.0), before);
            if stage == "unlinked" {
                for (path, source) in &original.files {
                    assert_eq!(fs::read(f.root().join(path)).unwrap(), source.as_bytes());
                }
            }
            continue;
        }
        if stage == "configured" {
            let error = crate::resolution::load_root_config(&f.root()).unwrap_err();
            assert!(error.to_string().contains("unfinished root setup"));
        }
        let plan = prepare_root_setup(&f.root()).unwrap();
        assert!(plan.resuming);
        let result = apply_root_setup(&plan).unwrap();
        assert!(result.resumed);
        for (path, source) in &original.files {
            assert_eq!(
                fs::read(f.root().join(path)).unwrap(),
                source.as_bytes(),
                "{stage}: {path:?}"
            );
        }
        assert!(!f.root().join(JOURNAL).exists());
        crate::resolution::load_root_config(&f.root()).unwrap();
    }
}

#[test]
fn bootstrap_to_snapshot_bound_evidenced_population_preserves_source_repository() {
    use crate::{
        OnboardingBatchRequest, ProposedNote, apply_approved_onboarding_batch,
        preview_onboarding_batch,
    };
    let f = Fixture::new();
    apply_root_setup(&prepare_root_setup(&f.root()).unwrap()).unwrap();
    fs::create_dir(f.0.join("repository")).unwrap();
    let repository = f.0.join("repository");
    fs::write(
        repository.join("README.md"),
        "# Synthetic project\nAn offline command-line tool.\n",
    )
    .unwrap();
    initialize_project(&InitRequest {
        root_override: Some(f.root()),
        project: "sample".into(),
        cwd: repository.clone(),
        environment: ResolutionEnvironment::default(),
    })
    .unwrap();
    let source_before = snapshot(&repository);
    let fingerprint =
        crate::state::content_fingerprint(&fs::read(repository.join("README.md")).unwrap());
    let note = format!(
        "---\nschema_version: 1\nentity: overview\nkind: component\nstatus: active\nreviewed: 2026-10-04\nevidence:\n  - kind: fact\n    claim: README describes an offline command-line tool.\n    sources:\n      - path: README.md\n        fingerprint: {fingerprint:?}\n  - kind: inference\n    claim: Offline operation is an intended capability.\n    rationale: README explicitly describes offline operation.\n    sources:\n      - path: README.md\n        fingerprint: {fingerprint:?}\n  - kind: unknown\n    claim: Release process is unknown.\n    rationale: No release documentation is available in this fixture.\n---\n\n# Overview\n\nObserved: offline command-line tool. Inferred: offline capability is intentional. Unknown: release process.\n"
    );
    let mut request = OnboardingBatchRequest {
        resolution: f.request(),
        notes: vec![ProposedNote {
            note_type: "entity".into(),
            path: "overview.md".into(),
            source: note,
        }],
        index: "# Index\n\n[[Projects/sample/entities/overview|Overview]]\n".into(),
        roadmap: "# Roadmap\n\nNo supported tasks discovered.\n".into(),
    };
    let before = snapshot(&f.root());
    let preview = preview_onboarding_batch(&request).unwrap();
    assert_eq!(
        snapshot(&f.root()),
        before,
        "preview and cancellation are read-only"
    );
    request.index.push_str("\nChanged after review.\n");
    assert_eq!(
        apply_approved_onboarding_batch(&request, &preview.preview_id)
            .unwrap_err()
            .exit_code(),
        5
    );
    assert_eq!(snapshot(&f.root()), before, "stale approval cannot publish");
    let fresh = preview_onboarding_batch(&request).unwrap();
    let applied = apply_approved_onboarding_batch(&request, &fresh.preview_id).unwrap();
    assert_eq!(applied.created_notes.len(), 1);
    assert_eq!(validate_project(&f.request()).unwrap().canonical_notes, 1);
    assert_eq!(snapshot(&repository), source_before);
    assert_eq!(
        prepare_onboarding(&f.request())
            .unwrap()
            .existing_notes
            .len(),
        1
    );
    let populated = snapshot(&f.root());
    assert!(apply_approved_onboarding_batch(&request, &fresh.preview_id).is_err());
    assert_eq!(snapshot(&f.root()), populated);
}

#[cfg(unix)]
#[test]
fn writer_scope_releases_inherited_descriptor() {
    let f = Fixture::new();
    let path = f.0.join("setup.lock");
    let owner = acquire_lock(&path).unwrap();
    let inherited = owner.0.try_clone().unwrap();
    assert!(acquire_lock(&path).is_err());
    drop(owner);
    let reacquired = acquire_lock(&path);
    drop(inherited);
    assert!(reacquired.is_ok());
}
