use anyhow::Result;
use tokio_rusqlite::Connection;

use super::db;

pub async fn run(conn: &Connection) -> Result<()> {
    let projects = db::list_projects(conn).await?;

    if projects.is_empty() {
        println!("No projects found.");
        return Ok(());
    }

    println!("{:<36} {:<26} {:<26} {:<8} {:<8} {:<8} {:<8}", "ID", "Project", "File", "Status", "Total", "TODO", "Done");
    println!("{}", "-".repeat(120));
    for p in &projects {
        let status = if p.is_done { "Done" } else { "Active" };
        println!("{:<36} {:<26} {:<26} {status:<8} {:<8} {:<8} {:<8}", p.id, p.title, p.file_name, p.total_tasks, p.todo_tasks, p.done_tasks);
    }

    Ok(())
}
