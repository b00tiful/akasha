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

fn source_selection(citation: usize, quote: Option<&str>) -> String {
    let item = match quote {
        Some(quote) => serde_json::json!({"citation": citation, "quote": quote}),
        None => serde_json::json!({"citation": citation}),
    };
    serde_json::json!({"selections": [item]}).to_string()
}

#[test]
fn source_passages_are_exact_unicode_text_with_derived_status_and_lines() {
    use akasha_core::librarian::check_librarian_sources;
    let f = fixture("source-passages-unicode");
    let id = "Projects/alpha/entities/core.md";
    append_link_source(&f.root, id, "\nСбой 🌍\nsecond line\n\n");
    refresh(&f);
    let packet = prepare_librarian(&f.request, "Сбой", &[], None).unwrap();
    let before = link_fixture_snapshot(&f.root);
    let checked = check_librarian_sources(
        &f.request,
        &packet,
        &source_selection(1, Some("Сбой 🌍\nsecond line\n\n")),
    )
    .unwrap();
    let item = &checked.matches[0];
    assert_eq!(checked.kind, "source_passages");
    assert_eq!(item.id, id);
    assert_eq!(item.status, packet.evidence[0].status);
    let source = fs::read_to_string(f.root.join(id)).unwrap();
    let start = source.find("Сбой 🌍").unwrap();
    let line = 1 + source[..start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count();
    assert_eq!((item.line_start, item.line_end), (line, line + 2));
    assert_eq!(item.quote, "Сбой 🌍\nsecond line\n\n");
    assert!(item.partial_body);
    let whole = check_librarian_sources(&f.request, &packet, &source_selection(1, None)).unwrap();
    assert_eq!(whole.matches[0].quote, packet.evidence[0].excerpt);
    assert_eq!(before, link_fixture_snapshot(&f.root));
}

#[test]
fn source_passages_reject_inventions_ambiguity_controls_and_extra_claims() {
    use akasha_core::librarian::check_librarian_sources;
    let f = fixture("source-passages-refusal");
    append_link_source(
        &f.root,
        "Projects/alpha/entities/core.md",
        "\nmarker aaa 世界世界\n",
    );
    refresh(&f);
    let packet = prepare_librarian(&f.request, "marker", &[], None).unwrap();
    let before = link_fixture_snapshot(&f.root);
    for bad in [
        source_selection(1, Some("invented cause")),
        source_selection(1, Some("aa")),
        source_selection(1, Some("世界")),
        source_selection(1, Some("\u{1b}[31m")),
        source_selection(1, Some(" ")),
        source_selection(0, None),
        source_selection(2, None),
        r#"{"selections":[{"citation":1},{"citation":1}]}"#.into(),
        r#"{"selections":[{"citation":1,"explanation":"invented"}]}"#.into(),
        r#"{"selections":[],"claims":["invented"]}"#.into(),
        "x".repeat(16_001),
    ] {
        assert_eq!(
            check_librarian_sources(&f.request, &packet, &bad)
                .unwrap_err()
                .exit_code(),
            4,
            "{bad}"
        );
    }
    assert!(
        check_librarian_sources(&f.request, &packet, r#"{"selections":[]}"#)
            .unwrap()
            .matches
            .is_empty()
    );
    assert_eq!(before, link_fixture_snapshot(&f.root));
}

#[test]
fn source_links_use_full_markdown_context_and_only_quoted_packet_targets() {
    use akasha_core::librarian::check_librarian_sources;
    let f = fixture("source-passages-links");
    let id = "Projects/alpha/entities/core.md";
    append_link_source(
        &f.root,
        id,
        "\nNavigation\n[[Projects/alpha/records/tasks/ship-mvp|Ship]]\n`[[Projects/alpha/records/tasks/ship-mvp|inline]]`\n```\n[[Projects/alpha/records/tasks/ship-mvp|code]]\n```\n<!-- [[Projects/alpha/records/tasks/ship-mvp|comment]] -->\n[[Projects/zeta/entities/renderer|foreign]]\n[[#heading|local]]\n",
    );
    fs::write(
        f.root.join("Projects/zeta/entities/renderer.md"),
        "FOREIGN_SECRET invalid YAML",
    )
    .unwrap();
    refresh(&f);
    let before = link_fixture_snapshot(&f.root);
    let packet =
        prepare_librarian(&f.request, "Navigation", &["Navigation".into()], Some(id)).unwrap();
    let checked = check_librarian_sources(&f.request, &packet, &source_selection(1, None)).unwrap();
    let item = &checked.matches[0];
    assert_eq!(item.links.len(), 2);
    assert_eq!(
        item.links[0].target,
        "Projects/alpha/records/tasks/ship-mvp.md"
    );
    assert_eq!(
        item.links[0].wikilink,
        "[[Projects/alpha/records/tasks/ship-mvp|Ship]]"
    );
    assert_eq!(item.links[1].target, id);
    assert_eq!(item.omitted_links, 1);
    for quote in [
        "[[Projects/alpha/records/tasks/ship-mvp|code]]",
        "[[Projects/alpha/records/tasks/ship-mvp|inline]]",
        "[[Projects/alpha/records/tasks/ship-mvp|comment]]",
        "[[Projects/alpha/records/tasks/ship-mvp|Ship",
    ] {
        let result =
            check_librarian_sources(&f.request, &packet, &source_selection(1, Some(quote)))
                .unwrap();
        assert!(result.matches[0].links.is_empty(), "{quote}");
    }
    let packet = prepare_librarian(&f.request, "Navigation", &["Navigation".into()], None).unwrap();
    assert_eq!(packet.evidence.len(), 1);
    let checked = check_librarian_sources(&f.request, &packet, &source_selection(1, None)).unwrap();
    assert_eq!(checked.matches[0].links.len(), 1);
    assert_eq!(checked.matches[0].omitted_links, 2);
    assert_eq!(before, link_fixture_snapshot(&f.root));
}

#[test]
fn source_passages_refuse_stale_forged_and_pending_packets_without_writes() {
    use akasha_core::librarian::check_librarian_sources;
    let f = fixture("source-passages-stale");
    let packet = prepare_librarian(&f.request, "core", &[], None).unwrap();
    let mut forged = packet.clone();
    forged.evidence[0].status = Some("resolved".into());
    assert_eq!(
        check_librarian_sources(&f.request, &forged, &source_selection(1, None))
            .unwrap_err()
            .exit_code(),
        5
    );
    append_link_source(
        &f.root,
        "Projects/alpha/entities/core.md",
        "\nsource changed\n",
    );
    refresh(&f);
    assert_eq!(
        check_librarian_sources(&f.request, &packet, &source_selection(1, None))
            .unwrap_err()
            .exit_code(),
        5
    );
    let packet = prepare_librarian(&f.request, "core", &[], None).unwrap();
    fs::write(
        f.root
            .join("Projects/alpha")
            .join(akasha_core::NOTE_EDIT_JOURNAL_FILE),
        "{}",
    )
    .unwrap();
    let before = link_fixture_snapshot(&f.root);
    assert!(check_librarian_sources(&f.request, &packet, &source_selection(1, None)).is_err());
    assert_eq!(before, link_fixture_snapshot(&f.root));
}

#[test]
fn automatic_queries_filter_function_words_and_match_whole_tokens() {
    let f = fixture("source-query-precision");
    append_link_source(
        &f.root,
        "Projects/alpha/entities/core.md",
        "\nSynthetic scoreboard\n",
    );
    refresh(&f);
    for question in ["THE WHAT WHICH", "score"] {
        assert!(
            prepare_librarian(&f.request, question, &[], None)
                .unwrap()
                .evidence
                .is_empty()
        );
    }
    for term in ["the", "score"] {
        let packet = prepare_librarian(&f.request, "literal", &[term.into()], None).unwrap();
        assert!(
            packet
                .evidence
                .iter()
                .any(|entry| entry.id == "Projects/alpha/entities/core.md")
        );
    }
    assert!(
        !prepare_librarian(&f.request, "SCOREBOARD", &[], None)
            .unwrap()
            .evidence
            .is_empty()
    );
}

#[test]
fn source_link_limit_reports_omissions() {
    use akasha_core::librarian::check_librarian_sources;
    let f = fixture("source-link-cap");
    let links = (0..25)
        .map(|i| format!("[[#heading|alias{i}]]\n"))
        .collect::<String>();
    append_link_source(
        &f.root,
        "Projects/alpha/entities/core.md",
        &format!("\ncap-marker\n{links}"),
    );
    refresh(&f);
    let packet = prepare_librarian(&f.request, "cap-marker", &["cap-marker".into()], None).unwrap();
    let checked = check_librarian_sources(&f.request, &packet, &source_selection(1, None)).unwrap();
    assert_eq!(checked.matches[0].links.len(), 20);
    assert_eq!(checked.matches[0].omitted_links, 5);
}
