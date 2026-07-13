use std::fs;
use std::path::{Path, PathBuf};

use akasha_core::{
    parse_leading_frontmatter, parse_leading_frontmatter_bytes, parse_project_registry,
};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/validation")
}

fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(fixtures().join(path)).expect("read validation fixture")
}

#[test]
fn preserves_frontmatter_body_unicode_whitespace_and_final_newline() {
    let source = read("notes/fidelity.md");
    let parsed = parse_leading_frontmatter(&source).expect("parse fidelity fixture");

    assert_eq!(parsed.schema_version, 1);
    assert!(parsed.frontmatter_yaml.contains("title: \"Snowman ☃\""));
    assert!(parsed.frontmatter_yaml.contains("# retain this comment"));
    assert!(parsed.body.contains("[[Projects/example/index|example]]"));
    assert_eq!(format!("{}{}", parsed.raw_frontmatter, parsed.body), source);
    assert!(parsed.body.ends_with("\n\n"));
}

#[test]
fn preserves_crlf_and_supports_an_empty_body() {
    let source = "---\r\nschema_version: 1\r\ntitle: exact\r\n---\r\nBody\r\n";
    let parsed = parse_leading_frontmatter(source).expect("parse CRLF note");
    assert_eq!(format!("{}{}", parsed.raw_frontmatter, parsed.body), source);
    assert_eq!(parsed.body, "Body\r\n");

    let empty = "---\nschema_version: 1\n---";
    let parsed = parse_leading_frontmatter(empty).expect("parse empty-body note");
    assert_eq!(parsed.body, "");
    assert_eq!(parsed.raw_frontmatter, empty);
}

#[test]
fn rejects_missing_or_unterminated_delimiters() {
    for (fixture, expected) in [
        ("notes/missing-frontmatter.md", "must begin"),
        ("notes/unterminated-frontmatter.md", "no exact closing"),
    ] {
        let source = read(fixture);
        let error = parse_leading_frontmatter(&source).expect_err("delimiter error must fail");
        assert_eq!(error.exit_code(), 4);
        assert!(error.to_string().contains(expected));
    }

    let duplicate = "---\n---\n---\nschema_version: 1\n---\n";
    let error = parse_leading_frontmatter(duplicate)
        .expect_err("a duplicate empty delimiter pair must not form a canonical note");
    assert_eq!(error.exit_code(), 4);
}

#[test]
fn rejects_malformed_ambiguous_or_unsafe_frontmatter() {
    for fixture in [
        "notes/malformed-yaml.md",
        "notes/duplicate-key.md",
        "notes/unsafe-tag.md",
        "notes/wrong-schema-type.md",
    ] {
        let source = read(fixture);
        let error = parse_leading_frontmatter(&source)
            .err()
            .unwrap_or_else(|| panic!("{fixture} should fail"));
        assert_eq!(error.exit_code(), 4, "fixture {fixture}");
    }
}

#[test]
fn rejects_unknown_schema_versions() {
    let source = read("notes/unknown-schema.md");
    let error = parse_leading_frontmatter(&source).expect_err("unknown schema must fail");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("unsupported schema_version 2"));
}

#[test]
fn rejects_invalid_utf8_before_parsing() {
    let source = b"---\nschema_version: 1\n---\n\xff";
    let error = parse_leading_frontmatter_bytes(source).expect_err("invalid UTF-8 must fail");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("not valid UTF-8"));
}

#[test]
fn parses_the_canonical_project_registry_shape() {
    let source = read("registries/valid.yaml");
    let registry = parse_project_registry(&source).expect("parse valid registry");

    assert_eq!(registry.projects.len(), 2);
    assert_eq!(
        registry.projects["example"].path,
        PathBuf::from("~/code/example")
    );
    assert_eq!(registry.projects["example"].status, "active");
}

#[test]
fn rejects_ambiguous_or_malformed_registry_entries() {
    for fixture in [
        "registries/duplicate-slug.yaml",
        "registries/invalid-slug.yaml",
        "registries/empty-path.yaml",
        "registries/wrong-field-type.yaml",
        "registries/unknown-field.yaml",
        "registries/unsafe-tag.yaml",
        "registries/merge-key.yaml",
    ] {
        let source = read(fixture);
        let error = parse_project_registry(&source)
            .err()
            .unwrap_or_else(|| panic!("{fixture} should fail"));
        assert_eq!(error.exit_code(), 4, "fixture {fixture}");
    }
}
