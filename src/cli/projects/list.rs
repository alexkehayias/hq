use anyhow::Result;
use std::env;

use crate::core::db;

pub async fn run() -> Result<()> {
    let storage_path = env::var("HQ_STORAGE_PATH").unwrap_or_else(|_| "./".to_string());
    let vec_db_path = format!("{storage_path}/db");

    let db = db::async_db(&vec_db_path).await?;

    let projects = db
        .call(|conn| {
            let mut stmt = conn.prepare(
                "SELECT n.title, n.file_name, n.tags
                 FROM note_meta n
                 WHERE n.type = 'note' AND n.tags LIKE '%project%'
                 ORDER BY n.title",
            )?;

            let rows = stmt
                .query_map([], |row| {
                    let title: String = row.get(0)?;
                    let file_name: Option<String> = row.get(1)?;
                    let tags: Option<String> = row.get(2)?;
                    Ok((title, file_name, tags))
                })?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();

            Ok(rows)
        })
        .await?;

    if projects.is_empty() {
        println!("No projects found.");
        return Ok(());
    }

    // Count tasks per project and check status
    let mut results: Vec<(String, bool, usize, usize, usize)> = Vec::new();

    for (title, file_name, tags) in &projects {
        let is_done = tags
            .as_deref()
            .map_or(false, |t| t.split(',').any(|tag| tag.trim() == "project_done"));

        let counts = db
            .call({
                let file_name = file_name.clone();
                move |conn| {
                    let mut stmt = conn.prepare(
                        "SELECT
                           COUNT(*) as total,
                           COALESCE(SUM(CASE WHEN status IN ('done', 'canceled', 'someday') THEN 1 ELSE 0 END), 0) as done,
                           COALESCE(SUM(CASE WHEN status NOT IN ('done', 'canceled', 'someday') THEN 1 ELSE 0 END), 0) as todo
                         FROM note_meta
                         WHERE type = 'task' AND file_name = ?",
                    )?;

                    let (total, done, todo): (usize, usize, usize) = stmt
                        .query_row([&file_name], |row| {
                            let total: usize = row.get(0)?;
                            let done: usize = row.get(1)?;
                            let todo: usize = row.get(2)?;
                            Ok((total, done, todo))
                        })?;

                    Ok((total, done, todo))
                }
            })
            .await?;

        results.push((title.clone(), is_done, counts.0, counts.1, counts.2));
    }

    println!("{:<30} {:<8} {:<8} {:<8} {:<8}", "Project", "Status", "Total", "TODO", "Done");
    println!("{}", "-".repeat(70));
    for (title, is_done, total, done, todo) in &results {
        let status = if *is_done { "Done" } else { "Active" };
        println!("{title:<30} {status:<8} {total:<8} {todo:<8} {done:<8}");
    }

    Ok(())
}
