use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use akasha_core::{
    AgentClient, InitRequest, LinkRequest, ResolveRequest, apply_agent_wiring, assemble_context,
    assemble_session_breadcrumb, create_event, create_mutable_note, initialize_project,
    link_project, prepare_agent_wiring, prepare_agent_wiring_removal, remove_agent_wiring,
    resolve_project, update_entity, update_record, validate_project,
};
use clap::{Parser, Subcommand, ValueEnum};

mod render;

use render::{
    OutputMode, render_agent_wiring_plan, render_agent_wiring_result, render_breadcrumb,
    render_context, render_event_creation, render_init, render_link, render_mutable_note_creation,
    render_resolution, render_validation,
};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AgentClientArgument {
    Codex,
    Claude,
}

impl From<AgentClientArgument> for AgentClient {
    fn from(client: AgentClientArgument) -> Self {
        match client {
            AgentClientArgument::Codex => Self::Codex,
            AgentClientArgument::Claude => Self::Claude,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "akasha",
    version,
    about = "Project-agnostic memory for agent-primary development"
)]
struct Cli {
    /// Use this data root instead of AKASHA_ROOT or the user configuration.
    #[arg(long, global = true, value_name = "PATH")]
    root: Option<PathBuf>,

    /// Use this project instead of searching for a repository pointer.
    #[arg(long, global = true, value_name = "SLUG")]
    project: Option<String>,

    /// Render the command result as JSON.
    #[arg(long, global = true)]
    json: bool,

    /// Disable colored terminal output.
    #[arg(long, global = true)]
    no_color: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create, register, and link a new empty Akasha project.
    Init {
        /// New project slug for the current repository.
        #[arg(value_name = "SLUG")]
        slug: String,
    },
    /// Link an already-registered Akasha project to a repository.
    Link {
        /// Registered project slug to link.
        #[arg(value_name = "SLUG")]
        slug: String,

        /// Repository directory. Defaults to the current directory.
        #[arg(long, value_name = "PATH")]
        repo: Option<PathBuf>,
    },
    /// Create one immutable event from its configured template.
    CreateEvent {
        /// Configured event note type, such as `session` or `handoff`.
        #[arg(value_name = "TYPE")]
        note_type: String,

        /// Markdown path relative to the configured note-type folder.
        #[arg(value_name = "RELATIVE.md")]
        path: PathBuf,

        /// Exact template field in NAME=VALUE form. Repeat for multiple fields.
        #[arg(long, value_name = "NAME=VALUE")]
        field: Vec<String>,
    },
    /// Create one record or entity and accept its complete maintained projection.
    CreateNote {
        /// Configured record or entity note type, such as `task` or `entity`.
        #[arg(value_name = "TYPE")]
        note_type: String,

        /// Markdown path relative to the configured note-type folder.
        #[arg(value_name = "RELATIVE.md")]
        path: PathBuf,

        /// UTF-8 file containing the complete accepted roadmap (record) or index (entity).
        #[arg(long, value_name = "PATH")]
        projection: PathBuf,

        /// Exact template field in NAME=VALUE form. Repeat for multiple fields.
        #[arg(long, value_name = "NAME=VALUE")]
        field: Vec<String>,
    },
    /// Update one record from exact source bytes and accept its complete roadmap.
    UpdateRecord {
        /// Complete vault-relative Markdown identity of the configured project record.
        #[arg(value_name = "ID")]
        id: String,

        /// UTF-8 file containing the exact record source previously read by the caller.
        #[arg(long, value_name = "PATH")]
        expected: PathBuf,

        /// UTF-8 file containing the complete replacement record source.
        #[arg(long, value_name = "PATH")]
        replacement: PathBuf,

        /// UTF-8 file containing the complete accepted roadmap after the update.
        #[arg(long, value_name = "PATH")]
        roadmap: PathBuf,
    },
    /// Update one entity from exact source bytes and accept its complete index.
    UpdateEntity {
        /// Complete vault-relative Markdown identity of the configured project entity.
        #[arg(value_name = "ID")]
        id: String,

        /// UTF-8 file containing the exact entity source previously read by the caller.
        #[arg(long, value_name = "PATH")]
        expected: PathBuf,

        /// UTF-8 file containing the complete replacement entity source.
        #[arg(long, value_name = "PATH")]
        replacement: PathBuf,

        /// UTF-8 file containing the complete accepted index after the update.
        #[arg(long, value_name = "PATH")]
        index: PathBuf,
    },
    /// Prepare an exact, read-only user instruction-file change for one agent client.
    PrepareAgentWiring {
        /// Agent client whose user-level instructions should load Akasha.
        #[arg(value_enum, value_name = "CLIENT")]
        client: AgentClientArgument,

        /// Client configuration directory. Defaults to CODEX_HOME/~/.codex or ~/.claude.
        #[arg(long, value_name = "PATH")]
        home: Option<PathBuf>,

        /// Prepare exact managed-section removal instead of application.
        #[arg(long)]
        remove: bool,
    },
    /// Apply an exact prepared user instruction-file plan for one agent client.
    ApplyAgentWiring {
        /// Agent client whose user-level instructions should load Akasha.
        #[arg(value_enum, value_name = "CLIENT")]
        client: AgentClientArgument,

        /// Exact plan ID returned by prepare-agent-wiring.
        #[arg(long, value_name = "SHA256")]
        plan_id: String,

        /// Client configuration directory. Defaults to CODEX_HOME/~/.codex or ~/.claude.
        #[arg(long, value_name = "PATH")]
        home: Option<PathBuf>,
    },
    /// Remove an exact prepared managed section or Akasha-created instruction file.
    RemoveAgentWiring {
        /// Agent client whose user-level Akasha instructions should be removed.
        #[arg(value_enum, value_name = "CLIENT")]
        client: AgentClientArgument,

        /// Exact plan ID returned by prepare-agent-wiring --remove.
        #[arg(long, value_name = "SHA256")]
        plan_id: String,

        /// Client configuration directory. Defaults to CODEX_HOME/~/.codex or ~/.claude.
        #[arg(long, value_name = "PATH")]
        home: Option<PathBuf>,
    },
    /// Resolve and print the current data root and project identity.
    Resolve,
    /// Validate the selected project's configuration, layout, and canonical notes.
    Validate,
    /// Assemble a deterministic, bounded orientation bundle for the selected project.
    Context,
    /// Print the compact session-start project breadcrumb.
    Breadcrumb,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => ExitCode::from(code),
    }
}

