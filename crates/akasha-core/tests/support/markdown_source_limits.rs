use super::*;
use akasha_core::{MAX_MARKDOWN_SOURCE_BYTES, assemble_context, validate_project};

#[test]
fn source_limits_preserve_exact_boundary_notes_and_projections_and_refuse_excess() {
    let fixture = fixture("source-limits");
    let project = fixture.root.join("Projects/alpha");
    for id in [
        "Projects/alpha/entities/core.md",
        "Global/entities/rust-pattern.md",
        "Projects/alpha/index.md",
        "Projects/alpha/roadmap.md",
    ] {
        let path = fixture.root.join(id);
        let original = fs::read(&path).unwrap();
        let mut source = original.clone();
        source.extend_from_slice("\nExact ☃ CRLF  \r\n".as_bytes());
        source.resize(MAX_MARKDOWN_SOURCE_BYTES, b' ');
        fs::write(&path, &source).unwrap();
        support::write_project_state(
            &project,
            &[],
            &["entities/core.md"],
            &["records/tasks/ship-mvp.md"],
        );
        validate_project(&fixture.request).unwrap();
        build_library_projection(&fixture.request).unwrap();
        if id.contains("/entities/") {
            assert_eq!(
                load_library_document(&fixture.request, id)
                    .unwrap()
                    .source
                    .as_bytes(),
                source
            );
        } else {
            // Projection source is used without trimming/truncation on disk; bounded context
            // may omit it, while exact graph inspection must still succeed.
            assemble_context(&fixture.request).unwrap();
            inspect_note_links(&fixture.request, id, NoteLinkDirection::Outgoing, 30).unwrap();
        }
        let before = link_fixture_snapshot(&fixture.root);
        source.push(0xff); // Excess classification must precede sentinel UTF-8 decoding/state drift.
        fs::write(&path, &source).unwrap();
        let excessive = link_fixture_snapshot(&fixture.root);
        for error in [
            build_library_projection(&fixture.request).unwrap_err(),
            load_library_document(&fixture.request, "Projects/alpha/entities/core.md").unwrap_err(),
            akasha_core::search_library(&fixture.request, "core", None, 30).unwrap_err(),
        ] {
            assert_eq!(error.exit_code(), 4);
            let message = error.to_string();
            assert!(message.contains("16777216-byte"), "{message}");
            assert!(message.contains(id), "{message}");
            assert!(!message.contains("UTF-8"));
        }
        assert_eq!(
            inspect_note_links(
                &fixture.request,
                "Projects/alpha/entities/core.md",
                NoteLinkDirection::Incoming,
                30
            )
            .unwrap_err()
            .exit_code(),
            4
        );
        assert_eq!(link_fixture_snapshot(&fixture.root), excessive);
        fs::write(&path, &source[..MAX_MARKDOWN_SOURCE_BYTES]).unwrap();
        assert_eq!(link_fixture_snapshot(&fixture.root), before);
        fs::write(&path, original).unwrap();
    }
}

#[test]
fn source_limits_preserve_earlier_validation_failure_and_tiny_budget_precedence() {
    let fixture = fixture("source-limit-errors");
    let path = fixture.root.join("Projects/alpha/entities/core.md");
    let mut oversized = vec![b'x'; MAX_MARKDOWN_SOURCE_BYTES + 1];
    oversized[0] = 0xff;
    fs::write(&path, oversized).unwrap();
    let before = link_fixture_snapshot(&fixture.root);
    let error = akasha_core::assemble_context_with_budget(&fixture.request, 1).unwrap_err();
    assert!(error.to_string().contains("16777216-byte"));
    // The entity folder is visited before task. An in-bound malformed entity sorts before core.
    let earlier = fixture.root.join("Projects/alpha/entities/aaa.md");
    fs::write(&earlier, b"invalid note without frontmatter").unwrap();
    let error = build_library_projection(&fixture.request).unwrap_err();
    assert!(error.to_string().contains("aaa.md"));
    assert!(!error.to_string().contains("16777216-byte"));
    fs::remove_file(earlier).unwrap();
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}
