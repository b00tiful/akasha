//! Interrupt actual shared-journal writers inside the atomic file helpers.

use super::*;
use crate::writes::crash_tests::{self as io_fault, Stage as IoStage};

#[test]
fn staging_residue_is_preserved_and_excluded_from_every_reader() {
    let fixture = Fixture::new();
    let baseline = crate::build_library_projection(&fixture.request).unwrap();
    let context = crate::assemble_context(&fixture.request).unwrap();
    let before = snapshot(&fixture.project);
    let names = [
        "entities/.core.md.akasha-123-0.tmp",
        "events/sessions/.partial.md.akasha-123-18446744073709551615.tmp",
        "records/tasks/.Δ.akasha-x.md.akasha-4294967295-7.tmp",
    ];
    for name in names {
        // Not even UTF-8: readers must not parse, fingerprint, or expose this content.
        fs::write(fixture.project.join(name), b"orphan-secret-marker\xff\x00").unwrap();
    }
    let global = fixture
        .base
        .join("root/Global/entities/.partial.md.akasha-123-0.tmp");
    fs::write(&global, b"orphan-secret-marker\xff").unwrap();
    assert_eq!(
        crate::build_library_projection(&fixture.request).unwrap(),
        baseline
    );
    assert_eq!(
        crate::render_context_markdown(&crate::assemble_context(&fixture.request).unwrap()),
        crate::render_context_markdown(&context)
    );
    assert_eq!(
        crate::search_library(&fixture.request, "orphan-secret-marker", None, 100)
            .unwrap()
            .total_matches,
        0
    );
    for name in names {
        assert_eq!(
            crate::load_library_document(&fixture.request, &format!("Projects/example/{name}"))
                .unwrap_err()
                .exit_code(),
            4
        );
        assert_eq!(
            fs::read(fixture.project.join(name)).unwrap(),
            b"orphan-secret-marker\xff\x00"
        );
    }
    assert_eq!(fs::read(global).unwrap(), b"orphan-secret-marker\xff");
    for (path, source) in before {
        assert_eq!(fs::read(fixture.project.join(path)).unwrap(), source);
    }
}

#[test]
fn staging_exception_rejects_malformed_names_and_keeps_hidden_markdown_canonical() {
    let fixture = Fixture::new();
    for name in [
        "note.tmp",
        ".note.tmp",
        "note.md.akasha-1-0.tmp",
        ".akasha-1-0.tmp",
        ".note.md.akasha-0-0.tmp",
        ".note.md.akasha-01-0.tmp",
        ".note.md.akasha-1-00.tmp",
        ".note.md.akasha-+1-0.tmp",
        ".note.md.akasha-1-+0.tmp",
        ".note.md.akasha-4294967296-0.tmp",
        ".note.md.akasha-1-18446744073709551616.tmp",
        ".note.md.akasha-pid-0.tmp",
        ".note.md.akasha-1-.tmp",
        ".note.md.akasha-1-2-3.tmp",
        ".note.md.akasha-1-0.tmp.bak",
        ".note.md.akasha-1-0.tmp.md",
        ".hidden.md",
    ] {
        let path = fixture.project.join("entities").join(name);
        fs::write(&path, b"invalid canonical content").unwrap();
        assert_eq!(
            validate_project(&fixture.request).unwrap_err().exit_code(),
            4,
            "{name}"
        );
        assert_eq!(fs::read(&path).unwrap(), b"invalid canonical content");
        fs::remove_file(path).unwrap();
    }
    validate_project(&fixture.request).unwrap();
}

#[test]
fn staging_exception_rejects_nonregular_entries_without_following_them() {
    let fixture = Fixture::new();
    let path = fixture.project.join("entities/.core.md.akasha-123-0.tmp");
    fs::create_dir(&path).unwrap();
    assert_eq!(
        validate_project(&fixture.request).unwrap_err().exit_code(),
        4
    );
    assert!(path.is_dir());
    fs::remove_dir(&path).unwrap();
    #[cfg(unix)]
    {
        let outside = fixture.base.join("external");
        fs::write(&outside, "private outside bytes").unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert_eq!(
            validate_project(&fixture.request).unwrap_err().exit_code(),
            4
        );
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(outside).unwrap(), b"private outside bytes");
    }
}

