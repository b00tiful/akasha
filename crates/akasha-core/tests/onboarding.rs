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

fn focused_source_text() -> String {
    (1..=400)
        .map(|line| match line {
            82 => "pub const LEASE_SECONDS: u64 = 37; // lease deadline\r\n".into(),
            83 => "pub const CLOCK_KIND: &str = \"monotonic\"; // lease deadline\r\n".into(),
            146 => "pub const RETRIES: usize = 2; // lease deadline\r\n".into(),
            _ => format!("// Привет 世界 ordinary source line {line:04}\r\n"),
        })
        .collect()
}

#[test]
fn focused_sampling_finds_off_position_behavior_with_original_coordinates_and_drift_checks() {
    use akasha_core::onboarding_local::{
        compile_local_onboarding, prepare_local_onboarding_focused,
        prepare_local_onboarding_sampled,
    };
    let f = Fixture::new("focus-windows");
    let text = focused_source_text();
    fs::write(f.resolution.cwd.join("input.rs"), &text).unwrap();
    let paths = &["input.rs".into()];
    let baseline = prepare_local_onboarding_sampled(&f.resolution, paths).unwrap();
    assert!(!baseline.sources[0].text.contains("LEASE_SECONDS"));
    let packet = prepare_local_onboarding_focused(
        &f.resolution,
        paths,
        &[" Deadline ".into(), "LEASE".into(), "lease".into()],
        true,
    )
    .unwrap();
    assert_eq!(packet.schema_version, 4);
    assert_eq!(packet.focus.as_ref().unwrap().terms, ["deadline", "lease"]);
    assert!(serde_json::to_vec(&packet).unwrap().len() <= 16_000);
    let source = &packet.sources[0];
    assert!(source.text.contains("LEASE_SECONDS") && source.text.contains("RETRIES"));
    assert!(!source.text.contains("source line 0400"));
    assert_eq!(source.excerpts.len(), 3);
    let original = text.split_inclusive('\n').collect::<Vec<_>>();
    let mut coverage = vec![0; 400];
    for excerpt in &source.excerpts {
        assert_eq!(
            &source.text[excerpt.byte_start..excerpt.byte_end],
            original[excerpt.line_start - 1..excerpt.line_end].concat()
        );
        for count in &mut coverage[excerpt.line_start - 1..excerpt.line_end] {
            *count += 1;
        }
    }
    for range in &source.sampling.as_ref().unwrap().omitted_lines {
        for count in &mut coverage[range.line_start - 1..range.line_end] {
            *count += 1;
        }
    }
    assert!(coverage.iter().all(|count| *count == 1));
    let mut plan = sampled_claim_plan();
    plan["notes"][0]["evidence"][0]["claim"] = "The lease duration constant is 37 seconds.".into();
    plan["notes"][0]["evidence"][0]["sources"][0]["excerpt"] = 2.into();
    let before = f.project_snapshot();
    let draft = compile_local_onboarding(&f.resolution, &packet, &plan.to_string()).unwrap();
    assert!(
        serde_json::to_string(&draft.review)
            .unwrap()
            .contains("LEASE_SECONDS")
    );
    assert_eq!(draft.review.uncited_focus_excerpts.len(), 1);
    assert_eq!(draft.review.uncited_focus_excerpts[0].line_start, 145);
    let mut opening_only = plan.clone();
    opening_only["notes"][0]["evidence"][0]["sources"][0]["excerpt"] = 1.into();
    let neglected =
        compile_local_onboarding(&f.resolution, &packet, &opening_only.to_string()).unwrap();
    assert_eq!(neglected.review.uncited_focus_excerpts.len(), 2);
    for mutate in [0, 1, 2, 3] {
        let mut altered = packet.clone();
        match mutate {
            0 => altered.focus.as_mut().unwrap().terms = vec!["ordinary".into()],
            1 => altered.focus = None,
            2 => altered.schema_version = 3,
            _ => altered.focus.as_mut().unwrap().sample_large_files = false,
        }
        assert!(compile_local_onboarding(&f.resolution, &altered, &plan.to_string()).is_err());
    }
    // Omitted content changes also invalidate a saved focused packet.
    fs::write(
        f.resolution.cwd.join("input.rs"),
        text.replace("source line 0300", "changed line 0300"),
    )
    .unwrap();
    assert!(compile_local_onboarding(&f.resolution, &packet, &plan.to_string()).is_err());
    assert_eq!(f.project_snapshot(), before);
}

