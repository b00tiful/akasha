//! Child-process acceptance. All interruption hooks are compiled out of product builds.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::ResolutionEnvironment;
use crate::event::{capture_handoff, create_event};
use crate::note_creation::create_mutable_note;
use crate::onboarding::{OnboardingBatchRequest, ProposedNote, apply_onboarding_batch};

const ENTITY: &str = "entities/core.md";
const ENTITY_ID: &str = "Projects/example/entities/core.md";
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    PublishedJournal,
    PublishedNote,
    PublishedProjection,
    PublishedState,
    RecoveredState,
    RecoveredProjection(usize),
    RecoveredNote(usize),
    BeforeCleanup,
    JournalRemoved,
    CleanupSynced,
}

impl Stage {
    const PUBLICATION: &[Self] = &[
        Self::PublishedJournal,
        Self::PublishedNote,
        Self::PublishedProjection,
        Self::PublishedState,
    ];
    const RECOVERY: &[Self] = &[
        Self::RecoveredState,
        Self::RecoveredProjection(0),
        Self::RecoveredProjection(1),
        Self::RecoveredNote(0),
        Self::RecoveredNote(1),
    ];
    const CLEANUP: &[Self] = &[
        Self::BeforeCleanup,
        Self::JournalRemoved,
        Self::CleanupSynced,
    ];
}

thread_local! {
    static EXIT_STAGE: Cell<Option<Stage>> = const { Cell::new(None) };
}

pub(crate) fn interrupt_at(stage: Stage) {
    if EXIT_STAGE.get() == Some(stage) {
        // Deliberately bypass unwind and every lock/file/fixture destructor.
        std::process::exit(73);
    }
}

#[test]
fn recovers_event_and_handoff_after_real_publication_exits() {
    check_lifecycle_publication_exits(&[
        ("event", "events/sessions/recovery.md", None, 1),
        ("handoff", "events/handoffs/recovery.md", None, 1),
    ]);
}

#[test]
fn recovers_mutable_creation_and_record_update_after_real_publication_exits() {
    check_lifecycle_publication_exits(&[
        (
            "create-record",
            "records/tasks/recovery.md",
            Some("roadmap.md"),
            2,
        ),
        ("create-entity", "entities/recovery.md", Some("index.md"), 2),
        (
            "update-record",
            "records/tasks/active.md",
            Some("roadmap.md"),
            2,
        ),
    ]);
}

fn check_lifecycle_publication_exits(cases: &[(&str, &str, Option<&str>, u32)]) {
    for &(action, note, projection, schema) in cases {
        for &stage in Stage::PUBLICATION {
            if projection.is_none() && stage == Stage::PublishedProjection {
                continue;
            }
            let fixture = Fixture::new();
            fixture.install_lifecycle_templates();
            let before = snapshot(&fixture.project);
            fixture.publish_lifecycle(action).unwrap();
            let after = snapshot(&fixture.project);
            assert_ne!(
                before[Path::new(PROJECT_STATE_FILE)],
                after[Path::new(PROJECT_STATE_FILE)]
            );
            assert!(
                after[Path::new(note)].ends_with("Exact Δ text with trailing spaces  ".as_bytes())
            );
            assert!(
                after[Path::new(note)]
                    .windows(2)
                    .any(|bytes| bytes == b"\r\n")
            );
            let text = |image: &Snapshot, path: &str| {
                String::from_utf8(image[Path::new(path)].clone()).unwrap()
            };
            let journal = render_journal(&NoteEditJournal {
                schema_version: schema,
                project: "example".into(),
                id: format!("Projects/example/{note}"),
                note_before: before
                    .get(Path::new(note))
                    .map(|source| String::from_utf8(source.clone()).unwrap()),
                note_after: text(&after, note),
                projection: projection.map(|path| JournalProjection {
                    id: format!("Projects/example/{path}"),
                    before: text(&before, path),
                    after: text(&after, path),
                }),
                state_before: text(&before, PROJECT_STATE_FILE),
                state_after: text(&after, PROJECT_STATE_FILE),
            })
            .unwrap();
            fixture.restore(&before);
            fixture.run_child(schema, action, stage);

            let committed = stage == Stage::PublishedState;
            let mut interrupted = before.clone();
            if stage != Stage::PublishedJournal {
                interrupted.insert(note.into(), after[Path::new(note)].clone());
            }
            if matches!(stage, Stage::PublishedProjection | Stage::PublishedState)
                && let Some(path) = projection
            {
                interrupted.insert(path.into(), after[Path::new(path)].clone());
            }
            if committed {
                interrupted.insert(
                    PROJECT_STATE_FILE.into(),
                    after[Path::new(PROJECT_STATE_FILE)].clone(),
                );
            }
            interrupted.insert(NOTE_EDIT_JOURNAL_FILE.into(), journal);
            assert_eq!(
                snapshot(&fixture.project),
                interrupted,
                "{action}/{stage:?}"
            );

            // An external edit after interruption must never be removed or overwritten.
            if stage == Stage::PublishedNote {
                let path = projection.unwrap_or(note);
                fs::write(
                    fixture.project.join(path),
                    "external Ω bytes\r\n  retained  ",
                )
                .unwrap();
                let conflict = snapshot(&fixture.project);
                for _ in 0..2 {
                    assert_eq!(
                        recover_pending_note_edit(&fixture.request)
                            .unwrap_err()
                            .exit_code(),
                        5
                    );
                    assert_eq!(snapshot(&fixture.project), conflict);
                }
                fs::write(fixture.project.join(path), &interrupted[Path::new(path)]).unwrap();
            }
            fixture.assert_recovered(
                if committed {
                    NoteEditRecovery::Finalized
                } else if stage == Stage::PublishedJournal {
                    NoteEditRecovery::Discarded
                } else {
                    NoteEditRecovery::RolledBack
                },
                if committed { &after } else { &before },
            );
            if committed && action != "update-record" {
                assert_eq!(fixture.publish_lifecycle(action).unwrap_err(), 5);
            } else if !committed {
                fixture.publish_lifecycle(action).unwrap();
            }
            assert_eq!(snapshot(&fixture.project), after);
            validate_project(&fixture.request).unwrap();
        }
    }
}

