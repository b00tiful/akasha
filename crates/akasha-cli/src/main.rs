use std::path::PathBuf;
use std::process::ExitCode;

use akasha_core::{
    InitRequest, InitResult, LinkRequest, LinkResult, NoteClass, ProjectValidationReport,
    ResolveRequest, ResolvedProject, assemble_context, initialize_project, link_project,
    render_context_markdown, resolve_project, validate_project,
};
use clap::{Parser, Subcommand};

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

    /// Disable colored output. Current output is always uncolored.
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
        no_color: _,
        command,
    } = cli;

    if let Some(selected) = project.as_ref() {
        let positional = match &command {
            Command::Init { slug } | Command::Link { slug, .. } => Some(slug),
            Command::Resolve | Command::Validate | Command::Context => None,
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
            render_init(&result, json).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::Link { slug, repo } => {
            let request = LinkRequest::from_process(root, slug, repo).map_err(report_resolution)?;
            let result = link_project(&request).map_err(report_link)?;
            render_link(&result, json).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::Resolve => {
            let request = ResolveRequest::from_process(root, project).map_err(report_resolution)?;
            let resolved = resolve_project(&request).map_err(report_resolution)?;
            render_resolution(&resolved, json).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::Validate => {
            let request = ResolveRequest::from_process(root, project).map_err(report_resolution)?;
            let report = validate_project(&request).map_err(report_validation)?;
            render_validation(&report, json).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
        Command::Context => {
            let request = ResolveRequest::from_process(root, project).map_err(report_resolution)?;
            let context = assemble_context(&request).map_err(report_context)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&context).map_err(|error| {
                        eprintln!("akasha: failed to render command output: {error}");
                        6
                    })?
                );
            } else {
                print!("{}", render_context_markdown(&context));
            }
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

fn render_init(result: &InitResult, json: bool) -> Result<(), serde_json::Error> {
    if json {
        println!("{}", serde_json::to_string_pretty(result)?);
    } else {
        println!("initialized: {}", result.project);
        println!("repository: {}", result.repository_dir.display());
        println!("project directory: {}", result.project_dir.display());
        println!("templates copied: {}", result.template_files);
        println!("registry: {}", result.registry.display());
        println!("pointer: {}", result.pointer.display());
    }

    Ok(())
}

fn render_link(result: &LinkResult, json: bool) -> Result<(), serde_json::Error> {
    if json {
        println!("{}", serde_json::to_string_pretty(result)?);
    } else {
        println!("linked: {}", result.project);
        println!("repository: {}", result.repository_dir.display());
        println!("pointer: {}", result.pointer.display());
        println!("project directory: {}", result.project_dir.display());
    }

    Ok(())
}

fn render_resolution(resolved: &ResolvedProject, json: bool) -> Result<(), serde_json::Error> {
    if json {
        println!("{}", serde_json::to_string_pretty(resolved)?);
    } else {
        println!("root: {}", resolved.root.display());
        println!("root source: {}", root_source_name(resolved.root_source));
        println!("project: {}", resolved.project);
        println!(
            "project source: {}",
            project_source_name(resolved.project_source)
        );
        match &resolved.pointer {
            Some(pointer) => println!("pointer: {}", pointer.display()),
            None => println!("pointer: none (project selected explicitly)"),
        }
        println!("registry: {}", resolved.registry.display());
        println!("repository: {}", resolved.repository_dir.display());
        println!("project directory: {}", resolved.project_dir.display());
    }

    Ok(())
}

fn render_validation(
    report: &ProjectValidationReport,
    json: bool,
) -> Result<(), serde_json::Error> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
    } else {
        println!("valid: {}", report.project);
        println!("registry: {}", report.registry.display());
        println!("repository: {}", report.repository_dir.display());
        println!("project directory: {}", report.project_dir.display());
        println!("registry projects: {}", report.registry_projects);
        println!("canonical notes: {}", report.canonical_notes);
        println!("wikilinks: {}", report.wikilinks);
        for (name, note_type) in &report.note_types {
            println!(
                "note type: {name} ({}) — {}",
                note_class_name(note_type.class),
                note_type.notes
            );
        }
    }

    Ok(())
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
