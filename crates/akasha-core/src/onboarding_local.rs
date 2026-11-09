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
pub const MAX_LOCAL_ONBOARDING_SAMPLED_FILE_BYTES: usize = 262_144;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalOnboardingSource {
    pub path: String,
    pub fingerprint: String,
    pub text: String,
    pub lines: usize,
    pub excerpts: Vec<LocalOnboardingExcerpt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling: Option<LocalOnboardingSampling>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalOnboardingLineRange {
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalOnboardingSampling {
    pub total_bytes: usize,
    pub omitted_lines: Vec<LocalOnboardingLineRange>,
}

/// One complete-line window. UTF-8 byte boundaries refer to the packet's source text;
/// line ranges always refer to the original file, including in sampled packets.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<LocalOnboardingFocus>,
    pub preparation: serde_json::Value,
    pub sources: Vec<LocalOnboardingSource>,
    pub index: String,
    pub roadmap: String,
}

/// Literal, operator-owned targeting. Stored in the packet so rebuilding cannot lose it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalOnboardingFocus {
    pub terms: Vec<String>,
    pub sample_large_files: bool,
}

pub(crate) fn checked_focus(terms: &[String]) -> Result<Vec<String>, OnboardingBatchError> {
    if terms.is_empty()
        || terms.len() > 8
        || terms.iter().any(|term| {
            term.len() > 80 || term.trim().is_empty() || term.chars().any(char::is_control)
        })
    {
        return Err(invalid(
            "focus requires 1..=8 nonblank literal terms, at most 80 UTF-8 bytes each, without control characters",
        ));
    }
    let normalized: Vec<_> = terms
        .iter()
        .map(|term| term.trim().to_lowercase())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if normalized.iter().any(|term| term.len() > 80) {
        return Err(invalid("normalized focus terms exceed 80 UTF-8 bytes"));
    }
    Ok(normalized)
}

pub(crate) fn focus_matches(terms: &[String], path: &str, text: &str) -> Vec<String> {
    if terms.is_empty() {
        return Vec::new();
    }
    let path = path.to_lowercase();
    let text = text.to_lowercase();
    terms
        .iter()
        .filter(|term| path.contains(term.as_str()) || text.contains(term.as_str()))
        .cloned()
        .collect()
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
    pub sampled_sources: Vec<LocalOnboardingSampleReview>,
    /// Matching text windows with no overlapping claim citation, not a semantic coverage score.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub uncited_focus_excerpts: Vec<LocalFocusExcerptGap>,
}

