//! Local onboarding input, draft review and approved application. Inference stays in an adapter.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::onboarding::{
    OnboardingBatchError, OnboardingBatchPreview, OnboardingBatchRequest, OnboardingBatchResult,
    ProposedNote, apply_approved_onboarding_batch, prepare_onboarding, preview_onboarding_batch,
    validate_normalized_relative_path,
};
use crate::resolution::{ResolveRequest, load_root_config};
use crate::state::content_fingerprint;
use crate::validation::parse_leading_frontmatter_bytes;

pub const MAX_LOCAL_ONBOARDING_BYTES: usize = 16_000;
pub const MAX_LOCAL_ONBOARDING_FILES: usize = 8;
pub const MAX_LOCAL_ONBOARDING_FILE_BYTES: usize = 8_192;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalOnboardingSource {
    pub path: String,
    pub fingerprint: String,
    pub text: String,
    pub lines: usize,
    pub excerpts: Vec<LocalOnboardingExcerpt>,
}

/// One complete-line window. UTF-8 byte boundaries refer to the original source.
/// The adapter only formats these core-owned windows. Handles are 1-based.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalOnboardingExcerpt {
    pub byte_start: usize,
    pub byte_end: usize,
    pub line_start: usize,
    pub line_end: usize,
}

fn source_excerpts(text: &str) -> Vec<LocalOnboardingExcerpt> {
    let mut excerpts: Vec<LocalOnboardingExcerpt> = Vec::new();
    let mut offset = 0;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        // Keep complete text for quote plans, but never silently omit a long line
        // from the catalog. The excerpt-based adapter refuses an empty catalog.
        if line.len() > 2048 {
            return Vec::new();
        }
        if let Some(last) = excerpts.last_mut()
            && offset + line.len() - last.byte_start <= 2048
            && index + 1 - last.line_start < 16
        {
            last.byte_end = offset + line.len();
            last.line_end = index + 1;
        } else {
            excerpts.push(LocalOnboardingExcerpt {
                byte_start: offset,
                byte_end: offset + line.len(),
                line_start: index + 1,
                line_end: index + 1,
            });
        }
        offset += line.len();
    }
    excerpts
}

/// Complete, bounded explicit inputs, including current templates and projections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalOnboardingPacket {
    pub schema_version: u32,
    pub preparation: serde_json::Value,
    pub sources: Vec<LocalOnboardingSource>,
    pub index: String,
    pub roadmap: String,
}

/// Same proposal shape accepted by the existing onboarding MCP.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalOnboardingProposal {
    pub notes: Vec<LocalOnboardingNote>,
    pub index: String,
    pub roadmap: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalOnboardingNote {
    pub note_type: String,
    pub path: PathBuf,
    pub source: String,
}

