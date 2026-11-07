use super::*;
use akasha_core::librarian::{check_librarian_answer, prepare_librarian};

#[test]
fn librarian_never_loads_foreign_bodies_and_preserves_sources() {
    let f = fixture("librarian-scope");
    append_link_source(
        &f.root,
        "Projects/alpha/entities/core.md",
        "\nEngine overheating. See [[Projects/zeta/entities/renderer]].\n",
    );
    fs::write(
        f.root.join("Projects/zeta/entities/renderer.md"),
        b"FOREIGN_SECRET invalid YAML",
    )
    .unwrap();
    fs::write(
        f.root.join("Global/entities/rust-pattern.md"),
        b"GLOBAL_SECRET invalid YAML",
    )
    .unwrap();
    refresh(&f);
    let before = link_fixture_snapshot(&f.root);
    let packet =
        prepare_librarian(&f.request, "What overheats?", &["overheating".into()], None).unwrap();
    assert_eq!(packet.scanned_notes, 2);
    assert_eq!(packet.evidence.len(), 1);
    assert!(
        !serde_json::to_string(&packet)
            .unwrap()
            .contains("FOREIGN_SECRET")
    );
    assert!(
        !serde_json::to_string(&packet)
            .unwrap()
            .contains("GLOBAL_SECRET")
    );
    assert!(packet.evidence[0].excerpt.contains("Engine overheating"));
    assert_eq!(before, link_fixture_snapshot(&f.root));
}

#[test]
fn librarian_anchor_expands_only_canonical_same_project_neighbors() {
    let f = fixture("librarian-neighbors");
    let id = "Projects/alpha/entities/core.md";
    append_link_source(
        &f.root,
        id,
        "\n[[Projects/alpha/records/tasks/ship-mvp]] [[Projects/zeta/entities/renderer]]\n",
    );
    refresh(&f);
    let packet = prepare_librarian(&f.request, "Explain", &[], Some(id)).unwrap();
    assert_eq!(packet.evidence.len(), 2);
    assert_eq!(packet.evidence[0].id, id);
    assert!(
        packet
            .evidence
            .iter()
            .all(|item| item.id.starts_with("Projects/alpha/"))
    );
    for bad in [
        "Projects/zeta/entities/renderer.md",
        "Projects/alpha/index.md",
        "../alpha/entities/core.md",
        "Projects/alpha/entities/./core.md",
    ] {
        assert!(prepare_librarian(&f.request, "Explain", &[], Some(bad)).is_err());
    }
}

#[test]
fn librarian_excerpts_preserve_unicode_line_ranges_and_bound_long_lines() {
    let f = fixture("librarian-unicode");
    let id = "Projects/alpha/entities/core.md";
    append_link_source(
        &f.root,
        id,
        &format!("\nNothing relevant\nПРИВЕТ {}\n", "世界".repeat(900)),
    );
    refresh(&f);
    let packet = prepare_librarian(&f.request, "Привет", &[], None).unwrap();
    let item = &packet.evidence[0];
    assert_eq!(item.excerpt.chars().count(), 1000);
    assert!(item.excerpt_truncated);
    let source = fs::read_to_string(f.root.join(id)).unwrap();
    assert!(
        source
            .lines()
            .nth(item.line_start - 1)
            .unwrap()
            .starts_with(&item.excerpt)
    );
    assert_eq!(item.line_start, item.line_end);
}