fn run(cli: Cli) -> Result<(), u8> {
    let Cli {
        root,
        project,
        json,
        no_color,
        command,
    } = cli;
    let output = OutputMode::detect(json, no_color);

    if let Some(selected) = project.as_ref() {
        let positional = match &command {
            Command::Init { slug } | Command::Link { slug, .. } => Some(slug),
            Command::CreateEvent { .. }
            | Command::CreateNote { .. }
            | Command::UpdateRecord { .. }
            | Command::UpdateEntity { .. }
            | Command::PrepareAgentWiring { .. }
            | Command::ApplyAgentWiring { .. }
            | Command::RemoveAgentWiring { .. }
            | Command::Resolve
            | Command::Validate
            | Command::Context
            | Command::Breadcrumb => None,
        };
        if let Some(slug) = positional
            && slug != selected
        {
            eprintln!("akasha: command slug {slug:?} does not match --project {selected:?}");
            return Err(3);
        }
    }

    match command {
        Command::Init { slug } => {
            let request = InitRequest::from_process(root, slug).map_err(report_resolution)?;
            let result = initialize_project(&request).map_err(report_init)?;
            render_init(&result, output).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::Link { slug, repo } => {
            let request = LinkRequest::from_process(root, slug, repo).map_err(report_resolution)?;
            let result = link_project(&request).map_err(report_link)?;
            render_link(&result, output).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::CreateEvent {
            note_type,
            path,
            field,
        } => {
            let request = ResolveRequest::from_process(root, project).map_err(report_resolution)?;
            let fields = parse_template_fields(field)?;
            let result = create_event(&request, &note_type, &path, &fields)
                .map_err(report_event_creation)?;
            render_event_creation(&result, output).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::CreateNote {
            note_type,
            path,
            projection,
            field,
        } => {
            let request = ResolveRequest::from_process(root, project).map_err(report_resolution)?;
            let fields = parse_template_fields(field)?;
            let projection_source = read_utf8_input(&projection, "projection")?;
            let result =
                create_mutable_note(&request, &note_type, &path, &fields, &projection_source)
                    .map_err(report_mutable_note_creation)?;
            render_mutable_note_creation(&result, output).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::UpdateRecord {
            id,
            expected,
            replacement,
            roadmap,
        } => {
            let request = ResolveRequest::from_process(root, project).map_err(report_resolution)?;
            let expected_source = read_utf8_input(&expected, "expected record")?;
            let replacement_source = read_utf8_input(&replacement, "replacement record")?;
            let roadmap_source = read_utf8_input(&roadmap, "roadmap")?;
            let result = update_record(
                &request,
                &id,
                &expected_source,
                &replacement_source,
                &roadmap_source,
            )
            .map_err(report_note_edit)?;
            render::render_record_update(&result, output).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::UpdateEntity {
            id,
            expected,
            replacement,
            index,
        } => {
            let request = ResolveRequest::from_process(root, project).map_err(report_resolution)?;
            let expected_source = read_utf8_input(&expected, "expected entity")?;
            let replacement_source = read_utf8_input(&replacement, "replacement entity")?;
            let index_source = read_utf8_input(&index, "index")?;
            let result = update_entity(
                &request,
                &id,
                &expected_source,
                &replacement_source,
                &index_source,
            )
            .map_err(report_note_edit)?;
            render::render_entity_update(&result, output).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::PrepareAgentWiring {
            client,
            home,
            remove,
        } => {
            let client = AgentClient::from(client);
            let home = resolve_agent_home(client, home)?;
            let request = ResolveRequest::from_process(root, None).map_err(report_resolution)?;
            let plan = if remove {
                prepare_agent_wiring_removal(&request, client, &home)
            } else {
                prepare_agent_wiring(&request, client, &home)
            }
            .map_err(report_agent_wiring)?;
            render_agent_wiring_plan(&plan, output).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::ApplyAgentWiring {
            client,
            plan_id,
            home,
        } => {
            let client = AgentClient::from(client);
            let home = resolve_agent_home(client, home)?;
            let request = ResolveRequest::from_process(root, None).map_err(report_resolution)?;
            let result = apply_agent_wiring(&request, client, &home, &plan_id)
                .map_err(report_agent_wiring)?;
            render_agent_wiring_result(&result, output).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::RemoveAgentWiring {
            client,
            plan_id,
            home,
        } => {
            let client = AgentClient::from(client);
            let home = resolve_agent_home(client, home)?;
            let request = ResolveRequest::from_process(root, None).map_err(report_resolution)?;
            let result = remove_agent_wiring(&request, client, &home, &plan_id)
                .map_err(report_agent_wiring)?;
            render_agent_wiring_result(&result, output).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::Resolve => {
            let request = ResolveRequest::from_process(root, project).map_err(report_resolution)?;
            let resolved = resolve_project(&request).map_err(report_resolution)?;
            render_resolution(&resolved, output).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::Validate => {
            let request = ResolveRequest::from_process(root, project).map_err(report_resolution)?;
            let report = validate_project(&request).map_err(report_validation)?;
            render_validation(&report, output).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::Context => {
            let request = ResolveRequest::from_process(root, project).map_err(report_resolution)?;
            let context = assemble_context(&request).map_err(report_context)?;
            render_context(&context, output).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::Breadcrumb => {
            let request = ResolveRequest::from_process(root, project).map_err(report_resolution)?;
            let breadcrumb = assemble_session_breadcrumb(&request).map_err(report_context)?;
            render_breadcrumb(&breadcrumb, output).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
    }

    Ok(())
}

fn report_resolution(error: akasha_core::ResolveError) -> u8 {
    eprintln!("akasha: {error}");
    error.exit_code()
}

fn report_validation(error: akasha_core::ProjectValidationError) -> u8 {
    eprintln!("akasha: {error}");
    error.exit_code()
}

fn report_context(error: akasha_core::ContextError) -> u8 {
    eprintln!("akasha: {error}");
    error.exit_code()
}

fn report_link(error: akasha_core::LinkError) -> u8 {
    eprintln!("akasha: {error}");
    error.exit_code()
}

fn report_init(error: akasha_core::InitError) -> u8 {
    eprintln!("akasha: {error}");
    error.exit_code()
}

fn report_event_creation(error: akasha_core::EventCreationError) -> u8 {
    eprintln!("akasha: {error}");
    error.exit_code()
}

fn report_mutable_note_creation(error: akasha_core::MutableNoteCreationError) -> u8 {
    eprintln!("akasha: {error}");
    error.exit_code()
}

fn report_note_edit(error: akasha_core::NoteEditError) -> u8 {
    eprintln!("akasha: {error}");
    error.exit_code()
}

fn report_agent_wiring(error: akasha_core::AgentWiringError) -> u8 {
    eprintln!("akasha: {error}");
    error.exit_code()
}

fn resolve_agent_home(client: AgentClient, explicit: Option<PathBuf>) -> Result<PathBuf, u8> {
    if let Some(home) = explicit {
        return Ok(home);
    }
    if client == AgentClient::Codex
        && let Some(home) = env::var_os("CODEX_HOME")
    {
        if home.is_empty() {
            eprintln!("akasha: CODEX_HOME is set but empty");
            return Err(3);
        }
        return Ok(PathBuf::from(home));
    }
    let Some(home) = env::var_os("HOME") else {
        eprintln!(
            "akasha: no agent home was provided and HOME{} is not set",
            if client == AgentClient::Codex {
                " or CODEX_HOME"
            } else {
                ""
            }
        );
        return Err(3);
    };
    if home.is_empty() {
        eprintln!("akasha: HOME is set but empty");
        return Err(3);
    }
    Ok(PathBuf::from(home).join(match client {
        AgentClient::Codex => ".codex",
        AgentClient::Claude => ".claude",
    }))
}

fn read_utf8_input(path: &std::path::Path, label: &str) -> Result<String, u8> {
    fs::read_to_string(path).map_err(|error| {
        eprintln!(
            "akasha: failed to read {label} input {}: {error}",
            path.display()
        );
        6
    })
}

fn parse_template_fields(fields: Vec<String>) -> Result<BTreeMap<String, String>, u8> {
    let mut parsed = BTreeMap::new();
    for field in fields {
        let Some((name, value)) = field.split_once('=') else {
            eprintln!("akasha: template fields must use NAME=VALUE syntax; received {field:?}");
            return Err(2);
        };
        if parsed.insert(name.to_owned(), value.to_owned()).is_some() {
            eprintln!("akasha: template field {name:?} was supplied more than once");
            return Err(2);
        }
    }
    Ok(parsed)
}
