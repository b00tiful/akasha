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

#[test]
fn frontmatter_budget_counts_exact_yaml_bytes_with_all_delimiter_endings() {
    for ending in ["\n", "\r\n"] {
        let prefix = format!("schema_version: 1{ending}#");
        let remaining = 65_536 - prefix.len() - ending.len();
        let padding = format!("{}{}", "é".repeat(remaining / 2), "x".repeat(remaining % 2));
        let yaml = format!("{prefix}{padding}{ending}");
        assert_eq!(yaml.len(), 65_536);
        for close_ending in ["", ending] {
            let source = format!("---{ending}{yaml}---{close_ending}");
            let parsed = parse_leading_frontmatter(&source).unwrap();
            assert_eq!(parsed.frontmatter_yaml, yaml);
            assert_eq!(parsed.raw_frontmatter, source);
            let oversized = source.replacen('#', "#x", 1);
            let error = parse_leading_frontmatter(&oversized).unwrap_err();
            assert_eq!(error.exit_code(), 4);
            assert!(error.to_string().contains("65536-byte resource limit"));
        }
    }
}

#[test]
fn bounded_delimiter_search_refuses_huge_unterminated_lines_and_headers() {
    for content in ["x".repeat(2_000_000), "x\n".repeat(1_000_000)] {
        for suffix in ["", "\n---\nbody"] {
            let source = format!("---\n{content}{suffix}");
            let error = parse_leading_frontmatter(&source).unwrap_err();
            assert!(error.to_string().contains("65536-byte resource limit"));
        }
    }
    assert!(
        parse_leading_frontmatter("---\nsmall")
            .unwrap_err()
            .to_string()
            .contains("no exact closing")
    );
}

#[test]
fn metadata_budget_leaves_large_markdown_bodies_byte_exact() {
    let body = "---\r\n!tag &anchor [☃]\r\n".repeat(100_000);
    let source = format!("---\r\nschema_version: 1\r\n---\r\n{body}");
    let parsed = parse_leading_frontmatter_bytes(source.as_bytes()).unwrap();
    assert_eq!(parsed.body, body);
    assert_eq!(format!("{}{}", parsed.raw_frontmatter, parsed.body), source);
}

#[test]
fn yaml_depth_budget_accepts_boundary_and_refuses_deeper_flow_and_block_values() {
    let valid = format!(
        "---\nschema_version: 1\nvalue: {}0{}\n---\n",
        "[".repeat(31),
        "]".repeat(31)
    );
    parse_leading_frontmatter(&valid).unwrap();
    for yaml in [
        format!(
            "schema_version: 1\nvalue: {}0{}\n",
            "[".repeat(32),
            "]".repeat(32)
        ),
        format!(
            "schema_version: 1\n{}",
            (0..40)
                .map(|n| format!("{}x:\n", "  ".repeat(n)))
                .collect::<String>()
        ),
    ] {
        let error = parse_leading_frontmatter(&format!("---\n{yaml}---\n")).unwrap_err();
        assert_eq!(error.exit_code(), 4);
        assert!(
            error.to_string().to_lowercase().contains("depth"),
            "{error}"
        );
    }
}

#[test]
fn yaml_node_budget_accepts_boundary_and_refuses_wide_metadata() {
    for (count, accepted) in [(8_187, true), (8_188, false)] {
        let source = format!(
            "---\nschema_version: 1\nvalues: [{}]\n---\n",
            vec!["0"; count].join(",")
        );
        let result = parse_leading_frontmatter(&source);
        assert_eq!(result.is_ok(), accepted, "{result:?}");
        if let Err(error) = result {
            assert_eq!(error.exit_code(), 4);
            assert!(
                error.to_string().to_lowercase().contains("nodes"),
                "{error}"
            );
        }
    }
}

