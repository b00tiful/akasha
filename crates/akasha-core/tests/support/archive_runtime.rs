use super::*;
use akasha_core::{inspect_pending_note_edit, search_library};
use std::time::Instant;

#[test]
fn long_unicode_paragraphs_preserve_archive_graph_search_and_source() {
    let fixture = fixture("long-paragraphs");
    let paragraph = "Synthetic Unicode ☃ 世界 paragraph. ".repeat(256);
    let project = fixture.root.join("Projects/alpha/entities/core.md");
    let global = fixture.root.join("Global/entities/rust-pattern.md");
    for path in [&project, &global] {
        let original = fs::read_to_string(path).unwrap();
        fs::write(
            path,
            format!(
                "{original}\n{paragraph} archive-needle [[Projects/alpha/entities/core|Ядро]]\r\n"
            ),
        )
        .unwrap();
    }
    support::write_project_state(
        &fixture.root.join("Projects/alpha"),
        &[],
        &["entities/core.md"],
        &["records/tasks/ship-mvp.md"],
    );
    let before = link_fixture_snapshot(&fixture.root);
    let projection = build_library_projection(&fixture.request).unwrap();
    assert_eq!(projection.total_books, 4);
    let result = search_library(&fixture.request, "archive-needle", None, 30).unwrap();
    assert_eq!(result.total_matches, 2);
    assert!(!result.truncated);
    assert_eq!(
        result
            .hits
            .iter()
            .map(|hit| hit.id.as_str())
            .collect::<Vec<_>>(),
        [
            "Global/entities/rust-pattern.md",
            "Projects/alpha/entities/core.md"
        ]
    );
    let links = inspect_note_links(
        &fixture.request,
        "Projects/alpha/entities/core.md",
        NoteLinkDirection::Incoming,
        30,
    )
    .unwrap();
    assert_eq!(links.total_matches, 2);
    for id in [
        "Projects/alpha/entities/core.md",
        "Global/entities/rust-pattern.md",
    ] {
        let document = load_library_document(&fixture.request, id).unwrap();
        assert_eq!(
            document.source.as_bytes(),
            fs::read(fixture.root.join(id)).unwrap()
        );
    }
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}

#[test]
fn oversized_metadata_fails_library_and_search_without_writes() {
    let fixture = fixture("yaml-resource-refusal");
    let path = fixture.root.join("Global/entities/rust-pattern.md");
    fs::write(
        &path,
        format!("---\nschema_version: 1\n#{}\n---\n", "x".repeat(65_536)),
    )
    .unwrap();
    let before = link_fixture_snapshot(&fixture.root);
    for result in [
        build_library_projection(&fixture.request).map(|_| ()),
        search_library(&fixture.request, "missing", None, 1).map(|_| ()),
    ] {
        let error = result.unwrap_err();
        assert_eq!(error.exit_code(), 4);
        assert!(error.to_string().contains("65536-byte resource limit"));
    }
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}

#[test]
fn oversized_registry_is_classified_before_sentinel_utf8_decoding() {
    let fixture = fixture("registry-resource-refusal");
    let path = fixture.root.join("Meta/projects.yaml");
    let mut bytes = vec![b'#'; 1_048_576];
    // The sentinel splits a UTF-8 character; size must still own the diagnostic.
    bytes.extend_from_slice("é".as_bytes());
    fs::write(&path, bytes).unwrap();
    let before = link_fixture_snapshot(&fixture.root);
    let error = akasha_core::resolve_project(&fixture.request).unwrap_err();
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("1048576-byte resource limit"));
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}

/// Measures actual shared-core work; normal test runs never assert timing thresholds.
#[test]
#[ignore = "manual synthetic archive measurement; run with --ignored --nocapture"]
fn synthetic_archive_runtime() {
    for count in [100, 1_000, 10_000] {
        let fixture = fixture(&format!("archive-{count}"));
        let baseline = build_library_projection(&fixture.request)
            .unwrap()
            .total_books;
        let mut project_entities = vec!["entities/core.md".to_owned()];
        let mut added_bytes = 0;
        for number in 0..count {
            let entity = format!("bench-{number:05}");
            let global = number % 2 == 0;
            let directory = if global {
                "Global/entities"
            } else {
                "Projects/alpha/entities"
            };
            let source = format!(
                "---\nschema_version: 1\nentity: {entity}\nkind: subsystem\nstatus: active\nreviewed: 2026-10-26\n---\n\n# {entity}\n\n{}\narchive-needle\n",
                "Synthetic Unicode ☃ 世界 paragraph. ".repeat(64)
            );
            added_bytes += source.len();
            fs::write(
                fixture.root.join(directory).join(format!("{entity}.md")),
                source,
            )
            .unwrap();
            if !global {
                project_entities.push(format!("entities/{entity}.md"));
            }
        }
        support::write_project_state(
            &fixture.root.join("Projects/alpha"),
            &[],
            &project_entities
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            &["records/tasks/ship-mvp.md"],
        );
        let before = link_fixture_snapshot(&fixture.root);
        let started = Instant::now();
        let projection = build_library_projection(&fixture.request).unwrap();
        let projection_us = started.elapsed().as_micros();
        assert_eq!(projection.total_books, baseline + count);
        let started = Instant::now();
        let search = search_library(&fixture.request, "archive-needle", None, 30).unwrap();
        let search_us = started.elapsed().as_micros();
        assert_eq!(search.total_matches, count);
        assert_eq!(search.hits.len(), 30);
        assert!(search.truncated);
        let started = Instant::now();
        assert!(!inspect_pending_note_edit(&fixture.request).unwrap().pending);
        let checked = build_library_projection(&fixture.request).unwrap();
        assert_eq!(projection, checked);
        let background_core_us = started.elapsed().as_micros();
        // Isolate raw reads, frontmatter parsing and hashing outside operation timings.
        let paths = projection
            .global
            .categories
            .iter()
            .chain(
                projection
                    .projects
                    .iter()
                    .flat_map(|shelf| &shelf.categories),
            )
            .flat_map(|category| &category.books)
            .map(|book| projection.root.join(&book.id))
            .collect::<Vec<_>>();
        let started = Instant::now();
        let sources = paths
            .iter()
            .map(|path| fs::read(path).unwrap())
            .collect::<Vec<_>>();
        let raw_read_us = started.elapsed().as_micros();
        let started = Instant::now();
        for source in &sources {
            std::hint::black_box(akasha_core::parse_leading_frontmatter_bytes(source).unwrap());
        }
        let frontmatter_parse_us = started.elapsed().as_micros();
        let started = Instant::now();
        for source in &sources {
            use sha2::Digest;
            std::hint::black_box(sha2::Sha256::digest(source));
        }
        let source_hash_us = started.elapsed().as_micros();
        drop(sources);
        drop(paths);
        assert_eq!(link_fixture_snapshot(&fixture.root), before);
        println!(
            "{}",
            serde_json::json!({
                "added_notes": count, "added_bytes": added_bytes,
                "total_books": projection.total_books,
                "projection_us": projection_us, "all_search_us": search_us,
                "background_core_us": background_core_us,
                "raw_read_us": raw_read_us, "frontmatter_parse_us": frontmatter_parse_us,
                "source_hash_us": source_hash_us,
                "search_matches": search.total_matches, "search_hits": search.hits.len(),
                "root_unchanged": true, "build": if cfg!(debug_assertions) { "debug" } else { "release" }
            })
        );
    }
}
