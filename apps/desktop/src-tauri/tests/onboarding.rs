use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::ResolveRequest;
use akasha_core::onboarding_local::{plan_local_onboarding, prepare_local_onboarding};
use akasha_desktop::onboarding::{
    DesktopSourceSelection, prepare_onboarding_sources, publish_onboarding_plan,
    review_onboarding_plan,
};
use akasha_desktop::{inspect_librarian_sources, open_librarian_source};
use serde_json::{Value, json};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    base: PathBuf,
    root: PathBuf,
    packet: String,
    plan: Value,
}
impl Fixture {
    fn new(version: u32) -> Self {
        let base = std::env::temp_dir().join(format!(
            "akasha-desktop-onboarding-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let root = base.join("valid-root");
        copy(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../tests/fixtures/resolution/valid-root"),
            &root,
        );
        fs::create_dir_all(base.join("repository")).unwrap();
        fs::write(
            base.join("repository/limits.rs"),
            "// Привет 🌘\r\npub const CAPACITY: usize = 37;\r\n",
        )
        .unwrap();
        let request =
            ResolveRequest::from_process(Some(root.clone()), Some("example".into())).unwrap();
        let packet = prepare_local_onboarding(&request, &["limits.rs".into()]).unwrap();
        let identities = plan_local_onboarding(&request, &packet).unwrap();
        let evidence = json!([{ "kind": "fact", "claim": "The capacity is 37.", "sources": [{"source": 1, "excerpt": 1}] }]);
        let plan = if version == 3 {
            json!({"schema_version": 3, "notes": [{"note_type": "entity", "name": "desktop-limits", "fields": {"entity": "desktop-limits", "kind": "source-group", "status": "draft", "reviewed": "unreviewed"}, "evidence": evidence}]})
        } else {
            let mut note = json!({"slot": 1, "evidence": evidence});
            if version == 4 {
                note["fields"] = serde_json::to_value(&identities.slots[0].fields).unwrap();
            }
            json!({"schema_version": version, "notes": [note]})
        };
        Self {
            base,
            root,
            packet: serde_json::to_string(&packet).unwrap(),
            plan,
        }
    }
    fn review(
        &self,
    ) -> Result<akasha_desktop::onboarding::DesktopOnboardingReview, akasha_desktop::DesktopError>
    {
        review_onboarding_plan(
            Some(self.root.clone()),
            "example".into(),
            &self.packet,
            &self.plan.to_string(),
        )
    }
    fn apply(
        &self,
        id: &str,
    ) -> Result<akasha_core::OnboardingBatchResult, akasha_desktop::DesktopError> {
        publish_onboarding_plan(
            Some(self.root.clone()),
            "example".into(),
            &self.packet,
            &self.plan.to_string(),
            id,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.base).unwrap();
    }
}
fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &to.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, path: &Path, output: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                walk(root, &entry.path(), output);
            } else {
                output.insert(
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut output = BTreeMap::new();
    walk(root, root, &mut output);
    output
}

#[test]
fn desktop_reviews_corrects_publishes_and_retrieves_all_excerpt_plan_versions() {
    for version in [3, 4, 5] {
        let mut fixture = Fixture::new(version);
        let before = snapshot(&fixture.root);
        let code_before = snapshot(&fixture.base.join("repository"));
        let original = fixture.review().unwrap();
        assert_eq!(
            original.excerpts[0].text,
            "// Привет 🌘\r\npub const CAPACITY: usize = 37;\r\n"
        );
        assert_eq!(original.excerpts[0].line_end, 2);
        assert_eq!(snapshot(&fixture.root), before);
        fixture.plan["notes"][0]["evidence"][0]["claim"] =
            "The queue capacity is exactly 37 entries.".into();
        let corrected = fixture.review().unwrap();
        assert_ne!(
            corrected.draft.preview.preview_id,
            original.draft.preview.preview_id
        );
        assert_eq!(
            fixture
                .apply(&original.draft.preview.preview_id)
                .unwrap_err()
                .code,
            5
        );
        assert_eq!(snapshot(&fixture.root), before);
        let result = fixture.apply(&corrected.draft.preview.preview_id).unwrap();
        assert_eq!(result.created_notes.len(), 1);
        let sources = inspect_librarian_sources(
            Some(fixture.root.clone()),
            "example".into(),
            "queue capacity",
            true,
        )
        .unwrap();
        assert!(
            sources
                .review
                .sources
                .matches
                .iter()
                .any(|source| source.quote.contains("exactly 37 entries"))
        );
        let passage = sources
            .review
            .sources
            .matches
            .iter()
            .find(|source| source.quote.contains("exactly 37 entries"))
            .unwrap();
        assert_eq!(passage.status.as_deref(), Some("draft"));
        let opened = open_librarian_source(
            Some(fixture.root.clone()),
            "example".into(),
            &sources.packet,
            passage.citation,
        )
        .unwrap();
        assert!(opened.document.source.contains("unreviewed"));
        assert_eq!(
            sources
                .review
                .provenance
                .as_ref()
                .unwrap()
                .notes
                .iter()
                .flat_map(|note| &note.claims)
                .next()
                .unwrap()
                .sources[0]
                .freshness,
            "unchanged"
        );
        assert_eq!(snapshot(&fixture.base.join("repository")), code_before);
        assert!(fixture.apply(&corrected.draft.preview.preview_id).is_err());
    }
}

#[test]
fn desktop_refuses_drift_tamper_foreign_scope_and_bounds_without_publication() {
    let fixture = Fixture::new(5);
    let draft = fixture.review().unwrap();
    let before = snapshot(&fixture.root);
    let invalid = "x".repeat(16001);
    assert_eq!(
        review_onboarding_plan(Some(fixture.root.clone()), "example".into(), &invalid, "{}")
            .unwrap_err()
            .code,
        4
    );
    assert!(
        review_onboarding_plan(
            Some(fixture.root.clone()),
            "example".into(),
            &fixture.packet,
            &invalid
        )
        .is_err()
    );
    assert!(
        review_onboarding_plan(
            Some(fixture.root.clone()),
            "other".into(),
            &fixture.packet,
            &fixture.plan.to_string()
        )
        .is_err()
    );
    let mut altered: Value = serde_json::from_str(&fixture.packet).unwrap();
    altered["sources"][0]["excerpts"][0]["byte_end"] = 999999.into();
    assert!(
        review_onboarding_plan(
            Some(fixture.root.clone()),
            "example".into(),
            &altered.to_string(),
            &fixture.plan.to_string()
        )
        .is_err()
    );
    assert!(fixture.apply("sha256:wrong").is_err());
    fs::write(
        fixture.base.join("repository/limits.rs"),
        "pub const CAPACITY: usize = 12;\n",
    )
    .unwrap();
    assert_eq!(fixture.review().unwrap_err().code, 5);
    assert_eq!(
        fixture
            .apply(&draft.draft.preview.preview_id)
            .unwrap_err()
            .code,
        5
    );
    assert_eq!(snapshot(&fixture.root), before);
}

#[test]
fn desktop_refuses_project_drift_invalid_claims_and_symlinked_sources() {
    let mut fixture = Fixture::new(5);
    let draft = fixture.review().unwrap();
    fixture.plan["notes"][0]["evidence"][0]["sources"][0]["source"] = 2.into();
    assert!(fixture.review().is_err());
    fixture.plan["notes"][0]["evidence"][0]["sources"][0]["source"] = 1.into();
    fs::write(
        fixture.root.join("Projects/example/index.md"),
        "# Changed project index\n",
    )
    .unwrap();
    let before = snapshot(&fixture.root);
    assert!(fixture.apply(&draft.draft.preview.preview_id).is_err());
    assert_eq!(snapshot(&fixture.root), before);
    // Preparing against symlinked evidence remains a core refusal, even for identical bytes.
    #[cfg(unix)]
    {
        let fixture = Fixture::new(5);
        let source = fixture.base.join("repository/limits.rs");
        fs::rename(&source, fixture.base.join("real.rs")).unwrap();
        std::os::unix::fs::symlink(fixture.base.join("real.rs"), source).unwrap();
        assert!(fixture.review().is_err());
    }
}

fn selection(mode: &str, paths: &[&str], focus: &[&str], sampling: bool) -> DesktopSourceSelection {
    DesktopSourceSelection {
        mode: mode.into(),
        paths: paths.iter().map(|s| (*s).into()).collect(),
        focus: focus.iter().map(|s| (*s).into()).collect(),
        sample_large_files: sampling,
    }
}

#[test]
fn desktop_selects_exact_sources_and_discovery_exposes_gaps_without_writes() {
    let fixture = Fixture::new(5);
    let repo = fixture.base.join("repository");
    for index in 0..10 {
        fs::create_dir(repo.join(format!("component-{index}"))).unwrap();
        fs::write(
            repo.join(format!("component-{index}/lib.rs")),
            format!("pub const ID: usize = {index};\n"),
        )
        .unwrap();
    }
    fs::write(repo.join(".secret.rs"), "secret example\n").unwrap();
    let before = snapshot(&fixture.root);
    let code_before = snapshot(&repo);
    let discovered = prepare_onboarding_sources(
        Some(fixture.root.clone()),
        "example".into(),
        selection("discover", &["."], &[], false),
    )
    .unwrap();
    assert_eq!(discovered.root, fixture.root);
    let coverage = discovered.coverage.unwrap();
    assert!(!coverage.omitted.is_empty());
    assert!(
        coverage
            .implementation_groups
            .iter()
            .any(|group| group.selected == 0)
    );
    assert!(
        coverage
            .excluded_examples
            .iter()
            .any(|entry| entry.path == ".secret.rs")
    );
    assert!(
        !discovered
            .excerpts
            .iter()
            .any(|excerpt| excerpt.path == ".secret.rs")
    );
    assert!(discovered.packet_json.len() <= 16_000);
    let prepared = prepare_onboarding_sources(
        Some(fixture.root.clone()),
        "example".into(),
        selection("files", &["limits.rs"], &[], false),
    )
    .unwrap();
    assert!(prepared.coverage.is_none());
    assert!(prepared.sampled_sources.is_empty());
    assert_eq!(prepared.packet_json, fixture.packet);
    assert_eq!(prepared.excerpts[0].line_start, 1);
    assert_eq!(prepared.identities.slots.len(), 1);
    let reviewed = review_onboarding_plan(
        Some(fixture.root.clone()),
        "example".into(),
        &prepared.packet_json,
        &fixture.plan.to_string(),
    )
    .unwrap();
    assert_eq!(reviewed.excerpts[0].text, prepared.excerpts[0].text);
    assert_eq!(snapshot(&fixture.root), before);
    assert_eq!(snapshot(&repo), code_before);
    publish_onboarding_plan(
        Some(fixture.root.clone()),
        "example".into(),
        &prepared.packet_json,
        &fixture.plan.to_string(),
        &reviewed.draft.preview.preview_id,
    )
    .unwrap();
    assert!(
        !inspect_librarian_sources(
            Some(fixture.root.clone()),
            "example".into(),
            "capacity",
            true
        )
        .unwrap()
        .review
        .sources
        .matches
        .is_empty()
    );
}

#[test]
fn desktop_source_selection_preserves_focused_sampling_and_refuses_changed_packet() {
    let fixture = Fixture::new(5);
    let repo = fixture.base.join("repository");
    let text = (1..=500)
        .map(|line| {
            if line == 82 {
                "pub const LEASE_SECONDS: usize = 37; // deadline\r\n".into()
            } else {
                format!("// ordinary multilingual 世界 filler {line}\r\n")
            }
        })
        .collect::<String>();
    fs::write(repo.join("limits.rs"), text).unwrap();
    let before = snapshot(&fixture.root);
    assert!(
        prepare_onboarding_sources(
            Some(fixture.root.clone()),
            "example".into(),
            selection("files", &["limits.rs"], &[], false)
        )
        .is_err()
    );
    let prepared = prepare_onboarding_sources(
        Some(fixture.root.clone()),
        "example".into(),
        selection("files", &["limits.rs"], &["deadline"], true),
    )
    .unwrap();
    let packet: Value = serde_json::from_str(&prepared.packet_json).unwrap();
    assert_eq!(packet["schema_version"], 4);
    assert_eq!(packet["focus"]["terms"], json!(["deadline"]));
    assert_eq!(prepared.sampled_sources.len(), 1);
    assert!(
        !prepared.sampled_sources[0]
            .sampling
            .omitted_lines
            .is_empty()
    );
    assert!(
        prepared
            .excerpts
            .iter()
            .any(|excerpt| excerpt.line_start <= 82
                && excerpt.line_end >= 82
                && excerpt.text.contains("LEASE_SECONDS"))
    );
    assert_eq!(snapshot(&fixture.root), before);
    fs::write(repo.join("limits.rs"), "// changed omitted lines\n").unwrap();
    assert!(
        review_onboarding_plan(
            Some(fixture.root.clone()),
            "example".into(),
            &prepared.packet_json,
            &fixture.plan.to_string()
        )
        .is_err()
    );
    assert_eq!(snapshot(&fixture.root), before);
}

#[test]
fn desktop_source_selection_refuses_scope_modes_limits_and_symlinks() {
    let fixture = Fixture::new(5);
    let before = snapshot(&fixture.root);
    for input in [
        selection("files", &["../outside.rs"], &[], false),
        selection("discover", &[".."], &[], false),
        selection("discover", &[".", "component"], &[], false),
        selection("invalid", &["limits.rs"], &[], false),
        selection("files", &[], &[], false),
        selection("files", &["limits.rs"; 9], &[], false),
        selection("files", &["limits.rs"], &[""], false),
    ] {
        assert!(
            prepare_onboarding_sources(Some(fixture.root.clone()), "example".into(), input)
                .is_err()
        );
    }
    assert!(
        prepare_onboarding_sources(
            Some(fixture.root.clone()),
            "foreign".into(),
            selection("files", &["limits.rs"], &[], false)
        )
        .is_err()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            fixture.base.join("repository"),
            fixture.base.join("repository/linked"),
        )
        .unwrap();
        assert!(
            prepare_onboarding_sources(
                Some(fixture.root.clone()),
                "example".into(),
                selection("discover", &["linked"], &[], false)
            )
            .is_err()
        );
        assert!(
            prepare_onboarding_sources(
                Some(fixture.root.clone()),
                "example".into(),
                selection("files", &["linked/limits.rs"], &[], false)
            )
            .is_err()
        );
    }
    assert_eq!(snapshot(&fixture.root), before);
}

/// Same synthetic publication/retrieval scenario for deterministic regression and explicit live
/// acceptance. Reviewer-authored corrections are kept separate from the original model capture.
async fn related_source_round_trip(live: bool) {
    use akasha_desktop::explore_librarian_sources;
    let fixture = Fixture::new(5);
    let repo = fixture.base.join("repository");
    fs::create_dir(repo.join("tests")).unwrap();
    fs::remove_file(repo.join("limits.rs")).unwrap();
    fs::write(repo.join("README.md"), "# Orchid delivery\nOrchid delivery retries are bounded.\nSee retry.rs for the policy and tests/retry.rs for concrete boundary assertions.\n").unwrap();
    fs::write(repo.join("retry.rs"), "pub fn delay_ms(attempt: u32) -> Option<u64> {\n    if attempt >= 3 { return None; }\n    Some(250 * (1_u64 << attempt))\n}\n").unwrap();
    fs::write(repo.join("tests/retry.rs"), "#[path = \"../retry.rs\"]\nmod retry;\n#[test]\nfn boundaries() {\n    assert_eq!(retry::delay_ms(0), Some(250));\n    assert_eq!(retry::delay_ms(1), Some(500));\n    assert_eq!(retry::delay_ms(2), Some(1000));\n    assert_eq!(retry::delay_ms(3), None);\n    assert_eq!(retry::delay_ms(u32::MAX), None);\n}\n").unwrap();
    let prepared = prepare_onboarding_sources(
        Some(fixture.root.clone()),
        "example".into(),
        selection(
            "files",
            &["README.md", "retry.rs", "tests/retry.rs"],
            &[],
            false,
        ),
    )
    .unwrap();
    let memory_before = snapshot(&fixture.root);
    let code_before = snapshot(&repo);
    let generated = if live {
        Some(
            akasha_desktop::onboarding_generation::generate_onboarding(
                Some(fixture.root.clone()),
                "example".into(),
                prepared.packet_json.clone(),
                akasha_desktop::inference::Profile::Vulkan,
                akasha_desktop::inference::Cancellation::new(),
            )
            .await
            .unwrap(),
        )
    } else {
        None
    };
    let identity = |role: &str| {
        let slot = prepared
            .identities
            .slots
            .iter()
            .find(|s| s.role == role)
            .unwrap();
        format!("Projects/example/{}", slot.path.display())
            .trim_end_matches(".md")
            .to_string()
    };
    let mut notes = Vec::new();
    for slot in &prepared.identities.slots {
        let (claim, path) = match slot.role.as_str() {
            "context" => (format!("Orchid delivery retries are bounded; see [[{}|retry policy]] for behavior and its verification link.", identity("implementation")), "README.md"),
            "implementation" => (format!("delay_ms returns 250, 500 and 1000 milliseconds for attempts 0, 1 and 2; attempts >= 3 return None. See [[{}|boundary assertions]].", identity("verification")), "retry.rs"),
            "verification" => ("The boundary test asserts Some(250), Some(500), Some(1000), None and None for inputs 0, 1, 2, 3 and u32::MAX respectively.".into(), "tests/retry.rs"),
            _ => unreachable!(),
        };
        let packet: Value = serde_json::from_str(&prepared.packet_json).unwrap();
        let handle = packet["sources"]
            .as_array()
            .unwrap()
            .iter()
            .position(|s| s["path"] == path)
            .unwrap()
            + 1;
        notes.push(json!({"slot":slot.slot,"evidence":[{"kind":"fact","claim":claim,"sources":[{"source":handle,"excerpt":1}]}]}));
    }
    let corrected = json!({"schema_version":5,"notes":notes}).to_string();
    let review = review_onboarding_plan(
        Some(fixture.root.clone()),
        "example".into(),
        &prepared.packet_json,
        &corrected,
    )
    .unwrap();
    assert_eq!(snapshot(&fixture.root), memory_before);
    let published = publish_onboarding_plan(
        Some(fixture.root.clone()),
        "example".into(),
        &prepared.packet_json,
        &corrected,
        &review.draft.preview.preview_id,
    )
    .unwrap();
    assert_eq!(published.created_notes.len(), 3);
    let memory_after = snapshot(&fixture.root);
    let initial =
        inspect_librarian_sources(Some(fixture.root.clone()), "example".into(), "Orchid", true)
            .unwrap();
    assert_eq!(initial.packet.evidence.len(), 1);
    assert!(
        !initial
            .packet
            .evidence
            .iter()
            .any(|e| e.excerpt.contains("1000 milliseconds"))
    );
    let related = explore_librarian_sources(
        Some(fixture.root.clone()),
        "example".into(),
        &initial.packet,
        1,
        true,
    )
    .unwrap();
    let behavior = related
        .packet
        .evidence
        .iter()
        .find(|e| e.excerpt.contains("1000 milliseconds"))
        .unwrap();
    assert!(behavior.excerpt.contains("attempts >= 3 return None"));
    let boundaries = explore_librarian_sources(
        Some(fixture.root.clone()),
        "example".into(),
        &related.packet,
        behavior.citation,
        true,
    )
    .unwrap();
    let assertion = boundaries
        .packet
        .evidence
        .iter()
        .find(|e| e.excerpt.contains("u32::MAX respectively"))
        .unwrap();
    let opened = open_librarian_source(
        Some(fixture.root.clone()),
        "example".into(),
        &boundaries.packet,
        assertion.citation,
    )
    .unwrap();
    assert!(opened.document.source.contains("u32::MAX respectively"));
    assert_eq!(initial.packet.question, boundaries.packet.question);
    assert!(
        boundaries
            .review
            .provenance
            .as_ref()
            .unwrap()
            .notes
            .iter()
            .flat_map(|n| &n.claims)
            .flat_map(|c| &c.sources)
            .all(|s| s.freshness == "unchanged")
    );
    assert_eq!(snapshot(&fixture.root), memory_after);
    assert_eq!(snapshot(&repo), code_before);
    assert!(
        explore_librarian_sources(
            Some(fixture.root.clone()),
            "foreign".into(),
            &initial.packet,
            1,
            false
        )
        .is_err()
    );
    if live {
        let evidence =
            std::env::temp_dir().join(format!("akasha-related-acceptance-{}", std::process::id()));
        fs::create_dir(&evidence).unwrap();
        for (name, value) in [
            ("prepared", serde_json::to_value(&prepared).unwrap()),
            ("generated", serde_json::to_value(generated).unwrap()),
            ("corrected-plan", serde_json::from_str(&corrected).unwrap()),
            ("review", serde_json::to_value(review).unwrap()),
            ("published", serde_json::to_value(published).unwrap()),
            ("initial", serde_json::to_value(initial).unwrap()),
            ("related", serde_json::to_value(related).unwrap()),
            ("boundaries", serde_json::to_value(&boundaries).unwrap()),
            ("opened", serde_json::to_value(&opened).unwrap()),
        ] {
            fs::write(
                evidence.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&value).unwrap(),
            )
            .unwrap();
        }
        copy(&repo, &evidence.join("repository"));
        copy(&fixture.root, &evidence.join("memory"));
        println!(
            "Native draft → reviewed publication → two-hop librarian PASS; evidence {}",
            evidence.display()
        );
    }
    // A later saved-note edit invalidates the original displayed packet; neither expansion nor
    // navigation repairs/recover/writes around it.
    let anchor = fixture.root.join(&boundaries.packet.evidence[0].id);
    let source = fs::read_to_string(&anchor).unwrap();
    fs::write(anchor, format!("{source}\nChanged saved source.\n")).unwrap();
    let changed = snapshot(&fixture.root);
    assert!(
        explore_librarian_sources(
            Some(fixture.root.clone()),
            "example".into(),
            &boundaries.packet,
            assertion.citation,
            false
        )
        .is_err()
    );
    assert_eq!(snapshot(&fixture.root), changed);
}

