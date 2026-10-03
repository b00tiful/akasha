use std::path::PathBuf;

use akasha_core::{
    LinkPlan, LinkRequest, ResolveRequest, apply_project_link, prepare_project_link,
};

pub(super) struct Review {
    pub request: LinkRequest,
    pub plan: LinkPlan,
    pub body: String,
}

pub(super) fn arguments(request: &ResolveRequest, argument: &str) -> Result<LinkRequest, String> {
    let (project, repository) = argument
        .split_once(char::is_whitespace)
        .map_or((argument, ""), |(project, repository)| {
            (project, repository.trim())
        });
    if project.is_empty() {
        return Err(
            "Usage: /link PROJECT [REPOSITORY]. Omitted repository means the launch directory."
                .into(),
        );
    }
    Ok(LinkRequest {
        root_override: request.root_override.clone(),
        project: project.into(),
        repository: (!repository.is_empty()).then(|| PathBuf::from(repository)),
        cwd: request.cwd.clone(),
        environment: request.environment.clone(),
    })
}

impl Review {
    pub fn prepare(mut request: LinkRequest) -> Result<Self, String> {
        let plan = prepare_project_link(&request).map_err(|error| error.to_string())?;
        let destination = &plan.destination;
        // Keep the exact reviewed location if an input alias later changes.
        request.root_override = Some(destination.root.clone());
        request.repository = Some(destination.repository_dir.clone());
        let source = serde_json::to_string(&plan.source).map_err(|error| error.to_string())?;
        let body = format!(
            "REPOSITORY LINK REVIEW · NO FILES CHANGED\n\nProject: {}\nRoot: {:?}\nRegistry: {:?}\nProject directory: {:?}\nRepository: {:?}\nPointer: {:?}\nAction: create only; destination must remain absent\n\nExact source (JSON-escaped UTF-8):\n{source}\n\nPlan ID: {}\n\nReview the destination and source above. To authorize only this plan, enter:\n/confirm {}\n\n/discard cancels without writing. Changed identities or an occupied destination require a fresh review. This creates a pointer for an already registered repository; it does not move or register a project.\n",
            destination.project,
            destination.root,
            destination.registry,
            destination.project_dir,
            destination.repository_dir,
            destination.pointer,
            plan.plan_id,
            plan.plan_id,
        );
        Ok(Self {
            request,
            plan,
            body,
        })
    }

    pub fn commit(request: &LinkRequest, plan: &LinkPlan) -> Result<String, String> {
        let result = apply_project_link(request, plan).map_err(|error| error.to_string())?;
        Ok(format!(
            "Repository linked.\nProject: {}\nRepository: {}\nPointer: {}\n\nThe repository now resolves this project when the same data root is selected.",
            result.project,
            result.repository_dir.display(),
            result.pointer.display()
        ))
    }
}
