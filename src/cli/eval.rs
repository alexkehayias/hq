use crate::core::db;
use crate::eval::run;

pub async fn run_eval(
    file: String,
    model: String,
    api_key: String,
    api_hostname: String,
    db_path: Option<String>,
) -> anyhow::Result<()> {
    let storage_path = std::env::var("HQ_STORAGE_PATH").unwrap_or("./".to_string());
    let path = db_path.unwrap_or(format!("{}/db", storage_path));

    let conn = db::async_db(&path).await?;

    let run_result = run::run_eval(
        &conn,
        &file,
        &model,
        &api_key,
        &api_hostname,
    )
    .await?;

    run::print_results(&conn, &run_result.id).await?;

    Ok(())
}