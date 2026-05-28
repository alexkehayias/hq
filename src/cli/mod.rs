use anyhow::Result;
use clap::{Parser, Subcommand};
use std::env;

pub mod auth;
pub mod chat;
pub mod eval;
pub mod index;
pub mod init;
pub mod job;
pub mod migrate;
pub mod query;
pub mod rebuild;
pub mod serve;
pub mod task;

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
    /// Run the server
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
        /// Index chat messages from non-background sessions
        #[arg(long, default_value = "false")]
        chat: bool,
    },
    /// Rebuild all indices from source
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
    /// Perform oauth and store credentials
    Auth {
        #[arg(long, value_enum)]
        service: ServiceKind,
    },
    /// Run a job
    Job {
        #[arg(long, value_enum)]
        id: JobId,
    },
    /// Run an eval
    Eval {
        #[arg(long)]
        file: String,
        /// Override the model from config
        #[arg(long)]
        model: Option<String>,
        /// Run without saving results to the database
        #[arg(long, default_value = "false")]
        dry_run: bool,
    },
    /// Create, update, or delete tasks
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
}

#[derive(Subcommand)]
enum TaskCommand {
    /// Create a new task
    Create {
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value = "TODO")]
        status: String,
    },
    /// Update an existing task by UUID
    Update {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        status: Option<String>,
    },
    /// Delete a task by UUID
    Delete {
        id: String,
    },
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
#[command(arg_required_else_help = true)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

pub async fn run() -> Result<()> {
    let args = Cli::parse();

    let storage_path = env::var("HQ_STORAGE_PATH").unwrap_or("./".to_string());
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
            chat,
        }) => {
            index::run(
                all,
                full_text,
                vector,
                chat,
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
            chat::run(&vec_db_path).await?;
        }
        Some(Command::Auth { service }) => {
            auth::run(service, &vec_db_path).await?;
        }
        Some(Command::Job { id }) => {
            job::run(id).await?;
        }
        Some(Command::Eval { model, file, dry_run }) => {

            let api_key = env::var("OPENAI_API_KEY").unwrap_or_else(|_| "thiswontworkforopenai".to_string());
            let api_hostname = env::var("HQ_LOCAL_LLM_HOST").unwrap_or_else(|_| "https://api.openai.com".to_string());
            let model = model.unwrap_or_else(|| env::var("HQ_LOCAL_LLM_MODEL").expect("Missing model name"));

            eval::run(vec_db_path, api_hostname, api_key, model, file, dry_run).await?;
        }
        Some(Command::Task { command }) => match command {
            TaskCommand::Create {
                title,
                body,
                project,
                status,
            } => {
                task::run_create(&notes_path, &title, body.as_deref(), project.as_deref(), &status)
                    .await?;
            }
            TaskCommand::Update {
                id,
                title,
                body,
                status,
            } => {
                task::run_update(
                    &notes_path,
                    &id,
                    title.as_deref(),
                    body.as_deref(),
                    status.as_deref(),
                )
                .await?;
            }
            TaskCommand::Delete { id } => {
                task::run_delete(&notes_path, &id).await?;
            }
        },
        None => {}
    }

    Ok(())
}