#[test]
fn recovers_publication_after_partial_staging_and_io_boundary_exits() {
    for schema in 1..=3 {
        let fixture = Fixture::new();
        let images = fixture.images(schema);
        let targets: Vec<_> = std::iter::once(NOTE_EDIT_JOURNAL_FILE)
            .chain(images.notes.iter().copied())
            .chain(images.projections.iter().copied())
            .chain(std::iter::once(PROJECT_STATE_FILE))
            .collect();
        for (index, &target) in targets.iter().enumerate() {
            for &stage in IoStage::ALL {
                fixture.restore(&images.before);
                run_child(&fixture, schema, "publish", target, stage);

                let mut expected = images.before.clone();
                for &path in targets.iter().take(index + usize::from(stage.published())) {
                    let source = if path == NOTE_EDIT_JOURNAL_FILE {
                        &images.journal
                    } else {
                        &images.after[Path::new(path)]
                    };
                    expected.insert(path.into(), source.clone());
                }
                let source = if target == NOTE_EDIT_JOURNAL_FILE {
                    &images.journal
                } else {
                    &images.after[Path::new(target)]
                };
                let residue = assert_interrupted(
                    &fixture,
                    &expected,
                    target,
                    source,
                    stage,
                    target == NOTE_EDIT_JOURNAL_FILE
                        || !images.before.contains_key(Path::new(target)),
                );

                // An external edit must be refused before any rollback or residue handling.
                if index > 0 && stage == IoStage::PartialWrite {
                    assert_external_refusal(&fixture, &expected, images.notes[0]);
                }
                let committed = target == PROJECT_STATE_FILE && stage.published();
                let mut recovered = if committed {
                    images.after.clone()
                } else {
                    images.before.clone()
                };
                recovered.extend(residue.clone());
                let outcome = if index == 0 && !stage.published() {
                    NoteEditRecovery::None
                } else if index == 0 || (index == 1 && !stage.published()) {
                    NoteEditRecovery::Discarded
                } else if committed {
                    NoteEditRecovery::Finalized
                } else {
                    NoteEditRecovery::RolledBack
                };
                fixture.assert_recovered(outcome, &recovered);
                // A new attempt must work with the untouched orphan still present.
                if !committed {
                    fixture.publish(schema);
                    let mut after = images.after.clone();
                    after.extend(residue);
                    assert_eq!(snapshot(&fixture.project), after);
                    validate_project(&fixture.request).unwrap();
                }
            }
        }
    }
}

#[test]
fn recovers_rollback_after_partial_staging_and_io_boundary_exits() {
    for schema in 1..=3 {
        let fixture = Fixture::new();
        let images = fixture.images(schema);
        // Recognized mixed image: state/projections after, notes before. This exercises every
        // replacement in reverse order, including state, without claiming publication order.
        let mut initial = images.before.clone();
        initial.insert(NOTE_EDIT_JOURNAL_FILE.into(), images.journal.clone());
        let targets: Vec<_> = std::iter::once(PROJECT_STATE_FILE)
            .chain(images.projections.iter().rev().copied())
            .collect();
        for &target in &targets {
            initial.insert(target.into(), images.after[Path::new(target)].clone());
        }
        check_rollback_targets(&fixture, schema, &images, &initial, &targets);

        if schema != 3 {
            // Separate note-only interruption exercises the mutable-note rollback stage.
            let mut initial = images.before.clone();
            initial.insert(NOTE_EDIT_JOURNAL_FILE.into(), images.journal.clone());
            initial.insert(ENTITY.into(), images.after[Path::new(ENTITY)].clone());
            check_rollback_targets(&fixture, schema, &images, &initial, &[ENTITY]);
        }
    }
}

fn check_rollback_targets(
    fixture: &Fixture,
    schema: u32,
    images: &Images,
    initial: &Snapshot,
    targets: &[&str],
) {
    for (index, &target) in targets.iter().enumerate() {
        for &stage in IoStage::ALL {
            fixture.restore(initial);
            run_child(fixture, schema, "recover", target, stage);
            let mut expected = initial.clone();
            for &path in targets.iter().take(index + usize::from(stage.published())) {
                expected.insert(path.into(), images.before[Path::new(path)].clone());
            }
            let residue = assert_interrupted(
                fixture,
                &expected,
                target,
                &images.before[Path::new(target)],
                stage,
                false,
            );
            if stage == IoStage::PartialWrite {
                assert_external_refusal(fixture, &expected, target);
            }
            expected.remove(Path::new(NOTE_EDIT_JOURNAL_FILE));
            let outcome = if expected == images.before {
                NoteEditRecovery::Discarded
            } else {
                NoteEditRecovery::RolledBack
            };
            let mut recovered = images.before.clone();
            recovered.extend(residue);
            fixture.assert_recovered(outcome, &recovered);
        }
    }
}

