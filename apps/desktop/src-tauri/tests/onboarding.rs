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
