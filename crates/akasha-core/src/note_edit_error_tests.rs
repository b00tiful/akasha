//! Ordinary error and retry acceptance over the actual shared-journal writers.

use super::*;
use crate::writes::error_tests::{Fault, Guard, Stage as IoStage};

const WRITE_STAGES: &[IoStage] = &[
    IoStage::Create,
    IoStage::PartialWrite,
    IoStage::FileSync,
    IoStage::Publish,
];

#[test]
fn primitive_errors_preserve_source_kind_exact_bytes_modes_and_clean_stages() {
    use crate::writes::{AtomicCreateError, CheckedReplaceError};
    let fixture = Fixture::new();
    let target = fixture.project.join("entities/errors.md");
    let source = b"\0before\xff\r\n  ";
    let replacement = b"\0after\xfe\r\n  ";
    for replacing in [false, true] {
        for stage in WRITE_STAGES
            .iter()
            .copied()
            .chain(replacing.then_some(IoStage::Permissions))
        {
            if replacing {
                fs::write(&target, source).unwrap();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
                }
            }
            let before = snapshot(&fixture.project);
            let guard = Guard::arm(vec![Fault::new(&target, stage, io::ErrorKind::StorageFull)]);
            let (error_path, kind) = if replacing {
                match replace_file_if_unchanged(&target, source, replacement).unwrap_err() {
                    CheckedReplaceError::FileSystem { path, source, .. } => (path, source.kind()),
                    error => panic!("wrong classification: {error}"),
                }
            } else {
                let error = create_file_atomically(&target, replacement).unwrap_err();
                assert_eq!(error.exit_code(), 6);
                match error {
                    AtomicCreateError::FileSystem { path, source, .. } => (path, source.kind()),
                    error => panic!("wrong classification: {error}"),
                }
            };
            assert_eq!(kind, io::ErrorKind::StorageFull);
            assert_eq!(error_path.parent(), target.parent());
            assert_eq!(guard.hits(), [1]);
            drop(guard);
            assert_eq!(snapshot(&fixture.project), before);
            if replacing {
                assert!(replace_file_if_unchanged(&target, source, replacement).unwrap());
            } else {
                create_file_atomically(&target, replacement).unwrap();
            }
            assert_eq!(fs::read(&target).unwrap(), replacement);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                    if replacing { 0o640 } else { 0o600 }
                );
            }
            fs::remove_file(&target).unwrap();
        }
    }
}

#[test]
fn returned_publication_errors_preserve_exact_images_and_allow_reapplication() {
    for schema in 1..=3 {
        let fixture = Fixture::new();
        let images = fixture.images(schema);
        let targets: Vec<_> = std::iter::once(NOTE_EDIT_JOURNAL_FILE)
            .chain(images.notes.iter().copied())
            .chain(images.projections.iter().copied())
            .chain(std::iter::once(PROJECT_STATE_FILE))
            .collect();
        for (index, target) in targets.iter().enumerate() {
            let target_path = fixture.project.join(target);
            let mut stages = WRITE_STAGES.to_vec();
            if images.before.contains_key(Path::new(target)) {
                stages.push(IoStage::Permissions);
            }
            stages.push(IoStage::DirectorySync);
            for stage in stages {
                for kind in [
                    io::ErrorKind::StorageFull,
                    io::ErrorKind::PermissionDenied,
                    io::ErrorKind::Other,
                ] {
                    fixture.restore(&images.before);
                    let fault = if stage == IoStage::DirectorySync {
                        let parent = target_path.parent().unwrap();
                        let prior_syncs = targets[..index]
                            .iter()
                            .filter(|target| fixture.project.join(target).parent() == Some(parent))
                            .count();
                        Fault::new(parent, stage, kind).skip(prior_syncs)
                    } else {
                        Fault::new(&target_path, stage, kind)
                    };
                    let guard = Guard::arm(vec![fault]);
                    let result = fixture.try_publish(schema);
                    assert_eq!(guard.hits(), [1], "{schema}/{target}/{stage:?}/{kind:?}");
                    drop(guard);
                    // Version 1/2 writers finalize a valid state-last image; onboarding's
                    // returned-error path deliberately rolls its own completed writes back.
                    let finalized = schema != 3
                        && *target == PROJECT_STATE_FILE
                        && stage == IoStage::DirectorySync;
                    if finalized {
                        assert_eq!(result, Ok(()));
                        assert_eq!(snapshot(&fixture.project), images.after);
                        fixture.assert_recovered(NoteEditRecovery::None, &images.after);
                    } else {
                        let (code, message) = result.unwrap_err();
                        assert_eq!(code, 6, "{message}");
                        assert!(
                            message.contains("synthetic returned I/O failure"),
                            "{message}"
                        );
                        let pending =
                            *target == NOTE_EDIT_JOURNAL_FILE && stage == IoStage::DirectorySync;
                        let mut expected = images.before.clone();
                        if pending {
                            expected.insert(NOTE_EDIT_JOURNAL_FILE.into(), images.journal.clone());
                        }
                        assert_eq!(
                            snapshot(&fixture.project),
                            expected,
                            "{schema}/{target}/{stage:?}"
                        );
                        drop(
                            ProjectWriteLock::acquire(&fixture.project)
                                .expect("returned error released writer lock"),
                        );
                        fixture.assert_recovered(
                            if pending {
                                NoteEditRecovery::Discarded
                            } else {
                                NoteEditRecovery::None
                            },
                            &images.before,
                        );
                        fixture.publish(schema);
                        assert_eq!(snapshot(&fixture.project), images.after);
                        validate_project(&fixture.request).unwrap();
                    }
                }
            }
        }
    }
}

