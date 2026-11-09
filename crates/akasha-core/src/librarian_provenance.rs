//! Opt-in inspection of recorded onboarding evidence. Byte freshness is not semantic support.
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use serde::Serialize;

use crate::ResolveRequest;
use crate::librarian::{CheckedLibrarianSources, LibrarianPacket, check_librarian_sources};
use crate::note_links::{NoteLinksError, read_checked_source};
use crate::onboarding::{
    MAX_ONBOARDING_EVIDENCE_CLAIMS, MAX_ONBOARDING_EVIDENCE_SOURCES, PersistentEvidenceClaim,
    PersistentEvidenceKind, PersistentEvidenceSource, validate_normalized_relative_path,
};
use crate::project_validation::ProjectValidationError;
use crate::resolution::resolve_project;
use crate::state::content_fingerprint;
use crate::validation::{ValidationError, parse_leading_frontmatter};

const MAX_FILES: usize = 8;
const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_CLAIMS: usize = 24;
const MAX_REFERENCES: usize = 48;
const MAX_EXCERPT_BYTES: usize = 2_048;
const MAX_REPORT_BYTES: usize = 65_536;

#[derive(Debug, Serialize)]
pub struct LibrarianProvenance {
    pub schema_version: u32,
    pub kind: &'static str,
    pub association: &'static str,
    pub sources: CheckedLibrarianSources,
    pub notes: Vec<LibrarianNoteProvenance>,
}

#[derive(Debug, Serialize)]
pub struct LibrarianNoteProvenance {
    pub citation: usize,
    pub evidence_state: &'static str,
    pub unmatched_claims: usize,
    pub claims: Vec<LibrarianClaimProvenance>,
}

#[derive(Debug, Serialize)]
pub struct LibrarianClaimProvenance {
    pub recorded_kind: &'static str,
    pub claim: String,
    pub rationale: Option<String>,
    pub note_line: usize,
    pub sources: Vec<LibrarianRepositoryEvidence>,
}

#[derive(Debug, Serialize)]
pub struct LibrarianRepositoryEvidence {
    pub path: String,
    pub recorded_fingerprint: String,
    pub observed_fingerprint: Option<String>,
    pub line_start: Option<usize>,
    pub line_end: Option<usize>,
    pub freshness: &'static str,
    pub excerpt_state: &'static str,
    pub excerpt: Option<String>,
}

fn invalid(path: &Path, message: &str) -> NoteLinksError {
    ProjectValidationError::InvalidDocument {
        path: path.to_path_buf(),
        source: Box::new(ValidationError::InvalidSchema {
            document: "librarian provenance",
            message: message.into(),
        }),
    }
    .into()
}

