//! Experimental read-only evidence and answer checks; no inference or model tools.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::library::vault_id;
use crate::note_links::{NoteLinksError, read_checked_source, refuse_pending_journals};
use crate::project_validation::{
    ProjectValidationError, canonical_note_paths, validate_project, wikilink_target_path,
};
use crate::resolution::{ResolveRequest, load_root_config, resolve_project};
use crate::state::content_fingerprint;
use crate::validation::{ValidationError, parse_leading_frontmatter};
use crate::wikilink::parse_wikilinks;

const MAX_SOURCES: usize = 6;
const EXCERPT_CHARS: usize = 1_000;
const EXCERPT_CONTEXT_CHARS: usize = 640;
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibrarianSourceSelection {
    pub selections: Vec<LibrarianSelectedQuote>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibrarianSelectedQuote {
    pub citation: usize,
    #[serde(default)]
    pub quote: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LibrarianRecordedLink {
    pub target: String,
    pub wikilink: String,
    pub line: usize,
}

#[derive(Debug, Serialize)]
pub struct LibrarianSourcePassage {
    pub citation: usize,
    pub id: String,
    pub fingerprint: String,
    pub status: Option<String>,
    pub line_start: usize,
    pub line_end: usize,
    pub partial_body: bool,
    pub quote: String,
    pub links: Vec<LibrarianRecordedLink>,
    pub omitted_links: usize,
}

#[derive(Debug, Serialize)]
pub struct CheckedLibrarianSources {
    pub schema_version: u32,
    pub kind: &'static str,
    pub project: String,
    pub question: String,
    pub link_scope: &'static str,
    pub matches: Vec<LibrarianSourcePassage>,
}

fn automatic_term(word: &str) -> bool {
    word.chars().count() >= 3
        && !matches!(
            word,
            "and"
                | "about"
                | "are"
                | "can"
                | "did"
                | "does"
                | "for"
                | "find"
                | "from"
                | "how"
                | "its"
                | "note"
                | "notes"
                | "please"
                | "show"
                | "tell"
                | "that"
                | "the"
                | "their"
                | "this"
                | "use"
                | "uses"
                | "was"
                | "were"
                | "what"
                | "when"
                | "where"
                | "which"
                | "who"
                | "why"
                | "with"
        )
}

fn matches_term(text: &str, term: &str, automatic: bool) -> bool {
    if automatic {
        text.split(|ch: char| !ch.is_alphanumeric())
            .any(|word| word == term)
    } else {
        text.contains(term)
    }
}

// Keep a line-aligned exact quote: choose coverage before proximity, then add bounded context.
fn ranked_excerpt(body: &str, needles: &BTreeSet<String>, automatic: bool) -> (usize, String) {
    if body.chars().take(EXCERPT_CHARS + 1).count() <= EXCERPT_CHARS {
        return (0, body.to_owned());
    }
    let mut best_hits = 0;
    let mut best_offset = 0;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let folded = line.to_lowercase();
        let hits = needles
            .iter()
            .filter(|term| matches_term(&folded, term, automatic))
            .count();
        if hits > best_hits {
            best_hits = hits;
            best_offset = offset;
        }
        offset += line.len();
    }
    if best_hits == 0 {
        return (0, body.chars().take(EXCERPT_CHARS).collect());
    }
    let mut excerpt = String::new();
    let mut chars = 0;
    for line in body[best_offset..].split_inclusive('\n') {
        let line_chars = line.chars().take(EXCERPT_CHARS + 1).count();
        if excerpt.is_empty() {
            excerpt.extend(line.chars().take(EXCERPT_CHARS));
            chars = line_chars.min(EXCERPT_CHARS);
        } else if chars + line_chars <= EXCERPT_CONTEXT_CHARS {
            excerpt.push_str(line);
            chars += line_chars;
        } else {
            break;
        }
    }
    (best_offset, excerpt)
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
            .map(str::to_lowercase)
            .filter(|word| automatic_term(word))
            .take(32)
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
    let mut best: Vec<((u8, usize), LibrarianEvidence)> = Vec::new();
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
        let filename = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_lowercase();
        let status = parsed
            .metadata
            .get("status")
            .and_then(|value| value.as_str());
        let folded_status = status.unwrap_or_default().to_lowercase();
        let hits = |text: &str| {
            needles
                .iter()
                .filter(|term| matches_term(text, term, terms.is_empty()))
                .count()
        };
        let lexical =
            hits(&folded) + hits(&folded_id) + 3 * hits(&filename) + 4 * hits(&folded_status);
        let priority = if note == Some(id.as_str()) {
            2
        } else {
            u8::from(neighbors.contains(id))
        };
        if priority == 0 && lexical == 0 {
            continue;
        }
        matched_notes += 1;
        let body_start = source.len() - parsed.body.len();
        let (body_offset, excerpt) = ranked_excerpt(parsed.body, &needles, terms.is_empty());
        let offset = body_start + body_offset;
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
            status: status.map(str::to_owned),
            line_start,
            line_end,
            excerpt_truncated: offset > body_start || offset + excerpt.len() < source.len(),
            excerpt,
        };
        best.push(((priority, lexical), evidence));
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

/// Verify selected source bytes and derive links, without accepting generated factual claims.
pub fn check_librarian_sources(
    request: &ResolveRequest,
    packet: &LibrarianPacket,
    selection: &str,
) -> Result<CheckedLibrarianSources, NoteLinksError> {
    if selection.len() > MAX_LIBRARIAN_ANSWER_BYTES {
        return Err(invalid(
            &request.cwd,
            "source selection exceeds 16000 bytes",
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
    let selected: LibrarianSourceSelection = serde_json::from_str(selection)
        .map_err(|_| invalid(&request.cwd, "invalid librarian source-selection JSON"))?;
    if selected.selections.len() > MAX_SOURCES {
        return Err(invalid(&request.cwd, "select at most six source passages"));
    }
    let resolved = resolve_project(request).map_err(ProjectValidationError::from)?;
    if resolved.project != packet.project {
        return Err(NoteLinksError::StaleSource("librarian project".into()));
    }
    let available: BTreeSet<_> = packet
        .evidence
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    let mut seen = BTreeSet::new();
    let mut matches = Vec::new();
    for item in selected.selections {
        let evidence = packet
            .evidence
            .iter()
            .find(|entry| entry.citation == item.citation)
            .ok_or_else(|| invalid(&resolved.root, "unknown source citation"))?;
        let quote = item.quote.unwrap_or_else(|| evidence.excerpt.clone());
        if !seen.insert(item.citation)
            || quote.trim().is_empty()
            || quote.chars().count() > EXCERPT_CHARS
            || quote
                .chars()
                .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
        {
            return Err(invalid(
                &resolved.root,
                "use unique citations and 1..1000 source characters without terminal controls",
            ));
        }
        let offset = evidence.excerpt.find(&quote).ok_or_else(|| {
            invalid(
                &resolved.root,
                "quote is not an exact source excerpt substring",
            )
        })?;
        let after_first = offset + quote.chars().next().expect("nonempty quote").len_utf8();
        if evidence.excerpt[after_first..].contains(&quote) {
            return Err(invalid(
                &resolved.root,
                "quote occurs more than once; select an unambiguous passage",
            ));
        }
        let path = resolved.root.join(&evidence.id);
        if !path.starts_with(&resolved.project_dir) {
            return Err(invalid(&path, "source is outside the selected project"));
        }
        let source = read_checked_source(&resolved.root, &path)?;
        if content_fingerprint(source.as_bytes()) != evidence.fingerprint {
            return Err(NoteLinksError::StaleSource(evidence.id.clone()));
        }
        let excerpt_start = if evidence.line_start == 1 {
            Some(0)
        } else {
            source
                .match_indices('\n')
                .nth(evidence.line_start - 2)
                .map(|(index, _)| index + 1)
        }
        .ok_or_else(|| NoteLinksError::StaleSource(evidence.id.clone()))?;
        if source.get(excerpt_start..excerpt_start + evidence.excerpt.len())
            != Some(evidence.excerpt.as_str())
        {
            return Err(NoteLinksError::StaleSource(evidence.id.clone()));
        }
        let quote_start = excerpt_start + offset;
        let quote_end = quote_start + quote.len();
        let line_start = 1 + source[..quote_start]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count();
        let line_end = line_start
            + quote
                .strip_suffix('\n')
                .unwrap_or(&quote)
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count();
        let parsed = parse_leading_frontmatter(&source)
            .map_err(|error| invalid(&path, &error.to_string()))?;
        let body_start = source.len() - parsed.body.len();
        let mut links = Vec::new();
        let mut omitted_links = 0;
        let mut seen_links = BTreeSet::new();
        for link in
            parse_wikilinks(parsed.body).map_err(|error| invalid(&path, &error.to_string()))?
        {
            let from = body_start + link.from;
            let to = body_start + link.to;
            if from < quote_start || to > quote_end {
                continue;
            }
            let target = match link.target {
                Some(target) => {
                    let relative =
                        wikilink_target_path(target).map_err(|error| invalid(&path, &error))?;
                    vault_id(&resolved.root, &resolved.root.join(relative))?
                }
                None => evidence.id.clone(),
            };
            let raw = &source[from..to];
            if !available.contains(target.as_str()) || links.len() == 20 {
                omitted_links += 1;
            } else if seen_links.insert((target.clone(), raw.to_owned())) {
                links.push(LibrarianRecordedLink {
                    target,
                    wikilink: raw.to_owned(),
                    line: 1 + source[..from].bytes().filter(|byte| *byte == b'\n').count(),
                });
            }
        }
        matches.push(LibrarianSourcePassage {
            citation: evidence.citation,
            id: evidence.id.clone(),
            fingerprint: evidence.fingerprint.clone(),
            status: evidence.status.clone(),
            line_start,
            line_end,
            partial_body: evidence.excerpt_truncated || quote != evidence.excerpt,
            quote,
            links,
            omitted_links,
        });
    }
    refuse_pending_journals(std::slice::from_ref(request))?;
    let checked = CheckedLibrarianSources {
        schema_version: 1,
        kind: "source_passages",
        project: packet.project.clone(),
        question: packet.question.clone(),
        link_scope: "complete_wikilinks_in_quotes_targeting_this_evidence_packet",
        matches,
    };
    if serde_json::to_vec(&checked)
        .expect("serializable source passages")
        .len()
        > 65_536
    {
        return Err(invalid(
            &resolved.root,
            "checked source passages exceed 65536 bytes",
        ));
    }
    Ok(checked)
}