#[test]
fn returned_lifecycle_errors_use_shared_rollback_and_retry_contract() {
    for (action, target) in [
        ("event", "events/sessions/recovery.md"),
        ("handoff", "events/handoffs/recovery.md"),
        ("create-record", "records/tasks/recovery.md"),
        ("create-entity", "entities/recovery.md"),
        ("update-record", "records/tasks/active.md"),
    ] {
        let fixture = Fixture::new();
        fixture.install_lifecycle_templates();
        let before = snapshot(&fixture.project);
        fixture.publish_lifecycle(action).unwrap();
        let after = snapshot(&fixture.project);
        for path in [target, PROJECT_STATE_FILE] {
            for &stage in WRITE_STAGES {
                fixture.restore(&before);
                let guard = Guard::arm(vec![Fault::new(
                    fixture.project.join(path),
                    stage,
                    io::ErrorKind::StorageFull,
                )]);
                assert_eq!(
                    fixture.publish_lifecycle(action),
                    Err(6),
                    "{action}/{path}/{stage:?}"
                );
                assert_eq!(guard.hits(), [1]);
                drop(guard);
                assert_eq!(snapshot(&fixture.project), before);
                drop(ProjectWriteLock::acquire(&fixture.project).unwrap());
                fixture.assert_recovered(NoteEditRecovery::None, &before);
                fixture.publish_lifecycle(action).unwrap();
                assert_eq!(snapshot(&fixture.project), after);
                validate_project(&fixture.request).unwrap();
            }
        }
    }
}

#[test]
fn failed_automatic_rollback_retains_authority_and_refuses_external_bytes() {
    for schema in 1..=3 {
        let fixture = Fixture::new();
        let images = fixture.images(schema);
        let note = images.notes[0];
        let rollback_fault = if schema == 3 {
            Fault::new(
                fixture.project.join(note),
                IoStage::Remove,
                io::ErrorKind::PermissionDenied,
            )
            .persistent()
        } else {
            Fault::new(
                fixture.project.join(note),
                IoStage::Publish,
                io::ErrorKind::PermissionDenied,
            )
            .skip(1)
            .persistent()
        };
        let guard = Guard::arm(vec![
            Fault::new(
                fixture.project.join(PROJECT_STATE_FILE),
                IoStage::FileSync,
                io::ErrorKind::StorageFull,
            ),
            rollback_fault,
        ]);
        let (code, message) = fixture.try_publish(schema).unwrap_err();
        assert_eq!(code, 6);
        assert!(message.contains("synthetic returned I/O failure"));
        assert_eq!(guard.hits(), [1, 1]);
        let mut expected = images.before.clone();
        expected.insert(note.into(), images.after[Path::new(note)].clone());
        expected.insert(NOTE_EDIT_JOURNAL_FILE.into(), images.journal.clone());
        assert_eq!(snapshot(&fixture.project), expected);
        drop(ProjectWriteLock::acquire(&fixture.project).unwrap());
        assert_eq!(
            recover_pending_note_edit(&fixture.request)
                .unwrap_err()
                .exit_code(),
            6
        );
        assert_eq!(guard.hits(), [1, 2]);
        assert_eq!(snapshot(&fixture.project), expected);
        drop(guard);
        fs::write(fixture.project.join(note), "External editor Δ\r\n  ").unwrap();
        let external = snapshot(&fixture.project);
        for _ in 0..2 {
            assert_eq!(
                recover_pending_note_edit(&fixture.request)
                    .unwrap_err()
                    .exit_code(),
                5
            );
            assert_eq!(snapshot(&fixture.project), external);
        }
        fixture.restore(&expected);
        fixture.assert_recovered(NoteEditRecovery::RolledBack, &images.before);
        fixture.publish(schema);
        assert_eq!(snapshot(&fixture.project), images.after);
        validate_project(&fixture.request).unwrap();
    }
}

