use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    MutableNoteLifecycleResult, NOTE_EDIT_JOURNAL_FILE, NoteClass, NoteEditRecovery,
    ResolutionEnvironment, ResolveRequest, apply_mutable_note_lifecycle,
    apply_mutable_note_lifecycle_preview, prepare_mutable_note_lifecycle,
    preview_mutable_note_lifecycle, recover_pending_note_edit, update_entity, update_record,
    validate_project,
};
use serde_json::json;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
const ENTITY_ID: &str = "Projects/example/entities/core.md";
const RECORD_ID: &str = "Projects/example/records/tasks/active.md";
const EVENT_ID: &str = "Projects/example/events/sessions/2026-07-13.md";
const INDEX_ID: &str = "Projects/example/index.md";

#[test]
fn prepares_all_mutable_classes_with_exact_sources_and_refuses_other_identities() {
    let fixture = Fixture::new("lifecycle-prepare");
    let before = lifecycle_snapshot(&fixture.root);
    for (id, class, projection) in [
        (ENTITY_ID, NoteClass::Entity, "index.md"),
        (RECORD_ID, NoteClass::Record, "roadmap.md"),
        (
            "Projects/example/records/problems/open.md",
            NoteClass::Record,
            "roadmap.md",
        ),
    ] {
        let form = prepare_mutable_note_lifecycle(&fixture.request, id).unwrap();
        assert_eq!(form.id, id);
        assert_eq!(form.class, class);
        assert_eq!(form.path, fixture.root.join(id));
        assert_eq!(form.source.as_bytes(), fs::read(&form.path).unwrap());
        assert_eq!(form.projection, fixture.project.join(projection));
        assert_eq!(
            form.projection_source.as_bytes(),
            fs::read(&form.projection).unwrap()
        );
    }
    for id in [
        EVENT_ID,
        "Global/entities/rust-pattern.md",
        "Projects/other/entities/core.md",
        "../outside.md",
    ] {
        assert_eq!(
            prepare_mutable_note_lifecycle(&fixture.request, id)
                .unwrap_err()
                .exit_code(),
            4
        );
    }
    assert_eq!(lifecycle_snapshot(&fixture.root), before);
}

#[test]
fn lifecycle_uses_configured_type_and_projection_paths_and_refuses_configuration_drift() {
    let fixture = Fixture::new("lifecycle-config");
    let old = prepare_mutable_note_lifecycle(&fixture.request, ENTITY_ID).unwrap();
    let config = fixture.root.join("akasha.toml");
    let source = fs::read_to_string(&config)
        .unwrap()
        .replace("index = \"index.md\"", "index = \"memory-map.md\"")
        .replace("roadmap = \"roadmap.md\"", "roadmap = \"plan.md\"")
        .replace(
            "[project.note_types.entity]",
            "[project.note_types.component]",
        );
    fs::write(config, source).unwrap();
    let global = fixture.root.join("Global/entities/rust-pattern.md");
    fs::write(
        &global,
        fs::read_to_string(&global)
            .unwrap()
            .replace("type: entity", "type: component"),
    )
    .unwrap();
    fs::rename(fixture.index_path(), fixture.project.join("memory-map.md")).unwrap();
    fs::rename(
        fixture.project.join("roadmap.md"),
        fixture.project.join("plan.md"),
    )
    .unwrap();
    validate_project(&fixture.request).unwrap();
    let before = lifecycle_snapshot(&fixture.root);
    assert_eq!(
        apply_mutable_note_lifecycle(&fixture.request, &old, &old.source, &old.projection_source)
            .unwrap_err()
            .exit_code(),
        4
    );
    let entity = prepare_mutable_note_lifecycle(&fixture.request, ENTITY_ID).unwrap();
    assert_eq!(entity.note_type, "component");
    assert_eq!(entity.projection, fixture.project.join("memory-map.md"));
    let record = prepare_mutable_note_lifecycle(&fixture.request, RECORD_ID).unwrap();
    assert_eq!(record.projection, fixture.project.join("plan.md"));
    apply_mutable_note_lifecycle(
        &fixture.request,
        &entity,
        &entity.source,
        &entity.projection_source,
    )
    .unwrap();
    assert_eq!(lifecycle_snapshot(&fixture.root), before);
}

