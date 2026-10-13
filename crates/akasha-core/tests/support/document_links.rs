use super::*;

const ID: &str = "Projects/alpha/entities/core.md";

fn refresh(fixture: &Fixture) {
    support::write_project_state(
        &fixture.root.join("Projects/alpha"),
        &[],
        &["entities/core.md"],
        &["records/tasks/ship-mvp.md"],
    );
}

#[test]
fn binds_exact_utf16_occurrences_with_crlf_and_ignored_regions() {
    let fixture = fixture("document-link-ranges");
    fs::write(fixture.root.join("Inbox/other.md"), "# Other\n").unwrap();
    let source = fs::read_to_string(fixture.root.join(ID)).unwrap().replace(
        "schema_version: 1",
        "schema_version: 1\ndescription: '[[ignored-header]]'",
    );
    let body = "🌘 Кириллица\n\
        **[[Global/entities/rust-pattern|Same alias]]**\n\
        ![[Projects/zeta/entities/renderer.md#Details|Same alias]]\n\
        [[Global/entities/rust-pattern]] [[#Here]]\n\
        [[Projects/alpha/index]] [[Projects/alpha/roadmap.md]] [[Inbox/other]]\n\
        `[[ignored-inline]]`\n```md\n[[ignored-fence]]\n```\n\
        <!-- multiline\n[[ignored-html]]\n-->\n%% multiline\n[[ignored-percent]]\n%%\n\
        \\[[ignored-escape]]\n";
    let source = format!("{source}{body}").replace('\n', "\r\n");
    fs::write(fixture.root.join(ID), &source).unwrap();
    refresh(&fixture);
    let before = link_fixture_snapshot(&fixture.root);
    let loaded = load_linked_library_document(&fixture.request, ID).unwrap();
    assert_eq!(loaded.document.source, source);
    assert_eq!(loaded.links.len(), 7);
    let utf16: Vec<_> = source.encode_utf16().collect();
    let slice = |from, to| String::from_utf16(&utf16[from..to]).unwrap();
    assert_eq!(
        slice(loaded.links[0].label_from, loaded.links[0].label_to),
        "Same alias"
    );
    assert_eq!(
        slice(loaded.links[1].from, loaded.links[1].to),
        "![[Projects/zeta/entities/renderer.md#Details|Same alias]]"
    );
    assert_eq!(loaded.links[0].target_id, "Global/entities/rust-pattern.md");
    assert_eq!(loaded.links[2].target_id, loaded.links[0].target_id);
    assert_eq!(loaded.links[3].target_id, ID);
    assert_eq!(loaded.links[4].kind, NoteLinkKind::Projection);
    assert_eq!(loaded.links[5].kind, NoteLinkKind::Projection);
    assert_eq!(loaded.links[6].kind, NoteLinkKind::Other);
    for link in &loaded.links[..4] {
        let target =
            activate_library_document_link(&fixture.request, ID, &source, link.from).unwrap();
        assert_eq!(target.document.id, link.target_id);
        assert_eq!(
            target.document.source,
            fs::read_to_string(fixture.root.join(&link.target_id)).unwrap()
        );
    }
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
    // The old exact-source API and wire stay compatible.
    assert_eq!(
        serde_json::to_value(load_library_document(&fixture.request, ID).unwrap()).unwrap(),
        serde_json::json!({ "id": ID, "source": source })
    );
}

#[test]
fn refuses_non_books_unknown_ids_and_forged_occurrences_without_writes() {
    let fixture = fixture("document-link-boundary");
    fs::write(fixture.root.join("Inbox/other.md"), "# Other\n").unwrap();
    append_link_source(
        &fixture.root,
        ID,
        "[[Projects/alpha/index]] [[Inbox/other]]\n",
    );
    refresh(&fixture);
    let loaded = load_linked_library_document(&fixture.request, ID).unwrap();
    let before = link_fixture_snapshot(&fixture.root);
    for link in &loaded.links {
        assert_eq!(
            activate_library_document_link(
                &fixture.request,
                ID,
                &loaded.document.source,
                link.from
            )
            .unwrap_err()
            .exit_code(),
            4
        );
    }
    for id in [
        "core",
        "Projects/alpha/index.md",
        "Inbox/other.md",
        "../outside.md",
        "/etc/passwd",
    ] {
        assert_eq!(
            load_linked_library_document(&fixture.request, id)
                .unwrap_err()
                .exit_code(),
            4
        );
    }
    for from in [0, loaded.links[0].from + 1, usize::MAX] {
        assert_eq!(
            activate_library_document_link(&fixture.request, ID, &loaded.document.source, from)
                .unwrap_err()
                .exit_code(),
            4
        );
    }
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}

