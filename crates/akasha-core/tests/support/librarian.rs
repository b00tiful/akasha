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
fn complete_review_preserves_unhighlighted_sources_and_checks_their_freshness() {
    use akasha_core::librarian::review_librarian_sources;
    let f = fixture("complete-source-review");
    append_link_source(
        &f.root,
        "Projects/alpha/entities/core.md",
        "\nPublication uses checked previews. 🌍\n",
    );
    append_link_source(
        &f.root,
        "Projects/alpha/records/tasks/ship-mvp.md",
        "\nPublication rejects stale previews.\n",
    );
    refresh(&f);
    let packet = prepare_librarian(&f.request, "Publication", &[], None).unwrap();
    assert_eq!(packet.evidence.len(), 2);
    let before = link_fixture_snapshot(&f.root);
    for selection in [source_selection(2, None), r#"{"selections":[]}"#.into()] {
        let review = review_librarian_sources(&f.request, &packet, &selection).unwrap();
        assert_eq!(review.kind, "source_review");
        assert_eq!(review.sources.matches.len(), packet.evidence.len());
        for (passage, evidence) in review.sources.matches.iter().zip(&packet.evidence) {
            assert_eq!(passage.quote, evidence.excerpt);
            assert_eq!(passage.fingerprint, evidence.fingerprint);
            assert_eq!(passage.status, evidence.status);
            assert_eq!(
                (passage.line_start, passage.line_end),
                (evidence.line_start, evidence.line_end)
            );
        }
        assert_eq!(review.omitted_notes, packet.omitted_notes);
        assert_eq!(
            review.highlighted_citations,
            if selection.contains("citation") {
                vec![2]
            } else {
                vec![]
            }
        );
    }
    assert_eq!(before, link_fixture_snapshot(&f.root));
    // The omitted source must still refuse when it changes after inference.
    append_link_source(
        &f.root,
        &packet.evidence[0].id,
        "\nChanged unhighlighted source.\n",
    );
    refresh(&f);
    assert_eq!(
        review_librarian_sources(&f.request, &packet, &source_selection(2, None))
            .unwrap_err()
            .exit_code(),
        5
    );
}