#[test]
fn lifecycle_applies_each_class_exactly_and_reprepared_noop_preserves_bytes() {
    for id in [
        ENTITY_ID,
        RECORD_ID,
        "Projects/example/records/problems/open.md",
    ] {
        let fixture = Fixture::new("lifecycle-apply");
        let form = prepare_mutable_note_lifecycle(&fixture.request, id).unwrap();
        let note = format!(
            "{}\r\nCurrent truth: Привет 世界  ",
            form.source.replace('\n', "\r\n")
        );
        let projection = format!(
            "{}\r\nReviewed current truth  ",
            form.projection_source.replace('\n', "\r\n")
        );
        let result =
            apply_mutable_note_lifecycle(&fixture.request, &form, &note, &projection).unwrap();
        match result {
            MutableNoteLifecycleResult::Entity(result) => {
                assert_eq!(form.class, NoteClass::Entity);
                assert!(result.changed && result.index_changed);
            }
            MutableNoteLifecycleResult::Record(result) => {
                assert_eq!(form.class, NoteClass::Record);
                assert!(result.changed && result.roadmap_changed);
            }
        }
        assert_eq!(fs::read(&form.path).unwrap(), note.as_bytes());
        assert_eq!(fs::read(&form.projection).unwrap(), projection.as_bytes());
        assert!(!fixture.journal().exists());
        validate_project(&fixture.request).unwrap();
        let before = lifecycle_snapshot(&fixture.root);
        let fresh = prepare_mutable_note_lifecycle(&fixture.request, id).unwrap();
        let result =
            apply_mutable_note_lifecycle(&fixture.request, &fresh, &note, &projection).unwrap();
        match result {
            MutableNoteLifecycleResult::Entity(result) => {
                assert!(!result.changed && !result.index_changed)
            }
            MutableNoteLifecycleResult::Record(result) => {
                assert!(!result.changed && !result.roadmap_changed)
            }
        }
        assert_eq!(lifecycle_snapshot(&fixture.root), before);
    }
}

#[test]
fn lifecycle_refuses_valid_concurrent_note_or_projection_changes_before_any_write() {
    for id in [ENTITY_ID, RECORD_ID] {
        for change_note in [false, true] {
            let fixture = Fixture::new("lifecycle-conflict");
            let form = prepare_mutable_note_lifecycle(&fixture.request, id).unwrap();
            let external_note = if change_note {
                format!("{}\nExternal current truth.\n", form.source)
            } else {
                form.source.clone()
            };
            let external_projection = if change_note {
                form.projection_source.clone()
            } else {
                format!("{}\nExternal projection.\n", form.projection_source)
            };
            if form.class == NoteClass::Entity {
                update_entity(
                    &fixture.request,
                    id,
                    &form.source,
                    &external_note,
                    &external_projection,
                )
                .unwrap();
            } else {
                update_record(
                    &fixture.request,
                    id,
                    &form.source,
                    &external_note,
                    &external_projection,
                )
                .unwrap();
            }
            validate_project(&fixture.request).unwrap();
            let before = lifecycle_snapshot(&fixture.root);
            for _ in 0..2 {
                let error = apply_mutable_note_lifecycle(
                    &fixture.request,
                    &form,
                    &format!("{}\nLocal draft.\n", form.source),
                    &format!("{}\nLocal projection draft.\n", form.projection_source),
                )
                .unwrap_err();
                assert_eq!(error.exit_code(), 5);
                assert_eq!(lifecycle_snapshot(&fixture.root), before);
            }
        }
    }
}

#[test]
fn lifecycle_revalidates_every_prepared_identity_under_the_write_lock() {
    let fixture = Fixture::new("lifecycle-identities");
    let form = prepare_mutable_note_lifecycle(&fixture.request, ENTITY_ID).unwrap();
    let before = lifecycle_snapshot(&fixture.root);
    for field in [
        "root",
        "project",
        "note_type",
        "class",
        "path",
        "projection",
    ] {
        let mut invalid = form.clone();
        match field {
            "root" => invalid.root = fixture.root.join("different"),
            "project" => invalid.project = "other".into(),
            "note_type" => invalid.note_type = "other".into(),
            "class" => invalid.class = NoteClass::Record,
            "path" => invalid.path = fixture.root.join("different.md"),
            "projection" => invalid.projection = fixture.project.join("different.md"),
            _ => unreachable!(),
        }
        assert_eq!(
            apply_mutable_note_lifecycle(
                &fixture.request,
                &invalid,
                &form.source,
                &form.projection_source
            )
            .unwrap_err()
            .exit_code(),
            4,
            "{field}"
        );
        assert_eq!(lifecycle_snapshot(&fixture.root), before);
    }
}

