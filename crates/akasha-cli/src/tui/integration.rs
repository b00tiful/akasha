use std::path::PathBuf;

use akasha_core::{
    AgentClient, ResolveRequest, apply_agent_wiring, apply_session_hook_wiring,
    prepare_agent_wiring, prepare_agent_wiring_removal, prepare_session_hook_removal,
    prepare_session_hook_wiring, remove_agent_wiring, remove_session_hook_wiring,
};

use crate::render::{agent_wiring_action_name, session_hook_action_name};

#[derive(Clone)]
pub(super) struct Operation {
    pub request: ResolveRequest,
    pub client: AgentClient,
    pub home: PathBuf,
    pub remove: bool,
    pub instructions: bool,
}

pub(super) struct Review {
    pub operation: Operation,
    pub plan_id: String,
    pub body: String,
}

impl Operation {
    pub fn prepare(mut self) -> Result<Review, String> {
        self.request.project_override = None;
        let (root, target, action, current, result, plan_id, start, end, replacement, source) =
            if self.instructions {
                let prepare = if self.remove {
                    prepare_agent_wiring_removal
                } else {
                    prepare_agent_wiring
                };
                let plan = prepare(&self.request, self.client, &self.home)
                    .map_err(|error| error.to_string())?;
                let source = format!(
                    "Source: {}\nSource SHA-256: {}\n",
                    plan.source.display(),
                    plan.source_sha256
                );
                (
                    plan.root,
                    plan.target,
                    agent_wiring_action_name(plan.action),
                    plan.current_sha256,
                    plan.result_sha256,
                    plan.plan_id,
                    plan.patch.start,
                    plan.patch.end,
                    plan.patch.replacement,
                    source,
                )
            } else {
                let prepare = if self.remove {
                    prepare_session_hook_removal
                } else {
                    prepare_session_hook_wiring
                };
                let plan = prepare(&self.request, self.client, &self.home)
                    .map_err(|error| error.to_string())?;
                (
                    plan.root,
                    plan.target,
                    session_hook_action_name(plan.action),
                    plan.current_sha256,
                    plan.result_sha256,
                    plan.plan_id,
                    plan.patch.start,
                    plan.patch.end,
                    plan.patch.replacement,
                    "Hook installation does not imply client trust or activation.\n".into(),
                )
            };
        // Keep the reviewed canonical root and home even if an input alias later changes.
        self.request.root_override = Some(root);
        self.home = target
            .parent()
            .expect("core target has a parent")
            .to_owned();
        let replacement = serde_json::to_string(&replacement).map_err(|error| error.to_string())?;
        let body = format!(
            "INTEGRATION REVIEW · NO FILES CHANGED\n\nClient: {}\nOperation: {} {}\nTarget: {}\nAction: {action}\n{source}Current SHA-256: {}\nResult SHA-256: {}\nPlan ID: {plan_id}\n\nExact byte range: [{start}, {end})\nReplacement (JSON-escaped UTF-8):\n{replacement}\n\nReview the action, target and patch above. To authorize only this plan, enter:\n/confirm {plan_id}\n\n/discard cancels without writing. Target/source drift requires a fresh review.\n",
            self.client.as_str(),
            if self.remove { "remove" } else { "apply" },
            if self.instructions {
                "instructions"
            } else {
                "hook"
            },
            target.display(),
            current.as_deref().unwrap_or("absent"),
            result.as_deref().unwrap_or("absent")
        );
        Ok(Review {
            operation: self,
            plan_id,
            body,
        })
    }

    pub fn commit(self, plan_id: &str) -> Result<String, String> {
        if self.instructions {
            let commit = if self.remove {
                remove_agent_wiring
            } else {
                apply_agent_wiring
            };
            let result = commit(&self.request, self.client, &self.home, plan_id)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "Instruction operation completed: {}\nTarget: {}\nChanged: {}\nRecovery: {:?}\nPlan ID: {}",
                agent_wiring_action_name(result.action),
                result.target.display(),
                result.changed,
                result.recovery,
                result.plan_id
            ))
        } else {
            let commit = if self.remove {
                remove_session_hook_wiring
            } else {
                apply_session_hook_wiring
            };
            let result = commit(&self.request, self.client, &self.home, plan_id)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "Hook operation completed: {}\nTarget: {}\nChanged: {}\nRecovery: {:?}\nPlan ID: {}\nClient trust and activation are separate.",
                session_hook_action_name(result.action),
                result.target.display(),
                result.changed,
                result.recovery,
                result.plan_id
            ))
        }
    }
}
