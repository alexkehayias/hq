use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Local;
use tokio::fs;
use tokio_rusqlite::Connection;
use uuid::Uuid;

use crate::cli::projects;
use crate::core::orgmode;
use crate::org;

/// Create a new project file on disk and register it in the database.
async fn create_project_file(db: &Connection, notes_path: &str, project_name: &str) -> Result<PathBuf> {
    let slug = slugify(project_name)?;
    let project_id = Uuid::new_v4().to_string();
    let today = Local::now().format("%Y-%m-%d");
    let filename = format!("{today}--project-{slug}.org");
    let full_path = format!("{notes_path}/{filename}");

    let content = org::Document::builder()
        .property("ID", &project_id)
        .title(project_name)
        .category(&slug)
        .date(&today.to_string())
        .filetags("private project")
        .build()
        .to_string();
    fs::write(&full_path, &content)
        .await
        .context("Failed to create project file")?;

    // Register in DB so subsequent lookups find the project without
    // requiring a full re-index.
    let db_id = project_id;
    let db_filename = filename;
    let db_title = project_name.to_string();
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO note_meta (id, file_name, title, type, tags)
             VALUES (?1, ?2, ?3, 'note', 'project')",
            rusqlite::params![db_id, db_filename, db_title],
        )?;
        Ok(())
    })
    .await?;

    Ok(PathBuf::from(full_path))
}

