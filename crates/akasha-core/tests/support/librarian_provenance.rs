use super::*;
use akasha_core::librarian::prepare_librarian;
use akasha_core::librarian_provenance::{
    LibrarianProvenance, inspect_librarian_provenance, review_librarian_provenance,
};
use serde_json::{Value, json};
use sha2::Digest;

const ID: &str = "Projects/alpha/entities/core.md";

fn reference(path: &str, text: &[u8], start: usize, end: usize) -> Value {
    let digest = sha2::Sha256::digest(text)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    json!({"path": path, "fingerprint": format!("sha256:{digest}"), "line_start": start, "line_end": end})
}

fn record(f: &Fixture, evidence: Option<Value>, body: &str) {
    let path = f.root.join(ID);
    let mut source = fs::read_to_string(&path).unwrap();
    if let Some(value) = evidence {
        source = source.replacen("\n---\n", &format!("\nevidence: {value}\n---\n"), 1);
    }
    fs::write(path, format!("{source}\n{body}\n")).unwrap();
    support::write_project_state(
        &f.root.join("Projects/alpha"),
        &[],
        &["entities/core.md"],
        &["records/tasks/ship-mvp.md"],
    );
}

fn inspect(
    f: &Fixture,
    quote: Option<&str>,
) -> Result<LibrarianProvenance, akasha_core::NoteLinksError> {
    let packet = prepare_librarian(&f.request, "core", &[], Some(ID)).unwrap();
    let mut selection = json!({"selections": [{"citation": 1}]});
    if let Some(quote) = quote {
        selection["selections"][0]["quote"] = quote.into();
    }
    inspect_librarian_provenance(&f.request, &packet, &selection.to_string())
}

#[test]
fn provenance_does_not_invent_evidence_for_ordinary_or_malformed_notes() {
    for (index, evidence) in [
        None,
        Some(json!("not evidence")),
        Some(json!([])),
        Some(json!([{"kind":"fact", "claim":"core", "sources":[]}])),
    ]
    .into_iter()
    .enumerate()
    {
        let f = fixture(&format!("provenance-schema-{index}"));
        record(&f, evidence, "core");
        let before = link_fixture_snapshot(&f.root);
        let report = inspect(&f, None).unwrap();
        assert_eq!(
            report.notes[0].evidence_state,
            if index == 0 {
                "not_recorded"
            } else {
                "invalid"
            }
        );
        assert!(report.notes[0].claims.is_empty());
        assert_eq!(before, link_fixture_snapshot(&f.root));
    }
}

#[test]
fn provenance_only_reads_unique_fully_selected_claims_and_preserves_unicode_crlf() {
    let f = fixture("provenance-quotes");
    let text = "// 世界\r\nconst LIMIT: usize = 37;\r\n";
    fs::write(f.request.cwd.join("limits.rs"), text).unwrap();
    record(
        &f,
        Some(json!([
            {"kind":"fact", "claim":"The limit is 37.", "sources":[reference("limits.rs", text.as_bytes(), 2, 2)]},
            {"kind":"fact", "claim":"Unselected claim.", "sources":[reference("missing.rs", b"", 1, 1)]}
        ])),
        "The limit is 37. Unselected claim.",
    );
    let report = inspect(&f, Some("The limit is 37.")).unwrap();
    assert_eq!(report.notes[0].unmatched_claims, 1);
    let evidence = &report.notes[0].claims[0].sources[0];
    assert_eq!(evidence.freshness, "unchanged");
    assert_eq!(
        evidence.excerpt.as_deref(),
        Some("const LIMIT: usize = 37;\r\n")
    );
    assert!(
        inspect(&f, Some("The limit")).unwrap().notes[0]
            .claims
            .is_empty()
    );
    let path = f.root.join(ID);
    fs::write(
        &path,
        fs::read_to_string(&path).unwrap() + "The limit is 37.\n",
    )
    .unwrap();
    support::write_project_state(
        &f.root.join("Projects/alpha"),
        &[],
        &["entities/core.md"],
        &["records/tasks/ship-mvp.md"],
    );
    let report = inspect(&f, None).unwrap();
    assert_eq!(report.notes[0].unmatched_claims, 1);
    assert_eq!(report.notes[0].claims.len(), 1); // only the other, unambiguous claim
}

