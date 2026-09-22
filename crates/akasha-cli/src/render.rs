use std::env;
use std::fmt::{Display, Write as _};
use std::io::{self, IsTerminal, Write as _};

use akasha_core::{
    AgentWiringAction, AgentWiringOperation, AgentWiringPlan, AgentWiringRecovery,
    AgentWiringResult, ContextBundle, EntityUpdateResult, EventCreationResult, InitRecovery,
    InitResult, LinkResult, MutableNoteCreationResult, NoteClass, NoteEditRecovery,
    ProjectValidationReport, RecordUpdateResult, ResolvedProject, SessionBreadcrumb,
    SessionHookWiringAction, SessionHookWiringOperation, SessionHookWiringPlan,
    SessionHookWiringRecovery, SessionHookWiringResult, render_context_markdown,
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

pub(crate) fn render_search(
    result: &akasha_core::LibrarySearchResult,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        writeln!(
            rendered,
            "{} matches; showing {}{}",
            result.total_matches,
            result.hits.len(),
            if result.truncated {
                " (truncated; narrow the query or increase --limit)"
            } else {
                ""
            }
        )
        .expect("writing to a string cannot fail");
        for hit in &result.hits {
            let id: String = hit
                .id
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            writeln!(
                rendered,
                "{}{}\n  {}",
                id,
                hit.line.map(|line| format!(":{line}")).unwrap_or_default(),
                hit.snippet
            )
            .expect("writing to a string cannot fail");
        }
    }
    write_stdout(&rendered)
}

pub(crate) fn render_init(result: &InitResult, output: OutputMode) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(&mut rendered, "initialized", &result.project, output);
        print_field(
            &mut rendered,
            "repository",
            result.repository_dir.display(),
            output,
        );
        print_field(
            &mut rendered,
            "project directory",
            result.project_dir.display(),
            output,
        );
        print_field(
            &mut rendered,
            "project state",
            result.state.display(),
            output,
        );
        print_field(
            &mut rendered,
            "templates copied",
            result.template_files,
            output,
        );
        print_field(&mut rendered, "registry", result.registry.display(), output);
        print_field(&mut rendered, "pointer", result.pointer.display(), output);
        print_field(
            &mut rendered,
            "recovery",
            init_recovery_name(result.recovery),
            output,
        );
    }
    write_stdout(&rendered)
}

fn init_recovery_name(recovery: InitRecovery) -> &'static str {
    match recovery {
        InitRecovery::None => "none",
        InitRecovery::Discarded => "discarded",
        InitRecovery::RolledBack => "rolled-back",
        InitRecovery::Finalized => "finalized",
    }
}

pub(crate) fn render_link(result: &LinkResult, output: OutputMode) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(&mut rendered, "linked", &result.project, output);
        print_field(
            &mut rendered,
            "repository",
            result.repository_dir.display(),
            output,
        );
        print_field(&mut rendered, "pointer", result.pointer.display(), output);
        print_field(
            &mut rendered,
            "project directory",
            result.project_dir.display(),
            output,
        );
    }
    write_stdout(&rendered)
}

pub(crate) fn render_agent_wiring_plan(
    plan: &AgentWiringPlan,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(plan))?;
    } else {
        print_status(
            &mut rendered,
            "prepared agent wiring",
            plan.client.as_str(),
            output,
        );
        print_field(&mut rendered, "data root", plan.root.display(), output);
        print_field(
            &mut rendered,
            "instruction source",
            plan.source.display(),
            output,
        );
        print_field(&mut rendered, "source sha256", &plan.source_sha256, output);
        print_field(&mut rendered, "target", plan.target.display(), output);
        print_field(
            &mut rendered,
            "operation",
            agent_wiring_operation_name(plan.operation),
            output,
        );
        print_field(
            &mut rendered,
            "action",
            agent_wiring_action_name(plan.action),
            output,
        );
        print_field(
            &mut rendered,
            "current sha256",
            plan.current_sha256.as_deref().unwrap_or("absent"),
            output,
        );
        print_field(
            &mut rendered,
            "result sha256",
            plan.result_sha256.as_deref().unwrap_or("absent"),
            output,
        );
        print_field(&mut rendered, "plan id", &plan.plan_id, output);
        print_field(
            &mut rendered,
            "patch range",
            format_args!("{}..{}", plan.patch.start, plan.patch.end),
            output,
        );
        print_field(
            &mut rendered,
            "replacement bytes",
            plan.patch.replacement.len(),
            output,
        );
        if plan.patch.replacement.is_empty() {
            print_field(&mut rendered, "replacement", "none", output);
        } else {
            print_field(&mut rendered, "replacement", "", output);
            rendered.push_str(&plan.patch.replacement);
        }
    }
    write_stdout(&rendered)
}

