//! Historical images are constructed directly: ordinary writers must reject these sizes.
use super::*;
use crate::MAX_MARKDOWN_SOURCE_BYTES;
use crate::event::{prepare_event_creation, preview_event_creation};
use crate::note_creation::{prepare_mutable_note_creation, preview_mutable_note_creation};

#[test]
fn source_limits_check_exact_reads_after_the_projection_was_built() {
    let fixture = Fixture::new();
    let projection = build_library_projection(&fixture.request).unwrap();
    let path = fixture.project.join(ENTITY);
    let mut source = fs::read(&path).unwrap();
    source.resize(MAX_MARKDOWN_SOURCE_BYTES + 1, b'x');
    fs::write(&path, source).unwrap();
    for error in [
        crate::library::document_from_projection(&projection, ENTITY_ID).unwrap_err(),
        crate::note_links::read_checked_source(&projection.root, &path).unwrap_err(),
    ] {
        assert_eq!(error.exit_code(), 4);
        assert!(error.to_string().contains("16777216-byte"));
    }
    let error = read_regular_markdown(&path, "read changed lifecycle source").unwrap_err();
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("16777216-byte"));
}

#[test]
fn source_limits_reject_new_writes_and_previews_without_publication() {
    let fixture = Fixture::new();
    fixture.install_lifecycle_templates();
    let before = snapshot(&fixture.project);
    let source = fs::read_to_string(fixture.project.join(ENTITY)).unwrap();
    let oversized = "x".repeat(MAX_MARKDOWN_SOURCE_BYTES + 1);
    let replacement = format!("{source}\n{oversized}");
    let assert_refused = |result: Result<(), u8>| {
        assert_eq!(result, Err(4));
        assert_eq!(snapshot(&fixture.project), before);
        assert!(!fixture.project.join(NOTE_EDIT_JOURNAL_FILE).exists());
    };
    assert_refused(
        replace_library_document(&fixture.request, ENTITY_ID, &source, &replacement)
            .map(|_| ())
            .map_err(|error| error.exit_code()),
    );
    assert_refused(
        update_entity(&fixture.request, ENTITY_ID, &source, &source, &oversized)
            .map(|_| ())
            .map_err(|error| error.exit_code()),
    );
    let record = fs::read_to_string(fixture.project.join("records/tasks/active.md")).unwrap();
    assert_refused(
        update_record(
            &fixture.request,
            "Projects/example/records/tasks/active.md",
            &record,
            &record,
            &oversized,
        )
        .map(|_| ())
        .map_err(|error| error.exit_code()),
    );
    let fields = BTreeMap::from([("body".into(), oversized.clone())]);
    for note_type in ["session", "handoff"] {
        assert_refused(
            create_event(&fixture.request, note_type, Path::new("limit.md"), &fields)
                .map(|_| ())
                .map_err(|error| error.exit_code()),
        );
        let form = prepare_event_creation(&fixture.request, note_type).unwrap();
        assert_refused(
            preview_event_creation(&form, Path::new("limit.md"), &fields)
                .map(|_| ())
                .map_err(|error| error.exit_code()),
        );
    }
    for note_type in ["task", "entity"] {
        let form = prepare_mutable_note_creation(&fixture.request, note_type).unwrap();
        assert_refused(
            create_mutable_note(
                &fixture.request,
                note_type,
                Path::new("limit.md"),
                &fields,
                &form.projection_source,
            )
            .map(|_| ())
            .map_err(|error| error.exit_code()),
        );
        assert_refused(
            preview_mutable_note_creation(
                &form,
                Path::new("limit.md"),
                &fields,
                &form.projection_source,
            )
            .map(|_| ())
            .map_err(|error| error.exit_code()),
        );
        let small = BTreeMap::from([("body".into(), "exact small body".into())]);
        assert_refused(
            create_mutable_note(
                &fixture.request,
                note_type,
                Path::new("limit.md"),
                &small,
                &oversized,
            )
            .map(|_| ())
            .map_err(|error| error.exit_code()),
        );
        assert_refused(
            preview_mutable_note_creation(&form, Path::new("limit.md"), &small, &oversized)
                .map(|_| ())
                .map_err(|error| error.exit_code()),
        );
    }
    let form = prepare_mutable_note_lifecycle(&fixture.request, ENTITY_ID).unwrap();
    for (note, projection) in [
        (&replacement, &form.projection_source),
        (&source, &oversized),
    ] {
        assert_refused(
            preview_mutable_note_lifecycle(&fixture.request, &form, note, projection)
                .map(|_| ())
                .map_err(|error| error.exit_code()),
        );
    }
}

