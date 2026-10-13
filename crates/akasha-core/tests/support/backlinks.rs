use super::*;

const ID: &str = "Projects/alpha/entities/core.md";
const GLOBAL: &str = "Global/entities/rust-pattern.md";

fn source(fixture: &Fixture) -> String {
    fs::read_to_string(fixture.root.join(ID)).unwrap()
}

fn refresh_alpha(fixture: &Fixture) {
    support::write_project_state(
        &fixture.root.join("Projects/alpha"),
        &[],
        &["entities/core.md"],
        &["records/tasks/ship-mvp.md"],
    );
}

#[test]
fn backlinks_bind_unicode_crlf_and_preserve_complete_inspection_coverage() {
    let fixture = fixture("backlinks-coverage");
    let source = format!("{}🌘 Кириллица\n[[#Self]]\n", source(&fixture)).replace('\n', "\r\n");
    fs::write(fixture.root.join(ID), &source).unwrap();
    append_link_source(
        &fixture.root,
        "Projects/alpha/index.md",
        &format!("[[{ID}|index]]\n"),
    );
    append_link_source(
        &fixture.root,
        "Projects/alpha/roadmap.md",
        &format!("[[{ID}#Heading]]\n"),
    );
    refresh_alpha(&fixture);
    let zeta = "Projects/zeta/entities/renderer.md";
    append_link_source(&fixture.root, zeta, &format!("![[{ID}|same]] [[{ID}]]\n"));
    support::write_project_state(
        &fixture.root.join("Projects/zeta"),
        &[],
        &["entities/renderer.md"],
        &[],
    );
    let before = link_fixture_snapshot(&fixture.root);
    let result = inspect_library_backlinks(&fixture.request, ID, &source, 30).unwrap();
    assert_eq!(
        result,
        inspect_note_links(&fixture.request, ID, NoteLinkDirection::Incoming, 30).unwrap()
    );
    assert_eq!(result.total_matches, 5);
    assert_eq!(result.scanned_canonical_notes, 4);
    assert_eq!(result.scanned_projections, 4);
    for link in result.links {
        let target = activate_library_backlink(&fixture.request, ID, &source, &link.id, 30);
        if link.kind == NoteLinkKind::Canonical {
            let target = target.unwrap();
            assert_eq!(target.document.id, link.id);
            assert_eq!(
                target.document.source,
                fs::read_to_string(fixture.root.join(&link.id)).unwrap()
            );
        } else {
            assert_eq!(target.unwrap_err().exit_code(), 4);
        }
    }
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}

#[test]
fn backlinks_enforce_bounds_empty_results_and_current_bounded_membership() {
    let fixture = fixture("backlinks-bounds");
    let source = source(&fixture);
    let global = fs::read_to_string(fixture.root.join(GLOBAL)).unwrap();
    for index in 0..103 {
        let name = format!("source-{index:03}");
        fs::write(
            fixture.root.join(format!("Global/entities/{name}.md")),
            global.replace("entity: rust-pattern", &format!("entity: {name}")),
        )
        .unwrap();
    }
    let before = link_fixture_snapshot(&fixture.root);
    for limit in [1, 30, 100] {
        let result = inspect_library_backlinks(&fixture.request, ID, &source, limit).unwrap();
        assert_eq!(result.total_matches, 104);
        assert_eq!(result.links.len(), limit);
        assert!(result.truncated);
        assert!(result.links.windows(2).all(|pair| pair[0].id < pair[1].id));
    }
    assert!(
        activate_library_backlink(
            &fixture.request,
            ID,
            &source,
            "Global/entities/source-050.md",
            30
        )
        .is_err()
    );
    assert!(
        activate_library_backlink(
            &fixture.request,
            ID,
            &source,
            "Global/entities/source-050.md",
            100
        )
        .is_ok()
    );
    for limit in [0, 101] {
        assert_eq!(
            inspect_library_backlinks(&fixture.request, ID, &source, limit)
                .unwrap_err()
                .exit_code(),
            4
        );
        assert_eq!(
            activate_library_backlink(&fixture.request, ID, &source, GLOBAL, limit)
                .unwrap_err()
                .exit_code(),
            4
        );
    }
    let empty_id = "Projects/zeta/entities/renderer.md";
    let empty_source = fs::read_to_string(fixture.root.join(empty_id)).unwrap();
    let empty = inspect_library_backlinks(&fixture.request, empty_id, &empty_source, 30).unwrap();
    assert_eq!(empty.total_matches, 0);
    assert!(empty.links.is_empty());
    assert!(!empty.truncated);
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}