fn assert_external_refusal(fixture: &Fixture, expected: &Snapshot, target: &str) {
    fs::write(
        fixture.project.join(target),
        "external Ω bytes\r\n  preserve  ",
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
    if let Some(source) = expected.get(Path::new(target)) {
        fs::write(fixture.project.join(target), source).unwrap();
    } else {
        fs::remove_file(fixture.project.join(target)).unwrap();
    }
}

fn staging_residue(project: &Path) -> Snapshot {
    snapshot(project)
        .into_iter()
        .filter(|(path, _)| path.extension().is_some_and(|extension| extension == "tmp"))
        .collect()
}

fn assert_interrupted(
    fixture: &Fixture,
    expected: &Snapshot,
    target: &str,
    source: &[u8],
    stage: IoStage,
    creation: bool,
) -> Snapshot {
    let residue = staging_residue(&fixture.project);
    let retains_stage = !stage.published() || (creation && stage == IoStage::Published);
    assert_eq!(
        residue.len(),
        usize::from(retains_stage),
        "{target}/{stage:?}"
    );
    for (path, bytes) in &residue {
        assert_eq!(path.parent(), Path::new(target).parent());
        let prefix = format!(
            ".{}.akasha-",
            Path::new(target).file_name().unwrap().to_str().unwrap()
        );
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(&prefix)
        );
        let length = match stage {
            IoStage::Created => 0,
            IoStage::PartialWrite => source.len() / 2,
            _ => source.len(),
        };
        assert_eq!(bytes, &source[..length]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(fixture.project.join(path))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            if creation || matches!(stage, IoStage::Created | IoStage::PartialWrite) {
                assert_eq!(mode, 0o600);
            } else {
                let target_mode = fs::metadata(fixture.project.join(target))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777;
                assert_eq!(mode, target_mode);
            }
        }
    }
    let mut all = expected.clone();
    all.extend(residue.clone());
    assert_eq!(snapshot(&fixture.project), all, "{target}/{stage:?}");
    residue
}

fn run_child(fixture: &Fixture, schema: u32, action: &str, target: &str, stage: IoStage) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "note_edit::crash_tests::io_crash_tests::io_exit_child",
            "--nocapture",
        ])
        .env("AKASHA_IO_CRASH_BASE", &fixture.base)
        .env("AKASHA_IO_CRASH_SCHEMA", schema.to_string())
        .env("AKASHA_IO_CRASH_ACTION", action)
        .env("AKASHA_IO_CRASH_TARGET", target)
        .env("AKASHA_IO_CRASH_STAGE", format!("{stage:?}"))
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(74),
        "{schema}/{action}/{target}/{stage:?}: {output:?}"
    );
    drop(ProjectWriteLock::acquire(&fixture.project).expect("crashed child released writer lock"));
}

#[test]
fn io_exit_child() {
    let Some(base) = std::env::var_os("AKASHA_IO_CRASH_BASE") else {
        return;
    };
    let fixture = std::mem::ManuallyDrop::new(Fixture::from_base(PathBuf::from(base)));
    let stage = std::env::var("AKASHA_IO_CRASH_STAGE").unwrap();
    let stage = *IoStage::ALL
        .iter()
        .find(|candidate| format!("{candidate:?}") == stage)
        .unwrap();
    let target = fixture
        .project
        .join(std::env::var_os("AKASHA_IO_CRASH_TARGET").unwrap());
    io_fault::arm(target, stage);
    let schema = std::env::var("AKASHA_IO_CRASH_SCHEMA")
        .unwrap()
        .parse()
        .unwrap();
    match std::env::var("AKASHA_IO_CRASH_ACTION").unwrap().as_str() {
        "publish" => fixture.publish(schema),
        "recover" => {
            recover_pending_note_edit(&fixture.request).unwrap();
        }
        _ => panic!("unknown I/O crash action"),
    }
    panic!("requested I/O interruption was not reached");
}