#[test]
fn recovers_version_one_and_two_after_real_publication_exits() {
    for schema in [1, 2] {
        for &stage in Stage::PUBLICATION {
            if schema == 1 && stage == Stage::PublishedProjection {
                continue;
            }
            let fixture = Fixture::new();
            let images = fixture.images(schema);
            fixture.run_child(schema, "publish", stage);

            let committed = stage == Stage::PublishedState;
            let mut interrupted = images.before.clone();
            if stage != Stage::PublishedJournal {
                interrupted.insert(ENTITY.into(), images.after[Path::new(ENTITY)].clone());
            }
            if schema == 2 && matches!(stage, Stage::PublishedProjection | Stage::PublishedState) {
                interrupted.insert(
                    "index.md".into(),
                    images.after[Path::new("index.md")].clone(),
                );
            }
            if committed {
                interrupted.insert(
                    PROJECT_STATE_FILE.into(),
                    images.after[Path::new(PROJECT_STATE_FILE)].clone(),
                );
            }
            interrupted.insert(NOTE_EDIT_JOURNAL_FILE.into(), images.journal.clone());
            assert_eq!(
                snapshot(&fixture.project),
                interrupted,
                "{schema}/{stage:?}"
            );

            fixture.assert_recovered(
                if committed {
                    NoteEditRecovery::Finalized
                } else if stage == Stage::PublishedJournal {
                    NoteEditRecovery::Discarded
                } else {
                    NoteEditRecovery::RolledBack
                },
                if committed {
                    &images.after
                } else {
                    &images.before
                },
            );
        }
    }
}