#[test]
fn ordinary_aliases_remain_supported_and_repeated_expansion_is_bounded() {
    let valid = "---\nschema_version: 1\na: &text snowman\nb: *text\n---\n";
    assert_eq!(
        parse_leading_frontmatter(valid).unwrap().raw_frontmatter,
        valid
    );
    let oversized = format!(
        "---\nschema_version: 1\na: &text snowman\nb: [{}]\n---\n",
        vec!["*text"; 33].join(",")
    );
    let error = parse_leading_frontmatter(&oversized).unwrap_err();
    assert_eq!(error.exit_code(), 4);
    assert!(
        error.to_string().to_lowercase().contains("alias"),
        "{error}"
    );
}

#[test]
fn compact_alias_amplification_fails_before_unbounded_value_materialization() {
    let mut yaml = String::from("---\nschema_version: 1\na: &a [0, 0, 0, 0]\n");
    for (previous, next) in [('a', 'b'), ('b', 'c'), ('c', 'd'), ('d', 'e'), ('e', 'f')] {
        yaml.push_str(&format!(
            "{next}: &{next} [{}]\n",
            vec![format!("*{previous}"); 8].join(",")
        ));
    }
    yaml.push_str("---\n");
    let error = parse_leading_frontmatter(&yaml).unwrap_err();
    assert_eq!(error.exit_code(), 4);
    // Replayed nodes reach the structural budget before the alias-specific limit.
    assert!(
        error.to_string().contains("budget breached: Nodes"),
        "{error}"
    );
}

#[test]
fn registry_budget_accepts_exact_bytes_and_rejects_oversize_and_nesting() {
    let mut source = String::from("example: {path: /tmp/example, status: active}\n#");
    source.push_str(&"x".repeat(1_048_576 - source.len()));
    assert_eq!(parse_project_registry(&source).unwrap().projects.len(), 1);
    source.push('é');
    let error = parse_project_registry(&source).unwrap_err();
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("1048576-byte resource limit"));
    let deep = format!("example: {}0{}", "[".repeat(100), "]".repeat(100));
    let error = parse_project_registry(&deep).unwrap_err();
    assert_eq!(error.exit_code(), 4);
    assert!(
        error.to_string().to_lowercase().contains("depth"),
        "{error}"
    );
}

#[test]
fn anchor_and_alias_event_limits_accept_boundaries_and_reject_one_more() {
    let mut anchors = String::from("---\nschema_version: 1\n");
    for n in 0..128 {
        anchors.push_str(&format!("key{n}: &a{n} value\n"));
    }
    parse_leading_frontmatter(&format!("{anchors}---\n")).unwrap();
    let error =
        parse_leading_frontmatter(&format!("{anchors}extra: &extra value\n---\n")).unwrap_err();
    assert!(error.to_string().contains("Anchors"), "{error}");

    let mut source = String::from("---\nschema_version: 1\n");
    for n in 0..16 {
        source.push_str(&format!("key{n}: &a{n} value\n"));
    }
    let mut aliases = (0..128)
        .map(|n| format!("*a{}", n % 16))
        .collect::<Vec<_>>();
    parse_leading_frontmatter(&format!("{source}values: [{}]\n---\n", aliases.join(","))).unwrap();
    aliases.push("*a0".to_owned());
    let error =
        parse_leading_frontmatter(&format!("{source}values: [{}]\n---\n", aliases.join(",")))
            .unwrap_err();
    assert!(error.to_string().contains("Aliases"), "{error}");
}

#[test]
fn registry_node_limit_is_independent_of_its_larger_byte_allowance() {
    let mut registry = String::new();
    for n in 0..10_922 {
        registry.push_str(&format!("p{n}: {{path: /tmp/p, status: active}}\n"));
    }
    assert!(registry.len() < 1_048_576);
    assert_eq!(
        parse_project_registry(&registry).unwrap().projects.len(),
        10_922
    );
    registry.push_str("extra: {path: /tmp/p, status: active}\n");
    let error = parse_project_registry(&registry).unwrap_err();
    assert_eq!(error.exit_code(), 4);
    assert!(
        error.to_string().contains("budget breached: Nodes"),
        "{error}"
    );
}