#[test]
fn focused_discovery_prioritizes_content_matches_preserves_context_and_reports_gaps() {
    use akasha_core::onboarding_discovery::{
        discover_local_onboarding, discover_local_onboarding_focused,
    };
    let f = Fixture::new("focus-discovery");
    let repo = &f.resolution.cwd;
    fs::create_dir_all(repo.join("src/zz")).unwrap();
    fs::create_dir_all(repo.join("tests")).unwrap();
    fs::write(
        repo.join("README.md"),
        "# Context\nThis project provides local services.\n",
    )
    .unwrap();
    for index in 0..12 {
        fs::write(
            repo.join(format!("src/a{index:02}.rs")),
            "pub const UNRELATED: usize = 1;\n",
        )
        .unwrap();
    }
    let target = "pub const LEASE_SECONDS: usize = 37; // deadline\n";
    fs::write(repo.join("src/zz/target.rs"), target).unwrap();
    fs::write(repo.join("src/zz/repeated.rs"), "// lease\n".repeat(30)).unwrap();
    fs::write(
        repo.join("tests/lease_test.rs"),
        "// lease deadline checks\n",
    )
    .unwrap();
    fs::write(repo.join("src/zz/secret_lease.rs"), "must never read\n").unwrap();
    let before = f.project_snapshot();
    let baseline = discover_local_onboarding(&f.resolution, &[".".into()]).unwrap();
    assert!(
        !baseline
            .packet
            .sources
            .iter()
            .any(|s| s.path == "src/zz/target.rs")
    );
    let discovered = discover_local_onboarding_focused(
        &f.resolution,
        &[".".into()],
        &["lease".into(), "deadline".into()],
        false,
    )
    .unwrap();
    assert_eq!(discovered.packet.schema_version, 4);
    let selected = &discovered.coverage.selected;
    assert!(
        selected
            .iter()
            .any(|s| s.path == "README.md" && s.matched_terms.is_empty())
    );
    assert!(selected.iter().any(|s| s.path == "Cargo.toml"));
    assert!(
        selected
            .iter()
            .any(|s| s.path == "src/zz/target.rs" && s.matched_terms == ["deadline", "lease"])
    );
    assert!(selected.iter().any(|s| s.path == "tests/lease_test.rs"));
    assert!(
        discovered
            .coverage
            .omitted
            .iter()
            .any(|s| s.path == "src/a00.rs" && s.reason == "focus_no_match")
    );
    assert!(
        discovered
            .coverage
            .excluded_examples
            .iter()
            .any(|s| s.path.ends_with("secret_lease.rs"))
    );
    let expected_bytes: usize = std::iter::once("README.md")
        .chain(std::iter::once("Cargo.toml"))
        .map(|p| fs::metadata(repo.join(p)).unwrap().len() as usize)
        .sum::<usize>()
        + 12 * "pub const UNRELATED: usize = 1;\n".len()
        + target.len()
        + "// lease\n".len() * 30
        + "// lease deadline checks\n".len();
    assert_eq!(discovered.coverage.focus_scan_bytes, Some(expected_bytes));
    assert!(
        discover_local_onboarding_focused(
            &f.resolution,
            &[".".into()],
            &["no-such-subsystem".into()],
            true
        )
        .is_err()
    );
    for index in 0..10 {
        fs::write(repo.join(format!("src/shallow{index:02}.rs")), "// lease\n").unwrap();
    }
    let ranked = discover_local_onboarding_focused(
        &f.resolution,
        &["src".into()],
        &["lease".into(), "deadline".into()],
        false,
    )
    .unwrap();
    assert_eq!(ranked.packet.sources.len(), 8);
    assert!(
        ranked
            .coverage
            .selected
            .iter()
            .any(|s| s.path == "src/zz/target.rs")
    );
    assert!(
        ranked
            .coverage
            .omitted
            .iter()
            .any(|s| s.reason == "file_limit")
    );
    assert_eq!(f.project_snapshot(), before);
}

#[test]
fn focused_selection_validates_raw_bounds_unicode_and_legacy_packet_compatibility() {
    use akasha_core::onboarding_local::{
        plan_local_onboarding, prepare_local_onboarding, prepare_local_onboarding_focused,
    };
    let f = Fixture::new("focus-validation");
    let paths = &["Cargo.toml".into()];
    for terms in [
        vec![],
        vec![" ".into()],
        vec!["x\ny".into()],
        vec!["x".repeat(81)],
        vec!["a".into(); 9],
        vec!["İ".repeat(40)],
    ] {
        assert!(prepare_local_onboarding_focused(&f.resolution, paths, &terms, false).is_err());
    }
    let legacy = prepare_local_onboarding(&f.resolution, paths).unwrap();
    assert!(
        serde_json::to_value(&legacy)
            .unwrap()
            .get("focus")
            .is_none()
    );
    let restored = serde_json::from_str(&serde_json::to_string(&legacy).unwrap()).unwrap();
    plan_local_onboarding(&f.resolution, &restored).unwrap();
    fs::write(f.resolution.cwd.join("Cargo.toml"), "# ПРИВЕТ 世界\n").unwrap();
    let unicode = prepare_local_onboarding_focused(
        &f.resolution,
        paths,
        &["ПРИВЕТ".into(), "世界".into()],
        false,
    )
    .unwrap();
    assert_eq!(unicode.focus.unwrap().terms, ["привет", "世界"]);
}

fn identity_fixture(
    label: &str,
) -> (
    Fixture,
    akasha_core::onboarding_local::LocalOnboardingPacket,
) {
    let f = Fixture::new(label);
    for path in ["src/alpha/lib.rs", "src/beta/lib.rs", "tests/check.rs"] {
        let target = f.resolution.cwd.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, "pub const LIMIT: usize = 37;\n").unwrap();
    }
    fs::write(
        f.resolution.cwd.join("README.md"),
        "# Fixture\nA bounded source fixture.\n",
    )
    .unwrap();
    let packet = akasha_core::onboarding_local::prepare_local_onboarding(
        &f.resolution,
        &[
            "README.md",
            "Cargo.toml",
            "src/alpha/lib.rs",
            "src/beta/lib.rs",
            "tests/check.rs",
        ]
        .map(str::to_owned),
    )
    .unwrap();
    (f, packet)
}