#[test]
fn recovery_retries_failed_directory_sync_even_when_bytes_already_match() {
    for schema in 1..=3 {
        let fixture = Fixture::new();
        let images = fixture.images(schema);
        let mut interrupted = images.before.clone();
        interrupted.insert(NOTE_EDIT_JOURNAL_FILE.into(), images.journal.clone());
        let note = Path::new(images.notes[0]);
        interrupted.insert(note.into(), images.after[note].clone());
        fixture.restore(&interrupted);
        let guard = Guard::arm(vec![
            Fault::new(
                fixture.project.join(note).parent().unwrap(),
                IoStage::DirectorySync,
                io::ErrorKind::Other,
            )
            .persistent(),
        ]);
        assert_eq!(
            recover_pending_note_edit(&fixture.request)
                .unwrap_err()
                .exit_code(),
            6
        );
        let mut restored = images.before.clone();
        restored.insert(NOTE_EDIT_JOURNAL_FILE.into(), images.journal.clone());
        assert_eq!(snapshot(&fixture.project), restored);
        assert_eq!(
            recover_pending_note_edit(&fixture.request)
                .unwrap_err()
                .exit_code(),
            6
        );
        assert_eq!(guard.hits(), [2]);
        assert_eq!(snapshot(&fixture.project), restored);
        drop(guard);
        fixture.assert_recovered(NoteEditRecovery::Discarded, &images.before);
    }
}

#[test]
fn recovery_requires_directory_barrier_for_unused_and_committed_images() {
    for schema in 1..=3 {
        let fixture = Fixture::new();
        let images = fixture.images(schema);
        for (image, outcome) in [
            (&images.before, NoteEditRecovery::Discarded),
            (&images.after, NoteEditRecovery::Finalized),
        ] {
            let mut pending = image.clone();
            pending.insert(NOTE_EDIT_JOURNAL_FILE.into(), images.journal.clone());
            fixture.restore(&pending);
            let guard = Guard::arm(vec![
                Fault::new(
                    fixture.project.join(images.notes[0]).parent().unwrap(),
                    IoStage::DirectorySync,
                    io::ErrorKind::Other,
                )
                .persistent(),
            ]);
            for _ in 0..2 {
                assert_eq!(
                    recover_pending_note_edit(&fixture.request)
                        .unwrap_err()
                        .exit_code(),
                    6
                );
                assert_eq!(snapshot(&fixture.project), pending);
                drop(ProjectWriteLock::acquire(&fixture.project).unwrap());
            }
            assert_eq!(guard.hits(), [2]);
            drop(guard);
            fixture.assert_recovered(outcome, image);
        }
    }
}