impl LocalOnboardingProposal {
    #[must_use]
    pub fn request(&self, resolution: &ResolveRequest) -> OnboardingBatchRequest {
        OnboardingBatchRequest {
            resolution: resolution.clone(),
            notes: self
                .notes
                .iter()
                .map(|note| ProposedNote {
                    note_type: note.note_type.clone(),
                    path: note.path.clone(),
                    source: note.source.clone(),
                })
                .collect(),
            index: self.index.clone(),
            roadmap: self.roadmap.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct LocalOnboardingDraft {
    pub schema_version: u32,
    pub proposal: LocalOnboardingProposal,
    pub preview: OnboardingBatchPreview,
    pub review: LocalOnboardingReview,
}

/// Apply only the exact reviewed proposal, after rebuilding its bounded local inputs.
/// The shared writer rechecks the preview under its project lock and owns recovery/publication.
pub fn apply_local_onboarding(
    request: &ResolveRequest,
    packet: &LocalOnboardingPacket,
    proposal_json: &str,
    approved_preview_id: &str,
) -> Result<OnboardingBatchResult, OnboardingBatchError> {
    let draft = preview_local_onboarding(request, packet, proposal_json)?;
    // Refuse an incorrect confirmation before entering the recovering writer.
    if draft.preview.preview_id != approved_preview_id {
        return Err(OnboardingBatchError::Conflict {
            path: PathBuf::from("<local-onboarding>"),
            message:
                "approved preview does not match the local proposal and project state; review again"
                    .into(),
        });
    }
    apply_approved_onboarding_batch(&draft.proposal.request(request), approved_preview_id)
}

/// A review aid, never a semantic approval. All excerpts come from the fresh core packet.
#[derive(Debug, Serialize)]
pub struct LocalOnboardingReview {
    pub requires_review: bool,
    pub notes: Vec<LocalNoteReview>,
    pub uncited_sources: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LocalNoteReview {
    pub note_type: String,
    pub path: PathBuf,
    pub fields: serde_json::Map<String, serde_json::Value>,
    pub claims: Vec<LocalClaimReview>,
}

#[derive(Debug, Serialize)]
pub struct LocalClaimReview {
    pub kind: String,
    pub claim: String,
    pub rationale: Option<String>,
    pub sources: Vec<LocalSourceReview>,
}

#[derive(Debug, Serialize)]
pub struct LocalSourceReview {
    pub path: String,
    pub fingerprint: String,
    pub line_start: usize,
    pub line_end: usize,
    pub excerpt: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalPlan {
    schema_version: u32,
    notes: Vec<PlannedNote>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlannedNote {
    note_type: String,
    name: String,
    fields: BTreeMap<String, String>,
    evidence: Vec<PlannedClaim>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlannedClaim {
    kind: String,
    claim: String,
    rationale: Option<String>,
    sources: Vec<PlannedSource>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum PlannedSource {
    Quote { source: usize, quote: String },
    Excerpt { source: usize, excerpt: usize },
}

/// Compile typed model plans to ordinary onboarding proposals. JSON frontmatter is valid YAML;
/// trusted serialization prevents source text or model fields from injecting YAML structure.
pub fn compile_local_onboarding(
    request: &ResolveRequest,
    packet: &LocalOnboardingPacket,
    plan_json: &str,
) -> Result<LocalOnboardingDraft, OnboardingBatchError> {
    if plan_json.len() > MAX_LOCAL_ONBOARDING_BYTES {
        return Err(invalid("local onboarding plan exceeds 16000 bytes"));
    }
    let plan: LocalPlan = serde_json::from_str(plan_json)
        .map_err(|error| invalid(format!("invalid local onboarding plan: {error}")))?;
    if ![2, 3].contains(&plan.schema_version) {
        return Err(invalid(
            "local onboarding plans require schema_version 2 (quotes) or 3 (excerpt handles); regenerate legacy body plans",
        ));
    }
    if plan.notes.is_empty() || plan.notes.len() > 3 {
        return Err(invalid("local onboarding plan requires 1..=3 notes"));
    }
    bounded_json(packet)?;
    let paths = packet
        .sources
        .iter()
        .map(|source| source.path.clone())
        .collect::<Vec<_>>();
    if packet != &prepare_local_onboarding(request, &paths)? {
        return Err(OnboardingBatchError::Conflict {
            path: PathBuf::from("<local-onboarding>"),
            message: "local onboarding packet is stale or altered; prepare again".into(),
        });
    }
    let resolved = crate::resolution::resolve_project(request)?;
    let config = load_root_config(&resolved.root)?;
    let relative = |path: &Path| -> Result<String, OnboardingBatchError> {
        path.strip_prefix(&resolved.root)
            .ok()
            .and_then(Path::to_str)
            .map(str::to_owned)
            .ok_or_else(|| invalid("local onboarding target is not a UTF-8 root-relative path"))
    };
    let index_link = relative(&resolved.project_dir.join(&config.project.index))?;
    let roadmap_link = relative(&resolved.project_dir.join(&config.project.roadmap))?;
    let mut index = packet.index.clone();
    let mut roadmap = packet.roadmap.clone();
    let mut notes = Vec::new();
    for note in plan.notes {
        if note.name.is_empty()
            || note.name.len() > 80
            || !note
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(invalid(
                "local plan names require 1..=80 lowercase ASCII letters, digits or hyphens",
            ));
        }
        let note_type = config
            .project
            .note_types
            .get(&note.note_type)
            .ok_or_else(|| invalid("unknown configured local plan note type"))?;
        let path = PathBuf::from(format!("{}.md", note.name));
        let target = relative(&resolved.project_dir.join(&note_type.folder).join(&path))?;
        let mut metadata = serde_json::Map::new();
        for (key, value) in note.fields {
            if key == "schema_version" || key == "evidence" {
                return Err(invalid(
                    "local plan fields cannot replace schema_version or evidence",
                ));
            }
            metadata.insert(key, value.into());
        }
        metadata.insert("schema_version".into(), 1.into());
        let mut claims = Vec::new();
        if note.evidence.is_empty() || note.evidence.len() > 4 {
            return Err(invalid("local plan notes require 1..=4 evidence claims"));
        }
        for claim in note.evidence {
            let mut evidence =
                serde_json::json!({ "kind": claim.kind, "claim": claim.claim, "sources": [] });
            if let Some(rationale) = claim.rationale {
                evidence["rationale"] = rationale.into();
            }
            let mut sources = Vec::new();
            for reference in claim.sources {
                let handle = match &reference {
                    PlannedSource::Quote { source, .. } | PlannedSource::Excerpt { source, .. } => {
                        *source
                    }
                };
                let source = handle
                    .checked_sub(1)
                    .and_then(|index| packet.sources.get(index))
                    .ok_or_else(|| invalid("unknown local onboarding source handle"))?;
                let (start, end) = match reference {
                    PlannedSource::Quote { quote, .. } if plan.schema_version == 2 => {
                        quote_lines(source, &quote)?
                    }
                    PlannedSource::Excerpt { excerpt, .. } if plan.schema_version == 3 => {
                        let window = excerpt
                            .checked_sub(1)
                            .and_then(|i| source.excerpts.get(i))
                            .ok_or_else(|| invalid("unknown local onboarding excerpt handle"))?;
                        if source.text[window.byte_start..window.byte_end]
                            .trim()
                            .is_empty()
                        {
                            return Err(invalid(
                                "local onboarding evidence excerpt must not be blank",
                            ));
                        }
                        (window.line_start, window.line_end)
                    }
                    _ => {
                        return Err(invalid(
                            "source reference does not match local plan version",
                        ));
                    }
                };
                let entry = serde_json::json!({ "path": source.path, "fingerprint": source.fingerprint,
                    "line_start": start, "line_end": end });
                sources.push(entry);
            }
            evidence["sources"] = sources.into();
            claims.push(evidence);
        }
        let body = claim_body(&note.name, &claims, &index_link, &roadmap_link);
        metadata.insert("evidence".into(), claims.into());
        let frontmatter =
            serde_json::to_string_pretty(&metadata).map_err(|error| invalid(error.to_string()))?;
        notes.push(LocalOnboardingNote {
            note_type: note.note_type,
            path,
            source: format!("---\n{frontmatter}\n---\n{body}"),
        });
        let link = format!("\n[[{target}]]\n");
        index.push_str(&link);
        roadmap.push_str(&link);
    }
    let proposal = LocalOnboardingProposal {
        notes,
        index,
        roadmap,
    };
    let json = serde_json::to_string(&proposal).map_err(|error| invalid(error.to_string()))?;
    preview_local_onboarding(request, packet, &json)
}

fn quote_lines(
    source: &LocalOnboardingSource,
    quote: &str,
) -> Result<(usize, usize), OnboardingBatchError> {
    if quote.trim().is_empty() || quote.chars().count() > 1024 {
        return Err(invalid(
            "source quotes require 1..=1024 nonblank characters",
        ));
    }
    let mut matches = source.text.match_indices(quote);
    let (offset, _) = matches
        .next()
        .ok_or_else(|| invalid("source quote does not match exact source text"))?;
    // Look for overlapping occurrences too (e.g. `aa` within `aaa`).
    let next = offset
        + source.text[offset..]
            .chars()
            .next()
            .expect("nonempty quote")
            .len_utf8();
    if source.text[next..].contains(quote) {
        return Err(invalid(
            "source quote is ambiguous; supply a unique longer excerpt",
        ));
    }
    let start = source.text[..offset]
        .bytes()
        .filter(|&b| b == b'\n')
        .count()
        + 1;
    let end = start
        + quote
            .strip_suffix('\n')
            .unwrap_or(quote)
            .bytes()
            .filter(|&b| b == b'\n')
            .count();
    Ok((start, end))
}

fn claim_body(name: &str, claims: &[serde_json::Value], index: &str, roadmap: &str) -> String {
    use std::fmt::Write as _;
    let mut body = format!("\n# {name}\n\n## Recorded claims\n");
    for (index, claim) in claims.iter().enumerate() {
        let kind = match claim["kind"].as_str() {
            Some("fact") => "Fact",
            Some("inference") => "Inference",
            _ => "Unknown",
        };
        let _ = write!(
            body,
            "\n{}. **{kind}:** {}\n",
            index + 1,
            claim["claim"].as_str().unwrap_or("")
        );
        if let Some(rationale) = claim["rationale"].as_str() {
            let _ = writeln!(body, "   Rationale: {rationale}");
        }
    }
    let _ = write!(body, "\n[[{index}|Index]] · [[{roadmap}|Roadmap]]\n");
    body
}

fn review_local_proposal(
    packet: &LocalOnboardingPacket,
    proposal: &LocalOnboardingProposal,
    index: &str,
    roadmap: &str,
) -> Result<LocalOnboardingReview, OnboardingBatchError> {
    let mut notes = Vec::new();
    let mut cited = BTreeSet::new();
    for note in &proposal.notes {
        let parsed = parse_leading_frontmatter_bytes(note.source.as_bytes())
            .map_err(|e| invalid(e.to_string()))?;
        let claims = parsed.metadata["evidence"]
            .as_array()
            .ok_or_else(|| invalid("missing claims"))?;
        let name = note
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| invalid("invalid note name"))?;
        if parsed.body != claim_body(name, claims, index, roadmap) {
            return Err(invalid(
                "local draft body must be rendered from its evidence claims; regenerate with a claim-only plan",
            ));
        }
        let mut reviewed_claims = Vec::new();
        for claim in claims {
            let mut sources = Vec::new();
            for reference in claim["sources"]
                .as_array()
                .ok_or_else(|| invalid("missing claim sources"))?
            {
                let source = packet
                    .sources
                    .iter()
                    .find(|s| reference["path"].as_str() == Some(&s.path))
                    .ok_or_else(|| invalid("unknown review source"))?;
                let line = |key| {
                    reference[key]
                        .as_u64()
                        .and_then(|v| usize::try_from(v).ok())
                        .filter(|&v| v > 0)
                        .ok_or_else(|| {
                            invalid("local draft evidence requires positive source line bounds")
                        })
                };
                let start = line("line_start")?;
                let end = line("line_end")?;
                if end < start || end > source.lines {
                    return Err(invalid(
                        "local draft evidence line bounds exceed the source",
                    ));
                }
                let excerpt = source
                    .text
                    .lines()
                    .skip(start - 1)
                    .take(end - start + 1)
                    .collect::<Vec<_>>()
                    .join("\n");
                if excerpt.len() > 2048 {
                    return Err(invalid(
                        "local draft review excerpt exceeds 2048 bytes; select a shorter range",
                    ));
                }
                cited.insert(source.path.clone());
                sources.push(LocalSourceReview {
                    path: source.path.clone(),
                    fingerprint: source.fingerprint.clone(),
                    line_start: start,
                    line_end: end,
                    excerpt,
                });
            }
            reviewed_claims.push(LocalClaimReview {
                kind: claim["kind"].as_str().unwrap_or("").into(),
                claim: claim["claim"].as_str().unwrap_or("").into(),
                rationale: claim["rationale"].as_str().map(str::to_owned),
                sources,
            });
        }
        let mut fields = parsed
            .metadata
            .as_object()
            .expect("validated metadata mapping")
            .clone();
        fields.remove("evidence");
        notes.push(LocalNoteReview {
            note_type: note.note_type.clone(),
            path: note.path.clone(),
            fields,
            claims: reviewed_claims,
        });
    }
    let review = LocalOnboardingReview {
        requires_review: true,
        notes,
        uncited_sources: packet
            .sources
            .iter()
            .filter(|s| !cited.contains(&s.path))
            .map(|s| s.path.clone())
            .collect(),
    };
    bounded_json(&review)?;
    Ok(review)
}

pub(crate) fn invalid(message: impl Into<String>) -> OnboardingBatchError {
    OnboardingBatchError::Validation {
        path: PathBuf::from("<local-onboarding>"),
        message: message.into(),
    }
}

pub(crate) fn bounded_json<T: Serialize>(value: &T) -> Result<(), OnboardingBatchError> {
    let bytes = serde_json::to_vec(value).map_err(|error| invalid(error.to_string()))?;
    if bytes.len() > MAX_LOCAL_ONBOARDING_BYTES {
        return Err(invalid(
            "local onboarding input/proposal exceeds 16000 serialized bytes; select fewer/smaller sources",
        ));
    }
    Ok(())
}

fn read_relative(
    boundary: &Path,
    relative: &str,
    limit: usize,
) -> Result<String, OnboardingBatchError> {
    validate_normalized_relative_path(relative, "local onboarding source", boundary)?;
    let path = boundary.join(relative);
    let io_error = |source| OnboardingBatchError::FileSystem {
        operation: "read local onboarding source",
        path: path.clone(),
        source,
    };
    let mut cursor = boundary.to_path_buf();
    for part in relative.split('/') {
        cursor.push(part);
        let metadata = fs::symlink_metadata(&cursor).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            return Err(invalid(
                "local onboarding source paths must not contain symlinks",
            ));
        }
    }
    if fs::canonicalize(&path).map_err(io_error)? != path {
        return Err(invalid(
            "local onboarding source identity changed or escaped its boundary",
        ));
    }
    if !fs::symlink_metadata(&path).map_err(io_error)?.is_file() {
        return Err(invalid("local onboarding sources must be regular files"));
    }
    let file = File::open(&path).map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(invalid(
            "local onboarding source changed to a nonregular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > limit {
        return Err(invalid(format!(
            "local onboarding source {relative:?} exceeds {limit} bytes"
        )));
    }
    let text =
        String::from_utf8(bytes).map_err(|_| invalid("local onboarding sources must be UTF-8"))?;
    if text.contains('\0') {
        return Err(invalid("local onboarding sources must not contain NUL"));
    }
    Ok(text)
}

/// Read only explicitly selected repository sources; never recurse or silently truncate.
pub fn prepare_local_onboarding(
    request: &ResolveRequest,
    paths: &[String],
) -> Result<LocalOnboardingPacket, OnboardingBatchError> {
    if paths.is_empty() || paths.len() > MAX_LOCAL_ONBOARDING_FILES {
        return Err(invalid(
            "local onboarding requires 1..=8 explicit source paths",
        ));
    }
    let unique = paths.iter().collect::<BTreeSet<_>>();
    if unique.len() != paths.len() {
        return Err(invalid("duplicate local onboarding source path"));
    }
    let (mut packet, repository) = local_onboarding_base(request)?;
    for path in unique {
        packet
            .sources
            .push(read_local_onboarding_source(&repository, path)?);
    }
    bounded_json(&packet)?;
    Ok(packet)
}

pub(crate) fn read_local_onboarding_source(
    repository: &Path,
    path: &str,
) -> Result<LocalOnboardingSource, OnboardingBatchError> {
    let text = read_relative(repository, path, MAX_LOCAL_ONBOARDING_FILE_BYTES)?;
    Ok(LocalOnboardingSource {
        path: path.into(),
        fingerprint: content_fingerprint(text.as_bytes()),
        lines: text.lines().count().max(1),
        excerpts: source_excerpts(&text),
        text,
    })
}

pub(crate) fn local_onboarding_base(
    request: &ResolveRequest,
) -> Result<(LocalOnboardingPacket, PathBuf), OnboardingBatchError> {
    let preparation = prepare_onboarding(request)?;
    let repository = preparation.repository_dir.clone();
    let config = load_root_config(&preparation.root)?;
    let projection = |path: &Path| {
        read_relative(
            &preparation.project_dir,
            path.to_str()
                .ok_or_else(|| invalid("projection path is not UTF-8"))?,
            MAX_LOCAL_ONBOARDING_BYTES,
        )
    };
    let packet = LocalOnboardingPacket {
        schema_version: 2,
        index: projection(&config.project.index)?,
        roadmap: projection(&config.project.roadmap)?,
        preparation: serde_json::to_value(preparation)
            .map_err(|error| invalid(error.to_string()))?,
        sources: Vec::new(),
    };
    bounded_json(&packet)?;
    Ok((packet, repository))
}

/// Rebuild exact input, constrain evidence to selected sources, then use the shared preview.
pub fn preview_local_onboarding(
    request: &ResolveRequest,
    packet: &LocalOnboardingPacket,
    proposal_json: &str,
) -> Result<LocalOnboardingDraft, OnboardingBatchError> {
    bounded_json(packet)?;
    let paths = packet
        .sources
        .iter()
        .map(|source| source.path.clone())
        .collect::<Vec<_>>();
    let fresh = prepare_local_onboarding(request, &paths)?;
    if packet != &fresh {
        return Err(OnboardingBatchError::Conflict {
            path: PathBuf::from("<local-onboarding>"),
            message: "local onboarding packet is stale or altered; prepare again".into(),
        });
    }
    if proposal_json.len() > MAX_LOCAL_ONBOARDING_BYTES {
        return Err(invalid("local onboarding proposal exceeds 16000 bytes"));
    }
    let proposal: LocalOnboardingProposal = serde_json::from_str(proposal_json)
        .map_err(|error| invalid(format!("invalid local onboarding proposal JSON: {error}")))?;
    bounded_json(&proposal)?;
    if proposal.notes.is_empty() || proposal.notes.len() > 3 {
        return Err(invalid("local onboarding draft requires 1..=3 notes"));
    }
    // Check membership before shared preview can open any model-named evidence path.
    for note in &proposal.notes {
        let parsed = parse_leading_frontmatter_bytes(note.source.as_bytes())
            .map_err(|error| invalid(error.to_string()))?;
        let claims = parsed
            .metadata
            .get("evidence")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| invalid("local onboarding notes require evidence"))?;
        for claim in claims {
            if let Some(sources) = claim.get("sources") {
                let sources = sources
                    .as_array()
                    .ok_or_else(|| invalid("evidence sources must be a list"))?;
                for source in sources {
                    if !fresh.sources.iter().any(|known| {
                        source.get("path").and_then(serde_json::Value::as_str) == Some(&known.path)
                            && source
                                .get("fingerprint")
                                .and_then(serde_json::Value::as_str)
                                == Some(&known.fingerprint)
                    }) {
                        return Err(invalid(
                            "proposal evidence must use only exact sources from the local onboarding packet",
                        ));
                    }
                }
            }
        }
    }
    let preview = preview_onboarding_batch(&proposal.request(request))?;
    // The general batch API checks canonical-note links. Local drafts also require usable
    // projection links and coverage, so generated navigation cannot point at missing notes.
    let config = load_root_config(&preview.root)?;
    let mut targets = BTreeSet::new();
    for note in &proposal.notes {
        let note_type = config
            .project
            .note_types
            .get(&note.note_type)
            .ok_or_else(|| invalid("configured note type changed during local preview"))?;
        targets.insert(preview.project_dir.join(&note_type.folder).join(&note.path));
    }
    for (path, body) in [
        (&config.project.index, &proposal.index),
        (&config.project.roadmap, &proposal.roadmap),
    ] {
        crate::project_validation::validate_wikilinks_with_targets(
            &preview.root,
            &preview.project_dir.join(path),
            body,
            &targets,
        )?;
        let linked = crate::wikilink::parse_wikilinks(body)
            .map_err(|error| invalid(error.to_string()))?
            .into_iter()
            .filter_map(|link| link.target)
            .map(|target| {
                crate::project_validation::wikilink_target_path(target)
                    .map(|path| preview.root.join(path))
            })
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(invalid)?;
        if !targets.is_subset(&linked) {
            return Err(invalid(
                "local draft index and roadmap must each link every proposed note",
            ));
        }
    }
    let relative = |path: &Path| {
        path.strip_prefix(&preview.root)
            .ok()
            .and_then(Path::to_str)
            .map(str::to_owned)
            .ok_or_else(|| invalid("review navigation is not root-relative UTF-8"))
    };
    let review = review_local_proposal(
        &fresh,
        &proposal,
        &relative(&preview.project_dir.join(&config.project.index))?,
        &relative(&preview.project_dir.join(&config.project.roadmap))?,
    )?;
    // Shared validation may take time; observe drift once more before returning the draft.
    if fresh != prepare_local_onboarding(request, &paths)? {
        return Err(invalid("local onboarding input changed during preview"));
    }
    Ok(LocalOnboardingDraft {
        schema_version: 2,
        proposal,
        preview,
        review,
    })
}