#[tokio::test]
async fn desktop_onboarding_feeds_two_hop_related_source_exploration() {
    related_source_round_trip(false).await;
}

#[tokio::test]
#[ignore = "requires installed selected Qwen9B, host loopback and Vulkan device"]
async fn native_onboarding_feeds_two_hop_related_source_exploration() {
    related_source_round_trip(true).await;
}

/// Explicit host acceptance: uses only synthetic memory/repository and the already installed
/// selected main model. Never runs as part of unattended workspace tests.
#[tokio::test]
#[ignore = "requires installed selected Qwen9B, host loopback and Vulkan device"]
async fn native_main_model_generates_reviewed_memory_and_librarian_navigation() {
    use akasha_desktop::inference::{Cancellation, Profile};
    use akasha_desktop::onboarding_generation::generate_onboarding;
    let fixture = Fixture::new(5);
    let repo = fixture.base.join("repository");
    fs::create_dir(repo.join("tests")).unwrap();
    fs::write(repo.join("README.md"), "# Parcel queue\nA bounded queue for local parcel dispatch.\nDuplicates are rejected before the capacity check.\n").unwrap();
    fs::write(repo.join("limits.rs"), "pub const CAPACITY: usize = 37;\npub fn enqueue(ids: &mut Vec<u64>, id: u64) -> Result<(), &'static str> {\n    if ids.contains(&id) { return Err(\"duplicate\"); }\n    if ids.len() >= CAPACITY { return Err(\"full\"); }\n    ids.push(id);\n    Ok(())\n}\n").unwrap();
    fs::write(repo.join("tests/queue.rs"), "#[test]\nfn rejects_duplicate_before_full() {\n    let mut ids = vec![5; 37];\n    assert_eq!(enqueue(&mut ids, 5), Err(\"duplicate\"));\n    assert_eq!(enqueue(&mut ids, 7), Err(\"full\"));\n}\n").unwrap();
    let source = prepare_onboarding_sources(
        Some(fixture.root.clone()),
        "example".into(),
        selection(
            "files",
            &["README.md", "limits.rs", "tests/queue.rs"],
            &[],
            false,
        ),
    )
    .unwrap();
    let before = snapshot(&fixture.root);
    let code_before = snapshot(&repo);
    let generated = generate_onboarding(
        Some(fixture.root.clone()),
        "example".into(),
        source.packet_json.clone(),
        Profile::Vulkan,
        Cancellation::new(),
    )
    .await
    .unwrap();
    assert_eq!(generated.metrics.len(), 3);
    assert_eq!(generated.review.draft.proposal.notes.len(), 3);
    assert_eq!(snapshot(&fixture.root), before);
    assert_eq!(snapshot(&repo), code_before);
    // Preserve raw model output. Correct and publish separately after direct source review.
    let mut corrected: Value = serde_json::from_str(&generated.plan_json).unwrap();
    for note in corrected["notes"].as_array_mut().unwrap() {
        let slot = source
            .identities
            .slots
            .iter()
            .find(|s| s.slot == note["slot"].as_u64().unwrap() as usize)
            .unwrap();
        let (claim, handle) = match slot.role.as_str() {
            "context" => (
                "The README describes a bounded queue for local parcel dispatch.",
                1,
            ),
            "implementation" => (
                "The queue capacity is 37; enqueue rejects duplicate IDs before checking whether the queue is full.",
                2,
            ),
            "verification" => (
                "The test fills 37 entries with ID 5, then expects duplicate for ID 5 and full for ID 7.",
                3,
            ),
            _ => panic!("unknown role"),
        };
        note["evidence"] =
            json!([{"kind":"fact","claim":claim,"sources":[{"source":handle,"excerpt":1}]}]);
    }
    let plan = corrected.to_string();
    let review = review_onboarding_plan(
        Some(fixture.root.clone()),
        "example".into(),
        &source.packet_json,
        &plan,
    )
    .unwrap();
    assert_eq!(snapshot(&fixture.root), before);
    assert!(
        publish_onboarding_plan(
            Some(fixture.root.clone()),
            "example".into(),
            &source.packet_json,
            &plan,
            "sha256:stale"
        )
        .is_err()
    );
    let published = publish_onboarding_plan(
        Some(fixture.root.clone()),
        "example".into(),
        &source.packet_json,
        &plan,
        &review.draft.preview.preview_id,
    )
    .unwrap();
    assert_eq!(published.created_notes.len(), 3);
    let after = snapshot(&fixture.root);
    let found = inspect_librarian_sources(
        Some(fixture.root.clone()),
        "example".into(),
        "queue capacity duplicate",
        true,
    )
    .unwrap();
    let passage = found
        .review
        .sources
        .matches
        .iter()
        .find(|source| source.quote.contains("queue capacity is 37"))
        .unwrap();
    let opened = open_librarian_source(
        Some(fixture.root.clone()),
        "example".into(),
        &found.packet,
        passage.citation,
    )
    .unwrap();
    assert!(opened.document.source.contains("queue capacity is 37"));
    assert_eq!(passage.status.as_deref(), Some("draft"));
    assert_eq!(snapshot(&fixture.root), after);
    assert_eq!(snapshot(&repo), code_before);
    let evidence =
        std::env::temp_dir().join(format!("akasha-native-acceptance-{}", std::process::id()));
    fs::create_dir(&evidence).unwrap();
    for (name, value) in [
        ("sources", serde_json::to_value(source).unwrap()),
        ("generated", serde_json::to_value(generated).unwrap()),
        ("corrected-plan", corrected),
        ("review", serde_json::to_value(review).unwrap()),
        ("published", serde_json::to_value(published).unwrap()),
        ("librarian", serde_json::to_value(found).unwrap()),
        ("opened", serde_json::to_value(opened).unwrap()),
    ] {
        fs::write(
            evidence.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
    println!(
        "Native onboarding → reviewed publication → librarian PASS; evidence {}",
        evidence.display()
    );
}

#[tokio::test]
async fn native_assistance_refuses_invalid_or_cancelled_requests_before_artifact_access() {
    use akasha_desktop::inference::{Cancellation, Profile};
    use akasha_desktop::librarian_expansion::assist_librarian;
    let missing = Some(PathBuf::from("/does-not-exist/native-assistance"));
    let invalid = assist_librarian(
        missing.clone(),
        "foreign".into(),
        " ".into(),
        false,
        Profile::Vulkan,
        Cancellation::new(),
    )
    .await
    .unwrap_err();
    assert!(invalid.message.contains("1..160"));
    let cancelled = Cancellation::new();
    cancelled.cancel();
    let failure = assist_librarian(
        missing,
        "foreign".into(),
        "queue".into(),
        false,
        Profile::Vulkan,
        cancelled,
    )
    .await
    .unwrap_err();
    assert!(failure.message.contains("cancelled"));
}

/// Frozen multilingual/paraphrase coverage, one unchanged question-only completion per case.
#[tokio::test]
#[ignore = "requires installed selected Qwen9B, host loopback and Vulkan device"]
async fn native_librarian_recovers_multilingual_memory_after_reviewed_onboarding() {
    use akasha_desktop::inference::{Cancellation, Profile};
    use akasha_desktop::librarian_expansion::assist_librarian;
    let mut fixture = Fixture::new(3);
    let repo = fixture.base.join("repository");
    let sources = [
        (
            "limits.rs",
            "pub const CAPACITY: usize = 37;\npub fn enqueue(ids: &mut Vec<u64>, id: u64) -> Result<(), &'static str> {\n    if ids.contains(&id) { return Err(\"duplicate\"); }\n    if ids.len() >= CAPACITY { return Err(\"full\"); }\n    ids.push(id); Ok(())\n}\n",
        ),
        (
            "worker.rs",
            "pub const LEASE_SECONDS: u64 = 45;\npub const RETRY_LIMIT: u32 = 3;\n",
        ),
        (
            "README.md",
            "# Parcel service\nCI runs cargo fmt, cargo clippy, and cargo test before integration.\n",
        ),
    ];
    for (name, source) in sources {
        fs::write(repo.join(name), source).unwrap();
    }
    let request =
        ResolveRequest::from_process(Some(fixture.root.clone()), Some("example".into())).unwrap();
    let packet = prepare_local_onboarding(
        &request,
        &["limits.rs".into(), "worker.rs".into(), "README.md".into()],
    )
    .unwrap();
    fixture.packet = serde_json::to_string(&packet).unwrap();
    fixture.plan = json!({"schema_version":3,"notes":[
        {"name":"parcel-queue","note_type":"entity","fields":{"entity":"parcel-queue","kind":"source-group","status":"draft","reviewed":"unreviewed"},"evidence":[{"kind":"fact","claim":"The queue capacity is 37; enqueue rejects duplicate IDs before checking whether the queue is full.","sources":[{"source":1,"excerpt":1}]}]},
        {"name":"parcel-worker","note_type":"entity","fields":{"entity":"parcel-worker","kind":"source-group","status":"draft","reviewed":"unreviewed"},"evidence":[{"kind":"fact","claim":"The configured lease expires after 45 seconds and the retry limit is 3.","sources":[{"source":2,"excerpt":1}]}]},
        {"name":"parcel-checks","note_type":"entity","fields":{"entity":"parcel-checks","kind":"source-group","status":"draft","reviewed":"unreviewed"},"evidence":[{"kind":"fact","claim":"The README says CI runs cargo fmt, cargo clippy, and cargo test before integration.","sources":[{"source":3,"excerpt":1}]}]}
    ]});
    // Reviewer-authored memory, independently checked against source; no inference approval.
    // Constants alone describe configuration, not enforcement by a missing worker implementation.
    let review = fixture.review().unwrap();
    let published = fixture.apply(&review.draft.preview.preview_id).unwrap();
    let before = snapshot(&fixture.root);
    let code_before = snapshot(&repo);
    let evidence = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../akasha/Research/evidence/2026-11-10-native-librarian");
    let criteria: Value =
        serde_json::from_slice(&fs::read(evidence.join("criteria.json")).unwrap()).unwrap();
    fs::write(
        evidence.join("reviewed-onboarding.json"),
        serde_json::to_vec_pretty(
            &json!({"packet":packet,"plan":fixture.plan,"review":review,"published":published}),
        )
        .unwrap(),
    )
    .unwrap();
    let mut results = Vec::new();
    for case in criteria["cases"].as_array().unwrap() {
        let started = std::time::Instant::now();
        let question = case["question"].as_str().unwrap();
        let found = assist_librarian(
            Some(fixture.root.clone()),
            "example".into(),
            question.into(),
            true,
            Profile::Vulkan,
            Cancellation::new(),
        )
        .await
        .unwrap();
        let initial = found
            .initial
            .review
            .sources
            .matches
            .iter()
            .map(|p| p.quote.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let expanded = found
            .expanded
            .as_ref()
            .map(|p| {
                p.review
                    .sources
                    .matches
                    .iter()
                    .map(|p| p.quote.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let spans = case["required"].as_array().unwrap();
        let initial_hit = spans.iter().all(|s| initial.contains(s.as_str().unwrap()));
        let retained = format!("{initial}\n{expanded}");
        let assisted_hit = spans.iter().all(|s| retained.contains(s.as_str().unwrap()));
        let mut opened = Vec::new();
        for span in spans {
            let span = span.as_str().unwrap();
            if let Some((report, passage)) = std::iter::once(&found.initial)
                .chain(found.expanded.as_ref())
                .find_map(|r| {
                    r.review
                        .sources
                        .matches
                        .iter()
                        .find(|p| p.quote.contains(span))
                        .map(|p| (r, p))
                })
            {
                let document = open_librarian_source(
                    Some(fixture.root.clone()),
                    "example".into(),
                    &report.packet,
                    passage.citation,
                )
                .unwrap();
                assert!(document.document.source.contains(span));
                assert_eq!(passage.status.as_deref(), Some("draft"));
                opened.push(serde_json::to_value(document).unwrap());
            }
        }
        assert_eq!(snapshot(&fixture.root), before);
        assert_eq!(snapshot(&repo), code_before);
        let row = json!({"id":case["id"],"initial_hit":initial_hit,"assisted_hit":assisted_hit,"ms":started.elapsed().as_millis(),"terms":found.terms,"opened":opened.len()});
        println!("{row}");
        fs::write(
            evidence.join(format!("{}.json", case["id"].as_str().unwrap())),
            serde_json::to_vec_pretty(
                &json!({"case":case,"score":row,"result":found,"opened":opened}),
            )
            .unwrap(),
        )
        .unwrap();
        results.push(row);
    }
    let recovered = results[..6]
        .iter()
        .filter(|r| r["initial_hit"] == false && r["assisted_hit"] == true)
        .count();
    let supported = results[..8]
        .iter()
        .filter(|r| r["assisted_hit"] == true)
        .count();
    let summary = json!({"results":results,"paraphrases_recovered":recovered,"supported_cases":supported,"source_snapshots_unchanged":true,"initial_packets_preserved":true});
    fs::write(
        evidence.join("summary.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();
    println!("{summary}");
    assert!(
        recovered >= 4,
        "frozen recovery gate failed; preserve raw results, do not tune/retry"
    );
    assert_eq!(supported, 8);
}

/// Explicit model-free replay of the preserved live misses; no inference call or tuning.
#[test]
#[ignore = "requires the preserved native librarian live acceptance capture"]
fn native_librarian_repairs_recorded_misses_with_reviewed_search_words() {
    use akasha_desktop::inspect_librarian_sources_with_terms;
    let evidence = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../akasha/Research/evidence/2026-11-10-native-librarian");
    let saved: Value =
        serde_json::from_slice(&fs::read(evidence.join("reviewed-onboarding.json")).unwrap())
            .unwrap();
    let mut fixture = Fixture::new(3);
    let repo = fixture.base.join("repository");
    let mut paths = Vec::new();
    for source in saved["packet"]["sources"].as_array().unwrap() {
        let path = source["path"].as_str().unwrap();
        fs::write(repo.join(path), source["text"].as_str().unwrap()).unwrap();
        paths.push(path.to_string());
    }
    let request =
        ResolveRequest::from_process(Some(fixture.root.clone()), Some("example".into())).unwrap();
    fixture.packet =
        serde_json::to_string(&prepare_local_onboarding(&request, &paths).unwrap()).unwrap();
    fixture.plan = saved["plan"].clone();
    fixture
        .apply(&fixture.review().unwrap().draft.preview.preview_id)
        .unwrap();
    let before = snapshot(&fixture.root);
    let code_before = snapshot(&repo);
    let mut repairs = Vec::new();
    for (case, term, span) in [
        ("lease-en", "lease", "lease expires after 45 seconds"),
        ("retry-ru", "retry", "retry limit is 3"),
    ] {
        let raw: Value =
            serde_json::from_slice(&fs::read(evidence.join(format!("{case}.json"))).unwrap())
                .unwrap();
        assert_eq!(raw["score"]["assisted_hit"], false);
        let question = raw["case"]["question"].as_str().unwrap();
        let report = inspect_librarian_sources_with_terms(
            Some(fixture.root.clone()),
            "example".into(),
            question,
            true,
            &[term.into()],
        )
        .unwrap();
        assert_eq!(report.packet.question, question);
        assert_eq!(report.packet.terms, vec![term]);
        let passage = report
            .review
            .sources
            .matches
            .iter()
            .find(|p| p.quote.contains(span))
            .unwrap();
        let opened = open_librarian_source(
            Some(fixture.root.clone()),
            "example".into(),
            &report.packet,
            passage.citation,
        )
        .unwrap();
        assert!(opened.document.source.contains(span));
        assert_eq!(snapshot(&fixture.root), before);
        assert_eq!(snapshot(&repo), code_before);
        repairs.push(json!({"id":case,"operator_terms":[term],"question":question,"report":report,"opened":opened}));
    }
    fs::write(
        evidence.join("manual-repairs.json"),
        serde_json::to_vec_pretty(
            &json!({"model_calls":0,"source_snapshots_unchanged":true,"repairs":repairs}),
        )
        .unwrap(),
    )
    .unwrap();
}
