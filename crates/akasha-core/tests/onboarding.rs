use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    InitRequest, NOTE_EDIT_JOURNAL_FILE, OnboardingBatchRequest, OnboardingNoteAction,
    ProposedNote, ResolutionEnvironment, ResolveRequest, apply_approved_onboarding_batch,
    apply_onboarding_batch, initialize_project, prepare_onboarding, preview_onboarding_batch,
    validate_project,
};
use sha2::{Digest, Sha256};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn applies_valid_notes_projections_and_state_as_one_checked_batch() {
    let fixture = Fixture::new("complete");
    let request = fixture.batch_request();

    let result = apply_onboarding_batch(&request).expect("apply onboarding batch");

    assert_eq!(result.created_notes.len(), 3);
    assert!(result.unchanged_notes.is_empty());
    assert_eq!(result.updated_projections.len(), 2);
    assert_eq!(
        fs::read_to_string(fixture.project.join("entities/core.md")).expect("read entity"),
        request.notes[0].source
    );
    assert_eq!(
        fs::read_to_string(fixture.project.join("index.md")).expect("read index"),
        request.index
    );
    assert_eq!(
        fs::read_to_string(fixture.project.join("roadmap.md")).expect("read roadmap"),
        request.roadmap
    );

    let report = validate_project(&fixture.resolution).expect("validate applied project");
    assert_eq!(report.canonical_notes, 3);
    assert_eq!(report.immutable_events, 1);
    assert_eq!(report.projections["index"].sources, 1);
    assert_eq!(report.projections["roadmap"].sources, 1);
    assert_eq!(report.wikilinks, 2);
    assert!(!fixture.project.join(NOTE_EDIT_JOURNAL_FILE).exists());
}

#[test]
fn exact_rerun_is_a_no_op() {
    let fixture = Fixture::new("rerun");
    let request = fixture.batch_request();
    apply_onboarding_batch(&request).expect("apply first batch");
    let state_before = fs::read(fixture.project.join(".akasha-state.toml")).expect("read state");

    let result = apply_onboarding_batch(&request).expect("rerun exact batch");

    assert!(result.created_notes.is_empty());
    assert_eq!(result.unchanged_notes.len(), 3);
    assert!(result.updated_projections.is_empty());
    assert_eq!(
        fs::read(fixture.project.join(".akasha-state.toml")).expect("read rerun state"),
        state_before
    );
}