#[test]
fn returned_rollback_errors_preserve_each_partial_image_and_retry() {
    for schema in 1..=3 {
        let fixture = Fixture::new();
        let images = fixture.images(schema);
        // State/projections after and notes before is a recognized mixed image, not a
        // claim about actual writer order. It exercises every replacement in rollback.
        let targets: Vec<_> = std::iter::once(PROJECT_STATE_FILE)
            .chain(images.projections.iter().rev().copied())
            .collect();
        let mut initial = images.before.clone();
        initial.insert(NOTE_EDIT_JOURNAL_FILE.into(), images.journal.clone());
        for target in &targets {
            initial.insert((*target).into(), images.after[Path::new(target)].clone());
        }
        for (index, target) in targets.iter().enumerate() {
            for stage in WRITE_STAGES
                .iter()
                .copied()
                .chain([IoStage::Permissions, IoStage::DirectorySync])
            {
                fixture.restore(&initial);
                let path = fixture.project.join(target);
                let fault = if stage == IoStage::DirectorySync {
                    Fault::new(path.parent().unwrap(), stage, io::ErrorKind::Other).skip(index)
                } else {
                    Fault::new(path, stage, io::ErrorKind::StorageFull)
                };
                let guard = Guard::arm(vec![fault]);
                assert_eq!(
                    recover_pending_note_edit(&fixture.request)
                        .unwrap_err()
                        .exit_code(),
                    6
                );
                assert_eq!(guard.hits(), [1]);
                drop(guard);
                let restored = index + usize::from(stage == IoStage::DirectorySync);
                let mut expected = initial.clone();
                for path in targets.iter().take(restored) {
                    expected.insert((*path).into(), images.before[Path::new(path)].clone());
                }
                assert_eq!(
                    snapshot(&fixture.project),
                    expected,
                    "{schema}/{target}/{stage:?}"
                );
                drop(ProjectWriteLock::acquire(&fixture.project).unwrap());
                fixture.assert_recovered(
                    if restored == targets.len() {
                        NoteEditRecovery::Discarded
                    } else {
                        NoteEditRecovery::RolledBack
                    },
                    &images.before,
                );
            }
        }
    }
}

#[test]
fn journal_cleanup_errors_retain_authority_or_report_completed_unlink() {
    for schema in 1..=3 {
        let fixture = Fixture::new();
        let images = fixture.images(schema);
        for (image, outcome) in [
            (&images.before, NoteEditRecovery::Discarded),
            (&images.after, NoteEditRecovery::Finalized),
        ] {
            let mut pending = image.clone();
            pending.insert(NOTE_EDIT_JOURNAL_FILE.into(), images.journal.clone());
            fixture.restore(&pending);
            let guard = Guard::arm(vec![
                Fault::new(
                    fixture.project.join(NOTE_EDIT_JOURNAL_FILE),
                    IoStage::Remove,
                    io::ErrorKind::PermissionDenied,
                )
                .persistent(),
            ]);
            for _ in 0..2 {
                assert_eq!(
                    recover_pending_note_edit(&fixture.request)
                        .unwrap_err()
                        .exit_code(),
                    6
                );
                assert_eq!(snapshot(&fixture.project), pending);
            }
            assert_eq!(guard.hits(), [2]);
            drop(guard);
            fixture.assert_recovered(outcome, image);

            fixture.restore(&pending);
            let guard = Guard::arm(vec![
                Fault::new(
                    &fixture.project,
                    IoStage::DirectorySync,
                    io::ErrorKind::Other,
                )
                .skip(1),
            ]); // The first root sync is the artifact completion barrier.
            let error = recover_pending_note_edit(&fixture.request).unwrap_err();
            assert_eq!(error.exit_code(), 6);
            assert!(
                error
                    .to_string()
                    .contains("sync removal of the note edit journal")
            );
            assert_eq!(guard.hits(), [1]);
            drop(guard);
            assert_eq!(snapshot(&fixture.project), *image);
            fixture.assert_recovered(NoteEditRecovery::None, image);
        }
    }
}

#[test]
fn publication_cleanup_failure_distinguishes_pending_commit_from_uncertain_cleanup() {
    for schema in 1..=3 {
        let fixture = Fixture::new();
        let images = fixture.images(schema);
        for persistent in [false, true] {
            fixture.restore(&images.before);
            let fault = Fault::new(
                fixture.project.join(NOTE_EDIT_JOURNAL_FILE),
                IoStage::Remove,
                io::ErrorKind::PermissionDenied,
            );
            let guard = Guard::arm(vec![if persistent {
                fault.persistent()
            } else {
                fault
            }]);
            let result = fixture.try_publish(schema);
            assert_eq!(guard.hits(), [if persistent { 2 } else { 1 }]);
            drop(guard);
            if persistent {
                assert_eq!(result.unwrap_err().0, 6);
                let mut pending = images.after.clone();
                pending.insert(NOTE_EDIT_JOURNAL_FILE.into(), images.journal.clone());
                assert_eq!(snapshot(&fixture.project), pending);
                fixture.assert_recovered(NoteEditRecovery::Finalized, &images.after);
            } else {
                assert_eq!(result, Ok(()));
                fixture.assert_recovered(NoteEditRecovery::None, &images.after);
            }
        }
    }
}
