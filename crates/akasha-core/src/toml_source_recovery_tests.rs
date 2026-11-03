//! Ordinary TOML limits must never strand historical shared-journal state images.
use super::*;
use crate::toml_source::MAX_STATE_BYTES;

#[test]
fn toml_state_reads_accept_exact_bytes_and_reject_growth_before_decoding() {
    let fixture = Fixture::new();
    let path = fixture.project.join(PROJECT_STATE_FILE);
    let original = fs::read(&path).unwrap();
    let mut exact = original.clone();
    exact.extend_from_slice("\n#雪".as_bytes());
    exact.resize(MAX_STATE_BYTES, b'x');
    fs::write(&path, &exact).unwrap();
    validate_project(&fixture.request).unwrap();
    assert_eq!(
        read_regular_state(&path, "read exact state").unwrap(),
        exact
    );
    let permissions = fs::metadata(&path).unwrap().permissions();
    exact[0] = 0xff;
    exact.push(b'x');
    fs::write(&path, &exact).unwrap();
    let before = snapshot(&fixture.project);
    for error in [
        validate_project(&fixture.request).unwrap_err(),
        crate::build_library_projection(&fixture.request).unwrap_err(),
    ] {
        assert_eq!(error.exit_code(), 4);
        assert!(error.to_string().contains("16777216-byte"), "{error}");
    }
    let error = read_regular_state(&path, "read grown state").unwrap_err();
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("16777216-byte"));
    let source = fs::read_to_string(fixture.project.join(ENTITY)).unwrap();
    assert_eq!(
        replace_library_document(
            &fixture.request,
            ENTITY_ID,
            &source,
            &format!("{source}\nedit")
        )
        .unwrap_err()
        .exit_code(),
        4
    );
    assert_eq!(snapshot(&fixture.project), before);
    assert_eq!(fs::metadata(&path).unwrap().permissions(), permissions);
    fs::write(&path, b"\xff").unwrap();
    let error = validate_project(&fixture.request).unwrap_err();
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("not valid UTF-8"));
    fs::write(&path, original).unwrap();
    fs::remove_file(&path).unwrap();
    assert_eq!(
        crate::toml_source::read_state_source(&path, "read absent state")
            .unwrap_err()
            .exit_code(),
        6
    );
}

#[test]
fn toml_state_limits_preserve_oversized_historical_recovery_in_every_version() {
    for schema in [1, 2, 3] {
        let fixture = Fixture::new();
        let mut images = fixture.images(schema);
        for image in [&mut images.before, &mut images.after] {
            let state = image.get_mut(Path::new(PROJECT_STATE_FILE)).unwrap();
            state.extend_from_slice(b"\n# historical state comment\n#");
            state.resize(MAX_STATE_BYTES + 1, b'x');
        }
        let mut journal: serde_json::Value = serde_json::from_slice(&images.journal).unwrap();
        for (field, image) in [
            ("state_before", &images.before),
            ("state_after", &images.after),
        ] {
            journal[field] = String::from_utf8(image[Path::new(PROJECT_STATE_FILE)].clone())
                .unwrap()
                .into();
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
            let path = fixture.project.join(PROJECT_STATE_FILE);
            let permissions = fs::metadata(&path).unwrap().permissions();
            let mut foreign = fs::read(&path).unwrap();
            foreign.push(b'!');
            fs::write(&path, &foreign).unwrap();
            let conflict = snapshot(&fixture.project);
            assert_eq!(
                recover_pending_note_edit(&fixture.request)
                    .unwrap_err()
                    .exit_code(),
                5
            );
            assert_eq!(snapshot(&fixture.project), conflict);
            fs::write(&path, &image[Path::new(PROJECT_STATE_FILE)]).unwrap();
            assert_eq!(
                recover_pending_note_edit(&fixture.request).unwrap(),
                outcome
            );
            assert_eq!(&snapshot(&fixture.project), expected);
            assert_eq!(fs::metadata(&path).unwrap().permissions(), permissions);
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

#[test]
fn toml_configuration_limits_cover_init_snapshots_and_recovery_without_writes() {
    let fixture = Fixture::new();
    let images = fixture.images(1);
    let mut before = images.before.clone();
    before.insert(NOTE_EDIT_JOURNAL_FILE.into(), images.journal);
    fixture.restore(&before);
    let path = fixture.base.join("root/akasha.toml");
    let original = fs::read(&path).unwrap();
    let mut excessive = original.clone();
    excessive.extend_from_slice(b"\n#");
    excessive.resize(crate::toml_source::MAX_CONFIG_BYTES + 1, b'x');
    fs::write(&path, &excessive).unwrap();
    let init = crate::InitRequest {
        root_override: fixture.request.root_override.clone(),
        project: "new-project".into(),
        cwd: fixture.request.cwd.clone(),
        environment: fixture.request.environment.clone(),
    };
    assert_eq!(
        crate::prepare_project_init(&init).unwrap_err().exit_code(),
        3
    );
    assert_eq!(
        crate::prepare_init_recovery(&fixture.request)
            .unwrap_err()
            .exit_code(),
        3
    );
    assert_eq!(
        recover_pending_note_edit(&fixture.request)
            .unwrap_err()
            .exit_code(),
        3
    );
    assert_eq!(fs::read(&path).unwrap(), excessive);
    assert_eq!(snapshot(&fixture.project), before);
    fs::write(&path, original).unwrap();
    assert_eq!(
        recover_pending_note_edit(&fixture.request).unwrap(),
        NoteEditRecovery::Discarded
    );
    assert_eq!(snapshot(&fixture.project), images.before);
}