/// Read only repository paths recorded in fully quoted claims, with no inference or writes.
/// Recheck both canonical evidence and all repository observations before returning.
pub fn inspect_librarian_provenance(
    request: &ResolveRequest,
    packet: &LibrarianPacket,
    selection: &str,
) -> Result<LibrarianProvenance, NoteLinksError> {
    let sources = check_librarian_sources(request, packet, selection)?;
    let resolved = resolve_project(request).map_err(ProjectValidationError::from)?;
    let mut cache = BTreeMap::new();
    let mut notes = Vec::new();
    let (mut claim_count, mut reference_count) = (0, 0);
    for passage in &sources.matches {
        let path = resolved.root.join(&passage.id);
        let source = read_checked_source(&resolved.root, &path)?;
        if content_fingerprint(source.as_bytes()) != passage.fingerprint {
            return Err(NoteLinksError::StaleSource(passage.id.clone()));
        }
        let parsed = parse_leading_frontmatter(&source)
            .map_err(|error| invalid(&path, &error.to_string()))?;
        let mut note = LibrarianNoteProvenance {
            citation: passage.citation,
            evidence_state: "not_recorded",
            unmatched_claims: 0,
            claims: Vec::new(),
        };
        if let Some(value) = parsed.metadata.get("evidence") {
            // Ordinary canonical notes need not use onboarding's evidence schema. Malformed
            // metadata cannot supply repository reads or a claimed freshness observation.
            let claims = serde_json::from_value::<Vec<PersistentEvidenceClaim>>(value.clone());
            if let Ok(claims) = claims
                && valid_claims(&claims)
            {
                note.evidence_state = "recorded";
                for claim in claims {
                    // Exact, unique text membership only; no entailment inference. Partial or
                    // ambiguous claims are explicitly unassociated and never trigger reads.
                    let mut positions = passage.quote.match_indices(&claim.claim);
                    let Some((offset, _)) = positions.next() else {
                        note.unmatched_claims += 1;
                        continue;
                    };
                    if positions.next().is_some() {
                        note.unmatched_claims += 1;
                        continue;
                    }
                    claim_count += 1;
                    reference_count += claim.sources.len();
                    if claim_count > MAX_CLAIMS || reference_count > MAX_REFERENCES {
                        return Err(invalid(
                            &path,
                            "provenance exceeds 24 claims or 48 references; narrow the selection",
                        ));
                    }
                    let mut evidence = Vec::new();
                    for reference in &claim.sources {
                        evidence.push(
                            inspect_reference(&resolved.repository_dir, reference, &mut cache)
                                .map_err(|message| invalid(&path, message))?,
                        );
                    }
                    note.claims.push(LibrarianClaimProvenance {
                        recorded_kind: match claim.kind {
                            PersistentEvidenceKind::Fact => "fact",
                            PersistentEvidenceKind::Inference => "inference",
                            PersistentEvidenceKind::Unknown => "unknown",
                        },
                        note_line: passage.line_start
                            + passage.quote[..offset]
                                .bytes()
                                .filter(|b| *b == b'\n')
                                .count(),
                        claim: claim.claim,
                        rationale: claim.rationale,
                        sources: evidence,
                    });
                }
            } else {
                note.evidence_state = "invalid";
            }
        }
        notes.push(note);
    }
    let report = LibrarianProvenance {
        schema_version: 1,
        kind: "source_provenance",
        association: "unique_exact_claim_text_in_selected_quote; freshness_is_not_semantic_support",
        sources,
        notes,
    };
    if serde_json::to_vec(&report)
        .expect("serializable provenance")
        .len()
        > MAX_REPORT_BYTES
    {
        return Err(invalid(
            &resolved.root,
            "provenance exceeds 65536 bytes; narrow the selection",
        ));
    }
    for (path, observed) in cache {
        if observe_source(&resolved.repository_dir, &path) != observed {
            return Err(NoteLinksError::StaleSource(path));
        }
    }
    // Also recheck notes, selection and pending journals after repository inspection.
    check_librarian_sources(request, packet, selection)?;
    let final_resolution = resolve_project(request).map_err(ProjectValidationError::from)?;
    if final_resolution.root != resolved.root
        || final_resolution.project != resolved.project
        || final_resolution.repository_dir != resolved.repository_dir
    {
        return Err(NoteLinksError::StaleSource(
            "librarian repository identity".into(),
        ));
    }
    Ok(report)
}

fn valid_claims(claims: &[PersistentEvidenceClaim]) -> bool {
    !claims.is_empty()
        && claims.len() <= MAX_ONBOARDING_EVIDENCE_CLAIMS
        && claims.iter().all(|claim| {
            let rationale = claim.rationale.as_deref();
            !claim.claim.trim().is_empty()
                && claim.claim.chars().count() <= 1_024
                && !claim.claim.chars().any(char::is_control)
                && claim.sources.len() <= MAX_ONBOARDING_EVIDENCE_SOURCES
                && rationale
                    .is_none_or(|text| !text.trim().is_empty() && text.chars().count() <= 2_048)
                && match claim.kind {
                    PersistentEvidenceKind::Fact => !claim.sources.is_empty(),
                    PersistentEvidenceKind::Inference => {
                        !claim.sources.is_empty() && rationale.is_some()
                    }
                    PersistentEvidenceKind::Unknown => {
                        claim.sources.is_empty() && rationale.is_some()
                    }
                }
        })
}