#[test]
fn provenance_rejects_unsafe_references_without_exposing_foreign_bytes() {
    for (index, path) in [
        "../foreign.txt",
        "/tmp/foreign.txt",
        "./foreign.txt",
        "dir//foreign.txt",
        "dir\\foreign.txt",
    ]
    .into_iter()
    .enumerate()
    {
        let f = fixture(&format!("provenance-path-{index}"));
        fs::write(
            f.request.cwd.parent().unwrap().join("foreign.txt"),
            "FOREIGN_SECRET",
        )
        .unwrap();
        record(
            &f,
            Some(
                json!([{"kind":"fact", "claim":"A claim.", "sources":[reference(path, b"FOREIGN_SECRET", 1, 1)]}]),
            ),
            "A claim.",
        );
        let report = inspect(&f, None).unwrap();
        assert_eq!(
            report.notes[0].claims[0].sources[0].freshness,
            "invalid_reference"
        );
        assert!(
            !serde_json::to_string(&report)
                .unwrap()
                .contains("FOREIGN_SECRET")
        );
    }
}

#[cfg(unix)]
#[test]
fn provenance_rejects_symlink_components_and_nonregular_files() {
    use std::os::unix::fs::symlink;
    let f = fixture("provenance-symlinks");
    let foreign = f.request.cwd.parent().unwrap().join("foreign");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("secret.rs"), "FOREIGN_SECRET").unwrap();
    symlink(&foreign, f.request.cwd.join("linked")).unwrap();
    symlink(foreign.join("secret.rs"), f.request.cwd.join("file.rs")).unwrap();
    fs::create_dir(f.request.cwd.join("directory.rs")).unwrap();
    record(
        &f,
        Some(json!([{"kind":"fact", "claim":"A claim.", "sources":[
            reference("linked/secret.rs", b"FOREIGN_SECRET", 1, 1), reference("file.rs", b"FOREIGN_SECRET", 1, 1), reference("directory.rs", b"", 1, 1)
        ]}])),
        "A claim.",
    );
    let report = inspect(&f, None).unwrap();
    assert_eq!(
        report.notes[0].claims[0]
            .sources
            .iter()
            .map(|s| s.excerpt_state)
            .collect::<Vec<_>>(),
        ["symlink", "symlink", "not_regular"]
    );
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("FOREIGN_SECRET")
    );
}

#[test]
fn provenance_bounds_files_excerpts_and_line_ranges_without_clipping() {
    for (index, text, end, state) in [
        (0, "x".repeat(256 * 1024 + 1), 1, "exceeds_file_limit"),
        (1, "x".repeat(2_049), 1, "exceeds_excerpt_limit"),
        (2, "one\ntwo\n".into(), 3, "invalid_line_range"),
        (3, "one\ntwo\n".into(), 2, "complete"),
        (4, "x".repeat(256 * 1024), 1, "exceeds_excerpt_limit"),
        (5, "x".repeat(2_048), 1, "complete"),
    ] {
        let f = fixture(&format!("provenance-limits-{index}"));
        fs::write(f.request.cwd.join("source.rs"), &text).unwrap();
        record(
            &f,
            Some(
                json!([{"kind":"fact", "claim":"A claim.", "sources":[reference("source.rs", text.as_bytes(), 1, end)]}]),
            ),
            "A claim.",
        );
        let report = inspect(&f, None).unwrap();
        let source = &report.notes[0].claims[0].sources[0];
        assert_eq!(source.excerpt_state, state);
        assert_eq!(source.excerpt.is_some(), state == "complete");
    }
}

#[test]
fn provenance_separates_unreadable_content_invalid_references_and_missing_line_ranges() {
    for (index, bytes, bounds, expected) in [
        (0, vec![0xff], (1, 1), "invalid_utf8"),
        (1, b"contains\0nul".to_vec(), (1, 1), "contains_nul"),
        (2, b"x".to_vec(), (0, 1), "not_available"),
        (3, b"x".to_vec(), (2, 1), "not_available"),
        (4, b"x".to_vec(), (1, 1), "no_line_range"),
    ] {
        let f = fixture(&format!("provenance-unavailable-{index}"));
        fs::write(f.request.cwd.join("source.rs"), &bytes).unwrap();
        let mut source = reference("source.rs", &bytes, bounds.0, bounds.1);
        if index == 4 {
            source.as_object_mut().unwrap().remove("line_start");
            source.as_object_mut().unwrap().remove("line_end");
        }
        record(
            &f,
            Some(json!([{"kind":"fact", "claim":"A claim.", "sources":[source]}])),
            "A claim.",
        );
        let report = inspect(&f, None).unwrap();
        let evidence = &report.notes[0].claims[0].sources[0];
        assert_eq!(evidence.excerpt_state, expected);
        assert!(evidence.excerpt.is_none());
        assert_eq!(
            evidence.freshness,
            if index < 2 {
                "unavailable"
            } else if index < 4 {
                "invalid_reference"
            } else {
                "unchanged"
            }
        );
    }
}