pub(crate) fn render_agent_wiring_result(
    result: &AgentWiringResult,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(
            &mut rendered,
            match result.operation {
                AgentWiringOperation::Apply => "applied agent wiring",
                AgentWiringOperation::Remove => "removed agent wiring",
            },
            result.client.as_str(),
            output,
        );
        print_field(&mut rendered, "target", result.target.display(), output);
        print_field(
            &mut rendered,
            "action",
            agent_wiring_action_name(result.action),
            output,
        );
        print_field(&mut rendered, "changed", result.changed, output);
        print_field(&mut rendered, "plan id", &result.plan_id, output);
        print_field(
            &mut rendered,
            "recovery",
            agent_wiring_recovery_name(result.recovery),
            output,
        );
    }
    write_stdout(&rendered)
}

pub(crate) fn render_session_hook_wiring_plan(
    plan: &SessionHookWiringPlan,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(plan))?;
    } else {
        print_status(
            &mut rendered,
            "prepared session hook",
            plan.client.as_str(),
            output,
        );
        print_field(&mut rendered, "data root", plan.root.display(), output);
        print_field(&mut rendered, "target", plan.target.display(), output);
        print_field(
            &mut rendered,
            "operation",
            session_hook_operation_name(plan.operation),
            output,
        );
        print_field(
            &mut rendered,
            "action",
            session_hook_action_name(plan.action),
            output,
        );
        print_field(
            &mut rendered,
            "current sha256",
            plan.current_sha256.as_deref().unwrap_or("absent"),
            output,
        );
        print_field(
            &mut rendered,
            "result sha256",
            plan.result_sha256.as_deref().unwrap_or("absent"),
            output,
        );
        print_field(&mut rendered, "plan id", &plan.plan_id, output);
        print_field(
            &mut rendered,
            "patch range",
            format_args!("{}..{}", plan.patch.start, plan.patch.end),
            output,
        );
        print_field(
            &mut rendered,
            "replacement bytes",
            plan.patch.replacement.len(),
            output,
        );
        if plan.patch.replacement.is_empty() {
            print_field(&mut rendered, "replacement", "none", output);
        } else {
            print_field(&mut rendered, "replacement", "", output);
            rendered.push_str(&plan.patch.replacement);
        }
    }
    write_stdout(&rendered)
}

pub(crate) fn render_session_hook_wiring_result(
    result: &SessionHookWiringResult,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(
            &mut rendered,
            match result.operation {
                SessionHookWiringOperation::Apply => "applied session hook",
                SessionHookWiringOperation::Remove => "removed session hook",
            },
            result.client.as_str(),
            output,
        );
        print_field(&mut rendered, "target", result.target.display(), output);
        print_field(
            &mut rendered,
            "action",
            session_hook_action_name(result.action),
            output,
        );
        print_field(&mut rendered, "changed", result.changed, output);
        print_field(&mut rendered, "plan id", &result.plan_id, output);
        print_field(
            &mut rendered,
            "recovery",
            session_hook_recovery_name(result.recovery),
            output,
        );
    }
    write_stdout(&rendered)
}

fn session_hook_operation_name(operation: SessionHookWiringOperation) -> &'static str {
    match operation {
        SessionHookWiringOperation::Apply => "apply",
        SessionHookWiringOperation::Remove => "remove",
    }
}