#[test]
fn resumes_version_one_two_and_three_after_rollback_and_cleanup_exits() {
    for schema in [1, 2, 3] {
        for mode in ["partial", "state-only", "unused", "committed"] {
            let restorations = match mode {
                "partial" => match schema {
                    1 => vec![Stage::RecoveredNote(0)],
                    2 => vec![Stage::RecoveredProjection(0), Stage::RecoveredNote(0)],
                    3 => vec![
                        Stage::RecoveredProjection(1),
                        Stage::RecoveredProjection(0),
                        Stage::RecoveredNote(1),
                        Stage::RecoveredNote(0),
                    ],
                    _ => unreachable!(),
                },
                "state-only" => vec![Stage::RecoveredState],
                _ => vec![],
            };
            for &stage in restorations.iter().chain(Stage::CLEANUP) {
                let fixture = Fixture::new();
                let images = fixture.images(schema);
                let mut interrupted = if matches!(mode, "partial" | "committed") {
                    images.after.clone()
                } else {
                    images.before.clone()
                };
                if mode == "partial" {
                    interrupted.insert(
                        PROJECT_STATE_FILE.into(),
                        images.before[Path::new(PROJECT_STATE_FILE)].clone(),
                    );
                } else if mode == "state-only" {
                    interrupted.insert(
                        PROJECT_STATE_FILE.into(),
                        images.after[Path::new(PROJECT_STATE_FILE)].clone(),
                    );
                }
                interrupted.insert(NOTE_EDIT_JOURNAL_FILE.into(), images.journal.clone());
                fixture.restore(&interrupted);
                fixture.run_child(schema, "recover", stage);

                // Independently derive the reverse-order prefix reached before exit.
                for &restored in &restorations {
                    let path = match restored {
                        Stage::RecoveredState => PROJECT_STATE_FILE,
                        Stage::RecoveredProjection(index) => images.projections[index],
                        Stage::RecoveredNote(index) => images.notes[index],
                        _ => unreachable!(),
                    };
                    if let Some(source) = images.before.get(Path::new(path)) {
                        interrupted.insert(path.into(), source.clone());
                    } else {
                        interrupted.remove(Path::new(path));
                    }
                    if restored == stage {
                        break;
                    }
                }
                let removed = matches!(stage, Stage::JournalRemoved | Stage::CleanupSynced);
                if removed {
                    interrupted.remove(Path::new(NOTE_EDIT_JOURNAL_FILE));
                }
                assert_eq!(
                    snapshot(&fixture.project),
                    interrupted,
                    "{schema}/{mode}/{stage:?}"
                );

                // An external writer after a rollback interruption still forces exact refusal.
                if restorations.first() == Some(&stage) {
                    let path = fixture.project.join(images.notes[0]);
                    let original = fs::read(&path).ok();
                    fs::write(&path, "external Δ edit\r\n  retained  ").unwrap();
                    let conflict = snapshot(&fixture.project);
                    for _ in 0..2 {
                        assert_eq!(
                            recover_pending_note_edit(&fixture.request)
                                .unwrap_err()
                                .exit_code(),
                            5
                        );
                        assert_eq!(snapshot(&fixture.project), conflict);
                    }
                    if let Some(original) = original {
                        fs::write(path, original).unwrap();
                    } else {
                        fs::remove_file(path).unwrap();
                    }
                }
                interrupted.remove(Path::new(NOTE_EDIT_JOURNAL_FILE));
                let outcome = if removed {
                    NoteEditRecovery::None
                } else if mode == "committed" {
                    NoteEditRecovery::Finalized
                } else if interrupted == images.before {
                    NoteEditRecovery::Discarded
                } else {
                    NoteEditRecovery::RolledBack
                };
                fixture.assert_recovered(
                    outcome,
                    if mode == "committed" {
                        &images.after
                    } else {
                        &images.before
                    },
                );
            }
        }
    }
}

#[test]
fn mutation_exit_child() {
    let Some(base) = std::env::var_os("AKASHA_NOTE_CRASH_BASE") else {
        return;
    };
    // Only the parent owns cleanup of this disposable tree.
    let fixture = std::mem::ManuallyDrop::new(Fixture::from_base(PathBuf::from(base)));
    let stage = std::env::var("AKASHA_NOTE_CRASH_STAGE").unwrap();
    let stage = *Stage::PUBLICATION
        .iter()
        .chain(Stage::RECOVERY)
        .chain(Stage::CLEANUP)
        .find(|candidate| format!("{candidate:?}") == stage)
        .expect("known interruption stage");
    EXIT_STAGE.set(Some(stage));
    let schema = std::env::var("AKASHA_NOTE_CRASH_SCHEMA")
        .unwrap()
        .parse()
        .unwrap();
    match std::env::var("AKASHA_NOTE_CRASH_ACTION").unwrap().as_str() {
        "publish" => fixture.publish(schema),
        "recover" => {
            recover_pending_note_edit(&fixture.request).unwrap();
        }
        action => fixture.publish_lifecycle(action).unwrap(),
    }
    panic!("requested interruption was not reached");
}

type Snapshot = BTreeMap<PathBuf, Vec<u8>>;

struct Images {
    before: Snapshot,
    after: Snapshot,
    journal: Vec<u8>,
    notes: Vec<&'static str>,
    projections: Vec<&'static str>,
}

struct Fixture {
    base: PathBuf,
    project: PathBuf,
    request: ResolveRequest,
}

