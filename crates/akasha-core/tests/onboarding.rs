use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    InitRequest, NOTE_EDIT_JOURNAL_FILE, OnboardingBatchRequest, OnboardingNoteAction,
    ProposedNote, ResolutionEnvironment, ResolveRequest, apply_approved_onboarding_batch,
    apply_onboarding_batch, initialize_project, prepare_onboarding, preview_onboarding_batch,
    validate_project,
};
use sha2::{Digest, Sha256};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn local_excerpt_catalog_is_lossless_bounded_and_resists_handle_tampering() {
    use akasha_core::onboarding_local::{compile_local_onboarding, prepare_local_onboarding};
    let fixture = Fixture::new("local-excerpt-handles");
    // Repeated text is intentionally legal: handles identify a specific physical window.
    let text = (0..38)
        .map(|i| format!("// Привет 世界 repeated {}\r\n", i % 2))
        .collect::<String>();
    fs::write(fixture.resolution.cwd.join("input.rs"), &text).unwrap();
    let packet = prepare_local_onboarding(&fixture.resolution, &["input.rs".into()]).unwrap();
    assert_eq!(packet.schema_version, 2);
    let source = &packet.sources[0];
    assert_eq!(source.excerpts.len(), 3);
    let mut bytes = String::new();
    for excerpt in &source.excerpts {
        assert!(excerpt.byte_end - excerpt.byte_start <= 2048);
        assert!(excerpt.line_end - excerpt.line_start < 16);
        bytes.push_str(&text[excerpt.byte_start..excerpt.byte_end]);
    }
    assert_eq!(bytes, text);
    let plan = serde_json::json!({ "schema_version": 3, "notes": [{ "note_type": "entity", "name": "core",
        "fields": { "entity": "core", "kind": "subsystem", "status": "active", "reviewed": "2026-11-09" },
        "evidence": [{ "kind": "fact", "claim": "The source includes repeated multilingual comments.", "sources": [{ "source": 1, "excerpt": 2 }] }]
    }] });
    let before = fixture.project_snapshot();
    let draft = compile_local_onboarding(&fixture.resolution, &packet, &plan.to_string()).unwrap();
    let review = &draft.review.notes[0].claims[0].sources[0];
    assert_eq!((review.line_start, review.line_end), (17, 32));
    assert_eq!(
        review.excerpt,
        text.lines()
            .skip(16)
            .take(16)
            .collect::<Vec<_>>()
            .join("\n")
    );
    for reference in [
        serde_json::json!({"source": 1, "excerpt": 0}),
        serde_json::json!({"source": 1, "excerpt": 4}),
        serde_json::json!({"source": 0, "excerpt": 1}),
        serde_json::json!({"source": 2, "excerpt": 1}),
        serde_json::json!({"source": 1, "excerpt": 1, "quote": "invented"}),
        serde_json::json!({"source": 1, "excerpt": 1, "line_start": 17}),
        serde_json::json!({"source": 1, "quote": "// Привет"}),
    ] {
        let mut changed = plan.clone();
        changed["notes"][0]["evidence"][0]["sources"][0] = reference;
        assert!(
            compile_local_onboarding(&fixture.resolution, &packet, &changed.to_string()).is_err()
        );
    }
    let mut altered = packet.clone();
    altered.sources[0].excerpts[1].line_start = 1;
    assert_eq!(
        compile_local_onboarding(&fixture.resolution, &altered, &plan.to_string())
            .unwrap_err()
            .exit_code(),
        5
    );
    altered = packet.clone();
    altered.sources[0].excerpts[1].byte_start = usize::MAX;
    assert_eq!(
        compile_local_onboarding(&fixture.resolution, &altered, &plan.to_string())
            .unwrap_err()
            .exit_code(),
        5
    );
    fs::write(
        fixture.resolution.cwd.join("input.rs"),
        text + "// changed\n",
    )
    .unwrap();
    assert_eq!(
        compile_local_onboarding(&fixture.resolution, &packet, &plan.to_string())
            .unwrap_err()
            .exit_code(),
        5
    );
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
fn local_excerpt_catalog_keeps_final_lines_byte_limits_and_blank_refusals_explicit() {
    use akasha_core::onboarding_local::{compile_local_onboarding, prepare_local_onboarding};
    let fixture = Fixture::new("local-excerpt-boundaries");
    for text in [
        "",
        "one",
        "first\n\nlast\n",
        "first\r\n\r\nlast",
        &format!("{}\n{}", "x".repeat(2047), "λ".repeat(1024)),
    ] {
        fs::write(fixture.resolution.cwd.join("input"), text).unwrap();
        let packet = prepare_local_onboarding(&fixture.resolution, &["input".into()]).unwrap();
        let source = &packet.sources[0];
        assert_eq!(
            source
                .excerpts
                .iter()
                .map(|e| &text[e.byte_start..e.byte_end])
                .collect::<String>(),
            text
        );
        if let Some(last) = source.excerpts.last() {
            assert_eq!(last.line_end, source.lines);
        }
    }
    fs::write(fixture.resolution.cwd.join("input"), "x".repeat(2049)).unwrap();
    let packet = prepare_local_onboarding(&fixture.resolution, &["input".into()]).unwrap();
    assert!(packet.sources[0].excerpts.is_empty());
    assert_eq!(packet.sources[0].text.len(), 2049); // No silent clipping, legacy quote input preserved.
    fs::write(fixture.resolution.cwd.join("input"), "\n\r\n  \n").unwrap();
    let packet = prepare_local_onboarding(&fixture.resolution, &["input".into()]).unwrap();
    let plan = serde_json::json!({ "schema_version": 3, "notes": [{ "note_type": "entity", "name": "core",
        "fields": { "entity": "core", "kind": "subsystem", "status": "active", "reviewed": "2026-11-09" },
        "evidence": [{ "kind": "fact", "claim": "A source exists.", "sources": [{ "source": 1, "excerpt": 1 }] }]
    }] });
    assert!(
        compile_local_onboarding(&fixture.resolution, &packet, &plan.to_string())
            .unwrap_err()
            .to_string()
            .contains("must not be blank")
    );
}

#[test]
fn local_packet_preserves_complete_unicode_sources_and_exact_fingerprints() {
    use akasha_core::onboarding_local::prepare_local_onboarding;
    let fixture = Fixture::new("local-packet");
    let source = "# Привет 世界\r\nA complete source.\r\n";
    fs::write(fixture.resolution.cwd.join("README.md"), source).unwrap();
    let before = fixture.project_snapshot();
    let packet = prepare_local_onboarding(
        &fixture.resolution,
        &["README.md".into(), "Cargo.toml".into()],
    )
    .unwrap();
    assert_eq!(packet.sources[0].path, "Cargo.toml");
    assert_eq!(packet.sources[1].text, source);
    assert_eq!(packet.sources[1].lines, 2);
    assert_eq!(
        packet.sources[1].fingerprint,
        content_fingerprint(source.as_bytes())
    );
    assert_eq!(packet.preparation["project"], "example");
    assert!(serde_json::to_vec(&packet).unwrap().len() <= 16_000);
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
fn local_packet_refuses_unsafe_duplicate_nonregular_and_excessive_inputs() {
    use akasha_core::onboarding_local::prepare_local_onboarding;
    let fixture = Fixture::new("local-bounds");
    for paths in [
        vec![],
        vec!["Cargo.toml"; 9],
        vec!["Cargo.toml"; 2],
        vec!["../escape"],
        vec!["/etc/passwd"],
        vec!["./Cargo.toml"],
        vec!["a\\b"],
        vec!["a//b"],
        vec!["."],
    ] {
        assert!(
            prepare_local_onboarding(
                &fixture.resolution,
                &paths.into_iter().map(str::to_owned).collect::<Vec<_>>()
            )
            .is_err()
        );
    }
    let path = fixture.resolution.cwd.join("input");
    for bytes in [vec![b'x'; 8193], vec![0xff], b"binary\0data".to_vec()] {
        fs::write(&path, bytes).unwrap();
        assert_eq!(
            prepare_local_onboarding(&fixture.resolution, &["input".into()])
                .unwrap_err()
                .exit_code(),
            4
        );
    }
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(prepare_local_onboarding(&fixture.resolution, &["input".into()]).is_err());
    fs::write(fixture.resolution.cwd.join("a"), "x".repeat(8192)).unwrap();
    fs::write(fixture.resolution.cwd.join("b"), "x".repeat(8192)).unwrap();
    assert!(
        prepare_local_onboarding(&fixture.resolution, &["a".into(), "b".into()])
            .unwrap_err()
            .to_string()
            .contains("16000")
    );
}

#[cfg(unix)]
#[test]
fn local_packet_rejects_file_and_ancestor_symlinks_even_inside_repository() {
    use akasha_core::onboarding_local::prepare_local_onboarding;
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new("local-symlinks");
    let repo = &fixture.resolution.cwd;
    fs::create_dir(repo.join("actual")).unwrap();
    fs::write(repo.join("actual/source"), "source").unwrap();
    symlink("actual", repo.join("alias")).unwrap();
    symlink("Cargo.toml", repo.join("link")).unwrap();
    for path in ["alias/source", "link"] {
        assert!(
            prepare_local_onboarding(&fixture.resolution, &[path.into()])
                .unwrap_err()
                .to_string()
                .contains("symlink")
        );
    }
}

fn local_draft_inputs(
    fixture: &Fixture,
) -> (akasha_core::onboarding_local::LocalOnboardingPacket, String) {
    let packet = akasha_core::onboarding_local::prepare_local_onboarding(
        &fixture.resolution,
        &["Cargo.toml".into()],
    )
    .unwrap();
    let mut request = fixture.evidenced_batch_request();
    request.notes.truncate(1);
    let parsed = akasha_core::parse_leading_frontmatter(&request.notes[0].source).unwrap();
    let mut metadata: serde_json::Value = serde_saphyr::from_str(parsed.frontmatter_yaml).unwrap();
    metadata["evidence"][0]["sources"][0]["line_start"] = 1.into();
    metadata["evidence"][0]["sources"][0]["line_end"] = 1.into();
    let claim = metadata["evidence"][0]["claim"].as_str().unwrap();
    request.notes[0].source = format!(
        "---\n{}\n---\n\n# core\n\n## Recorded claims\n\n1. **Fact:** {claim}\n\n[[Projects/example/index.md|Index]] · [[Projects/example/roadmap.md|Roadmap]]\n",
        serde_json::to_string_pretty(&metadata).unwrap()
    );
    request.index = "# Index\n\n[[Projects/example/entities/core|Core]]\n".into();
    request.roadmap = "# Roadmap\n\n[[Projects/example/entities/core|Core]]\n".into();
    let proposal = serde_json::json!({ "notes": request.notes.iter().map(|note| serde_json::json!({
        "note_type": note.note_type, "path": note.path, "source": note.source
    })).collect::<Vec<_>>(), "index": request.index, "roadmap": request.roadmap });
    (packet, proposal.to_string())
}

#[test]
fn local_draft_is_read_only_and_applies_through_existing_approved_core() {
    use akasha_core::onboarding_local::preview_local_onboarding;
    let fixture = Fixture::new("local-preview");
    let (packet, proposal) = local_draft_inputs(&fixture);
    let before = fixture.project_snapshot();
    let draft = preview_local_onboarding(&fixture.resolution, &packet, &proposal).unwrap();
    assert_eq!(fixture.project_snapshot(), before);
    assert_eq!(draft.preview.notes[0].evidence_claims, 1);
    apply_approved_onboarding_batch(
        &draft.proposal.request(&fixture.resolution),
        &draft.preview.preview_id,
    )
    .unwrap();
    validate_project(&fixture.resolution).unwrap();
    let evidence =
        akasha_core::librarian::prepare_librarian(&fixture.resolution, "Rust package", &[], None)
            .unwrap();
    assert!(
        evidence
            .evidence
            .iter()
            .any(|item| item.id.ends_with("entities/core.md") && item.excerpt.contains("core"))
    );
    assert!(preview_local_onboarding(&fixture.resolution, &packet, &proposal).is_err());
    let (fresh, _) = local_draft_inputs(&fixture);
    let rerun = preview_local_onboarding(&fixture.resolution, &fresh, &proposal).unwrap();
    assert_eq!(
        rerun.preview.notes[0].action,
        OnboardingNoteAction::Unchanged
    );
}

#[test]
fn local_draft_refuses_altered_packets_and_changed_sources_without_writes() {
    use akasha_core::onboarding_local::preview_local_onboarding;
    let fixture = Fixture::new("local-drift");
    let (packet, proposal) = local_draft_inputs(&fixture);
    let before = fixture.project_snapshot();
    for which in 0..7 {
        let mut altered = packet.clone();
        match which {
            0 => altered.schema_version += 1,
            1 => altered.sources[0].text.push('x'),
            2 => altered.sources[0].fingerprint.push('0'),
            3 => altered.sources[0].lines += 1,
            4 => altered.preparation["project"] = "other".into(),
            5 => altered.index.push('x'),
            _ => altered.roadmap.push('x'),
        }
        assert_eq!(
            preview_local_onboarding(&fixture.resolution, &altered, &proposal)
                .unwrap_err()
                .exit_code(),
            5
        );
    }
    fs::write(fixture.resolution.cwd.join("Cargo.toml"), "changed\n").unwrap();
    assert_eq!(
        preview_local_onboarding(&fixture.resolution, &packet, &proposal)
            .unwrap_err()
            .exit_code(),
        5
    );
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
fn local_draft_rejects_unselected_evidence_before_shared_preview_reads_it() {
    use akasha_core::onboarding_local::preview_local_onboarding;
    let fixture = Fixture::new("local-scope");
    let (packet, proposal) = local_draft_inputs(&fixture);
    let before = fixture.project_snapshot();
    for changed in [
        proposal.replace("Cargo.toml", "missing-secret-file"),
        proposal.replace("sha256:", "sha256:0"),
    ] {
        let error = preview_local_onboarding(&fixture.resolution, &packet, &changed).unwrap_err();
        assert_eq!(error.exit_code(), 4);
        assert!(error.to_string().contains("only exact sources"));
    }
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
fn local_draft_rejects_malformed_large_and_invalid_schema_or_link_proposals() {
    use akasha_core::onboarding_local::preview_local_onboarding;
    let fixture = Fixture::new("local-invalid");
    let (packet, proposal) = local_draft_inputs(&fixture);
    let before = fixture.project_snapshot();
    let mut extra: serde_json::Value = serde_json::from_str(&proposal).unwrap();
    extra["apply"] = true.into();
    let mut empty = extra.clone();
    empty.as_object_mut().unwrap().remove("apply");
    empty["notes"] = serde_json::json!([]);
    let mut many = empty.clone();
    many["notes"] = serde_json::json!([
        extra["notes"][0],
        extra["notes"][0],
        extra["notes"][0],
        extra["notes"][0]
    ]);
    for invalid in [
        "invalid".into(),
        "x".repeat(16_001),
        extra.to_string(),
        empty.to_string(),
        many.to_string(),
        proposal.replace("active", ""),
        proposal.replace("entities/core", "entities/missing"),
    ] {
        assert_eq!(
            preview_local_onboarding(&fixture.resolution, &packet, &invalid)
                .unwrap_err()
                .exit_code(),
            4
        );
    }
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
fn local_draft_requires_both_projections_to_link_every_proposed_note() {
    let fixture = Fixture::new("local-projection-coverage");
    let (packet, proposal) = local_draft_inputs(&fixture);
    for field in ["index", "roadmap"] {
        let mut value: serde_json::Value = serde_json::from_str(&proposal).unwrap();
        value[field] = "No note links.\n".into();
        let error = akasha_core::onboarding_local::preview_local_onboarding(
            &fixture.resolution,
            &packet,
            &value.to_string(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("each link every proposed note"));
    }
}

#[test]
fn typed_local_plans_compile_exact_evidence_and_cannot_inject_frontmatter() {
    use akasha_core::onboarding_local::compile_local_onboarding;
    let fixture = Fixture::new("local-typed-plan");
    let (packet, _) = local_draft_inputs(&fixture);
    let injected = "Quoted \"text\"\n---\nevidence: replaced\nПривет 世界";
    let mut plan = serde_json::json!({ "schema_version": 2, "notes": [{ "note_type": "entity", "name": "core",
        "fields": { "entity": "core", "kind": "subsystem", "status": "active", "reviewed": "2026-11-08", "description": injected },
        "evidence": [{ "kind": "fact", "claim": "The repository declares a Rust package.", "sources": [{ "source": 1, "quote": "[package]" }] }]
    }] });
    let before = fixture.project_snapshot();
    let draft = compile_local_onboarding(&fixture.resolution, &packet, &plan.to_string()).unwrap();
    let parsed = akasha_core::parse_leading_frontmatter(&draft.proposal.notes[0].source).unwrap();
    let metadata: serde_json::Value = serde_json::from_str(parsed.frontmatter_yaml).unwrap();
    assert_eq!(metadata["description"], injected);
    assert_eq!(metadata["schema_version"], 1);
    assert_eq!(
        metadata["evidence"][0]["sources"][0]["fingerprint"],
        packet.sources[0].fingerprint
    );
    assert_eq!(metadata["evidence"][0]["sources"][0]["path"], "Cargo.toml");
    for handle in [0, 99] {
        plan["notes"][0]["evidence"][0]["sources"][0]["source"] = handle.into();
        assert!(
            compile_local_onboarding(&fixture.resolution, &packet, &plan.to_string())
                .unwrap_err()
                .to_string()
                .contains("source handle")
        );
    }
    plan["notes"][0]["evidence"][0]["sources"][0]["source"] = 1.into();
    for field in ["schema_version", "evidence"] {
        let mut changed = plan.clone();
        changed["notes"][0]["fields"][field] = "override".into();
        assert!(
            compile_local_onboarding(&fixture.resolution, &packet, &changed.to_string())
                .unwrap_err()
                .to_string()
                .contains("cannot replace")
        );
    }
    plan["notes"][0]["evidence"] = serde_json::json!([]);
    assert!(compile_local_onboarding(&fixture.resolution, &packet, &plan.to_string()).is_err());
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
fn local_claim_review_checks_quotes_lines_and_rejects_untracked_prose() {
    use akasha_core::onboarding_local::{
        compile_local_onboarding, prepare_local_onboarding, preview_local_onboarding,
    };
    let fixture = Fixture::new("local-claim-review");
    let source = "// Привет 世界\r\nfn enqueue(n: usize, cap: usize) {\r\n    if n >= cap { return; }\r\n}\r\n";
    fs::write(fixture.resolution.cwd.join("input.rs"), source).unwrap();
    let packet = prepare_local_onboarding(
        &fixture.resolution,
        &["input.rs".into(), "Cargo.toml".into()],
    )
    .unwrap();
    let mut plan = serde_json::json!({ "schema_version": 2, "notes": [{ "note_type": "entity", "name": "core",
        "fields": { "entity": "core", "kind": "subsystem", "status": "active", "reviewed": "2026-11-09" },
        "evidence": [
            { "kind": "fact", "claim": "Enqueue stops at or above capacity.", "sources": [{ "source": 2, "quote": "    if n >= cap { return; }" }] },
            { "kind": "inference", "claim": "The bound may be intentional.", "rationale": "The comparison includes equality.", "sources": [{ "source": 2, "quote": "if n >= cap { return; }\r\n}" }] },
            { "kind": "unknown", "claim": "Deployment is unknown.", "rationale": "No deployment source was supplied.", "sources": [] }
        ]
    }] });
    let before = fixture.project_snapshot();
    let draft = compile_local_onboarding(&fixture.resolution, &packet, &plan.to_string()).unwrap();
    assert_eq!(draft.schema_version, 2);
    assert!(draft.review.requires_review);
    assert_eq!(draft.review.uncited_sources, ["Cargo.toml"]);
    let claims = &draft.review.notes[0].claims;
    assert_eq!(claims[0].sources[0].line_start, 3);
    assert_eq!(claims[0].sources[0].line_end, 3);
    assert_eq!(claims[0].sources[0].excerpt, "    if n >= cap { return; }");
    assert_eq!(claims[1].sources[0].line_end, 4);
    assert_eq!(claims[2].kind, "unknown");
    assert!(claims[2].sources.is_empty());
    assert!(
        draft.proposal.notes[0]
            .source
            .contains("**Inference:** The bound may be intentional.")
    );
    assert!(
        draft.proposal.notes[0]
            .source
            .contains("**Unknown:** Deployment is unknown.")
    );
    let json = serde_json::to_string(&draft.proposal).unwrap();
    let replay = preview_local_onboarding(&fixture.resolution, &packet, &json).unwrap();
    assert_eq!(replay.preview.preview_id, draft.preview.preview_id);
    assert_eq!(
        serde_json::to_value(replay.review).unwrap(),
        serde_json::to_value(&draft.review).unwrap()
    );
    let mut tampered = draft.proposal.clone();
    tampered.notes[0]
        .source
        .push_str("\nEnqueue has constant complexity.\n");
    assert!(
        preview_local_onboarding(
            &fixture.resolution,
            &packet,
            &serde_json::to_string(&tampered).unwrap()
        )
        .unwrap_err()
        .to_string()
        .contains("body must be rendered")
    );
    for quote in ["if n > cap", "", "   ", "unseen claim", &"x".repeat(1025)] {
        plan["notes"][0]["evidence"][0]["sources"][0]["quote"] = quote.into();
        assert!(compile_local_onboarding(&fixture.resolution, &packet, &plan.to_string()).is_err());
    }
    // Citation identity is not entailment: expose a false paraphrase for review, never auto-approve it.
    plan["notes"][0]["evidence"][0]["claim"] = "Enqueue stops only above capacity.".into();
    plan["notes"][0]["evidence"][0]["sources"][0]["quote"] = "if n >= cap { return; }".into();
    let false_claim =
        compile_local_onboarding(&fixture.resolution, &packet, &plan.to_string()).unwrap();
    assert!(false_claim.review.requires_review);
    assert!(
        false_claim.review.notes[0].claims[0].sources[0]
            .excerpt
            .contains(">=")
    );
    // A quote ending in a blank source line includes that physical line, excluding only the final terminator.
    fs::write(fixture.resolution.cwd.join("input.rs"), "first\n\nlast\n").unwrap();
    let blank_packet = prepare_local_onboarding(&fixture.resolution, &["input.rs".into()]).unwrap();
    plan["notes"][0]["evidence"] = serde_json::json!([{ "kind": "fact", "claim": "The source has a blank line.", "sources": [{ "source": 1, "quote": "first\n\n" }] }]);
    let blank =
        compile_local_onboarding(&fixture.resolution, &blank_packet, &plan.to_string()).unwrap();
    assert_eq!(blank.review.notes[0].claims[0].sources[0].line_end, 2);
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
fn local_plan_rejects_legacy_body_ambiguous_quotes_and_unbounded_review() {
    use akasha_core::onboarding_local::{compile_local_onboarding, prepare_local_onboarding};
    let fixture = Fixture::new("local-quote-refusals");
    fs::write(
        fixture.resolution.cwd.join("input"),
        "aaa\nrepeated\nrepeated\n",
    )
    .unwrap();
    let packet = prepare_local_onboarding(&fixture.resolution, &["input".into()]).unwrap();
    let plan = serde_json::json!({ "schema_version": 2, "notes": [{ "note_type": "entity", "name": "core",
        "fields": { "entity": "core", "kind": "subsystem", "status": "active", "reviewed": "2026-11-09" },
        "evidence": [{ "kind": "fact", "claim": "A source exists.", "sources": [{ "source": 1, "quote": "aaa" }] }]
    }] });
    for quote in ["aa", "repeated"] {
        let mut changed = plan.clone();
        changed["notes"][0]["evidence"][0]["sources"][0]["quote"] = quote.into();
        assert!(
            compile_local_onboarding(&fixture.resolution, &packet, &changed.to_string())
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
    }
    for version in [serde_json::Value::Null, 1.into(), 3.into()] {
        let mut changed = plan.clone();
        changed["schema_version"] = version;
        assert!(
            compile_local_onboarding(&fixture.resolution, &packet, &changed.to_string()).is_err()
        );
    }
    let mut legacy = plan.clone();
    legacy["notes"][0]["body"] = "untracked prose".into();
    assert!(compile_local_onboarding(&fixture.resolution, &packet, &legacy.to_string()).is_err());
    fs::write(
        fixture.resolution.cwd.join("input"),
        format!("prefix{}\n", "x".repeat(2048)),
    )
    .unwrap();
    let packet = prepare_local_onboarding(&fixture.resolution, &["input".into()]).unwrap();
    let mut changed = plan;
    changed["notes"][0]["evidence"][0]["sources"][0]["quote"] = "prefix".into();
    assert!(
        compile_local_onboarding(&fixture.resolution, &packet, &changed.to_string())
            .unwrap_err()
            .to_string()
            .contains("2048 bytes")
    );
}

#[test]
fn local_draft_uses_configured_note_type_and_folder() {
    let fixture = Fixture::new("local-configured-type");
    let root = fixture.resolution.root_override.as_ref().unwrap();
    let config = fs::read_to_string(root.join("akasha.toml"))
        .unwrap()
        .replace(
            "[project.note_types.entity]",
            "[project.note_types.component]",
        )
        .replace("folder = \"entities\"", "folder = \"components\"");
    fs::write(root.join("akasha.toml"), config).unwrap();
    fs::rename(
        fixture.project.join("entities"),
        fixture.project.join("components"),
    )
    .unwrap();
    let (packet, proposal) = local_draft_inputs(&fixture);
    assert!(packet.preparation["note_types"].get("component").is_some());
    let proposal = proposal
        .replace("\"note_type\":\"entity\"", "\"note_type\":\"component\"")
        .replace("entities/core", "components/core");
    let draft = akasha_core::onboarding_local::preview_local_onboarding(
        &fixture.resolution,
        &packet,
        &proposal,
    )
    .unwrap();
    assert_eq!(draft.preview.notes[0].note_type, "component");
    apply_approved_onboarding_batch(
        &draft.proposal.request(&fixture.resolution),
        &draft.preview.preview_id,
    )
    .unwrap();
    assert!(fixture.project.join("components/core.md").is_file());
    validate_project(&fixture.resolution).unwrap();
}

#[test]
fn applies_valid_notes_projections_and_state_as_one_checked_batch() {
    let fixture = Fixture::new("complete");
    let request = fixture.evidenced_batch_request();

    let result = apply_onboarding_batch(&request).expect("apply onboarding batch");

    assert_eq!(result.created_notes.len(), 3);
    assert!(result.unchanged_notes.is_empty());
    assert_eq!(result.updated_projections.len(), 2);
    assert_eq!(
        fs::read_to_string(fixture.project.join("entities/core.md")).expect("read entity"),
        request.notes[0].source
    );
    assert_eq!(
        fs::read_to_string(fixture.project.join("index.md")).expect("read index"),
        request.index
    );
    assert_eq!(
        fs::read_to_string(fixture.project.join("roadmap.md")).expect("read roadmap"),
        request.roadmap
    );

    let report = validate_project(&fixture.resolution).expect("validate applied project");
    assert_eq!(report.canonical_notes, 3);
    assert_eq!(report.immutable_events, 1);
    assert_eq!(report.projections["index"].sources, 1);
    assert_eq!(report.projections["roadmap"].sources, 1);
    assert_eq!(report.wikilinks, 2);
    assert!(!fixture.project.join(NOTE_EDIT_JOURNAL_FILE).exists());
}

#[test]
fn exact_rerun_is_a_no_op() {
    let fixture = Fixture::new("rerun");
    let request = fixture.evidenced_batch_request();
    apply_onboarding_batch(&request).expect("apply first batch");
    let state_before = fs::read(fixture.project.join(".akasha-state.toml")).expect("read state");

    let result = apply_onboarding_batch(&request).expect("rerun exact batch");

    assert!(result.created_notes.is_empty());
    assert_eq!(result.unchanged_notes.len(), 3);
    assert!(result.updated_projections.is_empty());
    assert_eq!(
        fs::read(fixture.project.join(".akasha-state.toml")).expect("read rerun state"),
        state_before
    );
}

#[test]
fn differing_existing_note_conflicts_without_mutation() {
    let fixture = Fixture::new("rerun-conflict");
    let request = fixture.evidenced_batch_request();
    apply_onboarding_batch(&request).expect("apply first batch");
    let before = fixture.project_snapshot();
    let mut changed = request.clone();
    changed.notes[0]
        .source
        .push_str("\nHuman-incompatible rewrite.\n");
    changed.index = "# Replaced index\n".to_owned();

    let error = apply_onboarding_batch(&changed).expect_err("changed note must conflict");

    assert_eq!(error.exit_code(), 5);
    assert!(
        error
            .to_string()
            .contains("differs from the proposed exact bytes")
    );
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
fn invalid_note_and_missing_wikilink_write_nothing() {
    for (label, source) in [
        (
            "missing-field",
            "---\nschema_version: 1\nentity: core\nkind: subsystem\nreviewed: 2026-07-13\n---\n\n# Core\n",
        ),
        (
            "missing-link",
            "---\nschema_version: 1\nentity: core\nkind: subsystem\nstatus: active\nreviewed: 2026-07-13\n---\n\n[[Projects/example/entities/missing]]\n",
        ),
    ] {
        let fixture = Fixture::new(label);
        let before = fixture.project_snapshot();
        let mut request = fixture.evidenced_batch_request();
        request.notes[0].source = source.to_owned();

        let error = apply_onboarding_batch(&request).expect_err("invalid proposal must fail");

        assert_eq!(error.exit_code(), 4);
        assert_eq!(fixture.project_snapshot(), before);
    }
}

#[test]
fn public_apply_enforces_preview_evidence_and_size_bounds() {
    let fixture = Fixture::new("public-apply-policy");
    let before = fixture.project_snapshot();

    let mut missing_evidence = fixture.batch_request();
    let error = apply_onboarding_batch(&missing_evidence)
        .expect_err("public apply must reject unattributed notes");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("non-empty `evidence` list"));
    assert_eq!(fixture.project_snapshot(), before);

    missing_evidence = fixture.evidenced_batch_request();
    missing_evidence.notes[0]
        .source
        .push_str(&"x".repeat(65_536));
    let error = apply_onboarding_batch(&missing_evidence)
        .expect_err("public apply must reject oversized notes");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("the maximum is 65536"));
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
fn unsafe_path_and_existing_lock_fail_without_writes() {
    let fixture = Fixture::new("unsafe-path");
    let before = fixture.project_snapshot();
    let mut request = fixture.evidenced_batch_request();
    request.notes[0].path = PathBuf::from("../escape.md");

    let error = apply_onboarding_batch(&request).expect_err("unsafe path must fail");
    assert_eq!(error.exit_code(), 4);
    assert_eq!(fixture.project_snapshot(), before);

    let request = fixture.evidenced_batch_request();
    let lock = fixture.project.join(".akasha-write.lock");
    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock)
        .expect("open project writer lock");
    lock_file.try_lock().expect("hold project writer lock");
    let error = apply_onboarding_batch(&request).expect_err("existing lock must conflict");
    assert_eq!(error.exit_code(), 5);
    drop(lock_file);
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
fn preparation_is_bounded_and_returns_project_contracts_without_note_bodies() {
    let fixture = Fixture::new("preparation");

    let preparation = prepare_onboarding(&fixture.resolution).expect("prepare onboarding");

    assert_eq!(preparation.project, "example");
    assert_eq!(preparation.note_types.len(), 5);
    assert_eq!(preparation.templates.len(), 1);
    assert_eq!(preparation.templates[0].path, PathBuf::from("entity.md"));
    assert!(preparation.template_characters > 0);
    assert_eq!(preparation.omitted_templates, 0);
    assert!(preparation.existing_notes.is_empty());
    assert_eq!(preparation.coverage_criteria.len(), 6);
    assert!(preparation.evidence_contract.contains("fingerprint"));
}

#[test]
fn preview_binds_source_attributed_proposal_and_approved_apply_delegates_to_batch() {
    let fixture = Fixture::new("approved");
    let request = fixture.evidenced_batch_request();

    let preview = preview_onboarding_batch(&request).expect("preview proposal");

    assert_eq!(preview.notes.len(), 3);
    assert!(
        preview
            .notes
            .iter()
            .all(|note| note.action == OnboardingNoteAction::Create)
    );
    assert_eq!(
        preview
            .notes
            .iter()
            .map(|note| note.evidence_claims)
            .sum::<usize>(),
        3
    );
    assert!(preview.proposal_id.starts_with("sha256:"));
    assert!(preview.preview_id.starts_with("sha256:"));
    assert!(preview.index_changed);
    assert!(preview.roadmap_changed);
    assert!(preview.state_changed);

    let result = apply_approved_onboarding_batch(&request, &preview.preview_id)
        .expect("apply approved proposal");
    assert_eq!(result.created_notes.len(), 3);
    validate_project(&fixture.resolution).expect("validate applied project");

    let rerun = preview_onboarding_batch(&request).expect("preview exact rerun");
    assert!(
        rerun
            .notes
            .iter()
            .all(|note| note.action == OnboardingNoteAction::Unchanged)
    );
    assert!(!rerun.index_changed);
    assert!(!rerun.roadmap_changed);
    assert!(!rerun.state_changed);
}

#[test]
fn stale_preview_and_invalid_evidence_write_nothing() {
    let fixture = Fixture::new("binding");
    let request = fixture.evidenced_batch_request();
    let preview = preview_onboarding_batch(&request).expect("preview proposal");
    let before = fixture.project_snapshot();
    let mut changed = request.clone();
    changed.notes[0]
        .source
        .push_str("\nChanged after preview.\n");

    let error = apply_approved_onboarding_batch(&changed, &preview.preview_id)
        .expect_err("changed proposal must not use an earlier preview");
    assert_eq!(error.exit_code(), 5);
    assert!(
        error
            .to_string()
            .contains("approved preview does not match")
    );
    assert_eq!(fixture.project_snapshot(), before);

    let mut missing_evidence = request.clone();
    missing_evidence.notes[0].source = missing_evidence.notes[0]
        .source
        .replace("evidence:\n", "other_evidence:\n");
    let error = preview_onboarding_batch(&missing_evidence)
        .expect_err("transport proposal evidence is mandatory");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("non-empty `evidence` list"));
    assert_eq!(fixture.project_snapshot(), before);

    let mut stale_evidence = request;
    stale_evidence.notes[0].source = stale_evidence.notes[0]
        .source
        .replace("sha256:", "sha256:0");
    let error = preview_onboarding_batch(&stale_evidence)
        .expect_err("source fingerprints must match repository bytes");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("fingerprint is stale"));
    assert_eq!(fixture.project_snapshot(), before);
}

#[cfg(unix)]
#[test]
fn non_utf8_onboarding_identity_paths_fail_before_preview_or_approved_apply() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    for byte in [0x80, 0x81] {
        let mut fixture = Fixture::new("non-utf8-root");
        let original_root = fixture.resolution.root_override.clone().unwrap();
        let invalid_root = fixture
            ._temp
            .path()
            .join(OsString::from_vec(vec![b'r', b'o', b'o', b't', byte]));
        fs::rename(&original_root, &invalid_root).unwrap();
        fixture.resolution.root_override = Some(invalid_root.clone());
        fixture.project = invalid_root.join("Projects/example");
        let request = fixture.evidenced_batch_request();
        let before = fixture.project_snapshot();

        let error = prepare_onboarding(&fixture.resolution)
            .expect_err("preparation cannot return a lossy root identity");
        assert_eq!(error.exit_code(), 4);
        assert!(error.to_string().contains("root path is not valid UTF-8"));
        let error = preview_onboarding_batch(&request)
            .expect_err("a non-UTF-8 root cannot enter preview identity hashing");
        assert_eq!(error.exit_code(), 4);
        assert!(error.to_string().contains("root path is not valid UTF-8"));
        let error = apply_approved_onboarding_batch(&request, "sha256:unapproved")
            .expect_err("approved apply must reject the same non-UTF-8 root");
        assert_eq!(error.exit_code(), 4);
        assert!(error.to_string().contains("root path is not valid UTF-8"));
        assert_eq!(fixture.project_snapshot(), before);
    }

    let fixture = Fixture::new("non-utf8-note");
    let mut request = fixture.evidenced_batch_request();
    request.notes[0].path = PathBuf::from(OsString::from_vec(vec![b'n', 0x80, b'.', b'm', b'd']));
    let before = fixture.project_snapshot();
    let error = preview_onboarding_batch(&request)
        .expect_err("a non-UTF-8 proposed note path cannot enter proposal hashing");
    assert_eq!(error.exit_code(), 4);
    assert!(
        error
            .to_string()
            .contains("proposal paths must be valid UTF-8")
    );
    assert_eq!(fixture.project_snapshot(), before);
}

struct Fixture {
    _temp: TempDir,
    project: PathBuf,
    resolution: ResolveRequest,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let temp = TempDir::new(label);
        let root = temp.path().join("root");
        let repository = temp.path().join("repository");
        for directory in [
            root.join("Meta"),
            root.join("templates"),
            root.join("Global"),
            root.join("Projects"),
            root.join("Inbox"),
            repository.clone(),
        ] {
            fs::create_dir_all(directory).expect("create fixture directory");
        }
        fs::write(
            root.join("akasha.toml"),
            include_str!("../../../tests/fixtures/resolution/valid-root/akasha.toml"),
        )
        .expect("write root config");
        fs::write(root.join("Meta/projects.yaml"), "{}\n").expect("write empty registry");
        fs::write(
            root.join("templates/entity.md"),
            "---\nschema_version: 1\n---\n\n# Entity template\n",
        )
        .expect("write onboarding template");
        fs::write(
            repository.join("Cargo.toml"),
            "[package]\nname = \"example\"\n",
        )
        .expect("write evidence source");
        initialize_project(&InitRequest {
            root_override: Some(root.clone()),
            project: "example".to_owned(),
            cwd: repository.clone(),
            environment: ResolutionEnvironment::default(),
        })
        .expect("initialize empty project");
        let project = root.join("Projects/example");
        let resolution = ResolveRequest {
            root_override: Some(root.clone()),
            project_override: None,
            cwd: repository,
            environment: ResolutionEnvironment::default(),
        };
        Self {
            _temp: temp,
            project,
            resolution,
        }
    }

    fn batch_request(&self) -> OnboardingBatchRequest {
        OnboardingBatchRequest {
            resolution: self.resolution.clone(),
            notes: vec![
                ProposedNote {
                    note_type: "entity".to_owned(),
                    path: PathBuf::from("core.md"),
                    source: "---\nschema_version: 1\nentity: core\nkind: subsystem\nstatus: active\nreviewed: 2026-07-13\n---\n\n# Core\n\n[[Projects/example/records/tasks/next]]\n".to_owned(),
                },
                ProposedNote {
                    note_type: "task".to_owned(),
                    path: PathBuf::from("next.md"),
                    source: "---\nschema_version: 1\nproject: example\ntype: task\nstatus: open\ncreated: 2026-07-13\nupdated: 2026-07-13\n---\n\n# Next\n\n[[Projects/example/entities/core]]\n".to_owned(),
                },
                ProposedNote {
                    note_type: "session".to_owned(),
                    path: PathBuf::from("2026-07-13.md"),
                    source: "---\nschema_version: 1\nproject: example\ntype: session\ndate: 2026-07-13\n---\n\n# Session\n".to_owned(),
                },
            ],
            index: "# Index\n\n- [[Projects/example/entities/core]]\n".to_owned(),
            roadmap: "# Roadmap\n\n- [[Projects/example/records/tasks/next]]\n".to_owned(),
        }
    }

    fn evidenced_batch_request(&self) -> OnboardingBatchRequest {
        let mut request = self.batch_request();
        let fingerprint = content_fingerprint(
            &fs::read(request.resolution.cwd.join("Cargo.toml")).expect("read evidence source"),
        );
        request.notes[0].source = add_evidence(
            &request.notes[0].source,
            &format!(
                "evidence:\n  - kind: fact\n    claim: The repository declares a Rust \
                 package.\n    sources:\n      - path: Cargo.toml\n        fingerprint: \
                 \"{fingerprint}\"\n        line_start: 1\n        line_end: 2\n"
            ),
        );
        request.notes[1].source = add_evidence(
            &request.notes[1].source,
            &format!(
                "evidence:\n  - kind: inference\n    claim: The package needs a documented next \
                 task.\n    rationale: A package manifest exists but the initialized roadmap is \
                 empty.\n    sources:\n      - path: Cargo.toml\n        fingerprint: \
                 \"{fingerprint}\"\n"
            ),
        );
        request.notes[2].source = add_evidence(
            &request.notes[2].source,
            "evidence:\n  - kind: unknown\n    claim: The repository release process is not yet \
             known.\n    rationale: No release evidence was supplied in this bounded fixture.\n",
        );
        request
    }

    fn project_snapshot(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        collect_files(&self.project, &self.project, &mut files);
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    }
}

fn add_evidence(source: &str, evidence: &str) -> String {
    source.replacen("\n---\n\n#", &format!("\n{evidence}---\n\n#"), 1)
}

fn content_fingerprint(source: &[u8]) -> String {
    let digest = Sha256::digest(source);
    let mut fingerprint = String::with_capacity(71);
    fingerprint.push_str("sha256:");
    for byte in digest {
        write!(fingerprint, "{byte:02x}").expect("writing to a string cannot fail");
    }
    fingerprint
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
    let mut entries = fs::read_dir(directory)
        .expect("read project directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("read project entries");
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if entry.file_type().expect("read project entry type").is_dir() {
            collect_files(root, &path, files);
        } else {
            files.push((
                path.strip_prefix(root)
                    .expect("project-relative path")
                    .to_path_buf(),
                fs::read(path).expect("read project file"),
            ));
        }
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "akasha-onboarding-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create temporary directory");
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