pub(crate) const fn session_hook_action_name(action: SessionHookWiringAction) -> &'static str {
    match action {
        SessionHookWiringAction::Create => "create",
        SessionHookWiringAction::AddHooks => "add-hooks",
        SessionHookWiringAction::AddSessionStart => "add-session-start",
        SessionHookWiringAction::AppendSessionStart => "append-session-start",
        SessionHookWiringAction::RemoveSessionStartEntry => "remove-session-start-entry",
        SessionHookWiringAction::RemoveSessionStartKey => "remove-session-start-key",
        SessionHookWiringAction::RemoveHooksKey => "remove-hooks-key",
        SessionHookWiringAction::RemoveManagedFile => "remove-managed-file",
        SessionHookWiringAction::NoChange => "no-change",
    }
}

fn session_hook_recovery_name(recovery: SessionHookWiringRecovery) -> &'static str {
    match recovery {
        SessionHookWiringRecovery::None => "none",
        SessionHookWiringRecovery::Discarded => "discarded",
        SessionHookWiringRecovery::Finalized => "finalized",
    }
}

pub(crate) fn render_event_creation(
    result: &EventCreationResult,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(&mut rendered, "created event", &result.id, output);
        print_field(&mut rendered, "project", &result.project, output);
        print_field(&mut rendered, "type", &result.note_type, output);
        print_field(&mut rendered, "path", result.path.display(), output);
        print_field(&mut rendered, "template", result.template.display(), output);
        print_field(
            &mut rendered,
            "template scope",
            template_scope_name(result.template_scope),
            output,
        );
        print_field(
            &mut rendered,
            "project state",
            result.state.display(),
            output,
        );
        print_field(
            &mut rendered,
            "recovery",
            recovery_name(result.recovery),
            output,
        );
    }
    write_stdout(&rendered)
}

pub(crate) fn render_mutable_note_creation(
    result: &MutableNoteCreationResult,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(&mut rendered, "created note", &result.id, output);
        print_field(&mut rendered, "project", &result.project, output);
        print_field(&mut rendered, "type", &result.note_type, output);
        print_field(
            &mut rendered,
            "class",
            note_class_name(result.class),
            output,
        );
        print_field(&mut rendered, "path", result.path.display(), output);
        print_field(&mut rendered, "template", result.template.display(), output);
        print_field(
            &mut rendered,
            "template scope",
            template_scope_name(result.template_scope),
            output,
        );
        print_field(
            &mut rendered,
            "projection",
            result.projection.display(),
            output,
        );
        print_field(
            &mut rendered,
            "projection changed",
            if result.projection_changed {
                "yes"
            } else {
                "no"
            },
            output,
        );
        print_field(
            &mut rendered,
            "project state",
            result.state.display(),
            output,
        );
        print_field(
            &mut rendered,
            "recovery",
            recovery_name(result.recovery),
            output,
        );
    }
    write_stdout(&rendered)
}

pub(crate) fn render_record_update(
    result: &RecordUpdateResult,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(&mut rendered, "updated record", &result.id, output);
        print_field(&mut rendered, "project", &result.project, output);
        print_field(&mut rendered, "type", &result.note_type, output);
        print_field(&mut rendered, "path", result.path.display(), output);
        print_field(
            &mut rendered,
            "record changed",
            if result.changed { "yes" } else { "no" },
            output,
        );
        print_field(&mut rendered, "roadmap", result.roadmap.display(), output);
        print_field(
            &mut rendered,
            "roadmap changed",
            if result.roadmap_changed { "yes" } else { "no" },
            output,
        );
        print_field(
            &mut rendered,
            "project state",
            result.state.display(),
            output,
        );
        print_field(
            &mut rendered,
            "recovery",
            recovery_name(result.recovery),
            output,
        );
    }
    write_stdout(&rendered)
}

pub(crate) fn render_entity_update(
    result: &EntityUpdateResult,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(&mut rendered, "updated entity", &result.id, output);
        print_field(&mut rendered, "project", &result.project, output);
        print_field(&mut rendered, "type", &result.note_type, output);
        print_field(&mut rendered, "path", result.path.display(), output);
        print_field(
            &mut rendered,
            "entity changed",
            if result.changed { "yes" } else { "no" },
            output,
        );
        print_field(&mut rendered, "index", result.index.display(), output);
        print_field(
            &mut rendered,
            "index changed",
            if result.index_changed { "yes" } else { "no" },
            output,
        );
        print_field(
            &mut rendered,
            "project state",
            result.state.display(),
            output,
        );
        print_field(
            &mut rendered,
            "recovery",
            recovery_name(result.recovery),
            output,
        );
    }
    write_stdout(&rendered)
}