#[test]
fn lifecycle_preparation_recovers_partial_publication_and_preserves_conflicting_journal_bytes() {
    let fixture = Fixture::new("lifecycle-recovery");
    let versions = fixture.successful_versions();
    fixture.restore_before(&versions);
    fixture.write_journal(&versions);
    fs::write(fixture.entity_path(), &versions.note_after).unwrap();
    let form = prepare_mutable_note_lifecycle(&fixture.request, ENTITY_ID).unwrap();
    assert_eq!(form.source, versions.note_before);
    assert_eq!(form.projection_source, versions.index_before);
    fixture.assert_before(&versions);
    validate_project(&fixture.request).unwrap();

    fixture.write_journal(&versions);
    fs::write(fixture.entity_path(), b"external editor bytes\r\n").unwrap();
    let before = lifecycle_snapshot(&fixture.root);
    assert_eq!(
        prepare_mutable_note_lifecycle(&fixture.request, ENTITY_ID)
            .unwrap_err()
            .exit_code(),
        5
    );
    assert_eq!(lifecycle_snapshot(&fixture.root), before);
}

#[test]
fn lifecycle_preview_is_read_only_and_applies_exact_record_problem_entity_pairs() {
    for id in [
        ENTITY_ID,
        RECORD_ID,
        "Projects/example/records/problems/open.md",
    ] {
        for separator in ["\n", "\r\n"] {
            let fixture = Fixture::new("lifecycle-preview-exact");
            let form = prepare_mutable_note_lifecycle(&fixture.request, id).unwrap();
            let note = format!(
                "{}{separator}# Reviewed 世界  {separator}{separator}Literal {{{{body}}}}  ",
                form.source.replace('\n', separator)
            );
            let projection = format!(
                "{}{separator}Caller-authored projection Привет  ",
                form.projection_source.replace('\n', separator)
            );
            let before = lifecycle_snapshot(&fixture.root);
            let preview =
                preview_mutable_note_lifecycle(&fixture.request, &form, &note, &projection)
                    .unwrap();
            assert_eq!(preview.prepared, form);
            assert_eq!(preview.replacement_source, note);
            assert_eq!(preview.projection_source, projection);
            assert_eq!(lifecycle_snapshot(&fixture.root), before);
            apply_mutable_note_lifecycle_preview(&fixture.request, &preview).unwrap();
            assert_eq!(fs::read(&form.path).unwrap(), note.as_bytes());
            assert_eq!(fs::read(&form.projection).unwrap(), projection.as_bytes());
            validate_project(&fixture.request).unwrap();
            let fresh = prepare_mutable_note_lifecycle(&fixture.request, id).unwrap();
            let before = lifecycle_snapshot(&fixture.root);
            let noop = preview_mutable_note_lifecycle(&fixture.request, &fresh, &note, &projection)
                .unwrap();
            apply_mutable_note_lifecycle_preview(&fixture.request, &noop).unwrap();
            assert_eq!(lifecycle_snapshot(&fixture.root), before);
        }
    }
}

