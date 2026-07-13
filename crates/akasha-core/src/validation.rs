use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::str::{self, Utf8Error};

use serde::Serialize;
use serde_json::Value;
use serde_saphyr::options::{DuplicateKeyPolicy, MergeKeyPolicy};

const NOTE_SCHEMA_VERSION: u32 = 1;

/// A canonical Markdown note split without changing any source bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedNote<'a> {
    pub schema_version: u32,
    /// Exact source from the opening delimiter through the closing delimiter and its line ending.
    pub raw_frontmatter: &'a str,
    /// Exact YAML between the delimiter lines.
    pub frontmatter_yaml: &'a str,
    /// Exact Markdown after the closing delimiter.
    pub body: &'a str,
}

/// One validated entry from the project registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectRegistryEntry {
    pub path: PathBuf,
    pub status: String,
}

/// A registry whose project identities are unique and schema-valid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectRegistry {
    pub projects: BTreeMap<String, ProjectRegistryEntry>,
}

/// A schema or canonical-input failure. All variants use CLI exit class 4.
#[derive(Debug)]
pub enum ValidationError {
    InvalidUtf8(Utf8Error),
    MissingFrontmatter,
    UnterminatedFrontmatter,
    InvalidYaml {
        document: &'static str,
        source: serde_saphyr::Error,
    },
    InvalidSchema {
        document: &'static str,
        message: String,
    },
    UnsupportedSchemaVersion {
        document: &'static str,
        version: u64,
        expected: u32,
    },
}

impl ValidationError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        4
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUtf8(source) => write!(formatter, "note is not valid UTF-8: {source}"),
            Self::MissingFrontmatter => {
                formatter.write_str("note must begin with an exact --- frontmatter delimiter")
            }
            Self::UnterminatedFrontmatter => {
                formatter.write_str("note frontmatter has no exact closing --- delimiter")
            }
            Self::InvalidYaml { document, source } => {
                write!(formatter, "invalid {document} YAML: {source}")
            }
            Self::InvalidSchema { document, message } => {
                write!(formatter, "invalid {document} schema: {message}")
            }
            Self::UnsupportedSchemaVersion {
                document,
                version,
                expected,
            } => write!(
                formatter,
                "unsupported schema_version {version} in {document}; expected {expected}"
            ),
        }
    }
}

impl Error for ValidationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidUtf8(source) => Some(source),
            Self::InvalidYaml { source, .. } => Some(source),
            Self::MissingFrontmatter
            | Self::UnterminatedFrontmatter
            | Self::InvalidSchema { .. }
            | Self::UnsupportedSchemaVersion { .. } => None,
        }
    }
}

/// Split and validate a canonical note's leading YAML frontmatter.
pub fn parse_leading_frontmatter(source: &str) -> Result<ParsedNote<'_>, ValidationError> {
    let slices = split_frontmatter(source)?;
    let value = parse_yaml_value(slices.yaml, "note frontmatter")?;
    let mapping = value
        .as_object()
        .ok_or_else(|| ValidationError::InvalidSchema {
            document: "note frontmatter",
            message: "top level must be a mapping".to_owned(),
        })?;
    let version = required_u64(mapping, "schema_version", "note frontmatter")?;
    if version != u64::from(NOTE_SCHEMA_VERSION) {
        return Err(ValidationError::UnsupportedSchemaVersion {
            document: "note frontmatter",
            version,
            expected: NOTE_SCHEMA_VERSION,
        });
    }

    Ok(ParsedNote {
        schema_version: NOTE_SCHEMA_VERSION,
        raw_frontmatter: &source[..slices.raw_end],
        frontmatter_yaml: slices.yaml,
        body: &source[slices.raw_end..],
    })
}

/// Validate UTF-8 before splitting a canonical note's leading frontmatter.
pub fn parse_leading_frontmatter_bytes(source: &[u8]) -> Result<ParsedNote<'_>, ValidationError> {
    let source = str::from_utf8(source).map_err(ValidationError::InvalidUtf8)?;
    parse_leading_frontmatter(source)
}