impl Fixture {
    fn new() -> Self {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let base =
            std::env::temp_dir().join(format!("akasha-note-crash-{}-{id}", std::process::id()));
        fs::create_dir(&base).unwrap();
        copy_tree(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/resolution/valid-root"),
            &base.join("root"),
        );
        fs::create_dir(base.join("repository")).unwrap();
        let fixture = Self::from_base(base);
        // Include the persistent lock file in both exact snapshots.
        drop(ProjectWriteLock::acquire(&fixture.project).unwrap());
        fixture
    }

    fn install_lifecycle_templates(&self) {
        for (name, metadata) in [
            (
                "session",
                "project: {{project}}\r\ntype: {{type}}\r\ndate: 2026-10-03",
            ),
            (
                "handoff",
                "project: {{project}}\r\ntype: {{type}}\r\ndate: 2026-10-03",
            ),
            (
                "task",
                "project: {{project}}\r\ntype: {{type}}\r\nstatus: active\r\ncreated: 2026-10-03\r\nupdated: 2026-10-03",
            ),
            (
                "entity",
                "entity: recovery\r\nkind: subsystem\r\nstatus: active\r\nreviewed: 2026-10-03",
            ),
        ] {
            fs::write(self.project.join(format!("templates/{name}.md")),
                format!("---\r\nschema_version: 1\r\n{metadata}\r\n---\r\n\r\n# Recovery Δ\r\n\r\n{{{{body}}}}"),
            ).unwrap();
        }
    }

    fn from_base(base: PathBuf) -> Self {
        Self {
            project: base.join("root/Projects/example"),
            request: ResolveRequest {
                root_override: Some(base.join("root")),
                project_override: Some("example".to_owned()),
                cwd: base.join("repository"),
                environment: ResolutionEnvironment::default(),
            },
            base,
        }
    }

    fn publish(&self, schema: u32) {
        let before = fs::read_to_string(self.project.join(ENTITY)).unwrap();
        let after = format!(
            "{}\r\nExact Δ edit with trailing spaces  ",
            before.replace('\n', "\r\n")
        );
        match schema {
            1 => {
                replace_library_document(&self.request, ENTITY_ID, &before, &after).unwrap();
            }
            2 => {
                let index = format!(
                    "{}\nReviewed Δ entity index.\n",
                    fs::read_to_string(self.project.join("index.md")).unwrap()
                );
                update_entity(&self.request, ENTITY_ID, &before, &after, &index).unwrap();
            }
            3 => {
                let notes = ["recovery-a", "recovery-b"].map(|name| ProposedNote {
                    note_type: "entity".into(),
                    path: format!("{name}.md").into(),
                    source: format!(
                        "---\nschema_version: 1\nentity: {name}\nkind: subsystem\nstatus: active\nreviewed: 2026-10-03\nevidence:\n  - kind: unknown\n    claim: Synthetic recovery fixture.\n    rationale: No production assertion.\n---\n\n# {name}\n\nExact Δ bytes.\n"
                    ),
                });
                apply_onboarding_batch(&OnboardingBatchRequest {
                    resolution: self.request.clone(),
                    notes: notes.into(),
                    index: format!(
                        "{}\n[[Projects/example/entities/recovery-a]]\n[[Projects/example/entities/recovery-b]]\n",
                        fs::read_to_string(self.project.join("index.md")).unwrap()
                    ),
                    roadmap: format!(
                        "{}\nReviewed recovery fixture.\n",
                        fs::read_to_string(self.project.join("roadmap.md")).unwrap()
                    ),
                }).unwrap();
            }
            _ => panic!("unsupported publication fixture"),
        }
    }

    fn publish_lifecycle(&self, action: &str) -> Result<(), u8> {
        let body = "Exact Δ text with trailing spaces  ";
        let fields = BTreeMap::from([("body".into(), body.into())]);
        let projection = |path: &str| {
            format!(
                "{}\r\nReviewed Δ projection.  ",
                fs::read_to_string(self.project.join(path)).unwrap()
            )
        };
        match action {
            "event" => create_event(&self.request, "session", Path::new("recovery.md"), &fields)
                .map(|_| ())
                .map_err(|error| error.exit_code()),
            "handoff" => capture_handoff(&self.request, Path::new("recovery.md"), &fields)
                .map(|_| ())
                .map_err(|error| error.exit_code()),
            "create-record" | "create-entity" => {
                let (note_type, path) = if action == "create-record" {
                    ("task", "roadmap.md")
                } else {
                    ("entity", "index.md")
                };
                create_mutable_note(
                    &self.request,
                    note_type,
                    Path::new("recovery.md"),
                    &fields,
                    &projection(path),
                )
                .map(|_| ())
                .map_err(|error| error.exit_code())
            }
            "update-record" => {
                let before =
                    fs::read_to_string(self.project.join("records/tasks/active.md")).unwrap();
                let after = format!(
                    "{}\r\n{body}",
                    before
                        .replace("status: active", "status: done")
                        .replace('\n', "\r\n")
                );
                update_record(
                    &self.request,
                    "Projects/example/records/tasks/active.md",
                    &before,
                    &after,
                    &projection("roadmap.md"),
                )
                .map(|_| ())
                .map_err(|error| error.exit_code())
            }
            _ => panic!("unknown lifecycle operation"),
        }
    }

