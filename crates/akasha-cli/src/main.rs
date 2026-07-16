use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use akasha_core::{
    InitRequest, LinkRequest, ResolveRequest, assemble_context, create_event, create_mutable_note,
    initialize_project, link_project, resolve_project, validate_project,
};
use clap::{Parser, Subcommand};

mod render;

use render::{
    OutputMode, render_context, render_event_creation, render_init, render_link,
    render_mutable_note_creation, render_resolution, render_validation,
};

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
    /// Resolve and print the current data root and project identity.
    Resolve,
    /// Validate the selected project's configuration, layout, and canonical notes.
    Validate,
    /// Assemble a deterministic, bounded orientation bundle for the selected project.
    Context,
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
            | Command::Resolve
            | Command::Validate
            | Command::Context => None,
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
            let projection_source = fs::read_to_string(&projection).map_err(|error| {
                eprintln!(
                    "akasha: failed to read projection input {}: {error}",
                    projection.display()
                );
                6
            })?;
            let result =
                create_mutable_note(&request, &note_type, &path, &fields, &projection_source)
                    .map_err(report_mutable_note_creation)?;
            render_mutable_note_creation(&result, output).map_err(|error| {
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
