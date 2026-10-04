//! Bounded, client-neutral instructions; no client detection, registration or invocation.
use crate::{OnboardingBatchError, ResolveRequest, prepare_onboarding};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OnboardingHandoff {
    pub root: PathBuf,
    pub project: String,
    pub repository: PathBuf,
    pub command: String,
    pub args: Vec<String>,
    pub agent_request: String,
    pub next_step: String,
}

/// Validate readiness without writing, then describe how an external agent can proceed.
pub fn prepare_onboarding_handoff(
    request: &ResolveRequest,
) -> Result<OnboardingHandoff, OnboardingBatchError> {
    let prepared = prepare_onboarding(request)?;
    Ok(OnboardingHandoff {
        args: vec!["--root".into(), prepared.root.to_str().expect("preparation checks UTF-8").into(),
            "--project".into(), prepared.project.clone(), "--repo".into(),
            prepared.repository_dir.to_str().expect("preparation checks UTF-8").into()],
        command: "akasha-onboarding-mcp".into(),
        agent_request: "Onboard this repository into its initialized Akasha project. Call akasha_onboarding_prepare, inspect the repository using your own tools, and propose only supported knowledge using the supplied templates and coverage criteria. Distinguish facts, inferences and unknowns; persist repository-relative source paths and whole-file SHA-256 fingerprints, with rationale for inferences and unknowns. Do not invent historical decisions, problems or tasks. Call akasha_onboarding_validate and akasha_onboarding_preview; show the exact summary and obtain human approval through the MCP host before akasha_onboarding_apply. Keep preview and approved apply in one server lifetime. After an uncertain result, inspect current state and obtain a fresh preview before another approved apply. Finish with project validation and report remaining unknowns.".into(),
        next_step: "No agent connection has been detected or started. If these tools are already available, send the agent request below. Otherwise build/install akasha-onboarding-mcp (from source: cargo build -p akasha-mcp), configure a temporary local stdio server in your MCP-capable client using the exact command/argument array below (or the executable's absolute path), then send the request. Host approval is required for apply. Offline use remains available; no client configuration was changed.".into(),
        root: prepared.root,
        project: prepared.project,
        repository: prepared.repository_dir,
    })
}