#[test]
fn lifecycle_preview_refuses_invalid_metadata_links_and_prepared_identities_without_writes() {
    for id in [ENTITY_ID, RECORD_ID] {
        let fixture = Fixture::new("lifecycle-preview-validation");
        let form = prepare_mutable_note_lifecycle(&fixture.request, id).unwrap();
        let before = lifecycle_snapshot(&fixture.root);
        let changed_identity = if id == ENTITY_ID {
            form.source.replace("entity: core", "entity: renamed")
        } else {
            form.source
                .replace("created: 2026-07-13", "created: 2026-10-03")
        };
        for note in [
            "invalid leading frontmatter".to_owned(),
            changed_identity,
            format!("{}\n[[Projects/example/entities/missing]]", form.source),
        ] {
            assert_eq!(
                preview_mutable_note_lifecycle(
                    &fixture.request,
                    &form,
                    &note,
                    &form.projection_source
                )
                .unwrap_err()
                .exit_code(),
                4
            );
            assert_eq!(lifecycle_snapshot(&fixture.root), before);
        }
        for field in [
            "root",
            "project",
            "id",
            "type",
            "class",
            "path",
            "projection",
        ] {
            let mut invalid = form.clone();
            match field {
                "root" => invalid.root = fixture.root.join("other"),
                "project" => invalid.project = "other".into(),
                "id" => invalid.id = EVENT_ID.into(),
                "type" => invalid.note_type = "other".into(),
                "class" => invalid.class = NoteClass::Event,
                "path" => invalid.path = fixture.root.join("outside.md"),
                "projection" => invalid.projection = fixture.root.join("outside.md"),
                _ => unreachable!(),
            }
            assert_eq!(
                preview_mutable_note_lifecycle(
                    &fixture.request,
                    &invalid,
                    &form.source,
                    &form.projection_source
                )
                .unwrap_err()
                .exit_code(),
                4,
                "{field}"
            );
            assert_eq!(lifecycle_snapshot(&fixture.root), before);
        }
        let reviewed = preview_mutable_note_lifecycle(
            &fixture.request,
            &form,
            &form.source,
            &form.projection_source,
        )
        .unwrap();
        let config_path = fixture.root.join("akasha.toml");
        let config = fs::read_to_string(&config_path).unwrap();
        fs::write(
            &config_path,
            config
                .replace("index = \"index.md\"", "index = \"memory-map.md\"")
                .replace("roadmap = \"roadmap.md\"", "roadmap = \"plan.md\""),
        )
        .unwrap();
        fs::rename(fixture.index_path(), fixture.project.join("memory-map.md")).unwrap();
        fs::rename(
            fixture.project.join("roadmap.md"),
            fixture.project.join("plan.md"),
        )
        .unwrap();
        let before = lifecycle_snapshot(&fixture.root);
        assert_eq!(
            preview_mutable_note_lifecycle(
                &fixture.request,
                &form,
                &form.source,
                &form.projection_source
            )
            .unwrap_err()
            .exit_code(),
            4
        );
        assert_eq!(lifecycle_snapshot(&fixture.root), before);
        assert_eq!(
            apply_mutable_note_lifecycle_preview(&fixture.request, &reviewed)
                .unwrap_err()
                .exit_code(),
            4
        );
        assert_eq!(lifecycle_snapshot(&fixture.root), before);
    }
}

#[test]
fn lifecycle_review_rechecks_both_baselines_after_valid_concurrent_writes_and_retries_fresh() {
    for id in [ENTITY_ID, RECORD_ID] {
        for change_note in [false, true] {
            let fixture = Fixture::new("lifecycle-review-conflict");
            let form = prepare_mutable_note_lifecycle(&fixture.request, id).unwrap();
            let preview = preview_mutable_note_lifecycle(
                &fixture.request,
                &form,
                &format!("{}\nLocal note.", form.source),
                &format!("{}\nLocal projection.", form.projection_source),
            )
            .unwrap();
            let note = if change_note {
                format!("{}\nConcurrent note.", form.source)
            } else {
                form.source.clone()
            };
            let projection = if change_note {
                form.projection_source.clone()
            } else {
                format!("{}\nConcurrent projection.", form.projection_source)
            };
            apply_mutable_note_lifecycle(&fixture.request, &form, &note, &projection).unwrap();
            let before = lifecycle_snapshot(&fixture.root);
            for _ in 0..2 {
                assert_eq!(
                    preview_mutable_note_lifecycle(
                        &fixture.request,
                        &form,
                        &preview.replacement_source,
                        &preview.projection_source
                    )
                    .unwrap_err()
                    .exit_code(),
                    5
                );
                assert_eq!(
                    apply_mutable_note_lifecycle_preview(&fixture.request, &preview)
                        .unwrap_err()
                        .exit_code(),
                    5
                );
                assert_eq!(lifecycle_snapshot(&fixture.root), before);
            }
            let fresh = prepare_mutable_note_lifecycle(&fixture.request, id).unwrap();
            let preview = preview_mutable_note_lifecycle(
                &fixture.request,
                &fresh,
                &format!("{}\nFresh local update.", fresh.source),
                &format!("{}\nFresh projection.", fresh.projection_source),
            )
            .unwrap();
            apply_mutable_note_lifecycle_preview(&fixture.request, &preview).unwrap();
            validate_project(&fixture.request).unwrap();
        }
    }
}

