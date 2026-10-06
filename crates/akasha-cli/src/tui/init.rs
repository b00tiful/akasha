use akasha_core::{
    InitPlan, InitRecoveryPlan, InitRecoveryResult, InitRequest, ResolveRequest,
    apply_init_recovery, apply_project_init, prepare_init_recovery, prepare_project_init,
};

pub(super) struct RecoveryReview {
    pub request: ResolveRequest,
    pub plan: InitRecoveryPlan,
    pub body: String,
}

impl RecoveryReview {
    pub fn prepare(mut request: ResolveRequest) -> Result<Option<Self>, String> {
        let Some(plan) = prepare_init_recovery(&request).map_err(|error| error.to_string())? else {
            return Ok(None);
        };
        request.root_override = Some(plan.root.clone());
        request.project_override = None;
        let body = format!(
            "{}\nTo authorize only this recovery, enter:\n/confirm {}\n\n/discard cancels without writing. F5 does not recover root initialization.\n",
            crate::render::init_recovery_plan_text(&plan),
            plan.plan_id,
        );
        Ok(Some(Self {
            request,
            plan,
            body,
        }))
    }

    pub fn commit(
        request: &ResolveRequest,
        plan: &InitRecoveryPlan,
    ) -> Result<InitRecoveryResult, String> {
        apply_init_recovery(request, plan).map_err(|error| error.to_string())
    }
}

pub(super) struct Review {
    pub request: InitRequest,
    pub plan: InitPlan,
    pub body: String,
}

pub(super) fn arguments(request: &ResolveRequest, argument: &str) -> Result<InitRequest, String> {
    let (project, repository) = argument
        .split_once(char::is_whitespace)
        .map_or((argument, ""), |(project, repository)| {
            (project, repository.trim())
        });
    if project.is_empty() {
        return Err("Usage: /init PROJECT [REPOSITORY]. Omitted repository means the launch directory; the data root must already exist.".into());
    }
    let mut environment = request.environment.clone();
    if let Some(root) = &environment.akasha_root
        && !root.is_empty()
    {
        environment.akasha_root = Some(request.cwd.join(root).into_os_string());
    }
    Ok(InitRequest {
        root_override: request
            .root_override
            .as_ref()
            .map(|root| request.cwd.join(root)),
        project: project.into(),
        cwd: if repository.is_empty() {
            request.cwd.clone()
        } else {
            request.cwd.join(repository)
        },
        environment,
    })
}

fn exact_bytes(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(source) => format!(
            "UTF-8 JSON: {}",
            serde_json::to_string(source).expect("string serialization")
        ),
        Err(_) => format!(
            "Binary hex: {}",
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
    }
}

impl Review {
    pub fn prepare(mut request: InitRequest) -> Result<Self, String> {
        let plan = prepare_project_init(&request).map_err(|error| error.to_string())?;
        let destination = &plan.destination;
        request.root_override = Some(destination.root.clone());
        request.cwd = destination.repository_dir.clone();
        let mut body = format!(
            "PROJECT INITIALIZATION REVIEW · NO FILES CHANGED\n\nPlan ID: {}\n\nProject: {}\nRoot: {:?}\nRepository: {:?}\nProject directory: {:?}\nTemplates copied from: {:?}\nConfiguration SHA-256: {}\n\nCreate project directory and relative directories:\n",
            plan.plan_id,
            destination.project,
            destination.root,
            destination.repository_dir,
            destination.project_dir,
            plan.template_source,
            plan.configuration_fingerprint,
        );
        for directory in &plan.directories {
            body.push_str(&format!("  {directory:?}\n"));
        }
        body.push_str("\nCreate files relative to project directory (exact bytes):\n");
        for (path, bytes) in &plan.files {
            body.push_str(&format!(
                "\n{path:?} ({} bytes)\n{}\n",
                bytes.len(),
                exact_bytes(bytes)
            ));
        }
        body.push_str(&format!(
            "\nCreate pointer: {:?}\n{}\n\nReplace registry: {:?}\nBefore:\n{}\nAfter (canonical formatting):\n{}\n\nPlan ID: {}\n\nTo authorize only this plan, enter:\n/confirm {}\n\n/discard cancels without writing. Configuration, templates, registry and destinations must still match. Apply uses the existing registry lock and init journal; a persistent lock file may remain even on refusal. Pending init journals refuse without recovery. Use /recover-init for a separate recovery-only review; /recovery and F5 concern only the selected project's note journal. The scaffold is empty; no repository knowledge is inferred. After success use F5 to refresh, then /project {} to select it.\n",
            destination.pointer, exact_bytes(&plan.pointer_source), destination.registry,
            exact_bytes(&plan.registry_before), exact_bytes(&plan.registry_after), plan.plan_id,
            plan.plan_id, destination.project,
        ));
        Ok(Self {
            request,
            plan,
            body,
        })
    }

    pub fn commit(request: &InitRequest, plan: &InitPlan) -> Result<String, String> {
        let result = apply_project_init(request, plan).map_err(|error| error.to_string())?;
        Ok(format!(
            "Project initialized.\nProject: {}\nRepository: {:?}\nProject directory: {:?}\nPointer: {:?}\nTemplates copied: {}\n\nUse F5 to refresh the library, then /project {} to select the empty project. Then /onboard prepares a read-only handoff for your connected agent; it does not start an agent.",
            result.project,
            result.repository_dir,
            result.project_dir,
            result.pointer,
            result.template_files,
            result.project,
        ))
    }
}
