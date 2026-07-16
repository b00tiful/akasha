use std::env;
use std::fmt::Display;
use std::io::{self, IsTerminal};

use akasha_core::{
    AgentWiringAction, AgentWiringOperation, AgentWiringPlan, AgentWiringRecovery,
    AgentWiringResult, ContextBundle, EntityUpdateResult, EventCreationResult, InitRecovery,
    InitResult, LinkResult, MutableNoteCreationResult, NoteClass, NoteEditRecovery,
    ProjectValidationReport, RecordUpdateResult, ResolvedProject, SessionBreadcrumb,
    SessionHookWiringAction, SessionHookWiringPlan, render_context_markdown,
    render_session_breadcrumb,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct OutputMode {
    json: bool,
    color: bool,
}

impl OutputMode {
    pub(crate) fn detect(json: bool, no_color: bool) -> Self {
        Self {
            json,
            color: !json
                && io::stdout().is_terminal()
                && !no_color
                && env::var_os("NO_COLOR").is_none(),
        }
    }
}

pub(crate) fn render_init(
    result: &InitResult,
    output: OutputMode,
) -> Result<(), serde_json::Error> {
    if output.json {
        println!("{}", serde_json::to_string_pretty(result)?);
    } else {
        print_status("initialized", &result.project, output);
        print_field("repository", result.repository_dir.display(), output);
        print_field("project directory", result.project_dir.display(), output);
        print_field("project state", result.state.display(), output);
        print_field("templates copied", result.template_files, output);
        print_field("registry", result.registry.display(), output);
        print_field("pointer", result.pointer.display(), output);
        print_field("recovery", init_recovery_name(result.recovery), output);
    }

    Ok(())
}

fn init_recovery_name(recovery: InitRecovery) -> &'static str {
    match recovery {
        InitRecovery::None => "none",
        InitRecovery::Discarded => "discarded",
        InitRecovery::RolledBack => "rolled-back",
        InitRecovery::Finalized => "finalized",
    }
}

pub(crate) fn render_link(
    result: &LinkResult,
    output: OutputMode,
) -> Result<(), serde_json::Error> {
    if output.json {
        println!("{}", serde_json::to_string_pretty(result)?);
    } else {
        print_status("linked", &result.project, output);
        print_field("repository", result.repository_dir.display(), output);
        print_field("pointer", result.pointer.display(), output);
        print_field("project directory", result.project_dir.display(), output);
    }

    Ok(())
}

pub(crate) fn render_agent_wiring_plan(
    plan: &AgentWiringPlan,
    output: OutputMode,
) -> Result<(), serde_json::Error> {
    if output.json {
        println!("{}", serde_json::to_string_pretty(plan)?);
    } else {
        print_status("prepared agent wiring", plan.client.as_str(), output);
        print_field("data root", plan.root.display(), output);
        print_field("instruction source", plan.source.display(), output);
        print_field("source sha256", &plan.source_sha256, output);
        print_field("target", plan.target.display(), output);
        print_field(
            "operation",
            agent_wiring_operation_name(plan.operation),
            output,
        );
        print_field("action", agent_wiring_action_name(plan.action), output);
        print_field(
            "current sha256",
            plan.current_sha256.as_deref().unwrap_or("absent"),
            output,
        );
        print_field(
            "result sha256",
            plan.result_sha256.as_deref().unwrap_or("absent"),
            output,
        );
        print_field("plan id", &plan.plan_id, output);
        print_field(
            "patch range",
            format_args!("{}..{}", plan.patch.start, plan.patch.end),
            output,
        );
        print_field("replacement bytes", plan.patch.replacement.len(), output);
        if plan.patch.replacement.is_empty() {
            print_field("replacement", "none", output);
        } else {
            print_field("replacement", "", output);
            print!("{}", plan.patch.replacement);
        }
    }
    Ok(())
}

pub(crate) fn render_agent_wiring_result(
    result: &AgentWiringResult,
    output: OutputMode,
) -> Result<(), serde_json::Error> {
    if output.json {
        println!("{}", serde_json::to_string_pretty(result)?);
    } else {
        print_status(
            match result.operation {
                AgentWiringOperation::Apply => "applied agent wiring",
                AgentWiringOperation::Remove => "removed agent wiring",
            },
            result.client.as_str(),
            output,
        );
        print_field("target", result.target.display(), output);
        print_field("action", agent_wiring_action_name(result.action), output);
        print_field("changed", result.changed, output);
        print_field("plan id", &result.plan_id, output);
        print_field(
            "recovery",
            agent_wiring_recovery_name(result.recovery),
            output,
        );
    }
    Ok(())
}