#[test]
fn differing_existing_note_conflicts_without_mutation() {
    let fixture = Fixture::new("rerun-conflict");
    let request = fixture.batch_request();
    apply_onboarding_batch(&request).expect("apply first batch");
    let before = fixture.project_snapshot();
    let mut changed = request.clone();
    changed.notes[0]
        .source
        .push_str("\nHuman-incompatible rewrite.\n");
    changed.index = "# Replaced index\n".to_owned();

    let error = apply_onboarding_batch(&changed).expect_err("changed note must conflict");

    assert_eq!(error.exit_code(), 5);
    assert!(
        error
            .to_string()
            .contains("differs from the proposed exact bytes")
    );
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
fn invalid_note_and_missing_wikilink_write_nothing() {
    for (label, source) in [
        (
            "missing-field",
            "---\nschema_version: 1\nentity: core\nkind: subsystem\nreviewed: 2026-07-13\n---\n\n# Core\n",
        ),
        (
            "missing-link",
            "---\nschema_version: 1\nentity: core\nkind: subsystem\nstatus: active\nreviewed: 2026-07-13\n---\n\n[[Projects/example/entities/missing]]\n",
        ),
    ] {
        let fixture = Fixture::new(label);
        let before = fixture.project_snapshot();
        let mut request = fixture.batch_request();
        request.notes[0].source = source.to_owned();

        let error = apply_onboarding_batch(&request).expect_err("invalid proposal must fail");

        assert_eq!(error.exit_code(), 4);
        assert_eq!(fixture.project_snapshot(), before);
    }
}

#[test]
fn unsafe_path_and_existing_lock_fail_without_writes() {
    let fixture = Fixture::new("unsafe-path");
    let before = fixture.project_snapshot();
    let mut request = fixture.batch_request();
    request.notes[0].path = PathBuf::from("../escape.md");

    let error = apply_onboarding_batch(&request).expect_err("unsafe path must fail");
    assert_eq!(error.exit_code(), 4);
    assert_eq!(fixture.project_snapshot(), before);

    let request = fixture.batch_request();
    let lock = fixture.project.join(".akasha-write.lock");
    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock)
        .expect("open project writer lock");
    lock_file.try_lock().expect("hold project writer lock");
    let error = apply_onboarding_batch(&request).expect_err("existing lock must conflict");
    assert_eq!(error.exit_code(), 5);
    drop(lock_file);
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
fn preparation_is_bounded_and_returns_project_contracts_without_note_bodies() {
    let fixture = Fixture::new("preparation");

    let preparation = prepare_onboarding(&fixture.resolution).expect("prepare onboarding");

    assert_eq!(preparation.project, "example");
    assert_eq!(preparation.note_types.len(), 5);
    assert_eq!(preparation.templates.len(), 1);
    assert_eq!(preparation.templates[0].path, PathBuf::from("entity.md"));
    assert!(preparation.template_characters > 0);
    assert_eq!(preparation.omitted_templates, 0);
    assert!(preparation.existing_notes.is_empty());
    assert_eq!(preparation.coverage_criteria.len(), 6);
    assert!(preparation.evidence_contract.contains("fingerprint"));
}

#[test]
fn preview_binds_source_attributed_proposal_and_approved_apply_delegates_to_batch() {
    let fixture = Fixture::new("approved");
    let request = fixture.evidenced_batch_request();

    let preview = preview_onboarding_batch(&request).expect("preview proposal");

    assert_eq!(preview.notes.len(), 3);
    assert!(
        preview
            .notes
            .iter()
            .all(|note| note.action == OnboardingNoteAction::Create)
    );
    assert_eq!(
        preview
            .notes
            .iter()
            .map(|note| note.evidence_claims)
            .sum::<usize>(),
        3
    );
    assert!(preview.proposal_id.starts_with("sha256:"));
    assert!(preview.preview_id.starts_with("sha256:"));
    assert!(preview.index_changed);
    assert!(preview.roadmap_changed);
    assert!(preview.state_changed);

    let result = apply_approved_onboarding_batch(&request, &preview.preview_id)
        .expect("apply approved proposal");
    assert_eq!(result.created_notes.len(), 3);
    validate_project(&fixture.resolution).expect("validate applied project");

    let rerun = preview_onboarding_batch(&request).expect("preview exact rerun");
    assert!(
        rerun
            .notes
            .iter()
            .all(|note| note.action == OnboardingNoteAction::Unchanged)
    );
    assert!(!rerun.index_changed);
    assert!(!rerun.roadmap_changed);
    assert!(!rerun.state_changed);
}

#[test]
fn stale_preview_and_invalid_evidence_write_nothing() {
    let fixture = Fixture::new("binding");
    let request = fixture.evidenced_batch_request();
    let preview = preview_onboarding_batch(&request).expect("preview proposal");
    let before = fixture.project_snapshot();
    let mut changed = request.clone();
    changed.notes[0]
        .source
        .push_str("\nChanged after preview.\n");

    let error = apply_approved_onboarding_batch(&changed, &preview.preview_id)
        .expect_err("changed proposal must not use an earlier preview");
    assert_eq!(error.exit_code(), 5);
    assert!(
        error
            .to_string()
            .contains("approved preview does not match")
    );
    assert_eq!(fixture.project_snapshot(), before);

    let mut missing_evidence = request.clone();
    missing_evidence.notes[0].source = missing_evidence.notes[0]
        .source
        .replace("evidence:\n", "other_evidence:\n");
    let error = preview_onboarding_batch(&missing_evidence)
        .expect_err("transport proposal evidence is mandatory");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("non-empty `evidence` list"));
    assert_eq!(fixture.project_snapshot(), before);

    let mut stale_evidence = request;
    stale_evidence.notes[0].source = stale_evidence.notes[0]
        .source
        .replace("sha256:", "sha256:0");
    let error = preview_onboarding_batch(&stale_evidence)
        .expect_err("source fingerprints must match repository bytes");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("fingerprint is stale"));
    assert_eq!(fixture.project_snapshot(), before);
}

