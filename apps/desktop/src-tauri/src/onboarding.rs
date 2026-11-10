//! Thin, model-free desktop source preparation and captured-plan review.
use std::path::PathBuf;

use akasha_core::onboarding_discovery::{
    LocalSourceCoverage, discover_local_onboarding, discover_local_onboarding_focused,
    discover_local_onboarding_sampled,
};
use akasha_core::onboarding_local::{
    LocalOnboardingDraft, LocalOnboardingIdentityPlan, LocalOnboardingPacket,
    MAX_LOCAL_ONBOARDING_BYTES, apply_local_onboarding, compile_local_onboarding,
    plan_local_onboarding, prepare_local_onboarding, prepare_local_onboarding_focused,
    prepare_local_onboarding_sampled,
};
use akasha_core::{OnboardingBatchResult, onboarding::OnboardingBatchError};
use serde::{Deserialize, Serialize};

use crate::{DesktopError, request};

#[derive(Debug, Serialize)]
pub struct DesktopOnboardingReview {
    pub draft: LocalOnboardingDraft,
    pub identities: Option<LocalOnboardingIdentityPlan>,
    pub excerpts: Vec<DesktopOnboardingExcerpt>,
}

#[derive(Debug, Serialize)]
pub struct DesktopOnboardingExcerpt {
    pub source: usize,
    pub excerpt: usize,
    pub path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopSourceSelection {
    pub mode: String,
    pub paths: Vec<String>,
    pub focus: Vec<String>,
    pub sample_large_files: bool,
}

#[derive(Debug, Serialize)]
pub struct DesktopOnboardingSources {
    pub root: PathBuf,
    pub project: String,
    /// Compact core packet, ready for the bounded generator input without reserialization.
    pub packet_json: String,
    pub identities: LocalOnboardingIdentityPlan,
    pub excerpts: Vec<DesktopOnboardingExcerpt>,
    pub sampled_sources: Vec<akasha_core::onboarding_local::LocalOnboardingSampleReview>,
    /// Discovery inventory describes its automatic selection only, never a later manual subset.
    pub coverage: Option<LocalSourceCoverage>,
}

pub fn prepare_onboarding_sources(
    root: Option<PathBuf>,
    project: String,
    selection: DesktopSourceSelection,
) -> Result<DesktopOnboardingSources, DesktopError> {
    // Bound IPC input before resolution/scanning. Core owns all path and focus validation.
    if selection.paths.is_empty()
        || selection.paths.len() > 8
        || selection.paths.iter().any(|path| path.len() > 1024)
        || selection.focus.len() > 8
        || selection.focus.iter().any(|term| term.len() > 1024)
    {
        return Err(input_error(
            "Use 1..8 bounded paths and at most 8 focus terms",
        ));
    }
    let request = request(root, Some(project.clone()))?;
    let (packet, coverage) = match selection.mode.as_str() {
        "discover" => {
            let discovery = if selection.focus.is_empty() {
                let discover = if selection.sample_large_files {
                    discover_local_onboarding_sampled
                } else {
                    discover_local_onboarding
                };
                discover(&request, &selection.paths)
            } else {
                discover_local_onboarding_focused(
                    &request,
                    &selection.paths,
                    &selection.focus,
                    selection.sample_large_files,
                )
            }
            .map_err(onboarding_error)?;
            (discovery.packet, Some(discovery.coverage))
        }
        "files" => (
            if selection.focus.is_empty() {
                let prepare = if selection.sample_large_files {
                    prepare_local_onboarding_sampled
                } else {
                    prepare_local_onboarding
                };
                prepare(&request, &selection.paths)
            } else {
                prepare_local_onboarding_focused(
                    &request,
                    &selection.paths,
                    &selection.focus,
                    selection.sample_large_files,
                )
            }
            .map_err(onboarding_error)?,
            None,
        ),
        _ => return Err(input_error("Choose directory discovery or explicit files")),
    };
    // Rebuild exact source/projection identities before exposing text. Planning is model-free.
    let identities = plan_local_onboarding(&request, &packet).map_err(onboarding_error)?;
    let packet_json = serde_json::to_string(&packet).map_err(|e| input_error(e.to_string()))?;
    let root = serde_json::from_value(packet.preparation["root"].clone())
        .map_err(|e| input_error(format!("Missing prepared root: {e}")))?;
    let sampled_sources = packet
        .sources
        .iter()
        .filter_map(|source| {
            source.sampling.clone().map(|sampling| {
                akasha_core::onboarding_local::LocalOnboardingSampleReview {
                    path: source.path.clone(),
                    total_lines: source.lines,
                    selected_lines: source
                        .excerpts
                        .iter()
                        .map(
                            |excerpt| akasha_core::onboarding_local::LocalOnboardingLineRange {
                                line_start: excerpt.line_start,
                                line_end: excerpt.line_end,
                            },
                        )
                        .collect(),
                    sampling,
                }
            })
        })
        .collect();
    Ok(DesktopOnboardingSources {
        root,
        project,
        packet_json,
        identities,
        excerpts: source_excerpts(&packet),
        sampled_sources,
        coverage,
    })
}

fn source_excerpts(packet: &LocalOnboardingPacket) -> Vec<DesktopOnboardingExcerpt> {
    packet
        .sources
        .iter()
        .enumerate()
        .flat_map(|(source, file)| {
            file.excerpts
                .iter()
                .enumerate()
                .map(move |(excerpt, window)| DesktopOnboardingExcerpt {
                    source: source + 1,
                    excerpt: excerpt + 1,
                    path: file.path.clone(),
                    line_start: window.line_start,
                    line_end: window.line_end,
                    text: file.text[window.byte_start..window.byte_end].to_owned(),
                })
        })
        .collect()
}

fn input_error(message: impl Into<String>) -> DesktopError {
    DesktopError {
        code: 4,
        message: message.into(),
    }
}

fn onboarding_error(error: OnboardingBatchError) -> DesktopError {
    DesktopError {
        code: error.exit_code(),
        message: error.to_string(),
    }
}

fn packet(json: &str) -> Result<LocalOnboardingPacket, DesktopError> {
    if json.len() > MAX_LOCAL_ONBOARDING_BYTES {
        return Err(input_error("Captured packet exceeds 16000 bytes"));
    }
    serde_json::from_str(json)
        .map_err(|error| input_error(format!("Invalid captured packet: {error}")))
}

pub fn review_onboarding_plan(
    root: Option<PathBuf>,
    project: String,
    packet_json: &str,
    plan_json: &str,
) -> Result<DesktopOnboardingReview, DesktopError> {
    let request = request(root, Some(project))?;
    let packet = packet(packet_json)?;
    // Compile before exposing any captured source text; Rust rebuilds every input.
    let draft = compile_local_onboarding(&request, &packet, plan_json).map_err(onboarding_error)?;
    let plan: serde_json::Value =
        serde_json::from_str(plan_json).map_err(|e| input_error(e.to_string()))?;
    if !matches!(plan["schema_version"].as_u64(), Some(3..=5)) {
        return Err(input_error(
            "Desktop review requires an excerpt-based version-3/4/5 plan",
        ));
    }
    let identities = if matches!(plan["schema_version"].as_u64(), Some(4 | 5)) {
        Some(plan_local_onboarding(&request, &packet).map_err(onboarding_error)?)
    } else {
        None
    };
    let excerpts = source_excerpts(&packet);
    Ok(DesktopOnboardingReview {
        draft,
        identities,
        excerpts,
    })
}

pub fn publish_onboarding_plan(
    root: Option<PathBuf>,
    project: String,
    packet_json: &str,
    plan_json: &str,
    approved_preview_id: &str,
) -> Result<OnboardingBatchResult, DesktopError> {
    let request = request(root, Some(project))?;
    let packet = packet(packet_json)?;
    let draft = compile_local_onboarding(&request, &packet, plan_json).map_err(onboarding_error)?;
    let proposal =
        serde_json::to_string(&draft.proposal).map_err(|e| input_error(e.to_string()))?;
    // No implicit approval or refresh: the existing writer repeats freshness/ID checks
    // and owns project locking, recovery and create-only publication.
    apply_local_onboarding(&request, &packet, &proposal, approved_preview_id)
        .map_err(onboarding_error)
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn prepare_local_onboarding_sources(
    root: Option<PathBuf>,
    project: String,
    selection: DesktopSourceSelection,
) -> Result<DesktopOnboardingSources, DesktopError> {
    tauri::async_runtime::spawn_blocking(move || {
        prepare_onboarding_sources(root, project, selection)
    })
    .await
    .map_err(|e| DesktopError {
        code: 3,
        message: e.to_string(),
    })?
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn review_local_onboarding(
    root: Option<PathBuf>,
    project: String,
    packet_json: String,
    plan_json: String,
) -> Result<DesktopOnboardingReview, DesktopError> {
    tauri::async_runtime::spawn_blocking(move || {
        review_onboarding_plan(root, project, &packet_json, &plan_json)
    })
    .await
    .map_err(|e| DesktopError {
        code: 3,
        message: e.to_string(),
    })?
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn publish_local_onboarding(
    root: Option<PathBuf>,
    project: String,
    packet_json: String,
    plan_json: String,
    approved_preview_id: String,
) -> Result<OnboardingBatchResult, DesktopError> {
    tauri::async_runtime::spawn_blocking(move || {
        publish_onboarding_plan(
            root,
            project,
            &packet_json,
            &plan_json,
            &approved_preview_id,
        )
    })
    .await
    .map_err(|e| DesktopError {
        code: 3,
        message: e.to_string(),
    })?
}