pub(crate) fn render_session_hook_wiring_plan(
    plan: &SessionHookWiringPlan,
    output: OutputMode,
) -> Result<(), serde_json::Error> {
    if output.json {
        println!("{}", serde_json::to_string_pretty(plan)?);
    } else {
        print_status("prepared session hook", plan.client.as_str(), output);
        print_field("data root", plan.root.display(), output);
        print_field("target", plan.target.display(), output);
        print_field("action", session_hook_action_name(plan.action), output);
        print_field(
            "current sha256",
            plan.current_sha256.as_deref().unwrap_or("absent"),
            output,
        );
        print_field("result sha256", &plan.result_sha256, output);
        print_field("plan id", &plan.plan_id, output);
        print_field(
            "patch range",
            format_args!("{}..{}", plan.patch.start, plan.patch.end),
            output,
        );
        print_field("replacement bytes", plan.patch.replacement.len(), output);
        if plan.patch.replacement.is_empty() {
            print_field("replacement", "none", output);
        } else {
            print_field("replacement", "", output);
            print!("{}", plan.patch.replacement);
        }
    }
    Ok(())
}

fn session_hook_action_name(action: SessionHookWiringAction) -> &'static str {
    match action {
        SessionHookWiringAction::Create => "create",
        SessionHookWiringAction::AddHooks => "add-hooks",
        SessionHookWiringAction::AddSessionStart => "add-session-start",
        SessionHookWiringAction::AppendSessionStart => "append-session-start",
        SessionHookWiringAction::NoChange => "no-change",
    }
}

pub(crate) fn render_event_creation(
    result: &EventCreationResult,
    output: OutputMode,
) -> Result<(), serde_json::Error> {
    if output.json {
        println!("{}", serde_json::to_string_pretty(result)?);
    } else {
        print_status("created event", &result.id, output);
        print_field("project", &result.project, output);
        print_field("type", &result.note_type, output);
        print_field("path", result.path.display(), output);
        print_field("template", result.template.display(), output);
        print_field(
            "template scope",
            template_scope_name(result.template_scope),
            output,
        );
        print_field("project state", result.state.display(), output);
        print_field("recovery", recovery_name(result.recovery), output);
    }
    Ok(())
}

pub(crate) fn render_mutable_note_creation(
    result: &MutableNoteCreationResult,
    output: OutputMode,
) -> Result<(), serde_json::Error> {
    if output.json {
        println!("{}", serde_json::to_string_pretty(result)?);
    } else {
        print_status("created note", &result.id, output);
        print_field("project", &result.project, output);
        print_field("type", &result.note_type, output);
        print_field("class", note_class_name(result.class), output);
        print_field("path", result.path.display(), output);
        print_field("template", result.template.display(), output);
        print_field(
            "template scope",
            template_scope_name(result.template_scope),
            output,
        );
        print_field("projection", result.projection.display(), output);
        print_field(
            "projection changed",
            if result.projection_changed {
                "yes"
            } else {
                "no"
            },
            output,
        );
        print_field("project state", result.state.display(), output);
        print_field("recovery", recovery_name(result.recovery), output);
    }
    Ok(())
}

pub(crate) fn render_record_update(
    result: &RecordUpdateResult,
    output: OutputMode,
) -> Result<(), serde_json::Error> {
    if output.json {
        println!("{}", serde_json::to_string_pretty(result)?);
    } else {
        print_status("updated record", &result.id, output);
        print_field("project", &result.project, output);
        print_field("type", &result.note_type, output);
        print_field("path", result.path.display(), output);
        print_field(
            "record changed",
            if result.changed { "yes" } else { "no" },
            output,
        );
        print_field("roadmap", result.roadmap.display(), output);
        print_field(
            "roadmap changed",
            if result.roadmap_changed { "yes" } else { "no" },
            output,
        );
        print_field("project state", result.state.display(), output);
        print_field("recovery", recovery_name(result.recovery), output);
    }
    Ok(())
}

pub(crate) fn render_entity_update(
    result: &EntityUpdateResult,
    output: OutputMode,
) -> Result<(), serde_json::Error> {
    if output.json {
        println!("{}", serde_json::to_string_pretty(result)?);
    } else {
        print_status("updated entity", &result.id, output);
        print_field("project", &result.project, output);
        print_field("type", &result.note_type, output);
        print_field("path", result.path.display(), output);
        print_field(
            "entity changed",
            if result.changed { "yes" } else { "no" },
            output,
        );
        print_field("index", result.index.display(), output);
        print_field(
            "index changed",
            if result.index_changed { "yes" } else { "no" },
            output,
        );
        print_field("project state", result.state.display(), output);
        print_field("recovery", recovery_name(result.recovery), output);
    }
    Ok(())
}

pub(crate) fn render_resolution(
    resolved: &ResolvedProject,
    output: OutputMode,
) -> Result<(), serde_json::Error> {
    if output.json {
        println!("{}", serde_json::to_string_pretty(resolved)?);
    } else {
        print_field("root", resolved.root.display(), output);
        print_field(
            "root source",
            root_source_name(resolved.root_source),
            output,
        );
        print_field("project", &resolved.project, output);
        print_field(
            "project source",
            project_source_name(resolved.project_source),
            output,
        );
        match &resolved.pointer {
            Some(pointer) => print_field("pointer", pointer.display(), output),
            None => print_field("pointer", "none (project selected explicitly)", output),
        }
        print_field("registry", resolved.registry.display(), output);
        print_field("repository", resolved.repository_dir.display(), output);
        print_field("project directory", resolved.project_dir.display(), output);
    }

    Ok(())
}

