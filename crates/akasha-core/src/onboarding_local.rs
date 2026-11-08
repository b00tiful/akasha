//! Read-only local onboarding input and draft checks. Inference remains in an optional adapter.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::onboarding::{
    OnboardingBatchError, OnboardingBatchPreview, OnboardingBatchRequest, ProposedNote,
    prepare_onboarding, preview_onboarding_batch, validate_normalized_relative_path,
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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalPlan {
    notes: Vec<PlannedNote>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlannedNote {
    note_type: String,
    name: String,
    fields: BTreeMap<String, String>,
    body: String,
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
#[serde(deny_unknown_fields)]
struct PlannedSource {
    source: usize,
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
                let source = reference
                    .source
                    .checked_sub(1)
                    .and_then(|index| packet.sources.get(index))
                    .ok_or_else(|| invalid("unknown local onboarding source handle"))?;
                let entry =
                    serde_json::json!({ "path": source.path, "fingerprint": source.fingerprint });
                sources.push(entry);
            }
            evidence["sources"] = sources.into();
            claims.push(evidence);
        }
        metadata.insert("evidence".into(), claims.into());
        let frontmatter =
            serde_json::to_string_pretty(&metadata).map_err(|error| invalid(error.to_string()))?;
        notes.push(LocalOnboardingNote {
            note_type: note.note_type,
            path,
            source: format!("---\n{frontmatter}\n---\n\n{}\n\n[[{index_link}|Index]] · [[{roadmap_link}|Roadmap]]\n", note.body),
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

fn invalid(message: impl Into<String>) -> OnboardingBatchError {
    OnboardingBatchError::Validation {
        path: PathBuf::from("<local-onboarding>"),
        message: message.into(),
    }
}

fn bounded_json<T: Serialize>(value: &T) -> Result<(), OnboardingBatchError> {
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
    let preparation = prepare_onboarding(request)?;
    let config = load_root_config(&preparation.root)?;
    let mut sources = Vec::new();
    for path in unique {
        let text = read_relative(
            &preparation.repository_dir,
            path,
            MAX_LOCAL_ONBOARDING_FILE_BYTES,
        )?;
        sources.push(LocalOnboardingSource {
            path: path.clone(),
            fingerprint: content_fingerprint(text.as_bytes()),
            lines: text.lines().count().max(1),
            text,
        });
    }
    let projection = |path: &Path| {
        read_relative(
            &preparation.project_dir,
            path.to_str()
                .ok_or_else(|| invalid("projection path is not UTF-8"))?,
            MAX_LOCAL_ONBOARDING_BYTES,
        )
    };
    let packet = LocalOnboardingPacket {
        schema_version: 1,
        index: projection(&config.project.index)?,
        roadmap: projection(&config.project.roadmap)?,
        preparation: serde_json::to_value(preparation)
            .map_err(|error| invalid(error.to_string()))?,
        sources,
    };
    bounded_json(&packet)?;
    Ok(packet)
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
    // Shared validation may take time; observe drift once more before returning the draft.
    if fresh != prepare_local_onboarding(request, &paths)? {
        return Err(invalid("local onboarding input changed during preview"));
    }
    Ok(LocalOnboardingDraft {
        schema_version: 1,
        proposal,
        preview,
    })
}