    fn images(&self, schema: u32) -> Images {
        let before = snapshot(&self.project);
        self.publish(schema);
        let after = snapshot(&self.project);
        let text = |image: &Snapshot, path: &str| {
            String::from_utf8(image[Path::new(path)].clone()).unwrap()
        };
        let notes = if schema == 3 {
            vec!["entities/recovery-a.md", "entities/recovery-b.md"]
        } else {
            vec![ENTITY]
        };
        let projections = match schema {
            1 => vec![],
            2 => vec!["index.md"],
            3 => vec!["index.md", "roadmap.md"],
            _ => unreachable!(),
        };
        let journal = if schema == 3 {
            render_journal(&OnboardingBatchJournal {
                schema_version: 3,
                project: "example".into(),
                notes: notes
                    .iter()
                    .map(|path| JournalCreatedNote {
                        id: format!("Projects/example/{path}"),
                        after: text(&after, path),
                    })
                    .collect(),
                projections: projections
                    .iter()
                    .map(|path| JournalProjection {
                        id: format!("Projects/example/{path}"),
                        before: text(&before, path),
                        after: text(&after, path),
                    })
                    .collect(),
                state_before: text(&before, PROJECT_STATE_FILE),
                state_after: text(&after, PROJECT_STATE_FILE),
            })
        } else {
            render_journal(&NoteEditJournal {
                schema_version: schema,
                project: "example".into(),
                id: ENTITY_ID.into(),
                note_before: Some(text(&before, ENTITY)),
                note_after: text(&after, ENTITY),
                projection: (schema == 2).then(|| JournalProjection {
                    id: "Projects/example/index.md".into(),
                    before: text(&before, "index.md"),
                    after: text(&after, "index.md"),
                }),
                state_before: text(&before, PROJECT_STATE_FILE),
                state_after: text(&after, PROJECT_STATE_FILE),
            })
        }
        .unwrap();
        self.restore(&before);
        Images {
            before,
            after,
            journal,
            notes,
            projections,
        }
    }

    fn restore(&self, image: &Snapshot) {
        for path in snapshot(&self.project).keys() {
            if !image.contains_key(path) {
                fs::remove_file(self.project.join(path)).unwrap();
            }
        }
        for (path, source) in image {
            fs::write(self.project.join(path), source).unwrap();
        }
    }

    fn run_child(&self, schema: u32, action: &str, stage: Stage) {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "note_edit::crash_tests::mutation_exit_child",
                "--nocapture",
            ])
            .env("AKASHA_NOTE_CRASH_BASE", &self.base)
            .env("AKASHA_NOTE_CRASH_SCHEMA", schema.to_string())
            .env("AKASHA_NOTE_CRASH_ACTION", action)
            .env("AKASHA_NOTE_CRASH_STAGE", format!("{stage:?}"))
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(73),
            "{schema}/{action}/{stage:?}: {output:?}"
        );
        // Acquire explicitly even when journal cleanup left no pending work.
        drop(ProjectWriteLock::acquire(&self.project).expect("child lock was released"));
    }

    fn assert_recovered(&self, outcome: NoteEditRecovery, expected: &Snapshot) {
        assert_eq!(recover_pending_note_edit(&self.request).unwrap(), outcome);
        assert_eq!(&snapshot(&self.project), expected);
        validate_project(&self.request).unwrap();
        assert_eq!(
            recover_pending_note_edit(&self.request).unwrap(),
            NoteEditRecovery::None
        );
        assert_eq!(&snapshot(&self.project), expected);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.base).unwrap();
    }
}

fn snapshot(directory: &Path) -> Snapshot {
    fn visit(base: &Path, directory: &Path, files: &mut Snapshot) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(base, &entry.path(), files);
            } else {
                files.insert(
                    entry.path().strip_prefix(base).unwrap().into(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut files = Snapshot::new();
    visit(directory, directory, &mut files);
    files
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
