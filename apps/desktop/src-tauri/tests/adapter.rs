use std::path::PathBuf;

use akasha_desktop::{library_document, library_projection};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/resolution/valid-root")
}

#[test]
fn adapter_returns_the_core_projection_and_exact_fallback() {
    let library = library_projection(Some(fixture_root()), Some("example".to_owned()))
        .expect("load fixture library");

    assert_eq!(library.projection.selected_project, "example");
    assert!(library.projection.total_books > 0);
    assert!(library.fallback_markdown.contains("# Akasha Library"));
    assert!(
        library
            .fallback_markdown
            .contains("`Global/entities/rust-pattern.md`")
    );
}

#[test]
fn adapter_loads_only_a_projected_exact_document() {
    let document = library_document(
        Some(fixture_root()),
        Some("example".to_owned()),
        "Projects/example/entities/core.md",
    )
    .expect("load projected fixture document");

    assert!(document.source.contains("# Synthetic entity"));

    let error = library_document(
        Some(fixture_root()),
        Some("example".to_owned()),
        "Projects/example/index.md",
    )
    .expect_err("reject non-projected document");
    assert_eq!(error.code, 4);
}
