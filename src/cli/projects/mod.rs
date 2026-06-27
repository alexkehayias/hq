use anyhow::Result;
use clap::Subcommand;
use tokio_rusqlite::Connection;

pub mod db;
pub mod list;

#[derive(Subcommand)]
pub enum ProjectsCommand {
    /// List all projects
    List,
}

pub async fn run(command: ProjectsCommand, db: &Connection) -> Result<()> {
    match command {
        ProjectsCommand::List => list::run(db).await,
    }
}
