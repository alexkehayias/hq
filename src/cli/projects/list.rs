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
        println!("{:<30} {:<30} {status:<8} {:<8} {:<8} {:<8}", p.title, p.file_name, p.total_tasks, p.todo_tasks, p.done_tasks);
    }

    Ok(())
}