type Observation = Result<(String, String), &'static str>;

fn inspect_reference(
    repository: &Path,
    reference: &PersistentEvidenceSource,
    cache: &mut BTreeMap<String, Observation>,
) -> Result<LibrarianRepositoryEvidence, &'static str> {
    let mut result = LibrarianRepositoryEvidence {
        path: reference.path.clone(),
        recorded_fingerprint: reference.fingerprint.clone(),
        observed_fingerprint: None,
        line_start: reference.line_start,
        line_end: reference.line_end,
        freshness: "invalid_reference",
        excerpt_state: "not_available",
        excerpt: None,
    };
    let hash = reference.fingerprint.strip_prefix("sha256:").unwrap_or("");
    if validate_normalized_relative_path(&reference.path, "evidence source", repository).is_err()
        || hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !matches!(
            (reference.line_start, reference.line_end),
            (None, None) | (Some(1..), Some(1..))
        )
        || reference.line_start > reference.line_end
    {
        return Ok(result);
    }
    if !cache.contains_key(&reference.path) {
        if cache.len() == MAX_FILES {
            return Err("provenance exceeds 8 repository files; narrow the selection");
        }
        cache.insert(
            reference.path.clone(),
            observe_source(repository, &reference.path),
        );
    }
    match &cache[&reference.path] {
        Err(reason) => {
            result.freshness = if *reason == "missing" {
                "missing"
            } else {
                "unavailable"
            };
            result.excerpt_state = reason;
        }
        Ok((fingerprint, text)) => {
            result.observed_fingerprint = Some(fingerprint.clone());
            if fingerprint != &reference.fingerprint {
                result.freshness = "changed";
                result.excerpt_state = "source_changed";
                return Ok(result);
            }
            result.freshness = "unchanged";
            let (Some(start), Some(end)) = (reference.line_start, reference.line_end) else {
                result.excerpt_state = "no_line_range";
                return Ok(result);
            };
            let lines = text.split_inclusive('\n').collect::<Vec<_>>();
            if end > lines.len() {
                result.excerpt_state = "invalid_line_range";
            } else {
                let excerpt = lines[start - 1..end].concat();
                if excerpt.len() > MAX_EXCERPT_BYTES {
                    result.excerpt_state = "exceeds_excerpt_limit";
                } else {
                    result.excerpt_state = "complete";
                    result.excerpt = Some(excerpt);
                }
            }
        }
    }
    Ok(result)
}

fn io_reason(error: io::Error) -> &'static str {
    if error.kind() == io::ErrorKind::NotFound {
        "missing"
    } else {
        "read_error"
    }
}

fn observe_source(repository: &Path, relative: &str) -> Observation {
    let mut path = repository.to_path_buf();
    for part in relative.split('/') {
        path.push(part);
        let metadata = fs::symlink_metadata(&path).map_err(io_reason)?;
        if metadata.file_type().is_symlink() {
            return Err("symlink");
        }
    }
    if fs::canonicalize(&path).map_err(io_reason)? != path {
        return Err("identity_changed");
    }
    if !fs::symlink_metadata(&path).map_err(io_reason)?.is_file() {
        return Err("not_regular");
    }
    let file = File::open(&path).map_err(io_reason)?;
    if !file.metadata().map_err(io_reason)?.is_file() {
        return Err("not_regular");
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_reason)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("exceeds_file_limit");
    }
    let fingerprint = content_fingerprint(&bytes);
    let text = String::from_utf8(bytes).map_err(|_| "invalid_utf8")?;
    if text.contains('\0') {
        return Err("contains_nul");
    }
    Ok((fingerprint, text))
}
