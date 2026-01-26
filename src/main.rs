use anyhow::Result;
use indexer::cli;

#[tokio::main]
async fn main() -> Result<()> {
    cli::run().await
}