pub(crate) fn render_validation(
    report: &ProjectValidationReport,
    output: OutputMode,
) -> Result<(), serde_json::Error> {
    if output.json {
        println!("{}", serde_json::to_string_pretty(report)?);
    } else {
        print_status("valid", &report.project, output);
        print_field("registry", report.registry.display(), output);
        print_field("repository", report.repository_dir.display(), output);
        print_field("project directory", report.project_dir.display(), output);
        print_field("registry projects", report.registry_projects, output);
        print_field("canonical notes", report.canonical_notes, output);
        print_field("immutable events", report.immutable_events, output);
        print_field("project state", report.state.display(), output);
        for (name, projection) in &report.projections {
            print_field(
                "projection",
                format_args!("{name} — {} sources", projection.sources),
                output,
            );
        }
        print_field("wikilinks", report.wikilinks, output);
        for (name, note_type) in &report.note_types {
            print_field(
                "note type",
                format_args!(
                    "{name} ({}) — {}",
                    note_class_name(note_type.class),
                    note_type.notes
                ),
                output,
            );
        }
    }

    Ok(())
}

pub(crate) fn render_context(
    context: &ContextBundle,
    output: OutputMode,
) -> Result<(), serde_json::Error> {
    if output.json {
        println!("{}", serde_json::to_string_pretty(context)?);
    } else {
        print!("{}", render_context_markdown(context));
    }
    Ok(())
}

pub(crate) fn render_breadcrumb(
    breadcrumb: &SessionBreadcrumb,
    output: OutputMode,
) -> Result<(), serde_json::Error> {
    if output.json {
        println!("{}", serde_json::to_string_pretty(breadcrumb)?);
    } else {
        print!("{}", render_session_breadcrumb(breadcrumb));
    }
    Ok(())
}

fn print_status(label: &str, value: impl Display, output: OutputMode) {
    if output.color {
        println!("\x1b[1;32m{label}\x1b[0m: {value}");
    } else {
        println!("{label}: {value}");
    }
}

fn print_field(label: &str, value: impl Display, output: OutputMode) {
    if output.color {
        println!("\x1b[1;36m{label}\x1b[0m: {value}");
    } else {
        println!("{label}: {value}");
    }
}

const fn root_source_name(source: akasha_core::RootSource) -> &'static str {
    match source {
        akasha_core::RootSource::CommandLine => "command-line",
        akasha_core::RootSource::Environment => "environment",
        akasha_core::RootSource::UserConfig => "user-config",
    }
}

const fn project_source_name(source: akasha_core::ProjectSource) -> &'static str {
    match source {
        akasha_core::ProjectSource::CommandLine => "command-line",
        akasha_core::ProjectSource::Pointer => "pointer",
    }
}

const fn note_class_name(class: NoteClass) -> &'static str {
    match class {
        NoteClass::Event => "event",
        NoteClass::Record => "record",
        NoteClass::Entity => "entity",
    }
}

const fn template_scope_name(scope: akasha_core::NoteTemplateScope) -> &'static str {
    match scope {
        akasha_core::NoteTemplateScope::Project => "project",
        akasha_core::NoteTemplateScope::Root => "root",
    }
}

const fn recovery_name(recovery: NoteEditRecovery) -> &'static str {
    match recovery {
        NoteEditRecovery::None => "none",
        NoteEditRecovery::Discarded => "discarded",
        NoteEditRecovery::RolledBack => "rolled-back",
        NoteEditRecovery::Finalized => "finalized",
    }
}

const fn agent_wiring_action_name(action: AgentWiringAction) -> &'static str {
    match action {
        AgentWiringAction::Create => "create",
        AgentWiringAction::Append => "append",
        AgentWiringAction::RefreshManagedSection => "refresh-managed-section",
        AgentWiringAction::RemoveManagedSection => "remove-managed-section",
        AgentWiringAction::RemoveCreatedFile => "remove-created-file",
        AgentWiringAction::NoChange => "no-change",
    }
}

const fn agent_wiring_operation_name(operation: AgentWiringOperation) -> &'static str {
    match operation {
        AgentWiringOperation::Apply => "apply",
        AgentWiringOperation::Remove => "remove",
    }
}

const fn agent_wiring_recovery_name(recovery: AgentWiringRecovery) -> &'static str {
    match recovery {
        AgentWiringRecovery::None => "none",
        AgentWiringRecovery::Discarded => "discarded",
        AgentWiringRecovery::Finalized => "finalized",
    }
}
