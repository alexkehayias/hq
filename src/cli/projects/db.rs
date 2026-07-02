use anyhow::Result;
use tokio_rusqlite::Connection;

pub struct ProjectRow {
    pub id: String,
    pub title: String,
    pub file_name: String,
    pub total_tasks: usize,
    pub done_tasks: usize,
    pub todo_tasks: usize,
    pub is_done: bool,
}

pub async fn list_projects(db: &Connection) -> Result<Vec<ProjectRow>> {
    let projects = db
        .call(|conn| {
            let mut stmt = conn.prepare(
                "SELECT
                   n.id,
                   n.title,
                   n.file_name,
                   COUNT(t.id) as total,
                   COALESCE(SUM(CASE WHEN t.status IN ('done', 'canceled', 'someday') THEN 1 ELSE 0 END), 0) as done,
                   COALESCE(SUM(CASE WHEN t.status NOT IN ('done', 'canceled', 'someday') THEN 1 ELSE 0 END), 0) as todo,
                   CASE WHEN n.tags LIKE '%project_done%' THEN 1 ELSE 0 END as is_done
                 FROM note_meta n
                 LEFT JOIN note_meta t ON t.file_name = n.file_name AND t.type = 'task'
                 WHERE n.type = 'note' AND n.tags LIKE '%project%'
                 GROUP BY n.id
                 ORDER BY n.title",
            )?;

            let rows = stmt
                .query_map([], |row| {
                    let id: String = row.get(0)?;
                    let title: String = row.get(1)?;
                    let file_name: String = row.get(2)?;
                    let total_tasks: usize = row.get(3)?;
                    let done_tasks: usize = row.get(4)?;
                    let todo_tasks: usize = row.get(5)?;
                    let is_done_int: i32 = row.get(6)?;
                    Ok(ProjectRow {
                        id,
                        title,
                        file_name,
                        total_tasks,
                        done_tasks,
                        todo_tasks,
                        is_done: is_done_int != 0,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();

            Ok::<_, rusqlite::Error>(rows)
        })
        .await?;

    Ok(projects)
}
