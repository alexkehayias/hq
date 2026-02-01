use anyhow::Result;
use hq::cli;

#[tokio::main]
async fn main() -> Result<()> {
    cli::run().await
}