#[test]
fn backlinks_refuse_forged_nonbook_and_removed_edges_without_writes() {
    let fixture = fixture("backlinks-edge");
    let source = source(&fixture);
    let before = link_fixture_snapshot(&fixture.root);
    for source_id in [
        ID,
        "Projects/zeta/entities/renderer.md",
        "Projects/alpha/index.md",
        "Inbox/other.md",
        "../outside.md",
        "Global/entities/rust-pattern",
    ] {
        assert_eq!(
            activate_library_backlink(&fixture.request, ID, &source, source_id, 30)
                .unwrap_err()
                .exit_code(),
            4
        );
    }
    for id in [
        "Projects/alpha/index.md",
        "Projects/alpha/entities/missing.md",
        "../outside.md",
    ] {
        assert_eq!(
            inspect_library_backlinks(&fixture.request, id, &source, 30)
                .unwrap_err()
                .exit_code(),
            4
        );
    }
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
    // Valid global changes are not fingerprinted, so removed-edge refusal must be explicit.
    let global = fs::read_to_string(fixture.root.join(GLOBAL)).unwrap();
    fs::write(
        fixture.root.join(GLOBAL),
        global.replace("See [[Projects/alpha/entities/core]].", "No current link."),
    )
    .unwrap();
    let before = link_fixture_snapshot(&fixture.root);
    assert_eq!(
        inspect_library_backlinks(&fixture.request, ID, &source, 30)
            .unwrap()
            .total_matches,
        0
    );
    assert_eq!(
        activate_library_backlink(&fixture.request, ID, &source, GLOBAL, 30)
            .unwrap_err()
            .exit_code(),
        4
    );
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}

#[test]
fn backlinks_refuse_valid_baseline_drift_and_invalid_state_then_allow_fresh_retry() {
    let fixture = fixture("backlinks-stale");
    let original = source(&fixture);
    fs::write(
        fixture.root.join(ID),
        format!("{original}\nFresh saved source.\n"),
    )
    .unwrap();
    refresh_alpha(&fixture);
    // A valid core-state update must still refuse the previously loaded baseline.
    let before = link_fixture_snapshot(&fixture.root);
    for error in [
        inspect_library_backlinks(&fixture.request, ID, &original, 30).unwrap_err(),
        activate_library_backlink(&fixture.request, ID, &original, GLOBAL, 30).unwrap_err(),
    ] {
        assert_eq!(error.exit_code(), 5);
        assert!(error.to_string().contains("saved source"));
    }
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
    let fresh = source(&fixture);
    assert!(inspect_library_backlinks(&fixture.request, ID, &fresh, 30).is_ok());
    assert!(activate_library_backlink(&fixture.request, ID, &fresh, GLOBAL, 30).is_ok());
    append_link_source(&fixture.root, "Projects/alpha/index.md", "external drift\n");
    let before = link_fixture_snapshot(&fixture.root);
    assert_eq!(
        inspect_library_backlinks(&fixture.request, ID, &fresh, 30)
            .unwrap_err()
            .exit_code(),
        4
    );
    assert_eq!(
        activate_library_backlink(&fixture.request, ID, &fresh, GLOBAL, 30)
            .unwrap_err()
            .exit_code(),
        4
    );
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}

#[test]
fn backlinks_refuse_every_project_journal_without_recovery() {
    let fixture = fixture("backlinks-journals");
    let source = source(&fixture);
    for project in ["alpha", "zeta"] {
        let journal = fixture
            .root
            .join("Projects")
            .join(project)
            .join(akasha_core::NOTE_EDIT_JOURNAL_FILE);
        fs::write(&journal, b"unfinished; never parse or recover").unwrap();
        let before = link_fixture_snapshot(&fixture.root);
        assert_eq!(
            inspect_library_backlinks(&fixture.request, ID, &source, 30)
                .unwrap_err()
                .exit_code(),
            5
        );
        assert_eq!(
            activate_library_backlink(&fixture.request, ID, &source, GLOBAL, 30)
                .unwrap_err()
                .exit_code(),
            5
        );
        assert_eq!(link_fixture_snapshot(&fixture.root), before);
        fs::remove_file(journal).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn backlinks_refuse_symlink_sources_and_preserve_external_bytes() {
    use std::os::unix::fs::symlink;
    let fixture = fixture("backlinks-symlink");
    let source = source(&fixture);
    let external = fixture._temp.path().join("outside.md");
    let bytes = fs::read(fixture.root.join(GLOBAL)).unwrap();
    fs::write(&external, &bytes).unwrap();
    fs::remove_file(fixture.root.join(GLOBAL)).unwrap();
    symlink(&external, fixture.root.join(GLOBAL)).unwrap();
    let before = link_fixture_snapshot(&fixture.root);
    assert_eq!(
        inspect_library_backlinks(&fixture.request, ID, &source, 30)
            .unwrap_err()
            .exit_code(),
        4
    );
    assert_eq!(
        activate_library_backlink(&fixture.request, ID, &source, GLOBAL, 30)
            .unwrap_err()
            .exit_code(),
        4
    );
    assert_eq!(fs::read(external).unwrap(), bytes);
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}