#[test]
fn lifecycle_preview_never_recovers_and_reviewed_apply_retains_existing_recovery_policy() {
    let fixture = Fixture::new("lifecycle-preview-recovery");
    let versions = fixture.successful_versions();
    fixture.restore_before(&versions);
    let form = prepare_mutable_note_lifecycle(&fixture.request, ENTITY_ID).unwrap();
    let preview = preview_mutable_note_lifecycle(
        &fixture.request,
        &form,
        &versions.note_after,
        &versions.index_after,
    )
    .unwrap();
    fixture.write_journal(&versions);
    fs::write(fixture.entity_path(), &versions.note_after).unwrap();
    let before = lifecycle_snapshot(&fixture.root);
    assert_eq!(
        preview_mutable_note_lifecycle(
            &fixture.request,
            &form,
            &versions.note_after,
            &versions.index_after
        )
        .unwrap_err()
        .exit_code(),
        5
    );
    assert_eq!(lifecycle_snapshot(&fixture.root), before);
    let result = apply_mutable_note_lifecycle_preview(&fixture.request, &preview).unwrap();
    let MutableNoteLifecycleResult::Entity(result) = result else {
        unreachable!()
    };
    assert_eq!(result.recovery, NoteEditRecovery::RolledBack);
    assert_eq!(fixture.entity(), versions.note_after);
    assert_eq!(fixture.index(), versions.index_after);
    validate_project(&fixture.request).unwrap();
    fixture.restore_before(&versions);
    fixture.write_journal(&versions);
    fs::write(
        fixture.entity_path(),
        b"unrecognized external editor bytes\r\n",
    )
    .unwrap();
    let before = lifecycle_snapshot(&fixture.root);
    for _ in 0..2 {
        assert_eq!(
            apply_mutable_note_lifecycle_preview(&fixture.request, &preview)
                .unwrap_err()
                .exit_code(),
            5
        );
        assert_eq!(lifecycle_snapshot(&fixture.root), before);
    }
}

fn lifecycle_snapshot(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, result: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, result);
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = std::collections::BTreeMap::new();
    visit(root, root, &mut result);
    result
}

#[test]
fn updates_entity_and_explicit_index_with_valid_state() {
    let fixture = Fixture::new("update");
    let before = fixture.entity();
    let replacement = replacement_entity();
    let index = replacement_index();

    let result = update_entity(&fixture.request, ENTITY_ID, &before, replacement, index)
        .expect("update maintained entity");

    assert!(result.changed);
    assert!(result.index_changed);
    assert_eq!(result.note_type, "entity");
    assert_eq!(result.id, ENTITY_ID);
    assert_eq!(result.recovery, NoteEditRecovery::None);
    assert_eq!(fixture.entity(), replacement);
    assert_eq!(fixture.index(), index);
    validate_project(&fixture.request).expect("updated project remains valid");
    assert!(!fixture.journal().exists());

    let no_op = update_entity(&fixture.request, ENTITY_ID, replacement, replacement, index)
        .expect("exact entity rerun is a no-op");
    assert!(!no_op.changed);
    assert!(!no_op.index_changed);
}

#[test]
fn rejects_stale_non_entity_and_canonical_name_changes_without_writes() {
    let fixture = Fixture::new("rejections");
    let entity_before = fixture.entity();
    let index_before = fixture.index();
    let state_before = fixture.state();

    let stale = update_entity(
        &fixture.request,
        ENTITY_ID,
        "stale source",
        replacement_entity(),
        replacement_index(),
    )
    .expect_err("stale expected bytes must conflict");
    assert_eq!(stale.exit_code(), 5);

    for id in [RECORD_ID, EVENT_ID] {
        let source = fs::read_to_string(fixture.root.join(id)).expect("read non-entity note");
        let error = update_entity(&fixture.request, id, &source, &source, replacement_index())
            .expect_err("only entities can use maintained entity update");
        assert_eq!(error.exit_code(), 4);
    }

    let renamed = replacement_entity().replace("entity: core", "entity: renamed-core");
    let error = update_entity(
        &fixture.request,
        ENTITY_ID,
        &entity_before,
        &renamed,
        replacement_index(),
    )
    .expect_err("canonical entity name must be immutable in an update");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("rename"));

    assert_eq!(fixture.entity(), entity_before);
    assert_eq!(fixture.index(), index_before);
    assert_eq!(fixture.state(), state_before);
    assert!(!fixture.journal().exists());
}