#[test]
fn source_limits_preserve_oversized_historical_recovery_in_all_versions() {
    for schema in [1, 2, 3] {
        let fixture = Fixture::new();
        let mut images = fixture.images(schema);
        let suffix = vec![b'x'; MAX_MARKDOWN_SOURCE_BYTES + 1];
        if schema < 3 {
            for image in [&mut images.before, &mut images.after] {
                image
                    .get_mut(Path::new(ENTITY))
                    .unwrap()
                    .extend_from_slice(&suffix);
            }
        } else {
            images
                .after
                .get_mut(Path::new(images.notes[0]))
                .unwrap()
                .extend_from_slice(&suffix);
        }
        if schema >= 2 {
            for image in [&mut images.before, &mut images.after] {
                image
                    .get_mut(Path::new("index.md"))
                    .unwrap()
                    .extend_from_slice(&suffix);
            }
        }
        for image in [&mut images.before, &mut images.after] {
            fixture.restore(image);
            let config = load_root_config(fixture.request.root_override.as_ref().unwrap()).unwrap();
            let evidence = crate::evidence::collect_canonical_evidence::<ProjectValidationError>(
                &fixture.project,
                &config,
                |path| {
                    fs::read(path).map_err(|source| ProjectValidationError::FileSystem {
                        operation: "read historical fixture",
                        path: path.to_owned(),
                        source,
                    })
                },
            )
            .unwrap();
            let state = render_updated_project_state(
                str::from_utf8(&image[Path::new(PROJECT_STATE_FILE)]).unwrap(),
                &fixture.project,
                &image[Path::new("index.md")],
                &image[Path::new("roadmap.md")],
                &evidence,
                &BTreeSet::new(),
            )
            .unwrap();
            image.insert(PROJECT_STATE_FILE.into(), state);
        }
        let text = |image: &Snapshot, path: &str| {
            String::from_utf8(image[Path::new(path)].clone()).unwrap()
        };
        let mut journal: serde_json::Value = serde_json::from_slice(&images.journal).unwrap();
        journal["state_before"] = text(&images.before, PROJECT_STATE_FILE).into();
        journal["state_after"] = text(&images.after, PROJECT_STATE_FILE).into();
        if schema < 3 {
            journal["note_before"] = text(&images.before, ENTITY).into();
            journal["note_after"] = text(&images.after, ENTITY).into();
            if schema == 2 {
                journal["projection"]["before"] = text(&images.before, "index.md").into();
                journal["projection"]["after"] = text(&images.after, "index.md").into();
            }
        } else {
            for (index, path) in images.notes.iter().enumerate() {
                journal["notes"][index]["after"] = text(&images.after, path).into();
            }
            for (index, path) in images.projections.iter().enumerate() {
                journal["projections"][index]["before"] = text(&images.before, path).into();
                journal["projections"][index]["after"] = text(&images.after, path).into();
            }
        }
        images.journal = serde_json::to_vec(&journal).unwrap();
        drop(journal);
        for (mut image, outcome, expected) in [
            (
                images.before.clone(),
                NoteEditRecovery::Discarded,
                &images.before,
            ),
            (
                images.after.clone(),
                NoteEditRecovery::RolledBack,
                &images.before,
            ),
            (
                images.after.clone(),
                NoteEditRecovery::Finalized,
                &images.after,
            ),
        ] {
            if outcome == NoteEditRecovery::RolledBack {
                image.insert(
                    PROJECT_STATE_FILE.into(),
                    images.before[Path::new(PROJECT_STATE_FILE)].clone(),
                );
            }
            image.insert(NOTE_EDIT_JOURNAL_FILE.into(), images.journal.clone());
            fixture.restore(&image);
            let note = fixture.project.join(images.notes[0]);
            let exact = image.get(Path::new(images.notes[0]));
            fs::write(&note, b"foreign bytes").unwrap();
            let conflict = snapshot(&fixture.project);
            assert_eq!(
                recover_pending_note_edit(&fixture.request)
                    .unwrap_err()
                    .exit_code(),
                5
            );
            assert_eq!(snapshot(&fixture.project), conflict);
            match exact {
                Some(bytes) => fs::write(&note, bytes).unwrap(),
                None => fs::remove_file(&note).unwrap(),
            }
            assert_eq!(
                recover_pending_note_edit(&fixture.request).unwrap(),
                outcome
            );
            assert_eq!(&snapshot(&fixture.project), expected);
            assert_eq!(
                validate_project(&fixture.request).unwrap_err().exit_code(),
                4
            );
            assert_eq!(
                recover_pending_note_edit(&fixture.request).unwrap(),
                NoteEditRecovery::None
            );
            assert_eq!(&snapshot(&fixture.project), expected);
        }
    }
}
