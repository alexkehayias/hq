use anyhow::Result;
use clap::Subcommand;

pub mod list;

#[derive(Subcommand)]
pub enum ProjectsCommand {
    /// List all projects
    List,
}

pub async fn run(command: ProjectsCommand) -> Result<()> {
    match command {
        ProjectsCommand::List => list::run().await,
    }
}