#[test]
fn recovers_entity_and_index_publication_in_reverse_order() {
    let fixture = Fixture::new("recovery");
    let versions = fixture.successful_versions();

    fixture.restore_before(&versions);
    fixture.write_journal(&versions);
    let discarded = recover_pending_note_edit(&fixture.request).expect("discard unused journal");
    assert_eq!(discarded, NoteEditRecovery::Discarded);

    fs::write(fixture.entity_path(), &versions.note_after).expect("seed published entity");
    fixture.write_journal(&versions);
    let note_rollback =
        recover_pending_note_edit(&fixture.request).expect("roll back entity-only publication");
    assert_eq!(note_rollback, NoteEditRecovery::RolledBack);
    fixture.assert_before(&versions);

    fs::write(fixture.entity_path(), &versions.note_after).expect("seed published entity");
    fs::write(fixture.index_path(), &versions.index_after).expect("seed published index");
    fixture.write_journal(&versions);
    let projection_rollback = recover_pending_note_edit(&fixture.request)
        .expect("roll back entity and index publication");
    assert_eq!(projection_rollback, NoteEditRecovery::RolledBack);
    fixture.assert_before(&versions);
    validate_project(&fixture.request).expect("rolled-back project validates");
}

#[test]
fn finalizes_complete_entity_update_and_refuses_unexpected_projection_bytes() {
    let fixture = Fixture::new("finalize");
    let versions = fixture.successful_versions();
    fixture.write_journal(&versions);

    let finalized = recover_pending_note_edit(&fixture.request).expect("finalize complete update");
    assert_eq!(finalized, NoteEditRecovery::Finalized);
    assert!(!fixture.journal().exists());
    validate_project(&fixture.request).expect("finalized project validates");

    fixture.restore_before(&versions);
    fixture.write_journal(&versions);
    fs::write(fixture.index_path(), "external index bytes\n").expect("seed unexpected index");
    let error = recover_pending_note_edit(&fixture.request)
        .expect_err("unexpected index bytes must refuse recovery");
    assert_eq!(error.exit_code(), 5);
    assert_eq!(fixture.index(), "external index bytes\n");
    assert!(fixture.journal().is_file());
}

#[test]
fn operator_reconciles_projection_or_state_conflict_from_backed_up_preimage() {
    for field in ["projection", "state"] {
        let fixture = Fixture::new(field);
        let versions = fixture.successful_versions();
        fs::write(fixture.state_path(), &versions.state_before).unwrap();
        fixture.write_journal(&versions);
        let target = if field == "projection" {
            fixture.index_path()
        } else {
            fixture.state_path()
        };
        fs::write(&target, b"external bytes\r\n").unwrap();
        let backup = fixture._temp.path().join("backup");
        copy_tree(&fixture.root, &backup);
        let journal = fs::read(fixture.journal()).unwrap();
        for _ in 0..2 {
            assert_eq!(
                recover_pending_note_edit(&fixture.request)
                    .unwrap_err()
                    .exit_code(),
                5
            );
            assert_eq!(fs::read(&target).unwrap(), b"external bytes\r\n");
            assert_eq!(fixture.entity(), versions.note_after);
            assert_eq!(fs::read(fixture.journal()).unwrap(), journal);
        }
        let saved: serde_json::Value = serde_json::from_slice(&journal).unwrap();
        let preimage = if field == "projection" {
            &saved["projection"]["before"]
        } else {
            &saved["state_before"]
        };
        fs::write(&target, preimage.as_str().unwrap()).unwrap();
        assert_eq!(
            recover_pending_note_edit(&fixture.request).unwrap(),
            NoteEditRecovery::RolledBack
        );
        fixture.assert_before(&versions);
        validate_project(&fixture.request).unwrap();
        assert_eq!(
            recover_pending_note_edit(&fixture.request).unwrap(),
            NoteEditRecovery::None
        );
        assert_eq!(
            fs::read(backup.join(target.strip_prefix(&fixture.root).unwrap())).unwrap(),
            b"external bytes\r\n"
        );
        assert_eq!(
            fs::read(backup.join("Projects/example").join(NOTE_EDIT_JOURNAL_FILE)).unwrap(),
            journal
        );
    }
}