#[test]
fn provenance_report_budget_refuses_repeated_large_windows_without_partial_output() {
    let f = fixture("provenance-report-limit");
    let text = format!("{}\n", "x".repeat(2_047)).repeat(16);
    fs::write(f.request.cwd.join("source.rs"), &text).unwrap();
    let references = (1..=16)
        .map(|line| reference("source.rs", text.as_bytes(), line, line))
        .collect::<Vec<_>>();
    let claims = (0..3)
        .map(|i| json!({"kind":"fact", "claim":format!("Claim {i}."), "sources":references}))
        .collect::<Vec<_>>();
    record(&f, Some(json!(claims)), "Claim 0. Claim 1. Claim 2.");
    assert!(
        inspect(&f, None)
            .unwrap_err()
            .to_string()
            .contains("65536 bytes")
    );
    let packet = prepare_librarian(&f.request, "core", &[], Some(ID)).unwrap();
    assert!(
        review_librarian_provenance(&f.request, &packet, r#"{"selections":[]}"#)
            .unwrap_err()
            .to_string()
            .contains("65536 bytes")
    );
}

#[test]
fn provenance_total_file_claim_and_reference_limits_fail_before_display() {
    for mode in ["files", "claims", "references"] {
        let f = fixture(&format!("provenance-total-{mode}"));
        let mut references = Vec::new();
        for index in 0..if mode == "files" { 9 } else { 1 } {
            let path = format!("source-{index}.rs");
            fs::write(f.request.cwd.join(&path), "x").unwrap();
            references.push(reference(&path, b"x", 1, 1));
        }
        if mode == "references" {
            references = vec![references[0].clone(); 13];
        }
        let claims = (0..if mode == "claims" {
            25
        } else if mode == "references" {
            4
        } else {
            1
        })
            .map(|i| json!({"kind":"fact", "claim":format!("Claim {i}."), "sources":references}))
            .collect::<Vec<_>>();
        let body = claims
            .iter()
            .map(|c| c["claim"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        record(&f, Some(json!(claims)), &body);
        let before = link_fixture_snapshot(&f.root);
        assert!(
            inspect(&f, None)
                .unwrap_err()
                .to_string()
                .contains("provenance exceeds")
        );
        let packet = prepare_librarian(&f.request, "core", &[], Some(ID)).unwrap();
        assert!(
            review_librarian_provenance(&f.request, &packet, r#"{"selections":[]}"#)
                .unwrap_err()
                .to_string()
                .contains("provenance exceeds")
        );
        assert_eq!(before, link_fixture_snapshot(&f.root));
    }
}

#[test]
fn complete_review_inspects_unhighlighted_claims_and_refuses_stale_or_invalid_inputs() {
    let f = fixture("complete-review-provenance");
    let text = "const LIMIT: usize = 37;\n";
    fs::write(f.request.cwd.join("limits.rs"), text).unwrap();
    record(
        &f,
        Some(json!([
            {"kind":"fact", "claim":"The limit is 37.", "sources":[reference("limits.rs", text.as_bytes(), 1, 1)]},
            {"kind":"unknown", "claim":"Deployment is unspecified.", "rationale":"No deployment evidence.", "sources":[]}
        ])),
        "The limit is 37. Deployment is unspecified.",
    );
    let packet = prepare_librarian(&f.request, "core", &[], Some(ID)).unwrap();
    let before = link_fixture_snapshot(&f.root);
    let empty = r#"{"selections":[]}"#;
    let review = review_librarian_provenance(&f.request, &packet, empty).unwrap();
    assert!(review.highlighted_citations.is_empty());
    assert_eq!(review.sources.matches.len(), packet.evidence.len());
    assert_eq!(review.omitted_notes, packet.omitted_notes);
    let provenance = review.provenance.unwrap();
    assert_eq!(provenance.notes.len(), packet.evidence.len());
    let claim = &provenance.notes[0].claims[0];
    assert_eq!(claim.recorded_kind, "fact");
    assert_eq!(claim.sources[0].excerpt.as_deref(), Some(text));
    assert_eq!(provenance.notes[0].claims[1].recorded_kind, "unknown");
    assert!(provenance.notes[0].claims[1].sources.is_empty());
    for selection in [
        r#"{"selections":[{"citation":99}]}"#,
        r#"{"selections":[{"citation":1},{"citation":1}]}"#,
        r#"{"selections":[{"citation":1,"quote":"The limit"}]}"#,
    ] {
        assert!(review_librarian_provenance(&f.request, &packet, selection).is_err());
    }
    fs::write(f.request.cwd.join("limits.rs"), "changed\n").unwrap();
    let review = review_librarian_provenance(&f.request, &packet, empty).unwrap();
    let reference = &review.provenance.unwrap().notes[0].claims[0].sources[0];
    assert_eq!(reference.freshness, "changed");
    assert!(reference.excerpt.is_none());
    let mut altered = packet;
    altered.evidence[0].excerpt.push_str("forged");
    assert!(review_librarian_provenance(&f.request, &altered, empty).is_err());
    assert_eq!(before, link_fixture_snapshot(&f.root));
}
