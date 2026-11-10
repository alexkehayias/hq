use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    hq_cli::run().await
}