struct Versions {
    note_before: String,
    note_after: String,
    index_before: String,
    index_after: String,
    state_before: String,
    state_after: String,
}

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    project: PathBuf,
    request: ResolveRequest,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let temp = TempDir::new(label);
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/resolution/valid-root");
        let root = temp.path().join("valid-root");
        copy_tree(&source, &root);
        let repository = temp.path().join("repository");
        fs::create_dir_all(&repository).expect("create registered repository");
        let project = root.join("Projects/example");
        let request = ResolveRequest {
            root_override: Some(root.clone()),
            project_override: Some("example".to_owned()),
            cwd: repository,
            environment: ResolutionEnvironment::default(),
        };
        Self {
            _temp: temp,
            root,
            project,
            request,
        }
    }

    fn entity_path(&self) -> PathBuf {
        self.root.join(ENTITY_ID)
    }

    fn index_path(&self) -> PathBuf {
        self.root.join(INDEX_ID)
    }

    fn state_path(&self) -> PathBuf {
        self.project.join(".akasha-state.toml")
    }

    fn journal(&self) -> PathBuf {
        self.project.join(NOTE_EDIT_JOURNAL_FILE)
    }

    fn entity(&self) -> String {
        fs::read_to_string(self.entity_path()).expect("read entity")
    }

    fn index(&self) -> String {
        fs::read_to_string(self.index_path()).expect("read index")
    }

    fn state(&self) -> String {
        fs::read_to_string(self.state_path()).expect("read state")
    }

    fn successful_versions(&self) -> Versions {
        let note_before = self.entity();
        let index_before = self.index();
        let state_before = self.state();
        update_entity(
            &self.request,
            ENTITY_ID,
            &note_before,
            replacement_entity(),
            replacement_index(),
        )
        .expect("create successful entity update versions");
        Versions {
            note_before,
            note_after: self.entity(),
            index_before,
            index_after: self.index(),
            state_before,
            state_after: self.state(),
        }
    }

    fn restore_before(&self, versions: &Versions) {
        fs::write(self.entity_path(), &versions.note_before).expect("restore entity");
        fs::write(self.index_path(), &versions.index_before).expect("restore index");
        fs::write(self.state_path(), &versions.state_before).expect("restore state");
    }

    fn assert_before(&self, versions: &Versions) {
        assert_eq!(self.entity(), versions.note_before);
        assert_eq!(self.index(), versions.index_before);
        assert_eq!(self.state(), versions.state_before);
        assert!(!self.journal().exists());
    }

    fn write_journal(&self, versions: &Versions) {
        let source = serde_json::to_string_pretty(&json!({
            "schema_version": 2,
            "project": "example",
            "id": ENTITY_ID,
            "note_before": versions.note_before,
            "note_after": versions.note_after,
            "projection": {
                "id": INDEX_ID,
                "before": versions.index_before,
                "after": versions.index_after,
            },
            "state_before": versions.state_before,
            "state_after": versions.state_after,
        }))
        .expect("serialize entity update journal");
        fs::write(self.journal(), format!("{source}\n")).expect("write entity update journal");
    }
}

fn replacement_entity() -> &'static str {
    "---\nschema_version: 1\nentity: core\nkind: service\nstatus: deprecated\nreviewed: 2026-07-16\n---\n\n# Synthetic entity\n\nCurrent understanding updated.\n"
}

fn replacement_index() -> &'static str {
    "# Example project\n\nSynthetic resolution fixture for Akasha's core tests.\n\n- [[Projects/example/entities/core|Core]] — deprecated service\n"
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create copied fixture directory");
    let mut entries = fs::read_dir(source)
        .expect("read fixture directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("read fixture entries");
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("read fixture entry type").is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(path, target).expect("copy fixture file");
        }
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "akasha-core-entity-update-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temporary directory");
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