pub(crate) fn render_resolution(resolved: &ResolvedProject, output: OutputMode) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(resolved))?;
    } else {
        print_field(&mut rendered, "root", resolved.root.display(), output);
        print_field(
            &mut rendered,
            "root source",
            root_source_name(resolved.root_source),
            output,
        );
        print_field(&mut rendered, "project", &resolved.project, output);
        print_field(
            &mut rendered,
            "project source",
            project_source_name(resolved.project_source),
            output,
        );
        match &resolved.pointer {
            Some(pointer) => print_field(&mut rendered, "pointer", pointer.display(), output),
            None => print_field(
                &mut rendered,
                "pointer",
                "none (project selected explicitly)",
                output,
            ),
        }
        print_field(
            &mut rendered,
            "registry",
            resolved.registry.display(),
            output,
        );
        print_field(
            &mut rendered,
            "repository",
            resolved.repository_dir.display(),
            output,
        );
        print_field(
            &mut rendered,
            "project directory",
            resolved.project_dir.display(),
            output,
        );
    }
    write_stdout(&rendered)
}

pub(crate) fn render_validation(
    report: &ProjectValidationReport,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(report))?;
    } else {
        print_status(&mut rendered, "valid", &report.project, output);
        print_field(&mut rendered, "registry", report.registry.display(), output);
        print_field(
            &mut rendered,
            "repository",
            report.repository_dir.display(),
            output,
        );
        print_field(
            &mut rendered,
            "project directory",
            report.project_dir.display(),
            output,
        );
        print_field(
            &mut rendered,
            "registry projects",
            report.registry_projects,
            output,
        );
        print_field(
            &mut rendered,
            "canonical notes",
            report.canonical_notes,
            output,
        );
        print_field(
            &mut rendered,
            "immutable events",
            report.immutable_events,
            output,
        );
        print_field(
            &mut rendered,
            "project state",
            report.state.display(),
            output,
        );
        for (name, projection) in &report.projections {
            print_field(
                &mut rendered,
                "projection",
                format_args!("{name} — {} sources", projection.sources),
                output,
            );
        }
        print_field(&mut rendered, "wikilinks", report.wikilinks, output);
        for (name, note_type) in &report.note_types {
            print_field(
                &mut rendered,
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
    write_stdout(&rendered)
}

pub(crate) fn render_context(context: &ContextBundle, output: OutputMode) -> io::Result<()> {
    if output.json {
        write_stdout(&json_line(serde_json::to_string_pretty(context))?)
    } else {
        write_stdout(&render_context_markdown(context))
    }
}

pub(crate) fn render_breadcrumb(
    breadcrumb: &SessionBreadcrumb,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        write_stdout(&json_line(serde_json::to_string_pretty(breadcrumb))?)
    } else {
        write_stdout(&render_session_breadcrumb(breadcrumb))
    }
}

fn print_status(rendered: &mut String, label: &str, value: impl Display, output: OutputMode) {
    if output.color {
        writeln!(rendered, "\x1b[1;32m{label}\x1b[0m: {value}")
            .expect("writing to a string cannot fail");
    } else {
        writeln!(rendered, "{label}: {value}").expect("writing to a string cannot fail");
    }
}

fn print_field(rendered: &mut String, label: &str, value: impl Display, output: OutputMode) {
    if output.color {
        writeln!(rendered, "\x1b[1;36m{label}\x1b[0m: {value}")
            .expect("writing to a string cannot fail");
    } else {
        writeln!(rendered, "{label}: {value}").expect("writing to a string cannot fail");
    }
}

fn json_line(serialized: Result<String, serde_json::Error>) -> io::Result<String> {
    let mut rendered = serialized.map_err(io::Error::other)?;
    rendered.push('\n');
    Ok(rendered)
}

fn write_stdout(rendered: &str) -> io::Result<()> {
    io::stdout().lock().write_all(rendered.as_bytes())
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

pub(crate) const fn agent_wiring_action_name(action: AgentWiringAction) -> &'static str {
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