#[derive(Debug, Serialize)]
pub struct LocalFocusExcerptGap {
    pub path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub matched_terms: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LocalOnboardingSampleReview {
    pub path: String,
    pub total_lines: usize,
    pub selected_lines: Vec<LocalOnboardingLineRange>,
    #[serde(flatten)]
    pub sampling: LocalOnboardingSampling,
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

/// Deterministic initial-note slots. Roles describe filename groups, not inferred components.
#[derive(Debug, Serialize)]
pub struct LocalOnboardingIdentityPlan {
    pub schema_version: u32,
    pub slots: Vec<LocalOnboardingSlot>,
}

#[derive(Debug, Serialize)]
pub struct LocalOnboardingSlot {
    pub slot: usize,
    pub role: String,
    pub note_type: String,
    pub name: String,
    /// Project-relative destination, using the configured entity folder.
    pub path: PathBuf,
    /// One-based handles in the original packet, never completion-local handles.
    pub sources: Vec<usize>,
    /// Mechanical initial metadata, never model-asserted project state or a review date.
    pub fields: BTreeMap<String, String>,
    /// Custom required metadata needs an explicit reviewed version-4 plan.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unresolved_fields: Vec<String>,
}

/// Plan a bounded partition before inference; an ambiguous entity schema refuses.
pub fn plan_local_onboarding(
    request: &ResolveRequest,
    packet: &LocalOnboardingPacket,
) -> Result<LocalOnboardingIdentityPlan, OnboardingBatchError> {
    bounded_json(packet)?;
    if packet != &rebuild_local_onboarding(request, packet)? {
        return Err(OnboardingBatchError::Conflict {
            path: PathBuf::from("<local-onboarding>"),
            message: "local onboarding packet is stale or altered; prepare again".into(),
        });
    }
    let types = packet.preparation["note_types"]
        .as_object()
        .ok_or_else(|| invalid("missing configured note types"))?;
    let entities = types
        .iter()
        .filter(|(_, schema)| schema["class"] == "entity")
        .collect::<Vec<_>>();
    if entities.len() != 1 {
        return Err(invalid(
            "automatic note planning requires exactly one configured entity type; use explicit components instead",
        ));
    }
    let (note_type, schema) = entities[0];
    let folder = schema["folder"]
        .as_str()
        .ok_or_else(|| invalid("missing entity folder"))?;
    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, source) in packet.sources.iter().enumerate() {
        if source.excerpts.is_empty() || source.text.trim().is_empty() {
            return Err(invalid(
                "automatic note planning requires nonempty source excerpt catalogs",
            ));
        }
        let group = match crate::onboarding_discovery::role(&source.path) {
            Some("tests") => "verification",
            Some("implementation") => "implementation",
            Some(_) => "context",
            None => {
                return Err(invalid(
                    "automatic note planning requires recognized source roles; use explicit components instead",
                ));
            }
        };
        groups.entry(group).or_default().push(index + 1);
    }
    let mut slots = Vec::new();
    for (role, sources) in groups {
        let paths = sources
            .iter()
            .map(|handle| &packet.sources[handle - 1].path)
            .collect::<Vec<_>>();
        // Content changes keep the identity; source-set changes are visible as a new plan.
        // JSON array encoding avoids ambiguous path concatenations. Never suffix a conflict.
        let identity =
            serde_json::to_vec(&(1, note_type, role, paths)).map_err(|e| invalid(e.to_string()))?;
        let digest = content_fingerprint(&identity);
        let name = format!("onboarding-{role}-{}", &digest[7..39]);
        let mut fields = BTreeMap::from([
            ("entity".into(), name.clone()),
            ("kind".into(), "source-group".into()),
            ("status".into(), "draft".into()),
            ("reviewed".into(), "unreviewed".into()),
        ]);
        let mut unresolved_fields = Vec::new();
        let required = schema["required_fields"]
            .as_array()
            .ok_or_else(|| invalid("missing entity required fields"))?;
        for field in required {
            let field = field
                .as_str()
                .ok_or_else(|| invalid("invalid entity required field"))?;
            match field {
                "schema_version" | "evidence" => {}
                "project" => {
                    let project = packet.preparation["project"]
                        .as_str()
                        .ok_or_else(|| invalid("missing project identity"))?;
                    fields.insert(field.into(), project.into());
                }
                "type" => {
                    fields.insert(field.into(), note_type.clone());
                }
                _ if fields.contains_key(field) => {}
                _ => unresolved_fields.push(field.into()),
            }
        }
        slots.push(LocalOnboardingSlot {
            slot: slots.len() + 1,
            role: role.into(),
            note_type: note_type.clone(),
            path: Path::new(folder).join(format!("{name}.md")),
            name,
            sources,
            fields,
            unresolved_fields,
        });
    }
    let plan = LocalOnboardingIdentityPlan {
        schema_version: 2,
        slots,
    };
    bounded_json(&plan)?;
    Ok(plan)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlottedPlan {
    schema_version: u32,
    notes: Vec<SlottedNote>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlottedNote {
    slot: usize,
    fields: Option<BTreeMap<String, String>>,
    evidence: Vec<PlannedClaim>,
}

fn resolve_slotted_plan(
    request: &ResolveRequest,
    packet: &LocalOnboardingPacket,
    json: &str,
) -> Result<LocalPlan, OnboardingBatchError> {
    let plan: SlottedPlan =
        serde_json::from_str(json).map_err(|e| invalid(format!("invalid slotted plan: {e}")))?;
    if ![4, 5].contains(&plan.schema_version) {
        return Err(invalid("slotted plans require schema_version 4 or 5"));
    }
    // Presence matters: even null/empty metadata in a version-5 plan must refuse.
    let raw: serde_json::Value = serde_json::from_str(json).map_err(|e| invalid(e.to_string()))?;
    if plan.schema_version == 5
        && raw["notes"]
            .as_array()
            .is_some_and(|notes| notes.iter().any(|note| note.get("fields").is_some()))
    {
        return Err(invalid("version-5 metadata is host-owned; omit fields"));
    }
    let identities = plan_local_onboarding(request, packet)?;
    if plan.notes.len() != identities.slots.len() {
        return Err(invalid(
            "slotted plan must cover every planned slot exactly once",
        ));
    }
    let mut notes = BTreeMap::new();
    for note in plan.notes {
        let slot = note
            .slot
            .checked_sub(1)
            .and_then(|index| identities.slots.get(index))
            .ok_or_else(|| invalid("unknown planned note slot"))?;
        let fields = if plan.schema_version == 5 {
            if !slot.unresolved_fields.is_empty() {
                return Err(invalid(format!(
                    "automatic metadata cannot supply required fields {:?}; use a reviewed version-4 plan",
                    slot.unresolved_fields
                )));
            }
            slot.fields.clone()
        } else {
            note.fields
                .ok_or_else(|| invalid("version-4 slotted notes require fields"))?
        };
        for claim in &note.evidence {
            for reference in &claim.sources {
                match reference {
                    PlannedSource::Excerpt { source, .. } if slot.sources.contains(source) => {}
                    _ => {
                        return Err(invalid(
                            "planned note evidence must select excerpt handles within its source group",
                        ));
                    }
                }
            }
        }
        if notes
            .insert(
                note.slot,
                PlannedNote {
                    note_type: slot.note_type.clone(),
                    name: slot.name.clone(),
                    fields,
                    evidence: note.evidence,
                },
            )
            .is_some()
        {
            return Err(invalid("duplicate planned note slot"));
        }
    }
    Ok(LocalPlan {
        schema_version: 3,
        notes: notes.into_values().collect(),
    })
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
    let version: serde_json::Value = serde_json::from_str(plan_json)
        .map_err(|error| invalid(format!("invalid local onboarding plan: {error}")))?;
    let plan: LocalPlan = if matches!(version["schema_version"].as_u64(), Some(4 | 5)) {
        resolve_slotted_plan(request, packet, plan_json)?
    } else {
        serde_json::from_str(plan_json)
            .map_err(|error| invalid(format!("invalid local onboarding plan: {error}")))?
    };
    if ![2, 3].contains(&plan.schema_version) {
        return Err(invalid(
            "local onboarding plans require schema_version 2 (quotes) or 3 (excerpt handles); regenerate legacy body plans",
        ));
    }
    if plan.notes.is_empty() || plan.notes.len() > 3 {
        return Err(invalid("local onboarding plan requires 1..=3 notes"));
    }
    bounded_json(packet)?;
    if packet != &rebuild_local_onboarding(request, packet)? {
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
    if source.sampling.is_some() {
        return Err(invalid(
            "sampled sources require excerpt-handle plans, not quotes",
        ));
    }
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
                let excerpt = selected_source_range(source, start, end)?;
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
    let mut uncited_focus_excerpts = Vec::new();
    if let Some(focus) = &packet.focus {
        for source in &packet.sources {
            for excerpt in &source.excerpts {
                let matched_terms = focus_matches(
                    &focus.terms,
                    "",
                    &source.text[excerpt.byte_start..excerpt.byte_end],
                );
                let cited = notes
                    .iter()
                    .flat_map(|note| &note.claims)
                    .flat_map(|claim| &claim.sources)
                    .any(|reference| {
                        reference.path == source.path
                            && reference.line_start <= excerpt.line_end
                            && reference.line_end >= excerpt.line_start
                    });
                if !matched_terms.is_empty() && !cited {
                    uncited_focus_excerpts.push(LocalFocusExcerptGap {
                        path: source.path.clone(),
                        line_start: excerpt.line_start,
                        line_end: excerpt.line_end,
                        matched_terms,
                    });
                }
            }
        }
    }
    let review = LocalOnboardingReview {
        requires_review: true,
        uncited_focus_excerpts,
        notes,
        uncited_sources: packet
            .sources
            .iter()
            .filter(|s| !cited.contains(&s.path))
            .map(|s| s.path.clone())
            .collect(),
        sampled_sources: packet
            .sources
            .iter()
            .filter_map(|source| {
                source
                    .sampling
                    .clone()
                    .map(|sampling| LocalOnboardingSampleReview {
                        path: source.path.clone(),
                        total_lines: source.lines,
                        selected_lines: source
                            .excerpts
                            .iter()
                            .map(|excerpt| LocalOnboardingLineRange {
                                line_start: excerpt.line_start,
                                line_end: excerpt.line_end,
                            })
                            .collect(),
                        sampling,
                    })
            })
            .collect(),
    };
    bounded_json(&review)?;
    Ok(review)
}

// A raw proposal must not cite the gaps between sampled windows. Map original line ranges
// back to the packed text without ever treating two nonadjacent windows as neighboring lines.
fn selected_source_range(
    source: &LocalOnboardingSource,
    start: usize,
    end: usize,
) -> Result<String, OnboardingBatchError> {
    if source.sampling.is_none() {
        return Ok(source
            .text
            .lines()
            .skip(start - 1)
            .take(end - start + 1)
            .collect::<Vec<_>>()
            .join("\n"));
    }
    let mut next = start;
    let mut lines = Vec::new();
    for excerpt in &source.excerpts {
        if excerpt.line_end < next {
            continue;
        }
        if excerpt.line_start > next {
            break;
        }
        let last = end.min(excerpt.line_end);
        lines.extend(
            source.text[excerpt.byte_start..excerpt.byte_end]
                .lines()
                .skip(next - excerpt.line_start)
                .take(last - next + 1),
        );
        next = last + 1;
        if next > end {
            return Ok(lines.join("\n"));
        }
    }
    Err(invalid(
        "local evidence includes lines omitted from the sampled source",
    ))
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
    prepare_local_onboarding_mode(request, paths, false, &[])
}

/// Opt-in deterministic first/middle/last windows from files up to 256 KiB.
/// Whole-file fingerprints and explicit omitted line ranges preserve honest partial coverage.
pub fn prepare_local_onboarding_sampled(
    request: &ResolveRequest,
    paths: &[String],
) -> Result<LocalOnboardingPacket, OnboardingBatchError> {
    prepare_local_onboarding_mode(request, paths, true, &[])
}

/// Target large-file windows with explicit literal terms, retaining the ordinary read bounds.
pub fn prepare_local_onboarding_focused(
    request: &ResolveRequest,
    paths: &[String],
    terms: &[String],
    sample_large_files: bool,
) -> Result<LocalOnboardingPacket, OnboardingBatchError> {
    prepare_local_onboarding_mode(request, paths, sample_large_files, &checked_focus(terms)?)
}

fn prepare_local_onboarding_mode(
    request: &ResolveRequest,
    paths: &[String],
    sample_large_files: bool,
    terms: &[String],
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
    if sample_large_files {
        packet.schema_version = 3;
    }
    if !terms.is_empty() {
        packet.schema_version = 4;
        packet.focus = Some(LocalOnboardingFocus {
            terms: terms.to_vec(),
            sample_large_files,
        });
    }
    for path in unique {
        packet.sources.push(read_local_onboarding_source(
            &repository,
            path,
            sample_large_files,
            terms,
        )?);
    }
    bounded_json(&packet)?;
    Ok(packet)
}

pub(crate) fn read_local_onboarding_source(
    repository: &Path,
    path: &str,
    sample_large_files: bool,
    terms: &[String],
) -> Result<LocalOnboardingSource, OnboardingBatchError> {
    let limit = if sample_large_files {
        MAX_LOCAL_ONBOARDING_SAMPLED_FILE_BYTES
    } else {
        MAX_LOCAL_ONBOARDING_FILE_BYTES
    };
    let text = read_relative(repository, path, limit)?;
    let mut source = LocalOnboardingSource {
        path: path.into(),
        fingerprint: content_fingerprint(text.as_bytes()),
        lines: text.lines().count().max(1),
        excerpts: source_excerpts(&text),
        sampling: None,
        text,
    };
    if source.text.len() > MAX_LOCAL_ONBOARDING_FILE_BYTES {
        if source.excerpts.is_empty() {
            return Err(invalid(
                "sampled source contains a line exceeding 2048 bytes",
            ));
        }
        let total_bytes = source.text.len();
        // Keep the opening context, then prefer the two most relevant complete-line windows.
        // Distinct terms beat repetition; stable original order breaks ties.
        let mut ranked: Vec<_> = source
            .excerpts
            .iter()
            .enumerate()
            .map(|(index, excerpt)| {
                let matches = focus_matches(
                    terms,
                    "",
                    &source.text[excerpt.byte_start..excerpt.byte_end],
                )
                .len();
                (index, matches)
            })
            .filter(|(_, matches)| *matches > 0)
            .collect();
        ranked.sort_by_key(|(index, matches)| (std::cmp::Reverse(*matches), *index));
        let mut indices = BTreeSet::from([0]);
        for (index, _) in ranked {
            indices.insert(index);
            if indices.len() == 3 {
                break;
            }
        }
        for index in [source.excerpts.len() / 2, source.excerpts.len() - 1] {
            if indices.len() == 3 {
                break;
            }
            indices.insert(index);
        }
        let mut text = String::new();
        let mut excerpts = Vec::new();
        let mut omitted_lines = Vec::new();
        let mut next = 1;
        for index in indices {
            let original = &source.excerpts[index];
            if next < original.line_start {
                omitted_lines.push(LocalOnboardingLineRange {
                    line_start: next,
                    line_end: original.line_start - 1,
                });
            }
            let byte_start = text.len();
            text.push_str(&source.text[original.byte_start..original.byte_end]);
            excerpts.push(LocalOnboardingExcerpt {
                byte_start,
                byte_end: text.len(),
                line_start: original.line_start,
                line_end: original.line_end,
            });
            next = original.line_end + 1;
        }
        if next <= source.lines {
            omitted_lines.push(LocalOnboardingLineRange {
                line_start: next,
                line_end: source.lines,
            });
        }
        source.text = text;
        source.excerpts = excerpts;
        source.sampling = Some(LocalOnboardingSampling {
            total_bytes,
            omitted_lines,
        });
    }
    Ok(source)
}

pub(crate) fn rebuild_local_onboarding(
    request: &ResolveRequest,
    packet: &LocalOnboardingPacket,
) -> Result<LocalOnboardingPacket, OnboardingBatchError> {
    let (sample_large_files, terms) = match (packet.schema_version, &packet.focus) {
        (2, None) => (false, Vec::new()),
        (3, None) => (true, Vec::new()),
        (4, Some(focus)) => (focus.sample_large_files, checked_focus(&focus.terms)?),
        _ => return Err(invalid("unsupported local onboarding packet version")),
    };
    let paths = packet
        .sources
        .iter()
        .map(|source| source.path.clone())
        .collect::<Vec<_>>();
    prepare_local_onboarding_mode(request, &paths, sample_large_files, &terms)
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
        focus: None,
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
    let fresh = rebuild_local_onboarding(request, packet)?;
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
    if fresh != rebuild_local_onboarding(request, packet)? {
        return Err(invalid("local onboarding input changed during preview"));
    }
    Ok(LocalOnboardingDraft {
        schema_version: 2,
        proposal,
        preview,
        review,
    })
}
