//! Thin, model-free desktop handoff for captured local plans.
use std::path::PathBuf;

use akasha_core::onboarding_local::{
    LocalOnboardingDraft, LocalOnboardingIdentityPlan, LocalOnboardingPacket,
    MAX_LOCAL_ONBOARDING_BYTES, apply_local_onboarding, compile_local_onboarding,
    plan_local_onboarding,
};
use akasha_core::{OnboardingBatchResult, onboarding::OnboardingBatchError};
use serde::Serialize;

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
    let excerpts = packet
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
        .collect();
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