struct Fixture {
    _temp: TempDir,
    project: PathBuf,
    resolution: ResolveRequest,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let temp = TempDir::new(label);
        let root = temp.path().join("root");
        let repository = temp.path().join("repository");
        for directory in [
            root.join("Meta"),
            root.join("templates"),
            root.join("Global"),
            root.join("Projects"),
            root.join("Inbox"),
            repository.clone(),
        ] {
            fs::create_dir_all(directory).expect("create fixture directory");
        }
        fs::write(
            root.join("akasha.toml"),
            include_str!("../../../tests/fixtures/resolution/valid-root/akasha.toml"),
        )
        .expect("write root config");
        fs::write(root.join("Meta/projects.yaml"), "{}\n").expect("write empty registry");
        fs::write(
            root.join("templates/entity.md"),
            "---\nschema_version: 1\n---\n\n# Entity template\n",
        )
        .expect("write onboarding template");
        fs::write(
            repository.join("Cargo.toml"),
            "[package]\nname = \"example\"\n",
        )
        .expect("write evidence source");
        initialize_project(&InitRequest {
            root_override: Some(root.clone()),
            project: "example".to_owned(),
            cwd: repository.clone(),
            environment: ResolutionEnvironment::default(),
        })
        .expect("initialize empty project");
        let project = root.join("Projects/example");
        let resolution = ResolveRequest {
            root_override: Some(root.clone()),
            project_override: None,
            cwd: repository,
            environment: ResolutionEnvironment::default(),
        };
        Self {
            _temp: temp,
            project,
            resolution,
        }
    }

    fn batch_request(&self) -> OnboardingBatchRequest {
        OnboardingBatchRequest {
            resolution: self.resolution.clone(),
            notes: vec![
                ProposedNote {
                    note_type: "entity".to_owned(),
                    path: PathBuf::from("core.md"),
                    source: "---\nschema_version: 1\nentity: core\nkind: subsystem\nstatus: active\nreviewed: 2026-07-13\n---\n\n# Core\n\n[[Projects/example/records/tasks/next]]\n".to_owned(),
                },
                ProposedNote {
                    note_type: "task".to_owned(),
                    path: PathBuf::from("next.md"),
                    source: "---\nschema_version: 1\nproject: example\ntype: task\nstatus: open\ncreated: 2026-07-13\nupdated: 2026-07-13\n---\n\n# Next\n\n[[Projects/example/entities/core]]\n".to_owned(),
                },
                ProposedNote {
                    note_type: "session".to_owned(),
                    path: PathBuf::from("2026-07-13.md"),
                    source: "---\nschema_version: 1\nproject: example\ntype: session\ndate: 2026-07-13\n---\n\n# Session\n".to_owned(),
                },
            ],
            index: "# Index\n\n- [[Projects/example/entities/core]]\n".to_owned(),
            roadmap: "# Roadmap\n\n- [[Projects/example/records/tasks/next]]\n".to_owned(),
        }
    }

    fn evidenced_batch_request(&self) -> OnboardingBatchRequest {
        let mut request = self.batch_request();
        let fingerprint = content_fingerprint(
            &fs::read(request.resolution.cwd.join("Cargo.toml")).expect("read evidence source"),
        );
        request.notes[0].source = add_evidence(
            &request.notes[0].source,
            &format!(
                "evidence:\n  - kind: fact\n    claim: The repository declares a Rust \
                 package.\n    sources:\n      - path: Cargo.toml\n        fingerprint: \
                 \"{fingerprint}\"\n        line_start: 1\n        line_end: 2\n"
            ),
        );
        request.notes[1].source = add_evidence(
            &request.notes[1].source,
            &format!(
                "evidence:\n  - kind: inference\n    claim: The package needs a documented next \
                 task.\n    rationale: A package manifest exists but the initialized roadmap is \
                 empty.\n    sources:\n      - path: Cargo.toml\n        fingerprint: \
                 \"{fingerprint}\"\n"
            ),
        );
        request.notes[2].source = add_evidence(
            &request.notes[2].source,
            "evidence:\n  - kind: unknown\n    claim: The repository release process is not yet \
             known.\n    rationale: No release evidence was supplied in this bounded fixture.\n",
        );
        request
    }

    fn project_snapshot(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        collect_files(&self.project, &self.project, &mut files);
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    }
}

fn add_evidence(source: &str, evidence: &str) -> String {
    source.replacen("\n---\n\n#", &format!("\n{evidence}---\n\n#"), 1)
}

fn content_fingerprint(source: &[u8]) -> String {
    let digest = Sha256::digest(source);
    let mut fingerprint = String::with_capacity(71);
    fingerprint.push_str("sha256:");
    for byte in digest {
        write!(fingerprint, "{byte:02x}").expect("writing to a string cannot fail");
    }
    fingerprint
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
    let mut entries = fs::read_dir(directory)
        .expect("read project directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("read project entries");
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if entry.file_type().expect("read project entry type").is_dir() {
            collect_files(root, &path, files);
        } else {
            files.push((
                path.strip_prefix(root)
                    .expect("project-relative path")
                    .to_path_buf(),
                fs::read(path).expect("read project file"),
            ));
        }
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "akasha-onboarding-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create temporary directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
