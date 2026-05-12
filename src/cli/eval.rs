use std::env;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::core::db::async_db;
use crate::eval::runner;

pub async fn run(
    db_path: String,
    api_hostname: String,
    api_key: String,
    model: String,
    file: String,
    dry_run: bool,
) -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("{}=debug", env!("CARGO_CRATE_NAME")).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Eval config — model: {}, api_hostname: {}", model, api_hostname);

    if dry_run {
        runner::run_eval_dry(
            &api_hostname,
            &api_key,
            &model,
            &file,
        ).await?;
    } else {
        let db = async_db(&db_path).await?;
        let run_result = runner::run_eval(
            &db,
            &api_hostname,
            &api_key,
            &model,
            &file,
        )
        .await?;

        runner::print_results(&db, &run_result.id).await?;
    }

    Ok(())
}
