use std::path::PathBuf;

use akasha_core::{ResolveRequest, prepare_onboarding};
use akasha_mcp::OnboardingMcpServer;
use clap::Parser;
use rmcp::{ServiceExt, transport::stdio};

#[derive(Debug, Parser)]
#[command(
    name = "akasha-onboarding-mcp",
    about = "Run the project-scoped Akasha onboarding MCP server over stdio",
    disable_version_flag = true
)]
struct Arguments {
    /// Akasha data root override. Otherwise normal AKASHA_ROOT/user-config resolution applies.
    #[arg(long)]
    root: Option<PathBuf>,

    /// Project identity override. Otherwise the nearest repository pointer is required.
    #[arg(long)]
    project: Option<String>,

    /// Repository used as the project-resolution working directory. Defaults to process cwd.
    #[arg(long)]
    repo: Option<PathBuf>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("akasha-onboarding-mcp: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = Arguments::parse();
    let mut resolution = ResolveRequest::from_process(arguments.root, arguments.project)?;
    if let Some(repo) = arguments.repo {
        resolution.cwd = if repo.is_absolute() {
            repo
        } else {
            resolution.cwd.join(repo)
        };
    }

    prepare_onboarding(&resolution)?;
    let service = OnboardingMcpServer::new(resolution).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