#[test]
fn complete_review_rejects_invalid_highlights_forged_packets_and_pending_recovery() {
    use akasha_core::librarian::review_librarian_sources;
    let f = fixture("complete-source-review-refusals");
    let packet = prepare_librarian(&f.request, "core", &[], None).unwrap();
    for bad in [
        source_selection(0, None),
        source_selection(8, None),
        source_selection(1, Some("core")),
        r#"{"selections":[{"citation":1},{"citation":1}]}"#.into(),
        r#"{"selections":[{"citation":1,"explanation":"claim"}]}"#.into(),
        r#"{"selections":[],"answer":"claim"}"#.into(),
        "x".repeat(16_001),
    ] {
        assert_eq!(
            review_librarian_sources(&f.request, &packet, &bad)
                .unwrap_err()
                .exit_code(),
            4
        );
    }
    let mut forged = packet.clone();
    forged.omitted_notes += 1;
    assert_eq!(
        review_librarian_sources(&f.request, &forged, r#"{"selections":[]}"#)
            .unwrap_err()
            .exit_code(),
        5
    );
    let empty = prepare_librarian(&f.request, "unfindableword", &[], None).unwrap();
    assert!(empty.evidence.is_empty());
    assert!(
        review_librarian_sources(&f.request, &empty, r#"{"selections":[]}"#)
            .unwrap()
            .sources
            .matches
            .is_empty()
    );
    fs::write(
        f.root
            .join("Projects/alpha")
            .join(akasha_core::NOTE_EDIT_JOURNAL_FILE),
        "pending",
    )
    .unwrap();
    assert!(review_librarian_sources(&f.request, &packet, r#"{"selections":[]}"#).is_err());
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

#[test]
fn librarian_selects_later_query_coverage_with_exact_physical_lines() {
    let f = fixture("librarian-ranked-lines");
    let id = "Projects/alpha/entities/core.md";
    let target = "Final renderer deadline is 45 seconds. 🌍\n";
    append_link_source(
        &f.root,
        id,
        &format!(
            "\nRenderer diary\n{}\n{target}",
            "Unrelated observation.\n".repeat(120)
        ),
    );
    refresh(&f);
    let before = link_fixture_snapshot(&f.root);
    let packet = prepare_librarian(&f.request, "Final renderer deadline", &[], None).unwrap();
    let item = &packet.evidence[0];
    assert_eq!(item.excerpt, target);
    assert!(item.excerpt_truncated);
    let source = fs::read_to_string(f.root.join(id)).unwrap();
    let expected_line = source[..source.find(target).unwrap()]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1;
    assert_eq!(
        (item.line_start, item.line_end),
        (expected_line, expected_line)
    );
    let checked = akasha_core::librarian::check_librarian_sources(
        &f.request,
        &packet,
        &source_selection(1, None),
    )
    .unwrap();
    assert_eq!(checked.matches[0].quote, target);
    assert_eq!(before, link_fixture_snapshot(&f.root));
}

#[test]
fn librarian_does_not_fill_a_quote_with_an_unrelated_long_following_line() {
    let f = fixture("librarian-context-target");
    let target = "Packet limit: 9 items.\n";
    append_link_source(
        &f.root,
        "Projects/alpha/entities/core.md",
        &format!("\n{target}{}\n", "z".repeat(4000)),
    );
    refresh(&f);
    let packet = prepare_librarian(&f.request, "packet limit", &[], None).unwrap();
    assert_eq!(packet.evidence[0].excerpt, target);
    assert!(packet.evidence[0].excerpt_truncated);
}

#[test]
fn librarian_preserves_complete_bodies_up_to_the_hard_cap_before_ranking_lines() {
    let f = fixture("librarian-complete-body-cap");
    let id = "Projects/alpha/entities/core.md";
    let header = "---\nschema_version: 1\nentity: core\nkind: subsystem\nstatus: observed\nreviewed: 2026-11-09\n---\n";
    // A later high-overlap claim must not hide an earlier answer when both fit.
    for length in [641, 800, 1000] {
        let first = "\nMaximum buffer size is 24 MiB.\r\n";
        let last = "\r\nBuffer reader checks size before returning bytes. 🌍\r\n";
        let body = format!(
            "{first}{}{last}",
            "界".repeat(length - first.chars().count() - last.chars().count())
        );
        fs::write(f.root.join(id), format!("{header}{body}")).unwrap();
        refresh(&f);
        let before = link_fixture_snapshot(&f.root);
        let packet =
            prepare_librarian(&f.request, "maximum buffer reader size", &[], None).unwrap();
        assert_eq!(packet.evidence[0].excerpt, body);
        assert!(!packet.evidence[0].excerpt_truncated);
        let checked = akasha_core::librarian::check_librarian_sources(
            &f.request,
            &packet,
            &source_selection(1, None),
        )
        .unwrap();
        assert_eq!(checked.matches[0].quote, body);
        assert!(!checked.matches[0].partial_body);
        assert_eq!(before, link_fixture_snapshot(&f.root));
    }
    let body = format!(
        "\nMaximum buffer size is 24 MiB.\n{}\nBuffer reader size details.\n",
        "界".repeat(1001)
    );
    fs::write(f.root.join(id), format!("{header}{body}")).unwrap();
    refresh(&f);
    let packet = prepare_librarian(&f.request, "buffer reader size", &[], None).unwrap();
    assert!(packet.evidence[0].excerpt.chars().count() <= 1000);
    assert!(packet.evidence[0].excerpt_truncated);
}

#[test]
fn librarian_short_notes_preserve_facts_before_a_query_heavy_reference() {
    let f = fixture("librarian-short-context");
    append_link_source(
        &f.root,
        "Projects/alpha/entities/core.md",
        "\nThe chosen retry limit is 9.\nSee RETRY_POLICY_TAG for details.\n",
    );
    refresh(&f);
    let packet = prepare_librarian(&f.request, "retry policy tag", &[], None).unwrap();
    assert!(
        packet.evidence[0]
            .excerpt
            .contains("The chosen retry limit is 9.")
    );
    assert!(packet.evidence[0].excerpt.contains("RETRY_POLICY_TAG"));
    assert!(!packet.evidence[0].excerpt_truncated);
}

#[test]
fn librarian_anchor_without_body_terms_keeps_the_bounded_leading_context() {
    let f = fixture("librarian-anchor-fallback");
    let id = "Projects/alpha/entities/core.md";
    append_link_source(
        &f.root,
        id,
        &format!("\nOpening detail {}\n", "x".repeat(2000)),
    );
    refresh(&f);
    let packet = prepare_librarian(&f.request, "Explain", &[], Some(id)).unwrap();
    assert_eq!(packet.evidence[0].id, id);
    assert!(packet.evidence[0].excerpt.contains("Opening detail"));
    assert_eq!(packet.evidence[0].excerpt.chars().count(), 1000);
    assert!(packet.evidence[0].excerpt_truncated);
}

#[test]
fn librarian_searches_recorded_status_without_a_status_taxonomy() {
    let f = fixture("librarian-status-terms");
    fs::write(f.root.join("Projects/alpha/entities/core.md"), "---\nschema_version: 1\nentity: core\nkind: subsystem\nstatus: awaiting-review\nreviewed: 2026-11-07\n---\n\nA compatibility transport.\n").unwrap();
    refresh(&f);
    let before = link_fixture_snapshot(&f.root);
    let packet = prepare_librarian(&f.request, "Find awaiting notes", &[], None).unwrap();
    assert_eq!(packet.evidence.len(), 1);
    assert_eq!(
        packet.evidence[0].status.as_deref(),
        Some("awaiting-review")
    );
    assert!(
        prepare_librarian(&f.request, "await", &[], None)
            .unwrap()
            .evidence
            .is_empty()
    );
    assert_eq!(
        prepare_librarian(&f.request, "literal", &["waiting-rev".into()], None)
            .unwrap()
            .evidence
            .len(),
        1
    );
    assert_eq!(before, link_fixture_snapshot(&f.root));
}

#[test]
fn librarian_filename_rank_resists_body_noise_and_retains_anchor_priority() {
    let f = fixture("librarian-identity-rank");
    let target = "Projects/alpha/entities/archive-retention.md";
    let base = "---\nschema_version: 1\nentity: sample\nkind: subsystem\nstatus: active\nreviewed: 2026-11-07\n---\n\n";
    fs::write(
        f.root.join(target),
        format!("{base}The accepted value is 14.\n"),
    )
    .unwrap();
    let mut entities = vec![
        "entities/core.md".to_owned(),
        "entities/archive-retention.md".into(),
    ];
    for i in 0..8 {
        let relative = format!("entities/chat-{i}.md");
        fs::write(
            f.root.join("Projects/alpha").join(&relative),
            format!("{base}Archive retention limit is a copied question, not an accepted value.\n"),
        )
        .unwrap();
        entities.push(relative);
    }
    let anchor = "Projects/alpha/entities/core.md";
    let neighbor = "Projects/alpha/records/tasks/ship-mvp.md";
    append_link_source(&f.root, anchor, &format!("\n[[{neighbor}|Next task]]\n"));
    support::write_project_state(
        &f.root.join("Projects/alpha"),
        &[],
        &entities.iter().map(String::as_str).collect::<Vec<_>>(),
        &["records/tasks/ship-mvp.md"],
    );
    let before = link_fixture_snapshot(&f.root);
    let packet = prepare_librarian(&f.request, "archive retention limit", &[], None).unwrap();
    assert_eq!(packet.evidence[0].id, target);
    assert_eq!(packet.evidence.len(), 6);
    assert!(packet.omitted_notes > 0);
    let anchored =
        prepare_librarian(&f.request, "archive retention limit", &[], Some(anchor)).unwrap();
    assert_eq!(anchored.evidence[0].id, anchor);
    assert_eq!(anchored.evidence[1].id, neighbor);
    assert_eq!(before, link_fixture_snapshot(&f.root));
}

#[test]
fn librarian_preserves_a_minority_question_part_beyond_the_first_six_candidates() {
    let f = fixture("librarian-question-parts");
    let base = "---\nschema_version: 1\nentity: sample\nkind: subsystem\nstatus: draft\nreviewed: 2026-11-07\n---\n\n";
    let mut entities = vec!["entities/core.md".to_owned()];
    for index in 0..24 {
        let path = format!("entities/delivery-queue-{index:02}.md");
        fs::write(
            f.root.join("Projects/alpha").join(&path),
            format!("{base}Delivery queue retries use backoff.\n"),
        )
        .unwrap();
        entities.push(path);
    }
    let cancel = "entities/cancellation.md";
    fs::write(
        f.root.join("Projects/alpha").join(cancel),
        format!("{base}Cancellation waits for the worker to exit.\n"),
    )
    .unwrap();
    entities.push(cancel.into());
    support::write_project_state(
        &f.root.join("Projects/alpha"),
        &[],
        &entities.iter().map(String::as_str).collect::<Vec<_>>(),
        &["records/tasks/ship-mvp.md"],
    );
    let before = link_fixture_snapshot(&f.root);
    let question = "How do delivery queue retries work and what does cancellation wait for?";
    let packet = prepare_librarian(&f.request, question, &[], None).unwrap();
    assert_eq!(packet.evidence.len(), 6);
    assert_eq!(packet.matched_notes, 25);
    assert_eq!(packet.omitted_notes, 19);
    assert!(packet.evidence[0].id.contains("delivery-queue"));
    assert!(packet.evidence[1].id.ends_with(cancel));
    assert_eq!(
        packet,
        prepare_librarian(&f.request, question, &[], None).unwrap()
    );
    let selected = serde_json::json!({"selections": packet.evidence.iter()
        .map(|item| serde_json::json!({"citation":item.citation})).collect::<Vec<_>>()});
    let checked =
        akasha_core::librarian::check_librarian_sources(&f.request, &packet, &selected.to_string())
            .unwrap();
    assert_eq!(
        checked.matches[1].quote,
        "\nCancellation waits for the worker to exit.\n"
    );
    assert_eq!(before, link_fixture_snapshot(&f.root));
}

#[test]
fn librarian_contiguous_windows_cover_adjacent_facts_in_unicode_crlf_sources() {
    let f = fixture("librarian-window-coverage");
    let id = "Projects/alpha/entities/core.md";
    let target =
        "Transport request timeout is 18 seconds. 🌍\r\nTransport buffer capacity is 48 KiB.\r\n";
    append_link_source(
        &f.root,
        id,
        &format!(
            "\r\nTransport request timeout buffer overview\r\n{}{target}",
            "Historical observation without configuration values.\r\n".repeat(80)
        ),
    );
    refresh(&f);
    let packet = prepare_librarian(
        &f.request,
        "What are the transport request timeout and buffer capacity?",
        &[],
        None,
    )
    .unwrap();
    let item = &packet.evidence[0];
    assert_eq!(item.excerpt, target);
    assert!(item.excerpt_truncated);
    let checked = akasha_core::librarian::check_librarian_sources(
        &f.request,
        &packet,
        &source_selection(item.citation, None),
    )
    .unwrap();
    let source = fs::read_to_string(f.root.join(id)).unwrap();
    assert!(
        source
            .split_inclusive('\n')
            .skip(checked.matches[0].line_start - 1)
            .collect::<String>()
            .starts_with(target)
    );
    assert_eq!(checked.matches[0].line_end, item.line_start + 1);
}

#[test]
fn librarian_clipped_long_line_cannot_claim_invisible_query_coverage() {
    let f = fixture("librarian-visible-window");
    let id = "Projects/alpha/entities/core.md";
    let visible = "Visible buffer capacity is 48 KiB.\n";
    append_link_source(
        &f.root,
        id,
        &format!("\n{} buffer capacity\n{visible}", "x".repeat(1200)),
    );
    refresh(&f);
    let packet = prepare_librarian(&f.request, "buffer capacity", &[], None).unwrap();
    assert_eq!(packet.evidence[0].excerpt, visible);
    assert!(packet.evidence[0].excerpt_truncated);
}

#[test]
fn librarian_focused_filenames_survive_crowded_ties_without_deduplicating_values() {
    let f = fixture("librarian-filename-ties");
    let header = "---\nschema_version: 1\nentity: sample\nkind: subsystem\nstatus: draft\nreviewed: 2026-11-07\n---\n\n";
    let mut entities = vec!["entities/core.md".to_owned()];
    for index in 0..24 {
        let path = format!("entities/upload-retry-run-{index:02}.md");
        fs::write(
            f.root.join("Projects/alpha").join(&path),
            format!("{header}Upload retry limit was checked in run {index}.\n"),
        )
        .unwrap();
        entities.push(path);
    }
    for (name, body, status) in [
        ("upload-retry", "Upload retry limit is 7.\n", "draft"),
        (
            "upload-retry-old",
            "Upload retry limit is 3.\n",
            "superseded",
        ),
        ("editor", "Editor storage uses Markdown.\n", "draft"),
    ] {
        let path = format!("entities/{name}.md");
        fs::write(
            f.root.join("Projects/alpha").join(&path),
            format!(
                "{}{body}",
                header.replace("status: draft", &format!("status: {status}"))
            ),
        )
        .unwrap();
        entities.push(path);
    }
    support::write_project_state(
        &f.root.join("Projects/alpha"),
        &[],
        &entities.iter().map(String::as_str).collect::<Vec<_>>(),
        &["records/tasks/ship-mvp.md"],
    );
    let before = link_fixture_snapshot(&f.root);
    let question = "What is the upload retry limit and editor storage?";
    let packet = prepare_librarian(&f.request, question, &[], None).unwrap();
    assert!(packet.evidence[0].id.ends_with("/upload-retry.md"));
    assert!(packet.evidence[1].id.ends_with("/editor.md"));
    assert!(
        packet
            .evidence
            .iter()
            .any(|source| source.excerpt.contains("limit is 3.")
                && source.status.as_deref() == Some("superseded"))
    );
    assert_eq!(packet.evidence.len(), 6);
    assert_eq!(packet.omitted_notes, packet.matched_notes - 6);
    assert_eq!(
        packet,
        prepare_librarian(&f.request, question, &[], None).unwrap()
    );
    // A longer exact selected identity still outranks filename specificity.
    let anchor = "Projects/alpha/entities/upload-retry-run-23.md";
    let anchored = prepare_librarian(&f.request, question, &[], Some(anchor)).unwrap();
    assert_eq!(anchored.evidence[0].id, anchor);
    assert!(anchored.evidence[0].excerpt.contains("run 23"));
    assert_eq!(before, link_fixture_snapshot(&f.root));
}