fn slotted_claim_plan(
    f: &Fixture,
    packet: &akasha_core::onboarding_local::LocalOnboardingPacket,
) -> serde_json::Value {
    let identities =
        akasha_core::onboarding_local::plan_local_onboarding(&f.resolution, packet).unwrap();
    serde_json::json!({"schema_version": 4, "notes": identities.slots.iter().map(|slot| serde_json::json!({
        "slot": slot.slot,
        "fields": {"entity": slot.role, "kind": "source-group", "status": "observed", "reviewed": "2026-11-09"},
        "evidence": [{"kind": "unknown", "claim": "Behavior needs source review.", "rationale": "This boundary test makes no semantic assertion.", "sources": []}]
    })).collect::<Vec<_>>()})
}

#[test]
fn automatic_note_slots_partition_sources_with_stable_unique_configured_identities() {
    use akasha_core::onboarding_local::{plan_local_onboarding, prepare_local_onboarding};
    let (f, packet) = identity_fixture("identity-partition");
    let before = f.project_snapshot();
    let plan = plan_local_onboarding(&f.resolution, &packet).unwrap();
    assert_eq!(
        plan.slots
            .iter()
            .map(|s| s.role.as_str())
            .collect::<Vec<_>>(),
        ["context", "implementation", "verification"]
    );
    let handles = plan
        .slots
        .iter()
        .flat_map(|s| s.sources.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(handles, (1..=5).collect());
    assert_eq!(plan.slots.iter().map(|s| s.sources.len()).sum::<usize>(), 5);
    assert_eq!(
        plan.slots
            .iter()
            .map(|s| &s.name)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
    assert!(plan.slots.iter().all(|s| s.path
        == Path::new("entities").join(format!("{}.md", s.name))
        && s.note_type == "entity"
        && s.name.len() <= 80));
    let paths = packet
        .sources
        .iter()
        .rev()
        .map(|s| s.path.clone())
        .collect::<Vec<_>>();
    let reordered = prepare_local_onboarding(&f.resolution, &paths).unwrap();
    assert_eq!(
        serde_json::to_value(plan_local_onboarding(&f.resolution, &reordered).unwrap()).unwrap(),
        serde_json::to_value(&plan).unwrap()
    );
    fs::write(
        f.resolution.cwd.join("src/alpha/lib.rs"),
        "pub const LIMIT: usize = 38;\n",
    )
    .unwrap();
    assert!(plan_local_onboarding(&f.resolution, &packet).is_err());
    let changed = prepare_local_onboarding(&f.resolution, &paths).unwrap();
    assert_eq!(
        serde_json::to_value(plan_local_onboarding(&f.resolution, &changed).unwrap()).unwrap(),
        serde_json::to_value(&plan).unwrap()
    );
    assert_eq!(f.project_snapshot(), before);
}

#[test]
fn slotted_compilation_refuses_missing_duplicate_injected_and_out_of_group_notes() {
    use akasha_core::onboarding_local::compile_local_onboarding;
    let (f, packet) = identity_fixture("identity-validation");
    let before = f.project_snapshot();
    let good = slotted_claim_plan(&f, &packet);
    let draft = compile_local_onboarding(&f.resolution, &packet, &good.to_string()).unwrap();
    assert_eq!(draft.proposal.notes.len(), 3);
    for variant in 0..7 {
        let mut plan = good.clone();
        match variant {
            0 => {
                plan["notes"].as_array_mut().unwrap().pop();
            }
            1 => plan["notes"][1]["slot"] = 1.into(),
            2 => plan["notes"][0]["slot"] = 0.into(),
            3 => plan["notes"][0]["name"] = "injected".into(),
            4 => plan["notes"][0]["note_type"] = "task".into(),
            5 => {
                plan["notes"][0]["evidence"] = serde_json::json!([{"kind": "fact", "claim": "Wrong source group", "sources": [{"source": 3, "excerpt": 1}]}])
            }
            _ => {
                plan["notes"][0]["evidence"] = serde_json::json!([{"kind": "fact", "claim": "Quote injection", "sources": [{"source": 1, "quote": "package"}]}])
            }
        }
        assert!(
            compile_local_onboarding(&f.resolution, &packet, &plan.to_string()).is_err(),
            "variant {variant}"
        );
    }
    let mut reordered = good;
    reordered["notes"].as_array_mut().unwrap().reverse();
    assert_eq!(
        compile_local_onboarding(&f.resolution, &packet, &reordered.to_string())
            .unwrap()
            .preview
            .proposal_id,
        draft.preview.proposal_id
    );
    assert_eq!(f.project_snapshot(), before);
}

#[test]
fn automatic_plan_uses_custom_entity_schema_and_refuses_ambiguous_or_unknown_roles() {
    use akasha_core::onboarding_local::{plan_local_onboarding, prepare_local_onboarding};
    let (f, _) = identity_fixture("identity-config");
    let root = f.resolution.root_override.as_ref().unwrap();
    let config_path = root.join("akasha.toml");
    let original = fs::read_to_string(&config_path).unwrap();
    let custom = original
        .replace(
            "[project.note_types.entity]",
            "[project.note_types.component]",
        )
        .replace("folder = \"entities\"", "folder = \"components\"");
    fs::rename(f.project.join("entities"), f.project.join("components")).unwrap();
    fs::write(&config_path, &custom).unwrap();
    let packet = prepare_local_onboarding(&f.resolution, &["Cargo.toml".into()]).unwrap();
    let plan = plan_local_onboarding(&f.resolution, &packet).unwrap();
    assert_eq!(plan.slots[0].note_type, "component");
    assert!(plan.slots[0].path.starts_with("components"));
    let extra = "\n[project.note_types.other]\nclass = \"entity\"\nfolder = \"other\"\nrequired_fields = [\"entity\"]\n";
    fs::create_dir(f.project.join("other")).unwrap();
    fs::write(&config_path, custom + extra).unwrap();
    let ambiguous = prepare_local_onboarding(&f.resolution, &["Cargo.toml".into()]).unwrap();
    assert!(
        plan_local_onboarding(&f.resolution, &ambiguous)
            .unwrap_err()
            .to_string()
            .contains("exactly one")
    );
    fs::write(&config_path, original).unwrap();
    fs::rename(f.project.join("components"), f.project.join("entities")).unwrap();
    fs::write(f.resolution.cwd.join("input.custom"), "custom syntax\n").unwrap();
    let unknown = prepare_local_onboarding(&f.resolution, &["input.custom".into()]).unwrap();
    assert!(
        plan_local_onboarding(&f.resolution, &unknown)
            .unwrap_err()
            .to_string()
            .contains("recognized source roles")
    );
}

#[test]
fn planned_note_conflicts_never_allocate_an_unreviewed_suffix() {
    use akasha_core::onboarding_local::{
        apply_local_onboarding, compile_local_onboarding, plan_local_onboarding,
        prepare_local_onboarding, preview_local_onboarding,
    };
    let (f, packet) = identity_fixture("identity-publication");
    let plan = slotted_claim_plan(&f, &packet);
    let draft = compile_local_onboarding(&f.resolution, &packet, &plan.to_string()).unwrap();
    apply_local_onboarding(
        &f.resolution,
        &packet,
        &serde_json::to_string(&draft.proposal).unwrap(),
        &draft.preview.preview_id,
    )
    .unwrap();
    let before = f.project_snapshot();
    let paths = packet
        .sources
        .iter()
        .map(|s| s.path.clone())
        .collect::<Vec<_>>();
    let fresh = prepare_local_onboarding(&f.resolution, &paths).unwrap();
    let identities = plan_local_onboarding(&f.resolution, &fresh).unwrap();
    assert_eq!(
        identities.slots[0].name.clone() + ".md",
        draft.proposal.notes[0].path.to_str().unwrap()
    );
    let replay = preview_local_onboarding(
        &f.resolution,
        &fresh,
        &serde_json::to_string(&draft.proposal).unwrap(),
    )
    .unwrap();
    assert!(
        replay
            .preview
            .notes
            .iter()
            .all(|n| n.action == OnboardingNoteAction::Unchanged)
    );
    let mut altered = plan;
    altered["notes"][0]["evidence"][0]["claim"] = "Different reviewed claim.".into();
    assert!(compile_local_onboarding(&f.resolution, &fresh, &altered.to_string()).is_err());
    assert_eq!(f.project_snapshot(), before);
}

#[test]
fn sampled_sources_preserve_original_coordinates_unicode_and_complete_omission_coverage() {
    use akasha_core::onboarding_local::{
        prepare_local_onboarding, prepare_local_onboarding_sampled,
    };
    let fixture = Fixture::new("sampled-coordinates");
    let text = (0..399)
        .map(|i| format!("// Привет 世界 source line {i:04}\r\n"))
        .collect::<String>()
        + "pub const TAIL: usize = 37;";
    fs::write(fixture.resolution.cwd.join("input.rs"), &text).unwrap();
    let paths = &["input.rs".into()];
    assert!(prepare_local_onboarding(&fixture.resolution, paths).is_err());
    let before = fixture.project_snapshot();
    let packet = prepare_local_onboarding_sampled(&fixture.resolution, paths).unwrap();
    assert_eq!(packet.schema_version, 3);
    assert!(serde_json::to_vec(&packet).unwrap().len() <= 16_000);
    let source = &packet.sources[0];
    assert_eq!(source.lines, 400);
    assert_eq!(source.fingerprint, content_fingerprint(text.as_bytes()));
    assert_eq!(source.sampling.as_ref().unwrap().total_bytes, text.len());
    assert_eq!(source.excerpts.len(), 3);
    assert!(source.text.len() <= 6144);
    assert_eq!(source.excerpts.first().unwrap().line_start, 1);
    assert_eq!(source.excerpts.last().unwrap().line_end, 400);
    assert!(source.text.ends_with("pub const TAIL: usize = 37;"));
    let original = text.split_inclusive('\n').collect::<Vec<_>>();
    let mut coverage = vec![0; source.lines];
    for excerpt in &source.excerpts {
        assert!(excerpt.line_end - excerpt.line_start < 16);
        assert!(excerpt.byte_end - excerpt.byte_start <= 2048);
        assert_eq!(
            &source.text[excerpt.byte_start..excerpt.byte_end],
            original[excerpt.line_start - 1..excerpt.line_end].concat()
        );
        for count in &mut coverage[excerpt.line_start - 1..excerpt.line_end] {
            *count += 1;
        }
    }
    for range in &source.sampling.as_ref().unwrap().omitted_lines {
        for count in &mut coverage[range.line_start - 1..range.line_end] {
            *count += 1;
        }
    }
    assert!(coverage.into_iter().all(|count| count == 1));
    assert_eq!(fixture.project_snapshot(), before);
}

fn sampled_claim_plan() -> serde_json::Value {
    serde_json::json!({ "schema_version": 3, "notes": [{ "note_type": "entity", "name": "sampled-core",
        "fields": { "entity": "sampled-core", "kind": "subsystem", "status": "active", "reviewed": "2026-11-09" },
        "evidence": [{ "kind": "fact", "claim": "The tail constant is 37.", "sources": [{ "source": 1, "excerpt": 3 }] }]
    }] })
}

#[test]
fn sampled_review_refuses_gap_citations_tampering_quotes_and_omitted_line_drift() {
    use akasha_core::onboarding_local::{
        compile_local_onboarding, prepare_local_onboarding_sampled, preview_local_onboarding,
    };
    let fixture = Fixture::new("sampled-review");
    let text =
        "// stable filler for sampled coverage\n".repeat(399) + "pub const TAIL: usize = 37;\n";
    let path = fixture.resolution.cwd.join("input.rs");
    fs::write(&path, &text).unwrap();
    let packet =
        prepare_local_onboarding_sampled(&fixture.resolution, &["input.rs".into()]).unwrap();
    let plan = sampled_claim_plan().to_string();
    let before = fixture.project_snapshot();
    let draft = compile_local_onboarding(&fixture.resolution, &packet, &plan).unwrap();
    let review = &draft.review.notes[0].claims[0].sources[0];
    assert_eq!((review.line_start, review.line_end), (385, 400));
    assert!(review.excerpt.ends_with("pub const TAIL: usize = 37;"));
    assert_eq!(draft.review.sampled_sources[0].total_lines, 400);
    for which in 0..5 {
        let mut altered = packet.clone();
        match which {
            0 => altered.sources[0].sampling.as_mut().unwrap().total_bytes += 1,
            1 => altered.sources[0]
                .sampling
                .as_mut()
                .unwrap()
                .omitted_lines
                .clear(),
            2 => altered.sources[0].excerpts[2].byte_start = usize::MAX,
            3 => altered.sources[0].excerpts[2].line_start = 1,
            _ => altered.schema_version = 2,
        }
        assert!(compile_local_onboarding(&fixture.resolution, &altered, &plan).is_err());
    }
    let mut quotes = sampled_claim_plan();
    quotes["schema_version"] = 2.into();
    quotes["notes"][0]["evidence"][0]["sources"][0] =
        serde_json::json!({"source": 1, "quote": "pub const TAIL: usize = 37;"});
    assert!(
        compile_local_onboarding(&fixture.resolution, &packet, &quotes.to_string())
            .unwrap_err()
            .to_string()
            .contains("excerpt-handle")
    );
    for (start, end) in [(17, 17), (16, 17), (16, 385), (384, 385)] {
        let mut proposal = draft.proposal.clone();
        let parsed = akasha_core::parse_leading_frontmatter(&proposal.notes[0].source).unwrap();
        let body = parsed.body.to_owned();
        let mut metadata: serde_json::Value =
            serde_json::from_str(parsed.frontmatter_yaml).unwrap();
        metadata["evidence"][0]["sources"][0]["line_start"] = start.into();
        metadata["evidence"][0]["sources"][0]["line_end"] = end.into();
        proposal.notes[0].source = format!(
            "---\n{}\n---\n{body}",
            serde_json::to_string_pretty(&metadata).unwrap()
        );
        assert!(
            preview_local_onboarding(
                &fixture.resolution,
                &packet,
                &serde_json::to_string(&proposal).unwrap()
            )
            .unwrap_err()
            .to_string()
            .contains("omitted")
        );
    }
    let mut lines = text
        .split_inclusive('\n')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    lines[99] = "// changed filler for sampled coverage\n".into();
    fs::write(&path, lines.concat()).unwrap();
    let fresh =
        prepare_local_onboarding_sampled(&fixture.resolution, &["input.rs".into()]).unwrap();
    assert_eq!(packet.sources[0].text, fresh.sources[0].text);
    assert_ne!(packet.sources[0].fingerprint, fresh.sources[0].fingerprint);
    assert_eq!(
        compile_local_onboarding(&fixture.resolution, &packet, &plan)
            .unwrap_err()
            .exit_code(),
        5
    );
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
fn sampled_onboarding_publishes_original_tail_evidence_and_replays_with_fresh_review() {
    use akasha_core::onboarding_local::{
        apply_local_onboarding, compile_local_onboarding, prepare_local_onboarding_sampled,
        preview_local_onboarding,
    };
    let fixture = Fixture::new("sampled-publication");
    let text =
        "// filler for a larger implementation\n".repeat(399) + "pub const TAIL: usize = 37;\n";
    fs::write(fixture.resolution.cwd.join("input.rs"), &text).unwrap();
    let paths = &["input.rs".into()];
    let packet = prepare_local_onboarding_sampled(&fixture.resolution, paths).unwrap();
    let draft = compile_local_onboarding(
        &fixture.resolution,
        &packet,
        &sampled_claim_plan().to_string(),
    )
    .unwrap();
    let proposal = serde_json::to_string(&draft.proposal).unwrap();
    let result = apply_local_onboarding(
        &fixture.resolution,
        &packet,
        &proposal,
        &draft.preview.preview_id,
    )
    .unwrap();
    assert_eq!(result.created_notes.len(), 1);
    validate_project(&fixture.resolution).unwrap();
    assert_eq!(
        fs::read_to_string(fixture.resolution.cwd.join("input.rs")).unwrap(),
        text
    );
    let fresh = prepare_local_onboarding_sampled(&fixture.resolution, paths).unwrap();
    let preview = preview_local_onboarding(&fixture.resolution, &fresh, &proposal).unwrap();
    let before = fixture.project_snapshot();
    let replay = apply_local_onboarding(
        &fixture.resolution,
        &fresh,
        &proposal,
        &preview.preview.preview_id,
    )
    .unwrap();
    assert!(replay.created_notes.is_empty());
    assert_eq!(replay.unchanged_notes.len(), 1);
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
fn sampled_file_bounds_and_discovery_preserve_default_complete_file_behavior() {
    use akasha_core::onboarding_discovery::{
        discover_local_onboarding, discover_local_onboarding_sampled,
    };
    use akasha_core::onboarding_local::{
        MAX_LOCAL_ONBOARDING_SAMPLED_FILE_BYTES, prepare_local_onboarding_sampled,
    };
    let fixture = Fixture::new("sampled-bounds");
    let repo = &fixture.resolution.cwd;
    fs::create_dir(repo.join("sources")).unwrap();
    let path = repo.join("sources/large.rs");
    let limit = MAX_LOCAL_ONBOARDING_SAMPLED_FILE_BYTES;
    fs::write(&path, "abc\n".repeat(limit / 4)).unwrap();
    let paths = &["sources/large.rs".into()];
    let before = fixture.project_snapshot();
    let packet = prepare_local_onboarding_sampled(&fixture.resolution, paths).unwrap();
    assert_eq!(
        packet.sources[0].sampling.as_ref().unwrap().total_bytes,
        limit
    );
    assert!(discover_local_onboarding(&fixture.resolution, &["sources".into()]).is_err());
    let discovered =
        discover_local_onboarding_sampled(&fixture.resolution, &["sources".into()]).unwrap();
    assert_eq!(discovered.packet, packet);
    fs::write(repo.join("sources/oversize.rs"), "x".repeat(limit + 1)).unwrap();
    let discovered =
        discover_local_onboarding_sampled(&fixture.resolution, &["sources".into()]).unwrap();
    assert!(
        discovered
            .coverage
            .omitted
            .iter()
            .any(|item| item.path == "sources/oversize.rs" && item.reason == "file_too_large")
    );
    for bytes in [
        vec![b'x'; limit + 1],
        vec![0xff; 9000],
        vec![0; 9000],
        ("ok\n".repeat(3000) + &"x".repeat(2049)).into_bytes(),
    ] {
        fs::write(&path, bytes).unwrap();
        assert_eq!(
            prepare_local_onboarding_sampled(&fixture.resolution, paths)
                .unwrap_err()
                .exit_code(),
            4
        );
    }
    fs::write(&path, "small complete source\n").unwrap();
    let small = prepare_local_onboarding_sampled(&fixture.resolution, paths).unwrap();
    assert!(small.sources[0].sampling.is_none());
    assert_eq!(small.sources[0].text, "small complete source\n");
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
fn discovery_balances_roles_and_directories_and_is_read_only() {
    use akasha_core::onboarding_discovery::discover_local_onboarding;
    use akasha_core::onboarding_local::prepare_local_onboarding;
    let fixture = Fixture::new("discovery-balanced");
    let repo = &fixture.resolution.cwd;
    for directory in ["src/alpha", "src/beta", "tests", "docs", "scripts"] {
        fs::create_dir_all(repo.join(directory)).unwrap();
    }
    for (path, text) in [
        ("README.md", "# Example\nComplete overview.\n"),
        ("src/alpha/a.rs", "fn alpha() {}\n"),
        ("src/alpha/b.rs", "fn extra() {}\n"),
        ("src/beta/b.rs", "fn beta() {}\n"),
        ("tests/check.rs", "#[test] fn check() {}\n"),
        ("docs/design.md", "# Design\nObserved behavior.\n"),
        ("scripts/check.sh", "cargo test\n"),
    ] {
        fs::write(repo.join(path), text).unwrap();
    }
    let before = fixture.project_snapshot();
    let result = discover_local_onboarding(&fixture.resolution, &[".".into()]).unwrap();
    assert_eq!(fixture.project_snapshot(), before);
    assert_eq!(result.coverage.selected.len(), 8);
    for role in [
        "overview",
        "manifest",
        "implementation",
        "tests",
        "documentation",
        "workflow",
    ] {
        assert!(result.coverage.selected.iter().any(|s| s.role == role));
    }
    assert!(
        result
            .coverage
            .selected
            .iter()
            .any(|s| s.path == "src/beta/b.rs")
    );
    let paths = result
        .packet
        .sources
        .iter()
        .map(|s| s.path.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        result.packet,
        prepare_local_onboarding(&fixture.resolution, &paths).unwrap()
    );
    assert_eq!(
        result.coverage.candidates,
        result.coverage.selected.len() + result.coverage.omitted.len()
    );
    fs::write(repo.join("src/alpha/c.rs"), "fn c() {}\n").unwrap();
    fs::write(repo.join("src/alpha/d.rs"), "fn d() {}\n").unwrap();
    let limited = discover_local_onboarding(&fixture.resolution, &[".".into()]).unwrap();
    assert!(
        limited
            .coverage
            .omitted
            .iter()
            .any(|s| s.reason == "file_limit")
    );
    fs::remove_file(repo.join("src/alpha/c.rs")).unwrap();
    fs::remove_file(repo.join("src/alpha/d.rs")).unwrap();
    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        serde_json::to_value(
            discover_local_onboarding(&fixture.resolution, &[".".into()]).unwrap()
        )
        .unwrap()
    );
    let scoped = discover_local_onboarding(&fixture.resolution, &["src/beta".into()]).unwrap();
    assert_eq!(scoped.packet.sources.len(), 1);
    assert_eq!(scoped.packet.sources[0].path, "src/beta/b.rs");
}

#[test]
fn discovery_reports_ineligible_files_and_keeps_packet_budget() {
    use akasha_core::onboarding_discovery::discover_local_onboarding;
    let fixture = Fixture::new("discovery-omissions");
    let repo = &fixture.resolution.cwd;
    fs::create_dir(repo.join("sample")).unwrap();
    for (name, bytes) in [
        ("README.md", vec![b'x'; 8193]),
        ("empty.rs", Vec::new()),
        ("binary.py", vec![255]),
        ("nul.rs", vec![0]),
        ("line.js", vec![b'x'; 2049]),
        ("large-a.rs", "// complete\n".repeat(600).into_bytes()),
        ("large-b.rs", "// complete\n".repeat(600).into_bytes()),
        ("small.rs", b"fn small() {}\n".to_vec()),
    ] {
        fs::write(repo.join("sample").join(name), bytes).unwrap();
    }
    let result = discover_local_onboarding(&fixture.resolution, &["sample".into()]).unwrap();
    for reason in [
        "file_too_large",
        "empty_or_overlong_line",
        "invalid_source",
        "packet_budget",
    ] {
        assert!(
            result.coverage.omitted.iter().any(|s| s.reason == reason),
            "missing {reason}"
        );
    }
    assert!(
        result
            .packet
            .sources
            .iter()
            .any(|s| s.path.ends_with("small.rs"))
    );
    assert!(serde_json::to_vec(&result.packet).unwrap().len() <= 16_000);
    assert_eq!(
        result.coverage.candidates,
        result.coverage.selected.len() + result.coverage.omitted.len()
    );
}

#[test]
fn discovery_refuses_ambiguous_scopes_and_scan_exhaustion_without_writes() {
    use akasha_core::onboarding_discovery::discover_local_onboarding;
    let fixture = Fixture::new("discovery-refusals");
    let repo = &fixture.resolution.cwd;
    fs::create_dir_all(repo.join("sample/child")).unwrap();
    let before = fixture.project_snapshot();
    for paths in [
        vec![],
        vec![".."],
        vec!["/tmp"],
        vec!["sample", "sample"],
        vec![".", "sample"],
        vec!["sample", "sample/child"],
        vec!["missing"],
        vec!["sample/../sample"],
        vec!["sample"],
    ] {
        assert!(
            discover_local_onboarding(
                &fixture.resolution,
                &paths.into_iter().map(str::to_owned).collect::<Vec<_>>()
            )
            .is_err()
        );
    }
    for index in 0..129 {
        fs::write(repo.join(format!("sample/{index}.rs")), "fn a() {}\n").unwrap();
    }
    assert!(
        discover_local_onboarding(&fixture.resolution, &["sample".into()])
            .unwrap_err()
            .to_string()
            .contains("128 candidates")
    );
    fs::create_dir(repo.join("entries")).unwrap();
    for index in 0..2049 {
        fs::write(repo.join(format!("entries/{index}.bin")), "").unwrap();
    }
    assert!(
        discover_local_onboarding(&fixture.resolution, &["entries".into()])
            .unwrap_err()
            .to_string()
            .contains("2048 entries")
    );
    fs::create_dir_all(repo.join("deep/a/b/c/d/e/f/g/h/i")).unwrap();
    assert!(
        discover_local_onboarding(&fixture.resolution, &["deep".into()])
            .unwrap_err()
            .to_string()
            .contains("8 directory levels")
    );
    assert_eq!(fixture.project_snapshot(), before);
}

#[test]
#[cfg(unix)]
fn discovery_never_reads_excluded_subtrees_or_symlinks() {
    use akasha_core::onboarding_discovery::discover_local_onboarding;
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new("discovery-exclusions");
    let repo = &fixture.resolution.cwd;
    for name in [".private", "node_modules", "target", "vendor", "secrets"] {
        fs::create_dir(repo.join(name)).unwrap();
        fs::write(repo.join(name).join("source.rs"), "MUST NOT BE SELECTED").unwrap();
    }
    fs::write(repo.join("credentials.py"), "MUST NOT BE SELECTED").unwrap();
    symlink("Cargo.toml", repo.join("link.rs")).unwrap();
    symlink("src", repo.join("alias")).unwrap();
    let result = discover_local_onboarding(&fixture.resolution, &[".".into()]).unwrap();
    assert_eq!(result.packet.sources.len(), 1);
    assert_eq!(result.packet.sources[0].path, "Cargo.toml");
    assert!(
        result
            .coverage
            .excluded_examples
            .iter()
            .any(|s| s.reason == "symlink")
    );
    assert!(
        !result
            .packet
            .sources
            .iter()
            .any(|s| s.text.contains("MUST NOT BE SELECTED"))
    );
    assert!(discover_local_onboarding(&fixture.resolution, &["alias".into()]).is_err());
}

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
fn local_approved_apply_binds_review_and_replays_only_after_fresh_review() {
    use akasha_core::onboarding_local::{apply_local_onboarding, preview_local_onboarding};
    let fixture = Fixture::new("local-approved-apply");
    let (packet, proposal) = local_draft_inputs(&fixture);
    let draft = preview_local_onboarding(&fixture.resolution, &packet, &proposal).unwrap();
    let before = fixture.project_snapshot();
    for id in ["", "yes", "sha256:unapproved"] {
        assert_eq!(
            apply_local_onboarding(&fixture.resolution, &packet, &proposal, id)
                .unwrap_err()
                .exit_code(),
            5
        );
        assert_eq!(fixture.project_snapshot(), before);
    }
    let mut changed: serde_json::Value = serde_json::from_str(&proposal).unwrap();
    changed["index"] = format!(
        "{}\nReviewed addition\n",
        changed["index"].as_str().unwrap()
    )
    .into();
    assert_eq!(
        apply_local_onboarding(
            &fixture.resolution,
            &packet,
            &changed.to_string(),
            &draft.preview.preview_id
        )
        .unwrap_err()
        .exit_code(),
        5
    );
    assert_eq!(fixture.project_snapshot(), before);
    let result = apply_local_onboarding(
        &fixture.resolution,
        &packet,
        &proposal,
        &draft.preview.preview_id,
    )
    .unwrap();
    assert_eq!(result.created_notes.len(), 1);
    validate_project(&fixture.resolution).unwrap();
    let applied = fixture.project_snapshot();
    assert!(
        apply_local_onboarding(
            &fixture.resolution,
            &packet,
            &proposal,
            &draft.preview.preview_id
        )
        .is_err()
    );
    assert_eq!(fixture.project_snapshot(), applied);
    let (fresh, _) = local_draft_inputs(&fixture);
    let rerun = preview_local_onboarding(&fixture.resolution, &fresh, &proposal).unwrap();
    assert_ne!(rerun.preview.preview_id, draft.preview.preview_id);
    let result = apply_local_onboarding(
        &fixture.resolution,
        &fresh,
        &proposal,
        &rerun.preview.preview_id,
    )
    .unwrap();
    assert!(result.created_notes.is_empty());
    assert_eq!(result.unchanged_notes.len(), 1);
    assert_eq!(fixture.project_snapshot(), applied);
}

#[test]
fn local_approved_apply_refuses_source_and_review_packet_drift_without_writes() {
    use akasha_core::onboarding_local::{apply_local_onboarding, preview_local_onboarding};
    for mode in ["source", "template", "projection", "packet"] {
        let fixture = Fixture::new(&format!("local-apply-drift-{mode}"));
        let (mut packet, proposal) = local_draft_inputs(&fixture);
        let draft = preview_local_onboarding(&fixture.resolution, &packet, &proposal).unwrap();
        match mode {
            "source" => fs::write(
                fixture.resolution.cwd.join("Cargo.toml"),
                "[package]\nname = \"changed\"\n",
            )
            .unwrap(),
            "template" => fs::write(
                fixture.project.join("templates/entity.md"),
                "human template change\n",
            )
            .unwrap(),
            "projection" => {
                fs::write(fixture.project.join("index.md"), "# Human index change\n").unwrap()
            }
            _ => packet.sources[0].text.push_str("altered"),
        }
        let before = fixture.project_snapshot();
        assert_eq!(
            apply_local_onboarding(
                &fixture.resolution,
                &packet,
                &proposal,
                &draft.preview.preview_id
            )
            .unwrap_err()
            .exit_code(),
            if mode == "projection" { 4 } else { 5 },
            "{mode}"
        );
        assert_eq!(fixture.project_snapshot(), before, "{mode}");
    }
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
            0 => altered.schema_version = 99,
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
            if which == 0 { 4 } else { 5 }
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
