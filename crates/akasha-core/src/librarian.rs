//! Experimental read-only evidence and answer checks; no inference or model tools.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::library::vault_id;
use crate::note_links::{NoteLinksError, read_checked_source, refuse_pending_journals};
use crate::project_validation::{
    ProjectValidationError, canonical_note_paths, validate_project, wikilink_target_path,
};
use crate::resolution::{ResolveRequest, load_root_config};
use crate::state::content_fingerprint;
use crate::validation::{ValidationError, parse_leading_frontmatter};
use crate::wikilink::parse_wikilinks;

const MAX_SOURCES: usize = 6;
const EXCERPT_CHARS: usize = 1_000;
pub const MAX_LIBRARIAN_PACKET_BYTES: usize = 16_000;
pub const MAX_LIBRARIAN_ANSWER_BYTES: usize = 16_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibrarianEvidence {
    pub citation: usize,
    pub id: String,
    pub fingerprint: String,
    pub status: Option<String>,
    pub line_start: usize,
    pub line_end: usize,
    /// Exact source substring, possibly ending part-way through a line.
    pub excerpt: String,
    pub excerpt_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibrarianPacket {
    pub schema_version: u32,
    pub project: String,
    pub question: String,
    pub terms: Vec<String>,
    pub note: Option<String>,
    pub scanned_notes: usize,
    pub matched_notes: usize,
    pub omitted_notes: usize,
    pub evidence: Vec<LibrarianEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibrarianClaim {
    pub text: String,
    pub citations: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibrarianAnswer {
    pub insufficient_evidence: bool,
    pub claims: Vec<LibrarianClaim>,
}

#[derive(Debug, Serialize)]
pub struct CheckedLibrarianAnswer {
    pub project: String,
    pub answer: LibrarianAnswer,
    pub evidence: Vec<LibrarianEvidence>,
}

fn invalid(path: &Path, message: &str) -> NoteLinksError {
    ProjectValidationError::InvalidDocument {
        path: path.to_path_buf(),
        source: Box::new(ValidationError::InvalidSchema {
            document: "librarian",
            message: message.to_owned(),
        }),
    }
    .into()
}

/// Collect only configured canonical notes belonging to the resolved project.
/// Validation may inspect linked target metadata; it does not read foreign note bodies.
pub fn prepare_librarian(
    request: &ResolveRequest,
    question: &str,
    terms: &[String],
    note: Option<&str>,
) -> Result<LibrarianPacket, NoteLinksError> {
    let question = question.trim();
    if question.is_empty() || question.chars().count() > 512 || terms.len() > 8 {
        return Err(invalid(
            &request.cwd,
            "use 1..512 question characters and at most 8 search terms",
        ));
    }
    if terms
        .iter()
        .any(|term| term.trim().is_empty() || term.chars().count() > 128)
    {
        return Err(invalid(
            &request.cwd,
            "search terms require 1..128 characters",
        ));
    }
    let needles: BTreeSet<String> = if terms.is_empty() {
        question
            .split(|ch: char| !ch.is_alphanumeric())
            .filter(|word| word.chars().count() >= 3)
            .take(32)
            .map(str::to_lowercase)
            .collect()
    } else {
        terms
            .iter()
            .map(|term| term.trim().to_lowercase())
            .collect()
    };
    refuse_pending_journals(std::slice::from_ref(request))?;
    let report = validate_project(request)?;
    let config = load_root_config(&report.root).map_err(ProjectValidationError::from)?;
    let mut paths = BTreeMap::new();
    for kind in config.project.note_types.values() {
        for path in canonical_note_paths(&report.project_dir.join(&kind.folder))? {
            paths.insert(vault_id(&report.root, &path)?, path);
        }
    }
    let mut neighbors = BTreeSet::new();
    let mut anchor_fingerprint = None;
    if let Some(id) = note {
        let path = paths.get(id).ok_or_else(|| {
            invalid(
                &report.root,
                "selected note must be an exact canonical identity in the selected project",
            )
        })?;
        let source = read_checked_source(&report.root, path)?;
        anchor_fingerprint = Some(content_fingerprint(source.as_bytes()));
        let parsed = parse_leading_frontmatter(&source)
            .map_err(|error| invalid(path, &error.to_string()))?;
        for link in
            parse_wikilinks(parsed.body).map_err(|error| invalid(path, &error.to_string()))?
        {
            if let Some(target) = link.target {
                let relative =
                    wikilink_target_path(target).map_err(|error| invalid(path, &error))?;
                let target = vault_id(&report.root, &report.root.join(relative))?;
                if paths.contains_key(&target) {
                    neighbors.insert(target);
                }
            }
        }
    }
    // Retain bounded excerpts only, not a second archive of canonical bodies.
    let mut best: Vec<(usize, LibrarianEvidence)> = Vec::new();
    let mut matched_notes = 0;
    for (id, path) in &paths {
        let source = read_checked_source(&report.root, path)?;
        let fingerprint = content_fingerprint(source.as_bytes());
        if note == Some(id.as_str()) && anchor_fingerprint.as_ref() != Some(&fingerprint) {
            return Err(NoteLinksError::StaleSource(id.clone()));
        }
        let parsed = parse_leading_frontmatter(&source)
            .map_err(|error| invalid(path, &error.to_string()))?;
        let folded = parsed.body.to_lowercase();
        let folded_id = id.to_lowercase();
        let score = needles
            .iter()
            .filter(|term| folded.contains(term.as_str()) || folded_id.contains(term.as_str()))
            .count()
            + usize::from(neighbors.contains(id)) * 64
            + usize::from(note == Some(id.as_str())) * 128;
        if score == 0 {
            continue;
        }
        matched_notes += 1;
        let body_start = source.len() - parsed.body.len();
        let mut offset = body_start;
        for line in parsed.body.split_inclusive('\n') {
            if needles
                .iter()
                .any(|term| line.to_lowercase().contains(term.as_str()))
            {
                break;
            }
            offset += line.len();
        }
        if offset == source.len() {
            offset = body_start;
        }
        let excerpt: String = source[offset..].chars().take(EXCERPT_CHARS).collect();
        let line_start = source[..offset]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        let line_end = line_start
            + excerpt
                .trim_end_matches('\n')
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count();
        let evidence = LibrarianEvidence {
            citation: 0,
            id: id.clone(),
            fingerprint,
            status: parsed
                .metadata
                .get("status")
                .and_then(|value| value.as_str())
                .map(str::to_owned),
            line_start,
            line_end,
            excerpt_truncated: offset > body_start || offset + excerpt.len() < source.len(),
            excerpt,
        };
        best.push((score, evidence));
        best.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
        best.truncate(MAX_SOURCES);
    }
    let mut evidence = Vec::new();
    for (_, mut entry) in best {
        let source = read_checked_source(&report.root, &paths[&entry.id])?;
        if content_fingerprint(source.as_bytes()) != entry.fingerprint {
            return Err(NoteLinksError::StaleSource(entry.id));
        }
        entry.citation = evidence.len() + 1;
        evidence.push(entry);
    }
    refuse_pending_journals(std::slice::from_ref(request))?;
    let packet = LibrarianPacket {
        schema_version: 1,
        project: report.project,
        question: question.to_owned(),
        terms: terms.to_vec(),
        note: note.map(str::to_owned),
        scanned_notes: paths.len(),
        matched_notes,
        omitted_notes: matched_notes - evidence.len(),
        evidence,
    };
    let size = serde_json::to_vec(&packet)
        .expect("serializable packet")
        .len();
    if size > MAX_LIBRARIAN_PACKET_BYTES {
        return Err(invalid(
            &report.root,
            "librarian evidence exceeds the 16000-byte packet budget; narrow the terms",
        ));
    }
    Ok(packet)
}

/// Check snapshot, response shape and citation membership, not semantic truth.
pub fn check_librarian_answer(
    request: &ResolveRequest,
    packet: &LibrarianPacket,
    answer: &str,
) -> Result<CheckedLibrarianAnswer, NoteLinksError> {
    if answer.len() > MAX_LIBRARIAN_ANSWER_BYTES {
        return Err(invalid(
            &request.cwd,
            "librarian answer exceeds 16000 bytes",
        ));
    }
    let fresh = prepare_librarian(
        request,
        &packet.question,
        &packet.terms,
        packet.note.as_deref(),
    )?;
    if fresh != *packet {
        return Err(NoteLinksError::StaleSource(
            "librarian evidence packet".into(),
        ));
    }
    let answer: LibrarianAnswer = serde_json::from_str(answer)
        .map_err(|_| invalid(&request.cwd, "invalid librarian answer JSON"))?;
    if answer.claims.len() > 8 || (answer.claims.is_empty() && !answer.insufficient_evidence) {
        return Err(invalid(
            &request.cwd,
            "answer requires cited claims or explicit insufficient evidence",
        ));
    }
    for claim in &answer.claims {
        let citations: BTreeSet<_> = claim.citations.iter().collect();
        if claim.text.trim().is_empty()
            || claim.text.chars().count() > 800
            || claim.text.chars().any(|ch| ch.is_control())
            || citations.is_empty()
            || citations.len() != claim.citations.len()
            || citations
                .iter()
                .any(|&&id| id == 0 || id > packet.evidence.len())
        {
            return Err(invalid(
                &request.cwd,
                "claims require 1..800 control-free characters and unique valid citations",
            ));
        }
    }
    Ok(CheckedLibrarianAnswer {
        project: packet.project.clone(),
        answer,
        evidence: packet.evidence.clone(),
    })
}
