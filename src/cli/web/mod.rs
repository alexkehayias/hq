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
        #[arg(long)]
        url: Option<String>,
        /// The URL to fetch (positional)
        args: Vec<String>,
    },
    /// Search the web
    Search {
        /// The search query
        #[arg(long)]
        query: Option<String>,
        /// The search query (positional)
        args: Vec<String>,
        /// Maximum number of results to return (default 10)
        #[arg(long, default_value = "10")]
        limit: u32,
    },
}

pub async fn run(command: WebCommand) -> Result<()> {
    match command {
        WebCommand::Fetch { url, args } => {
            let url = url.or_else(|| {
                let joined = args.join(" ");
                if joined.is_empty() { None } else { Some(joined) }
            }).ok_or_else(|| anyhow::anyhow!("A URL is required"))?;
            fetch::run(url).await
        }
        WebCommand::Search { query, args, limit } => {
            let query = query.unwrap_or_else(|| args.join(" "));
            if query.is_empty() {
                anyhow::bail!("A search query is required");
            }
            let api_key = env::var("HQ_GOOGLE_SEARCH_API_KEY")
                .map_err(|_| anyhow::anyhow!("HQ_GOOGLE_SEARCH_API_KEY is not set"))?;
            let cx_id = env::var("HQ_GOOGLE_SEARCH_CX_ID")
                .map_err(|_| anyhow::anyhow!("HQ_GOOGLE_SEARCH_CX_ID is not set"))?;
            search::run(api_key, cx_id, query, limit).await
        }
    }
}
