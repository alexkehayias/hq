use anyhow::Result;
use tokio_rusqlite::Connection;

use super::db;

pub async fn run(conn: &Connection) -> Result<()> {
    let projects = db::list_projects(conn).await?;

    if projects.is_empty() {
        println!("No projects found.");
        return Ok(());
    }

    println!("{:<30} {:<30} {:<8} {:<8} {:<8} {:<8}", "Project", "File", "Status", "Total", "TODO", "Done");
    println!("{}", "-".repeat(95));
    for p in &projects {
        let status = if p.is_done { "Done" } else { "Active" };
        let fname = p.file_name.as_deref().unwrap_or("");
        println!("{:<30} {:<30} {status:<8} {:<8} {:<8} {:<8}", p.title, fname, p.total_tasks, p.todo_tasks, p.done_tasks);
    }

    Ok(())
}