#[test]
fn librarian_rejects_unsafe_answers_and_observed_source_drift() {
    let f = fixture("librarian-check");
    let packet = prepare_librarian(&f.request, "core", &[], None).unwrap();
    let valid = r#"{"insufficient_evidence":false,"claims":[{"text":"The component is core.","citations":[1]}]}"#;
    assert!(check_librarian_answer(&f.request, &packet, valid).is_ok());
    for invalid in [
        valid.replace("[1]", "[0]"),
        valid.replace("[1]", "[2]"),
        valid.replace("[1]", "[1,1]"),
        valid.replace("[1]", "[]"),
        valid.replace("core.", "core.\\u001b[31m"),
        r#"{"insufficient_evidence":false,"claims":[]}"#.into(),
        r#"{"insufficient_evidence":true,"claims":[],"tools":[]}"#.into(),
    ] {
        assert!(
            check_librarian_answer(&f.request, &packet, &invalid).is_err(),
            "{invalid}"
        );
    }
    let mut forged = packet.clone();
    forged.evidence[0].excerpt.push_str("invented evidence");
    assert_eq!(
        check_librarian_answer(&f.request, &forged, valid)
            .unwrap_err()
            .exit_code(),
        5
    );
    append_link_source(
        &f.root,
        "Projects/alpha/entities/core.md",
        "\nChanged after generation\n",
    );
    refresh(&f);
    assert_eq!(
        check_librarian_answer(&f.request, &packet, valid)
            .unwrap_err()
            .exit_code(),
        5
    );
}

#[test]
fn librarian_empty_retrieval_abstains_and_input_bounds_fail_without_writes() {
    let f = fixture("librarian-empty");
    refresh(&f);
    let before = link_fixture_snapshot(&f.root);
    let packet = prepare_librarian(&f.request, "nonexistentneedle", &[], None).unwrap();
    assert!(packet.evidence.is_empty());
    let abstain = r#"{"insufficient_evidence":true,"claims":[]}"#;
    assert!(check_librarian_answer(&f.request, &packet, abstain).is_ok());
    for question in ["".into(), "x".repeat(513)] {
        assert!(prepare_librarian(&f.request, &question, &[], None).is_err());
    }
    assert!(prepare_librarian(&f.request, "core", &[" ".into()], None).is_err());
    assert!(prepare_librarian(&f.request, "core", &vec!["core".into(); 9], None).is_err());
    assert_eq!(before, link_fixture_snapshot(&f.root));
}

#[test]
fn librarian_uses_configured_folders_and_refuses_pending_journals() {
    let f = fixture("librarian-config");
    let config = fs::read_to_string(f.root.join("akasha.toml"))
        .unwrap()
        .replace("folder = \"entities\"", "folder = \"knowledge\"");
    fs::write(f.root.join("akasha.toml"), config).unwrap();
    let project = f.root.join("Projects/alpha");
    fs::rename(project.join("entities"), project.join("knowledge")).unwrap();
    fs::write(project.join("index.md"), "# Index\n").unwrap();
    support::write_project_state(
        &project,
        &[],
        &["knowledge/core.md"],
        &["records/tasks/ship-mvp.md"],
    );
    let packet = prepare_librarian(&f.request, "core", &[], None).unwrap();
    assert_eq!(packet.evidence[0].id, "Projects/alpha/knowledge/core.md");
    fs::write(project.join(akasha_core::NOTE_EDIT_JOURNAL_FILE), "{}").unwrap();
    let before = link_fixture_snapshot(&f.root);
    assert!(prepare_librarian(&f.request, "core", &[], None).is_err());
    assert_eq!(before, link_fixture_snapshot(&f.root));
}

#[cfg(unix)]
#[test]
fn librarian_rejects_symlinked_canonical_sources() {
    let f = fixture("librarian-symlink");
    let source = f.root.join("Projects/alpha/entities/core.md");
    fs::remove_file(&source).unwrap();
    std::os::unix::fs::symlink(f.root.join("Projects/zeta/entities/renderer.md"), source).unwrap();
    let before = link_fixture_snapshot(&f.root);
    assert!(prepare_librarian(&f.request, "core", &[], None).is_err());
    assert_eq!(before, link_fixture_snapshot(&f.root));
}

fn refresh(f: &Fixture) {
    support::write_project_state(
        &f.root.join("Projects/alpha"),
        &[],
        &["entities/core.md"],
        &["records/tasks/ship-mvp.md"],
    );
}