/// Parse the config-independent `slug -> {path, status}` project registry contract.
pub fn parse_project_registry(source: &str) -> Result<ProjectRegistry, ValidationError> {
    let value = parse_yaml_value(source, "project registry")?;
    let mapping = value
        .as_object()
        .ok_or_else(|| ValidationError::InvalidSchema {
            document: "project registry",
            message: "top level must be a mapping from project slug to entry".to_owned(),
        })?;
    let mut projects = BTreeMap::new();

    for (slug, value) in mapping {
        if !is_valid_slug(slug) {
            return Err(ValidationError::InvalidSchema {
                document: "project registry",
                message: format!(
                    "invalid project slug {slug:?}; use lowercase ASCII letters, digits, and hyphens"
                ),
            });
        }

        let entry = value
            .as_object()
            .ok_or_else(|| ValidationError::InvalidSchema {
                document: "project registry",
                message: format!("entry {slug:?} must be a mapping"),
            })?;
        reject_unknown_fields(entry, &["path", "status"], slug)?;
        let path = required_string(entry, "path", "project registry")?;
        let status = required_string(entry, "status", "project registry")?;
        if path.is_empty() {
            return Err(ValidationError::InvalidSchema {
                document: "project registry",
                message: format!("entry {slug:?} has an empty path"),
            });
        }
        if status.trim().is_empty() {
            return Err(ValidationError::InvalidSchema {
                document: "project registry",
                message: format!("entry {slug:?} has an empty status"),
            });
        }

        projects.insert(
            slug.clone(),
            ProjectRegistryEntry {
                path: PathBuf::from(path),
                status: status.to_owned(),
            },
        );
    }

    Ok(ProjectRegistry { projects })
}

struct FrontmatterSlices<'a> {
    raw_end: usize,
    yaml: &'a str,
}

fn split_frontmatter(source: &str) -> Result<FrontmatterSlices<'_>, ValidationError> {
    let yaml_start = if source.starts_with("---\n") {
        4
    } else if source.starts_with("---\r\n") {
        5
    } else {
        return Err(ValidationError::MissingFrontmatter);
    };
    let bytes = source.as_bytes();
    let mut line_start = yaml_start;

    loop {
        match source[line_start..].find('\n') {
            Some(relative_newline) => {
                let newline = line_start + relative_newline;
                let line_end = if newline > line_start && bytes[newline - 1] == b'\r' {
                    newline - 1
                } else {
                    newline
                };
                if &source[line_start..line_end] == "---" {
                    return Ok(FrontmatterSlices {
                        raw_end: newline + 1,
                        yaml: &source[yaml_start..line_start],
                    });
                }
                line_start = newline + 1;
            }
            None if &source[line_start..] == "---" => {
                return Ok(FrontmatterSlices {
                    raw_end: source.len(),
                    yaml: &source[yaml_start..line_start],
                });
            }
            None => return Err(ValidationError::UnterminatedFrontmatter),
        }
    }
}

fn parse_yaml_value(source: &str, document: &'static str) -> Result<Value, ValidationError> {
    reject_yaml_tags(source, document)?;
    serde_saphyr::from_str_with_options(
        source,
        serde_saphyr::options! {
            duplicate_keys: DuplicateKeyPolicy::Error,
            merge_keys: MergeKeyPolicy::Error,
            strict_booleans: true,
            with_snippet: false,
        },
    )
    .map_err(|source| ValidationError::InvalidYaml { document, source })
}

fn reject_yaml_tags(source: &str, document: &'static str) -> Result<(), ValidationError> {
    for next in serde_saphyr::granit_parser::Parser::new_from_str(source) {
        match next {
            Ok((event, _)) if event.tag().is_some() => {
                return Err(ValidationError::InvalidSchema {
                    document,
                    message: "explicit YAML tags are not supported".to_owned(),
                });
            }
            Ok(_) => {}
            // The typed deserializer below owns syntax diagnostics and their stable error type.
            Err(_) => break,
        }
    }
    Ok(())
}

fn required_u64(
    mapping: &serde_json::Map<String, Value>,
    field: &str,
    document: &'static str,
) -> Result<u64, ValidationError> {
    mapping
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| ValidationError::InvalidSchema {
            document,
            message: format!("{field} must be a non-negative integer"),
        })
}

fn required_string<'a>(
    mapping: &'a serde_json::Map<String, Value>,
    field: &str,
    document: &'static str,
) -> Result<&'a str, ValidationError> {
    mapping
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ValidationError::InvalidSchema {
            document,
            message: format!("{field} must be a string"),
        })
}

fn reject_unknown_fields(
    mapping: &serde_json::Map<String, Value>,
    expected: &[&str],
    slug: &str,
) -> Result<(), ValidationError> {
    if let Some(field) = mapping
        .keys()
        .find(|field| !expected.contains(&field.as_str()))
    {
        return Err(ValidationError::InvalidSchema {
            document: "project registry",
            message: format!("entry {slug:?} has unknown field {field:?}"),
        });
    }
    Ok(())
}

fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