#[test]
fn refuses_valid_source_drift_and_stale_state_then_allows_fresh_retry() {
    let fixture = fixture("document-link-stale");
    append_link_source(
        &fixture.root,
        ID,
        "[[Global/entities/rust-pattern|Before]]\n",
    );
    refresh(&fixture);
    let loaded = load_linked_library_document(&fixture.request, ID).unwrap();
    let replacement = loaded.document.source.replace("|Before", "|After");
    fs::write(fixture.root.join(ID), &replacement).unwrap();
    let before = link_fixture_snapshot(&fixture.root);
    assert_eq!(
        activate_library_document_link(
            &fixture.request,
            ID,
            &loaded.document.source,
            loaded.links[0].from
        )
        .unwrap_err()
        .exit_code(),
        4
    );
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
    refresh(&fixture);
    let before = link_fixture_snapshot(&fixture.root);
    let error = activate_library_document_link(
        &fixture.request,
        ID,
        &loaded.document.source,
        loaded.links[0].from,
    )
    .unwrap_err();
    assert_eq!(error.exit_code(), 5);
    assert!(error.to_string().contains("changed; reload"));
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
    let fresh = load_linked_library_document(&fixture.request, ID).unwrap();
    activate_library_document_link(
        &fixture.request,
        ID,
        &fresh.document.source,
        fresh.links[0].from,
    )
    .unwrap();
}

#[test]
fn refuses_pending_journals_in_every_project_without_recovery() {
    let fixture = fixture("document-link-journals");
    append_link_source(&fixture.root, ID, "[[Global/entities/rust-pattern]]\n");
    refresh(&fixture);
    let loaded = load_linked_library_document(&fixture.request, ID).unwrap();
    for project in ["alpha", "zeta"] {
        let journal = fixture
            .root
            .join("Projects")
            .join(project)
            .join(".akasha-edit-journal.json");
        fs::write(&journal, "unfinished journal; never parse or recover").unwrap();
        let before = link_fixture_snapshot(&fixture.root);
        assert_eq!(
            load_linked_library_document(&fixture.request, ID)
                .unwrap_err()
                .exit_code(),
            5
        );
        assert_eq!(
            activate_library_document_link(
                &fixture.request,
                ID,
                &loaded.document.source,
                loaded.links[0].from
            )
            .unwrap_err()
            .exit_code(),
            5
        );
        assert_eq!(link_fixture_snapshot(&fixture.root), before);
        fs::remove_file(journal).unwrap();
    }
}

#[test]
fn revalidates_target_source_and_configured_projection_classification() {
    let fixture = fixture("document-link-custom");
    let config = fs::read_to_string(fixture.root.join("akasha.toml"))
        .unwrap()
        .replace("index = \"index.md\"", "index = \"summary.md\"")
        .replace("roadmap = \"roadmap.md\"", "roadmap = \"plan.md\"");
    fs::write(fixture.root.join("akasha.toml"), config).unwrap();
    for project in ["alpha", "zeta"] {
        for (old, new) in [("index.md", "summary.md"), ("roadmap.md", "plan.md")] {
            fs::rename(
                fixture.root.join("Projects").join(project).join(old),
                fixture.root.join("Projects").join(project).join(new),
            )
            .unwrap();
        }
        let state = fixture
            .root
            .join("Projects")
            .join(project)
            .join(".akasha-state.toml");
        let source = fs::read_to_string(&state)
            .unwrap()
            .replace("index.md", "summary.md")
            .replace("roadmap.md", "plan.md");
        fs::write(state, source).unwrap();
    }
    // Preserve the existing source fingerprints by adding links only to global maintained knowledge.
    let id = "Global/entities/rust-pattern.md";
    append_link_source(
        &fixture.root,
        id,
        "[[Projects/zeta/summary]] [[Projects/zeta/plan]]\n",
    );
    let loaded = load_linked_library_document(&fixture.request, id).unwrap();
    assert_eq!(loaded.links[1].kind, NoteLinkKind::Projection);
    assert_eq!(loaded.links[2].kind, NoteLinkKind::Projection);
    fs::remove_file(fixture.root.join(ID)).unwrap();
    let before = link_fixture_snapshot(&fixture.root);
    assert!(
        activate_library_document_link(
            &fixture.request,
            id,
            &loaded.document.source,
            loaded.links[0].from
        )
        .is_err()
    );
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}

#[cfg(unix)]
#[test]
fn refuses_replaced_symlink_targets_without_changes() {
    use std::os::unix::fs::symlink;
    let fixture = fixture("document-link-symlink");
    append_link_source(&fixture.root, ID, "[[Global/entities/rust-pattern]]\n");
    refresh(&fixture);
    let loaded = load_linked_library_document(&fixture.request, ID).unwrap();
    let target = fixture.root.join(&loaded.links[0].target_id);
    let external = fixture._temp.path().join("outside.md");
    fs::write(&external, fs::read(&target).unwrap()).unwrap();
    fs::remove_file(&target).unwrap();
    symlink(&external, &target).unwrap();
    let before = link_fixture_snapshot(&fixture.root);
    assert_eq!(
        activate_library_document_link(
            &fixture.request,
            ID,
            &loaded.document.source,
            loaded.links[0].from
        )
        .unwrap_err()
        .exit_code(),
        4
    );
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}
