use std::path::PathBuf;
use std::process::ExitCode;

use akasha_core::{ResolveRequest, ResolvedProject, resolve_project};
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
    /// Resolve and print the current data root and project identity.
    Resolve,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => ExitCode::from(code),
    }
}

fn run(cli: Cli) -> Result<(), u8> {
    match cli.command {
        Command::Resolve => {
            let request = ResolveRequest::from_process(cli.root, cli.project).map_err(report)?;
            let resolved = resolve_project(&request).map_err(report)?;
            render_resolution(&resolved, cli.json).map_err(|error| {
                eprintln!("akasha: failed to render command output: {error}");
                6
            })?;
        }
    }

    Ok(())
}

fn report(error: akasha_core::ResolveError) -> u8 {
    eprintln!("akasha: {error}");
    error.exit_code()
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
        println!("project directory: {}", resolved.project_dir.display());
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
