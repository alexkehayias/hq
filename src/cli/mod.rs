use anyhow::Result;
use clap::{Parser, Subcommand};
use std::env;

pub mod auth;
pub mod chat;
pub mod index;
pub mod init;
pub mod job;
pub mod migrate;
pub mod query;
pub mod rebuild;
pub mod serve;

use auth::ServiceKind;
use job::JobId;

#[derive(Subcommand)]
enum Command {
    /// Initialize indices and clone notes from version control
    Init {
        #[arg(long, action, default_value = "false")]
        db: bool,
        #[arg(long, action, default_value = "false")]
        index: bool,
        #[arg(long, action, default_value = "false")]
        notes: bool,
    },
    /// Migrate indices and db schema
    Migrate {
        #[arg(long, action, default_value = "false")]
        db: bool,
        #[arg(long, action, default_value = "false")]
        index: bool,
    },
    /// Run the API server
    Serve {
        /// Set the server host address
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Set the server port
        #[arg(long, default_value = "2222")]
        port: String,
    },
    /// Index notes
    Index {
        #[arg(long, default_value = "false")]
        all: bool,
        #[arg(long, default_value = "false")]
        full_text: bool,
        #[arg(long, default_value = "false")]
        vector: bool,
    },
    /// Rebuild all of the indices from source
    Rebuild {},
    /// Query the search index
    Query {
        #[arg(long)]
        term: String,
        #[arg(long, default_value = "false")]
        vector: bool,
    },
    /// Start a chat bot session
    Chat {},
    /// Perform OAuth authentication and print tokens
    Auth {
        #[arg(long, value_enum)]
        service: ServiceKind,
    },
    /// Run a periodic job
    Job {
        #[arg(long, value_enum)]
        id: JobId,
    },
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

pub async fn run() -> Result<()> {
    let args = Cli::parse();

    let storage_path = env::var("INDEXER_STORAGE_PATH").unwrap_or("./".to_string());
    let index_path = format!("{}/index", storage_path);
    let notes_path = format!("{}/notes", storage_path);
    let vec_db_path = format!("{}/db", storage_path);

    // Handle each sub command
    match args.command {
        Some(Command::Init { db, index, notes }) => {
            init::run(db, index, notes, &vec_db_path, &index_path, &notes_path).await?;
        }
        Some(Command::Migrate { db, index }) => {
            migrate::run(db, index, &vec_db_path, &index_path).await?;
        }
        Some(Command::Serve { host, port }) => {
            serve::run(host, port).await;
        }
        Some(Command::Index {
            all,
            full_text,
            vector,
        }) => {
            index::run(
                all,
                full_text,
                vector,
                &index_path,
                &notes_path,
                &vec_db_path,
            )
            .await?;
        }
        Some(Command::Rebuild {}) => {
            rebuild::run(&index_path, &notes_path, &vec_db_path).await?;
        }
        Some(Command::Query { term, vector }) => {
            query::run(term, vector, &index_path, &vec_db_path).await?;
        }
        Some(Command::Chat {}) => {
            chat::run().await?;
        }
        Some(Command::Auth { service }) => {
            auth::run(service, &vec_db_path).await?;
        }
        Some(Command::Job { id }) => {
            job::run(id).await?;
        }
        None => {}
    }

    Ok(())
}