fn slugify(s: &str) -> Result<String> {
    let slug: String = s
        .to_lowercase()
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == '-' || c == ' ' {
                Some(c)
            } else {
                None
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join("-")
        .trim_matches('-')
        .to_string();
    if slug.is_empty() {
        anyhow::bail!("Cannot slugify empty string: input produced no valid characters");
    }
    Ok(slug)
}

pub async fn run_create(
    db: &Connection,
    notes_path: &str,
    title: &str,
    body: Option<&str>,
    project: Option<&str>,
    status: &str,
) -> Result<()> {
    let id = Uuid::new_v4().to_string();
    let body = body.unwrap_or_default();
    let status_upper = status.to_uppercase();

    if let Some(project_name) = project {
        // Look up existing project in DB, or create a new project file
        let project_path = match projects::db::find_project_file(db, notes_path, project_name).await? {
            Some(path) => path,
            None => create_project_file(db, notes_path, project_name).await?,
        };
        println!("Created project file: {}", project_path.display());
        let headline = orgmode::build_headline(&id, title, body, &status_upper, 1);
        let mut project_content = fs::read_to_string(&project_path).await?;
        if !project_content.ends_with('\n') {
            project_content.push('\n');
        }
        project_content.push_str(&headline);
        project_content.push('\n');
        fs::write(&project_path, &project_content)
            .await
            .context("Failed to write project file")?;
        println!("Created task '{title}' in project '{project_name}' (id: {id})");
    } else {
        let slug = slugify(title)?;
        let today = Local::now().format("%Y-%m-%d");
        let filename = format!("{notes_path}/{today}--{slug}.org");
        let content = orgmode::build_document(&id, title, body, &status_upper);
        fs::write(&filename, &content)
            .await
            .context("Failed to write task file")?;
        println!("Created task '{title}' (id: {id}, file: {filename})");
    }

    Ok(())
}

pub async fn run_update(
    db: &Connection,
    notes_path: &str,
    id: &str,
    title: Option<&str>,
    body: Option<&str>,
    status: Option<&str>,
    project: Option<&str>,
) -> Result<()> {
    if let Some(project_ref) = project {
        let path = projects::db::find_project_file(db, notes_path, project_ref).await?
            .ok_or_else(|| anyhow::anyhow!("Project '{project_ref}' not found"))?;
        let filename = path.file_name().and_then(|s| s.to_str()).unwrap();
        orgmode::update_task(notes_path, id, Some(filename), title, body, status).await?;
    } else {
        orgmode::update_task(notes_path, id, None, title, body, status).await?;
    }
    println!("Task {id} updated");
    Ok(())
}

/// Move a task from its current file into a project file.
///
/// Finds the task by UUID across all org files, removes the headline from its
/// source, and appends it to the target project file (creating the project
/// file if it doesn't exist yet).
pub async fn run_refile(db: &Connection, notes_path: &str, id: &str, project: &str) -> Result<()> {
    let location = orgmode::find_task(notes_path, id).await?;

    // Look up existing project in DB, or create a new project file
    let target_path = match projects::db::find_project_file(db, notes_path, project).await? {
        Some(path) => path,
        None => create_project_file(db, notes_path, project).await?,
    };

    if location.path == target_path {
        anyhow::bail!("Task is already in project '{project}'");
    }

    // Extract the raw headline text (preserves all org-mode structure)
    let headline_text = &location.content[location.range.start..location.range.end];

    // Remove the headline from the source file
    let before = &location.content[..location.range.start];
    let after = &location.content[location.range.end..];
    let after = after.strip_prefix('\n').unwrap_or(after);
    let new_source = format!("{before}{after}");
    fs::write(&location.path, &new_source)
        .await
        .context("Failed to write source file after refile")?;

    // Append the raw headline verbatim to the target project file
    let mut target_content = fs::read_to_string(&target_path).await?;
    if !target_content.ends_with('\n') {
        target_content.push('\n');
    }
    target_content.push_str(headline_text);
    target_content.push('\n');
    fs::write(&target_path, &target_content)
        .await
        .context("Failed to write target project file")?;

    println!(
        "Refiled task {id} ('{}') from {} to {}",
        location.current_title,
        location.path.display(),
        target_path.display()
    );

    Ok(())
}

pub async fn run_delete(notes_path: &str, id: &str) -> Result<()> {
    let location = orgmode::find_task(notes_path, id).await?;

    let before = &location.content[..location.range.start];
    let after = &location.content[location.range.end..];
    // Remove one trailing newline if present to avoid blank-line gaps
    let after = after.strip_prefix('\n').unwrap_or(after);
    let new_content = format!("{before}{after}");
    fs::write(&location.path, &new_content)
        .await
        .context("Failed to write project file after deletion")?;
    println!("Deleted task {id} from {}", location.path.display());

    Ok(())
}

pub async fn run_list(
    db: &Connection,
    notes_path: &str,
    project: Option<&str>,
    status: Option<&str>,
) -> Result<()> {
    let tasks = if let Some(project_ref) = project {
        let (filenames, display_prefix) = match project_ref {
            "refile" | "capture" => {
                let filename = format!("{project_ref}.org");
                (vec![filename], None)
            }
            _ => {
                let path = projects::db::find_project_file(db, notes_path, project_ref).await?
                    .ok_or_else(|| anyhow::anyhow!("Project '{project_ref}' not found"))?;
                let filename = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(project_ref)
                    .to_string();
                (vec![filename], None)
            }
        };
        list_tasks_from_files(db, &filenames, status, display_prefix).await?
    } else {
        list_tasks_from_files(db, &["refile.org".into(), "capture.org".into()], status, None).await?
    };

    if tasks.is_empty() {
        println!("No tasks found matching the given criteria.");
        return Ok(());
    }

    println!("{:<40} {:<10} {:<24} {}", "ID", "Status", "Project", "Title");
    println!("{}", "-".repeat(100));
    for (id, task_status, project_display, title) in &tasks {
        let short_id = if id.len() > 8 { &id[..8] } else { id };
        println!("{short_id:<40} {task_status:<10} {project_display:<24} {title}");
    }

    Ok(())
}

async fn list_tasks_from_files(
    db: &Connection,
    filenames: &[String],
    status_filter: Option<&str>,
    display_prefix: Option<&str>,
) -> Result<Vec<(String, String, String, String)>> {
    let status_lower = status_filter.map(|s| s.to_lowercase());
    let filenames_json = serde_json::to_string(filenames).unwrap();

    let tasks = db
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, title, status, file_name
                 FROM note_meta
                 WHERE type = 'task'
                   AND file_name IN (SELECT value FROM json_each(?))
                   AND (? IS NULL OR status = ?)
                 ORDER BY status, title",
            )?;

            let status_val: &dyn rusqlite::types::ToSql = match &status_lower {
                Some(s) => s,
                None => &rusqlite::types::Null,
            };

            let rows = stmt
                .query_map(
                    rusqlite::params![filenames_json.as_bytes(), status_val, status_val],
                    |row| {
                        let id: String = row.get(0)?;
                        let title: String = row.get(1)?;
                        let status: String = row.get(2)?;
                        let file_name: String = row.get(3)?;
                        Ok((id, title, status, file_name))
                    },
                )?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();

            Ok(rows)
        })
        .await?;

    let display_prefix = display_prefix.map(|s| s.to_string());
    let result: Vec<(String, String, String, String)> = tasks
        .into_iter()
        .map(|(id, title, status, file_name)| {
            let display = display_prefix.clone().unwrap_or_else(|| {
                file_name
                    .strip_suffix(".org")
                    .unwrap_or(&file_name)
                    .to_string()
            });
            (id, status.to_uppercase(), display, title)
        })
        .collect();

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orgize::ParseConfig;
    use rusqlite;
    use std::fs;
    use tempfile::TempDir;

    fn parsing_config() -> ParseConfig {
        ParseConfig {
            todo_keywords: (
                vec!["TODO".to_string(), "NEXT".to_string(), "WAITING".to_string()],
                vec!["DONE".to_string(), "CANCELED".to_string(), "SOMEDAY".to_string()],
            ),
            ..Default::default()
        }
    }

    /// Parse an org file at `path` and return the first headline's properties.
    fn parse_headline(path: &std::path::Path) -> (String, String, String) {
        let content = fs::read_to_string(path).unwrap();
        let config = parsing_config();
        let org = config.parse(&content);
        let h = org.document().headlines().next().unwrap();
        let status = h.todo_keyword().map(|k| k.to_string()).unwrap_or_default();
        let title = h.title_raw().trim().to_string();
        let props = h.properties().unwrap();
        let id = props.get("ID").unwrap().to_string();
        (id, status, title)
    }

    fn headline_count(path: &std::path::Path) -> usize {
        let content = fs::read_to_string(path).unwrap();
        let config = parsing_config();
        let org = config.parse(&content);
        org.document().headlines().count()
    }

    // -----------------------------------------------------------------------
    // slugify
    // -----------------------------------------------------------------------

    #[test]
    fn test_slugify_normal() {
        assert_eq!(slugify("Buy groceries").unwrap(), "buy-groceries");
    }

    #[test]
    fn test_slugify_multiple_spaces() {
        assert_eq!(slugify("  hello   world  ").unwrap(), "hello-world");
    }

    #[test]
    fn test_slugify_special_chars() {
        assert_eq!(slugify("Fix bug! (urgent) #42").unwrap(), "fix-bug-urgent-42");
    }

    #[test]
    fn test_slugify_only_special_chars() {
        assert!(slugify("!!! @@@ $$$").is_err());
    }

    #[test]
    fn test_slugify_empty_string() {
        assert!(slugify("").is_err());
    }

    #[test]
    fn test_slugify_already_slug() {
        assert_eq!(slugify("my-task-name").unwrap(), "my-task-name");
    }

    // -----------------------------------------------------------------------
    // run_create
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_create_standalone_task() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "Test Task", None, None, "TODO")
            .await
            .unwrap();

        // Should create a single .org file
        let entries: Vec<_> = fs::read_dir(&notes).unwrap().collect();
        assert_eq!(entries.len(), 1);
        let path = entries[0].as_ref().unwrap().path();
        assert!(path.extension().unwrap() == "org");

        let (id, status, title) = parse_headline(&path);
        assert_eq!(status, "TODO");
        assert_eq!(title, "Test Task");
        assert!(!id.is_empty(), "should have a UUID");
    }

    #[tokio::test]
    async fn test_create_standalone_task_with_body() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "Buy milk", Some("Milk, eggs, bread"), None, "TODO")
            .await
            .unwrap();

        let entries: Vec<_> = fs::read_dir(&notes).unwrap().collect();
        let path = entries[0].as_ref().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("Milk, eggs, bread"));
    }

    #[tokio::test]
    async fn test_create_standalone_task_custom_status() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "Urgent fix", None, None, "NEXT")
            .await
            .unwrap();

        let entries: Vec<_> = fs::read_dir(&notes).unwrap().collect();
        let path = entries[0].as_ref().unwrap().path();
        let (_, status, _) = parse_headline(&path);
        assert_eq!(status, "NEXT");
    }

    // -----------------------------------------------------------------------
    // run_create — with named project
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_create_project_task_creates_project_file() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "Fix login", None, Some("sprint-12"), "TODO")
            .await
            .unwrap();

        // Should create project file with one headline
        let entries: Vec<_> = fs::read_dir(&notes).unwrap().collect();
        assert_eq!(entries.len(), 1);
        let path = entries[0].as_ref().unwrap().path();
        assert!(path.to_str().unwrap().contains("--project-sprint-12"));
        assert_eq!(headline_count(&path), 1);

        let (id, status, title) = parse_headline(&path);
        assert_eq!(status, "TODO");
        assert_eq!(title, "Fix login");
        assert!(!id.is_empty());
    }

    #[tokio::test]
    async fn test_create_second_task_in_project() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "Task one", None, Some("sprint-12"), "TODO")
            .await
            .unwrap();

        // Second create reuses the same project file
        run_create(&db, &notes, "Task two", None, Some("sprint-12"), "DONE")
            .await
            .unwrap();

        // Single project file with two headlines
        let entries: Vec<_> = fs::read_dir(&notes).unwrap().collect();
        assert_eq!(entries.len(), 1);
        let path = entries[0].as_ref().unwrap().path();
        assert_eq!(headline_count(&path), 2);
    }

    // -----------------------------------------------------------------------
    // run_update
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_update_standalone_status() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "My task", None, None, "TODO")
            .await
            .unwrap();

        // Find the created task's ID
        let path = fs::read_dir(&notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        let id_start = content.find(":ID:").unwrap() + ":ID:       ".len();
        let id = content[id_start..].lines().next().unwrap().trim().to_string();

        run_update(&db, &notes, &id, None, None, Some("DONE"), None)            .await
            .unwrap();

        let (_, status, _) = parse_headline(&path);
        assert_eq!(status, "DONE");
    }

    #[tokio::test]
    async fn test_update_standalone_title_and_body() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "Old title", Some("Old body"), None, "TODO")
            .await
            .unwrap();

        let path = fs::read_dir(&notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        let id_start = content.find(":ID:").unwrap() + ":ID:       ".len();
        let id = content[id_start..].lines().next().unwrap().trim().to_string();

        run_update(&db, &notes, &id, Some("New title"), Some("New body"), None, None)            .await
            .unwrap();

        let (_, status, title) = parse_headline(&path);
        assert_eq!(status, "TODO");
        assert_eq!(title, "New title");
        let new_content = fs::read_to_string(&path).unwrap();
        assert!(new_content.contains("New body"));
        assert!(!new_content.contains("Old body"));
    }

    #[tokio::test]
    async fn test_update_project_task_status() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "Fix bug", None, Some("sprint-12"), "TODO")
            .await
            .unwrap();

        // Find the task ID from the project file
        let path = fs::read_dir(&notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        // The headline ID is the second occurrence (after the project-level ID)
        let id_marker = ":ID:       ";
        let second_id_start = content.match_indices(id_marker).nth(1).unwrap().0 + id_marker.len();
        let id = content[second_id_start..].lines().next().unwrap().trim().to_string();

        run_update(&db, &notes, &id, None, None, Some("DONE"), None)            .await
            .unwrap();

        // Re-parse and check the headline
        let config = parsing_config();
        let org = config.parse(&fs::read_to_string(&path).unwrap());
        let headlines: Vec<_> = org.document().headlines().collect();
        assert_eq!(headlines.len(), 1);
        assert_eq!(
            headlines[0].todo_keyword().unwrap().to_string(),
            "DONE"
        );
    }

    #[tokio::test]
    async fn test_update_nonexistent_task() {
        let (db, _dir, notes) = test_env().await;

        let result = run_update(&db, &notes, "nonexistent-uuid", None, None, Some("DONE"), None).await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // run_delete
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_delete_standalone_task() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "Temp task", None, None, "TODO")
            .await
            .unwrap();

        let path = fs::read_dir(&notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        let id_start = content.find(":ID:").unwrap() + ":ID:       ".len();
        let id = content[id_start..].lines().next().unwrap().trim().to_string();

        run_delete(&notes, &id).await.unwrap();

        // File should still exist (as a note with no task headlines)
        assert!(path.exists());
        assert_eq!(headline_count(&path), 0);
    }

    #[tokio::test]
    async fn test_delete_project_task() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "Task one", None, Some("sprint-12"), "TODO")
            .await
            .unwrap();

        // Second create reuses the same project file
        run_create(&db, &notes, "Task two", None, Some("sprint-12"), "TODO")
            .await
            .unwrap();

        let path = fs::read_dir(&notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        // Find the first headline's ID (second :ID: in file)
        let id_marker = ":ID:       ";
        let second_id_start = content.match_indices(id_marker).nth(1).unwrap().0 + id_marker.len();
        let id = content[second_id_start..].lines().next().unwrap().trim().to_string();

        run_delete(&notes, &id).await.unwrap();

        // One headline should remain
        assert_eq!(headline_count(&path), 1);
    }

    #[tokio::test]
    async fn test_delete_last_project_task_leaves_project_file() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "Only task", None, Some("sprint-12"), "TODO")
            .await
            .unwrap();

        let path = fs::read_dir(&notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        let id_marker = ":ID:       ";
        let second_id_start = content.match_indices(id_marker).nth(1).unwrap().0 + id_marker.len();
        let id = content[second_id_start..].lines().next().unwrap().trim().to_string();

        run_delete(&notes, &id).await.unwrap();

        // Project file should still exist (preamble remains)
        assert!(path.exists());
        assert_eq!(headline_count(&path), 0);
    }

    #[tokio::test]
    async fn test_delete_nonexistent_task() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        let result = run_delete(&notes, "nonexistent-uuid").await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // run_list
    // -----------------------------------------------------------------------

    /// Create a test database using the existing DB initialization functions.
    async fn test_db() -> (Connection, TempDir) {
        let dir = TempDir::new().unwrap();
        let vec_db_path = dir.path().to_str().unwrap().to_string();
        let db = crate::core::db::async_db(&vec_db_path).await.unwrap();
        db.call(|conn| {
            crate::core::db::initialize_db(conn).unwrap();
            Ok(())
        })
        .await
        .unwrap();
        (db, dir)
    }

    /// Create a test database + notes directory, returning the connection,
    /// temp dir guard, and notes path string.
    /// Notes are stored in a `notes/` subdirectory so DB files at the temp
    /// dir root don't interfere with file-count assertions.
    async fn test_env() -> (Connection, TempDir, String) {
        let dir = TempDir::new().unwrap();
        let notes = format!("{}/notes", dir.path().to_str().unwrap());
        std::fs::create_dir_all(&notes).unwrap();
        let db = crate::core::db::async_db(dir.path().to_str().unwrap()).await.unwrap();
        db.call(|conn| {
            crate::core::db::initialize_db(conn).unwrap();
            Ok(())
        })
        .await
        .unwrap();
        (db, dir, notes)
    }

    /// Insert a task row into note_meta for testing.
    fn insert_task(
        conn: &rusqlite::Connection,
        id: &str,
        file_name: &str,
        title: &str,
        status: &str,
    ) {
        conn.execute(
            "INSERT INTO note_meta (id, file_name, title, type, status)
             VALUES (?1, ?2, ?3, 'task', ?4)",
            rusqlite::params![id, file_name, title, status],
        )
        .unwrap();
    }

    /// Insert a project row into note_meta for testing.
    fn insert_project(
        conn: &rusqlite::Connection,
        id: &str,
        file_name: &str,
        title: &str,
    ) {
        conn.execute(
            "INSERT INTO note_meta (id, file_name, title, type, tags)
             VALUES (?1, ?2, ?3, 'note', 'project')",
            rusqlite::params![id, file_name, title],
        )
        .unwrap();
    }

    #[tokio::test]
    async fn test_list_refile_and_capture_both_empty() {
        let (db, _dir) = test_db().await;

        // No tasks in the DB
        let result = run_list(&db, "", None, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_refile_and_capture_with_tasks() {
        let (db, _dir) = test_db().await;
        db.call(|conn| {
            insert_task(conn, "task-1", "refile.org", "Buy groceries", "todo");
            insert_task(conn, "task-2", "refile.org", "Fix login", "done");
            insert_task(conn, "task-3", "capture.org", "Quick idea", "todo");
            Ok(())
        })
        .await
        .unwrap();

        let result = run_list(&db, "", None, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_refile_only() {
        let (db, _dir) = test_db().await;
        db.call(|conn| {
            insert_task(conn, "t1", "refile.org", "Task one", "todo");
            insert_task(conn, "t2", "refile.org", "Task two", "done");
            insert_task(conn, "t3", "capture.org", "Capture task", "todo");
            Ok(())
        })
        .await
        .unwrap();

        let result = run_list(&db, "", Some("refile"), None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_capture_only() {
        let (db, _dir) = test_db().await;
        db.call(|conn| {
            insert_task(conn, "t1", "refile.org", "Refile task", "todo");
            insert_task(conn, "t2", "capture.org", "Capture task", "todo");
            Ok(())
        })
        .await
        .unwrap();

        let result = run_list(&db, "", Some("capture"), None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_refile_filter_by_status() {
        let (db, _dir) = test_db().await;
        db.call(|conn| {
            insert_task(conn, "t1", "refile.org", "Task one", "todo");
            insert_task(conn, "t2", "refile.org", "Task two", "done");
            insert_task(conn, "t3", "refile.org", "Task three", "todo");
            Ok(())
        })
        .await
        .unwrap();

        let result = run_list(&db, "", None, Some("TODO")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_project_not_found() {
        let (db, _dir) = test_db().await;

        let result = run_list(&db, "", Some("nonexistent"), None).await;
        assert!(result.is_err(), "missing project should error");
    }

    #[tokio::test]
    async fn test_list_project_file() {
        let (db, dir) = test_db().await;
        let notes = dir.path().to_str().unwrap().to_string();

        // Create a project file on disk (find_project_file_by_id_or_name needs it)
        let project_path = dir.path().join("2026-05-31--project-my-project.org");
        std::fs::write(
            &project_path,
            "\
:PROPERTIES:
:ID:       proj-1
:END:
#+TITLE: my-project
#+CATEGORY: my-project
#+DATE: 2026-06-01
#+FILETAGS: private project

* TODO First task
:PROPERTIES:
:ID:       pt-1
:END:

* DONE Second task
:PROPERTIES:
:ID:       pt-2
:END:
",
        )
        .unwrap();

        // Insert project and tasks into the DB
        db.call(|conn| {
            insert_project(conn, "proj-1", "2026-05-31--project-my-project.org", "my-project");
            insert_task(conn, "pt-1", "2026-05-31--project-my-project.org", "First task", "todo");
            insert_task(conn, "pt-2", "2026-05-31--project-my-project.org", "Second task", "done");
            Ok(())
        })
        .await
        .unwrap();

        let result = run_list(&db, &notes, Some("my-project"), None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_project_filter_by_status() {
        let (db, dir) = test_db().await;
        let notes = dir.path().to_str().unwrap().to_string();

        let project_path = dir.path().join("2026-05-31--project-sprint-12.org");
        std::fs::write(
            &project_path,
            "\
:PROPERTIES:
:ID:       proj-1
:END:
#+TITLE: sprint-12
#+CATEGORY: sprint-12
#+DATE: 2026-06-01
#+FILETAGS: private project

* TODO Task A
:PROPERTIES:
:ID:       a
:END:

* DONE Task B
:PROPERTIES:
:ID:       b
:END:

* TODO Task C
:PROPERTIES:
:ID:       c
:END:
",
        )
        .unwrap();

        db.call(|conn| {
            insert_project(conn, "proj-1", "2026-05-31--project-sprint-12.org", "sprint-12");
            insert_task(conn, "a", "2026-05-31--project-sprint-12.org", "Task A", "todo");
            insert_task(conn, "b", "2026-05-31--project-sprint-12.org", "Task B", "done");
            insert_task(conn, "c", "2026-05-31--project-sprint-12.org", "Task C", "todo");
            Ok(())
        })
        .await
        .unwrap();

        let result = run_list(&db, &notes, Some("sprint-12"), Some("DONE")).await;
        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // find_task_in_file — now delegated to core::orgmode
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_find_task_in_file_found() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("project.org");
        fs::write(
            &path,
            "\
:PROPERTIES:
:ID:       proj-1
:END:
#+TITLE: My Project
#+FILETAGS: private project

* TODO Fix login
:PROPERTIES:
:ID:       task-1
:END:
Investigate the redirect

* DONE Setup CI
:PROPERTIES:
:ID:       task-2
:END:
",
        )
        .unwrap();

        let location = orgmode::find_task_in_file(&path, "task-1").await.unwrap();
        assert_eq!(location.current_title, "Fix login");
        assert_eq!(location.current_status, "TODO");
        assert!(location.current_body.contains("Investigate the redirect"));
        assert_eq!(location.current_level, 1);
    }

    #[tokio::test]
    async fn test_find_task_in_file_not_found() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("project.org");
        fs::write(
            &path,
            "\
:PROPERTIES:
:ID:       proj-1
:END:
",
        )
        .unwrap();

        let result = orgmode::find_task_in_file(&path, "nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_find_task_in_file_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.org");
        fs::write(&path, "").unwrap();

        let result = orgmode::find_task_in_file(&path, "task-1").await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // run_update — with --project flag
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_update_project_task_by_filename() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "Fix bug", None, Some("sprint-12"), "TODO")
            .await
            .unwrap();

        // Find the task ID from the project file
        let path = fs::read_dir(&notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        let id_marker = ":ID:       ";
        let second_id_start = content.match_indices(id_marker).nth(1).unwrap().0 + id_marker.len();
        let id = content[second_id_start..].lines().next().unwrap().trim().to_string();

        let filename = path.file_name().unwrap().to_str().unwrap().to_string();

        // Update using --project with filename
        run_update(&db, &notes, &id, Some("Fixed bug"), None, Some("DONE"), Some(&filename))            .await
            .unwrap();

        let config = parsing_config();
        let org = config.parse(&fs::read_to_string(&path).unwrap());
        let headlines: Vec<_> = org.document().headlines().collect();
        assert_eq!(headlines.len(), 1);
        assert_eq!(headlines[0].todo_keyword().unwrap().to_string(), "DONE");
        assert_eq!(headlines[0].title_raw().trim(), "Fixed bug");
    }

    #[tokio::test]
    async fn test_update_project_task_by_id() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "Add tests", None, Some("sprint-12"), "TODO")
            .await
            .unwrap();

        // Find the project file's ID (first :ID: in the file)
        let path = fs::read_dir(&notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        let id_marker = ":ID:       ";
        let first_id_start = content.find(id_marker).unwrap() + id_marker.len();
        let project_id = content[first_id_start..].lines().next().unwrap().trim().to_string();

        // Find the task ID (second :ID:)
        let second_id_start = content.match_indices(id_marker).nth(1).unwrap().0 + id_marker.len();
        let task_id = content[second_id_start..].lines().next().unwrap().trim().to_string();

        // Update using --project with project ID
        run_update(&db, &notes, &task_id, None, None, Some("DONE"), Some(&project_id))            .await
            .unwrap();

        let config = parsing_config();
        let org = config.parse(&fs::read_to_string(&path).unwrap());
        let headlines: Vec<_> = org.document().headlines().collect();
        assert_eq!(headlines.len(), 1);
        assert_eq!(headlines[0].todo_keyword().unwrap().to_string(), "DONE");
    }

    #[tokio::test]
    async fn test_update_project_task_not_found_in_project() {
        let (db, _dir, notes) = test_env().await;

        // Create a standalone task (not in any project)
        run_create(&db, &notes, "Standalone", None, None, "TODO")
            .await
            .unwrap();

        // Create a project with a different task
        run_create(&db, &notes, "Project task", None, Some("my-project"), "TODO")
            .await
            .unwrap();

        // Find the standalone task's ID
        let standalone_path = fs::read_dir(&notes)
            .unwrap()
            .filter_map(|e| {
                let p = e.unwrap().path();
                let n = p.file_name().unwrap().to_str().unwrap().to_string();
                if n.contains("--project-") { None } else { Some(p) }
            })
            .next()
            .unwrap();
        let content = fs::read_to_string(&standalone_path).unwrap();
        let id_start = content.find(":ID:").unwrap() + ":ID:       ".len();
        let task_id = content[id_start..].lines().next().unwrap().trim().to_string();

        // Updating scoped to a project where the task doesn't exist should error
        let result = run_update(&db, &notes, &task_id, None, None, Some("DONE"), Some("my-project")).await;
        assert!(result.is_err(), "should error when task not found in scoped project file");
    }

    #[tokio::test]
    async fn test_update_project_task_nonexistent_project() {
        let (db, _dir, notes) = test_env().await;

        let result = run_update(&db, &notes, "some-id", None, None, Some("DONE"), Some("nonexistent")).await;
        assert!(result.is_err(), "should error for nonexistent project");
    }

    // -----------------------------------------------------------------------
    // run_refile
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_refile_standalone_to_project() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "Buy groceries", Some("Milk, eggs, bread"), None, "TODO")
            .await
            .unwrap();

        // Find the standalone task's ID
        let standalone_path = fs::read_dir(&notes)
            .unwrap()
            .filter_map(|e| {
                let p = e.unwrap().path();
                let n = p.file_name().unwrap().to_str().unwrap().to_string();
                if n.contains("--project-") { None } else { Some(p) }
            })
            .next()
            .unwrap();
        let content = fs::read_to_string(&standalone_path).unwrap();
        let id_start = content.find(":ID:").unwrap() + ":ID:       ".len();
        let task_id = content[id_start..].lines().next().unwrap().trim().to_string();

        // Refile to a project
        run_refile(&db, &notes, &task_id, "errands").await.unwrap();

        // Task headline should be removed from the standalone file
        // (document-level preamble with #+TITLE: may retain the title)
        let standalone_content = fs::read_to_string(&standalone_path).unwrap();
        assert!(!standalone_content.contains("Milk, eggs, bread"),
            "headline body should not remain in source");
        assert_eq!(headline_count(&standalone_path), 0);

        // Task should now be in the project file
        let project_path = fs::read_dir(&notes)
            .unwrap()
            .filter_map(|e| {
                let p = e.unwrap().path();
                let n = p.file_name().unwrap().to_str().unwrap().to_string();
                if n.contains("--project-errands") { Some(p) } else { None }
            })
            .next()
            .unwrap();
        let project_content = fs::read_to_string(&project_path).unwrap();
        assert!(project_content.contains(&task_id));
        assert!(project_content.contains("Buy groceries"));
        assert!(project_content.contains("Milk, eggs, bread"));
        assert_eq!(headline_count(&project_path), 1);
    }

    #[tokio::test]
    async fn test_refile_from_one_project_to_another() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "Fix login bug", None, Some("sprint-12"), "TODO")
            .await
            .unwrap();

        // Find the task ID from the project file
        let project_path = fs::read_dir(&notes)
            .unwrap()
            .filter_map(|e| {
                let p = e.unwrap().path();
                let n = p.file_name().unwrap().to_str().unwrap().to_string();
                if n.contains("--project-sprint-12") { Some(p) } else { None }
            })
            .next()
            .unwrap();
        let content = fs::read_to_string(&project_path).unwrap();
        let id_marker = ":ID:       ";
        let second_id_start = content.match_indices(id_marker).nth(1).unwrap().0 + id_marker.len();
        let task_id = content[second_id_start..].lines().next().unwrap().trim().to_string();

        // Refile to a different project
        run_refile(&db, &notes, &task_id, "security").await.unwrap();

        // Task should no longer be in the original project
        let sprint_content = fs::read_to_string(&project_path).unwrap();
        assert!(!sprint_content.contains(&task_id));
        assert_eq!(headline_count(&project_path), 0);

        // Task should now be in the new project
        let security_path = fs::read_dir(&notes)
            .unwrap()
            .filter_map(|e| {
                let p = e.unwrap().path();
                let n = p.file_name().unwrap().to_str().unwrap().to_string();
                if n.contains("--project-security") { Some(p) } else { None }
            })
            .next()
            .unwrap();
        let security_content = fs::read_to_string(&security_path).unwrap();
        assert!(security_content.contains(&task_id));
        assert!(security_content.contains("Fix login bug"));
        assert_eq!(headline_count(&security_path), 1);
    }

    #[tokio::test]
    async fn test_refile_preserves_org_structure() {
        let (db, _dir, notes) = test_env().await;

        // Create a task with body text via refile.org
        let refile_path = std::path::Path::new(&notes).join("refile.org");
        fs::write(
            &refile_path,
            "\
* TODO Research API design
:PROPERTIES:
:ID:       task-with-body
:END:
Look into REST vs GraphQL options.
Consider authentication requirements.
",
        )
        .unwrap();

        // Refile to a project
        run_refile(&db, &notes, "task-with-body", "research").await.unwrap();

        // Read the project file
        let project_path = fs::read_dir(&notes)
            .unwrap()
            .filter_map(|e| {
                let p = e.unwrap().path();
                let n = p.file_name().unwrap().to_str().unwrap().to_string();
                if n.contains("--project-research") { Some(p) } else { None }
            })
            .next()
            .unwrap();
        let project_content = fs::read_to_string(&project_path).unwrap();

        // All org structure should be preserved verbatim
        assert!(project_content.contains("TODO Research API design"));
        assert!(project_content.contains(":ID:       task-with-body"));
        assert!(project_content.contains("Look into REST vs GraphQL options."));
        assert!(project_content.contains("Consider authentication requirements."));

        // Source file should no longer have the task
        let refile_content = fs::read_to_string(&refile_path).unwrap();
        assert!(!refile_content.contains("task-with-body"));
    }

    #[tokio::test]
    async fn test_refile_to_same_project_errors() {
        let (db, _dir, notes) = test_env().await;

        run_create(&db, &notes, "Task in project", None, Some("my-project"), "TODO")
            .await
            .unwrap();

        // Find the task ID
        let project_path = fs::read_dir(&notes)
            .unwrap()
            .filter_map(|e| {
                let p = e.unwrap().path();
                let n = p.file_name().unwrap().to_str().unwrap().to_string();
                if n.contains("--project-my-project") { Some(p) } else { None }
            })
            .next()
            .unwrap();
        let content = fs::read_to_string(&project_path).unwrap();
        let id_marker = ":ID:       ";
        let second_id_start = content.match_indices(id_marker).nth(1).unwrap().0 + id_marker.len();
        let task_id = content[second_id_start..].lines().next().unwrap().trim().to_string();

        // Refiling to the same project should fail
        let result = run_refile(&db, &notes, &task_id, "my-project").await;
        assert!(result.is_err(), "should error when refiling to same project");
        assert!(
            result.unwrap_err().to_string().contains("already in project"),
            "error should mention already-in-project"
        );
    }

    #[tokio::test]
    async fn test_refile_nonexistent_task() {
        let (db, _dir, notes) = test_env().await;

        let result = run_refile(&db, &notes, "nonexistent-uuid", "some-project").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_refile_from_refile_org_to_project() {
        let (db, _dir, notes) = test_env().await;

        // Simulate a task sitting in refile.org
        let refile_path = std::path::Path::new(&notes).join("refile.org");
        fs::write(
            &refile_path,
            "\
:PROPERTIES:
:ID:       inbox
:END:
#+TITLE: Refile

* TODO Review PR
:PROPERTIES:
:ID:       review-pr-42
:END:
Need to check the middleware changes.

* DONE Setup CI
:PROPERTIES:
:ID:       setup-ci
:END:
",
        )
        .unwrap();

        // Refile one task to a project
        run_refile(&db, &notes, "review-pr-42", "ops").await.unwrap();

        // The refiled task should be gone from refile.org
        let refile_content = fs::read_to_string(&refile_path).unwrap();
        assert!(!refile_content.contains("review-pr-42"));
        assert!(!refile_content.contains("Review PR"));

        // The other task should remain in refile.org
        assert!(refile_content.contains("setup-ci"));
        assert!(refile_content.contains("Setup CI"));

        // The refiled task should be in the project file
        let project_path = fs::read_dir(&notes)
            .unwrap()
            .filter_map(|e| {
                let p = e.unwrap().path();
                let n = p.file_name().unwrap().to_str().unwrap().to_string();
                if n.contains("--project-ops") { Some(p) } else { None }
            })
            .next()
            .unwrap();
        let project_content = fs::read_to_string(&project_path).unwrap();
        assert!(project_content.contains("review-pr-42"));
        assert!(project_content.contains("Review PR"));
        assert!(project_content.contains("middleware changes"));
        assert_eq!(headline_count(&project_path), 1);
    }
}
