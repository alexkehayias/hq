use anyhow::Result;
use clap::Subcommand;
use std::env;

pub mod fetch;
pub mod search;

#[derive(Subcommand)]
pub enum WebCommand {
    /// Fetch a URL and print its content as markdown
    Fetch {
        /// The URL to fetch
        url: String,
    },
    /// Search the web
    Search {
        /// The search query (may span multiple words)
        #[arg(required = true, num_args = 1..)]
        query: Vec<String>,
        /// Maximum number of results to return (default 10)
        #[arg(long, default_value = "10")]
        limit: u32,
    },
}

pub async fn run(command: WebCommand) -> Result<()> {
    match command {
        WebCommand::Fetch { url } => fetch::run(url).await,
        WebCommand::Search { query, limit } => {
            let query = query.join(" ");
            let api_key = env::var("HQ_GOOGLE_SEARCH_API_KEY")
                .map_err(|_| anyhow::anyhow!("HQ_GOOGLE_SEARCH_API_KEY is not set"))?;
            let cx_id = env::var("HQ_GOOGLE_SEARCH_CX_ID")
                .map_err(|_| anyhow::anyhow!("HQ_GOOGLE_SEARCH_CX_ID is not set"))?;
            search::run(api_key, cx_id, query, limit).await
        }
    }
}
