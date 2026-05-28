use anyhow::{Context, Result};
use chrono::Local;
use orgize::ParseConfig;
use orgize::rowan::ast::AstNode;
use std::ops::Range;
use std::path::PathBuf;
use tokio::fs;
use uuid::Uuid;

use crate::org;

fn todo_keywords_config() -> ParseConfig {
    ParseConfig {
        todo_keywords: (
            vec![
                "TODO".to_string(),
                "NEXT".to_string(),
                "WAITING".to_string(),
            ],
            vec![
                "DONE".to_string(),
                "CANCELED".to_string(),
                "SOMEDAY".to_string(),
            ],
        ),
        ..Default::default()
    }
}

fn slugify(s: &str) -> String {
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
        "untitled".to_string()
    } else {
        slug
    }
}

fn build_standalone_org(id: &str, title: &str, body: &str, status: &str) -> String {
    let mut headline = org::Headline::builder()
        .level(1)
        .status(status)
        .title(title)
        .property("ID", id);
    if !body.is_empty() {
        headline = headline.body(body);
    }
    org::Document::builder()
        .property("ID", id)
        .title(title)
        .filetags("task")
        .headline(headline.build())
        .build()
        .to_string()
}

fn build_headline(id: &str, title: &str, body: &str, status: &str, level: usize) -> String {
    let mut h = org::Headline::builder()
        .level(level)
        .status(status)
        .title(title)
        .property("ID", id);
    if !body.is_empty() {
        h = h.body(body);
    }
    h.build().to_string()
}

fn extract_body(raw: &str) -> String {
    let lines = raw.lines().skip(1);
    let mut body = Vec::new();
    let mut in_props = false;
    for line in lines {
        if line.trim() == ":PROPERTIES:" {
            in_props = true;
            continue;
        }
        if in_props {
            if line.trim() == ":END:" {
                in_props = false;
                continue;
            }
            continue; // skip property lines
        }
        body.push(line);
    }
    body.join("\n").trim().to_string()
}

struct TaskLocation {
    path: PathBuf,
    is_standalone: bool,
    range: Option<Range<usize>>,
    content: String,
    current_title: String,
    current_body: String,
    current_status: String,
    current_level: usize,
}

async fn find_task(notes_path: &str, id: &str) -> Result<TaskLocation> {
    let id_pattern = format!(":ID:       {id}");

    let mut dir = fs::read_dir(notes_path)
        .await
        .with_context(|| format!("Cannot read notes directory: {notes_path}"))?;

    while let Some(entry) = dir.next_entry().await? {
        let path = entry.path();
        if path.extension().map_or(true, |e| e != "org") {
            continue;
        }
        if path.file_name().unwrap_or_default() == "config.org" {
            continue;
        }

        let content = fs::read_to_string(&path).await?;
        if !content.contains(&id_pattern) {
            continue;
        }

        // Determine if standalone (ID before first headline) or headline-level
        let id_pos = content.find(&id_pattern).unwrap();
        let first_headline_pos = content.find("\n* ");

        let is_standalone = match first_headline_pos {
            Some(hpos) => id_pos < hpos,
            None => true,
        };

        if is_standalone {
            let config = todo_keywords_config();
            let org = config.parse(&content);
            let headline = org
                .document()
                .headlines()
                .next()
                .context("Standalone task file has no headline")?;

            let current_status = headline
                .todo_keyword()
                .map(|k| k.to_string())
                .unwrap_or_else(|| "TODO".to_string());
            let current_title = headline.title_raw().trim().to_string();
            let current_level = headline.level();
            let range = headline.syntax().text_range();
            let usize_range =
                u32::from(range.start()) as usize..u32::from(range.end()) as usize;
            let raw_text = &content[usize_range.clone()];
            let current_body = extract_body(raw_text);

            return Ok(TaskLocation {
                path,
                is_standalone: true,
                range: None,
                content,
                current_title,
                current_body,
                current_status,
                current_level,
            });
        }

        // Headline-level: parse to find the matching headline
        let config = todo_keywords_config();
        let org = config.parse(&content);
        for headline in org.document().headlines() {
            if let Some(props) = headline.properties() {
                if props.get("ID").is_some_and(|v| v == id) {
                    let range = headline.syntax().text_range();
                    let usize_range =
                        u32::from(range.start()) as usize..u32::from(range.end()) as usize;
                    let current_status = headline
                        .todo_keyword()
                        .map(|k| k.to_string())
                        .unwrap_or_else(|| "TODO".to_string());
                    let current_title = headline.title_raw().trim().to_string();
                    let current_level = headline.level();
                    let raw_text = &content[usize_range.clone()];
                    let current_body = extract_body(raw_text);

                    return Ok(TaskLocation {
                        path,
                        is_standalone: false,
                        range: Some(usize_range),
                        content,
                        current_title,
                        current_body,
                        current_status,
                        current_level,
                    });
                }
            }
        }

        anyhow::bail!(
            "Found ID {id} in {} but could not locate its headline",
            path.display()
        );
    }

    anyhow::bail!("Task with ID {id} not found in {notes_path}");
}

async fn find_or_create_project(notes_path: &str, project_name: &str) -> Result<PathBuf> {
    let slug = slugify(project_name);
    let pattern = format!("--project-{slug}.org");

    // Look for existing project file
    let mut dir = fs::read_dir(notes_path).await?;
    while let Some(entry) = dir.next_entry().await? {
        let path = entry.path();
        let file_name = path.file_name().unwrap().to_str().unwrap().to_string();
        if file_name.ends_with(&pattern) {
            return Ok(path);
        }
    }

    // Create new project file
    let project_id = Uuid::new_v4().to_string();
    let today = Local::now().format("%Y-%m-%d");
    let filename = format!("{notes_path}/{today}--project-{slug}.org");

    let content = org::Document::builder()
        .property("ID", &project_id)
        .title(project_name)
        .filetags("project")
        .build()
        .to_string();
    fs::write(&filename, &content)
        .await
        .context("Failed to create project file")?;
    println!("Created project file: {filename}");
    Ok(PathBuf::from(filename))
}

pub async fn run_create(
    notes_path: &str,
    title: &str,
    body: Option<&str>,
    project: Option<&str>,
    status: &str,
) -> Result<()> {
    let id = Uuid::new_v4().to_string();
    let body = body.unwrap_or_default();

    if let Some(project_name) = project {
        let project_path = find_or_create_project(notes_path, project_name).await?;
        let headline = build_headline(&id, title, body, status, 1);
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
        let slug = slugify(title);
        let today = Local::now().format("%Y-%m-%d");
        let filename = format!("{notes_path}/{today}--{slug}.org");
        let content = build_standalone_org(&id, title, body, status);
        fs::write(&filename, &content)
            .await
            .context("Failed to write task file")?;
        println!("Created task '{title}' (id: {id}, file: {filename})");
    }

    Ok(())
}

pub async fn run_update(
    notes_path: &str,
    id: &str,
    title: Option<&str>,
    body: Option<&str>,
    status: Option<&str>,
) -> Result<()> {
    let location = find_task(notes_path, id).await?;
    let new_title = title.unwrap_or(&location.current_title);
    let new_body = body.unwrap_or(&location.current_body);
    let new_status = status.as_deref().unwrap_or(&location.current_status);

    if location.is_standalone {
        let new_content = build_standalone_org(id, new_title, new_body, new_status);
        fs::write(&location.path, &new_content)
            .await
            .context("Failed to write updated task file")?;
    } else {
        let range = location.range.as_ref().unwrap();
        let new_headline = build_headline(id, new_title, new_body, new_status, location.current_level);
        let new_content = format!(
            "{before}{new_headline}{after}",
            before = &location.content[..range.start],
            after = &location.content[range.end..]
        );
        fs::write(&location.path, &new_content)
            .await
            .context("Failed to write updated project file")?;
    }

    println!("Task {id} updated");
    Ok(())
}

pub async fn run_delete(notes_path: &str, id: &str) -> Result<()> {
    let location = find_task(notes_path, id).await?;

    if location.is_standalone {
        fs::remove_file(&location.path)
            .await
            .context("Failed to delete task file")?;
        println!("Deleted task file: {}", location.path.display());
    } else {
        let range = location.range.as_ref().unwrap();
        let before = &location.content[..range.start];
        let after = &location.content[range.end..];
        // Remove one trailing newline if present to avoid blank-line gaps
        let after = after.strip_prefix('\n').unwrap_or(after);
        let new_content = format!("{before}{after}");
        fs::write(&location.path, &new_content)
            .await
            .context("Failed to write project file after deletion")?;
        println!("Deleted task {id} from {}", location.path.display());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use orgize::ParseConfig;
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
        assert_eq!(slugify("Buy groceries"), "buy-groceries");
    }

    #[test]
    fn test_slugify_multiple_spaces() {
        assert_eq!(slugify("  hello   world  "), "hello-world");
    }

    #[test]
    fn test_slugify_special_chars() {
        assert_eq!(slugify("Fix bug! (urgent) #42"), "fix-bug-urgent-42");
    }

    #[test]
    fn test_slugify_only_special_chars() {
        assert_eq!(slugify("!!! @@@ $$$"), "untitled");
    }

    #[test]
    fn test_slugify_empty_string() {
        assert_eq!(slugify(""), "untitled");
    }

    #[test]
    fn test_slugify_already_slug() {
        assert_eq!(slugify("my-task-name"), "my-task-name");
    }

    // -----------------------------------------------------------------------
    // extract_body
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_body_basic() {
        let raw = "* TODO Buy milk\n:PROPERTIES:\n:ID:       abc\n:END:\nMilk, eggs\n";
        assert_eq!(extract_body(raw), "Milk, eggs");
    }

    #[test]
    fn test_extract_body_no_body() {
        let raw = "* TODO Buy milk\n:PROPERTIES:\n:ID:       abc\n:END:\n";
        assert_eq!(extract_body(raw), "");
    }

    #[test]
    fn test_extract_body_multiline() {
        let raw = "* TODO Task\n:PROPERTIES:\n:ID:       abc\n:END:\nLine 1\nLine 2\nLine 3\n";
        assert_eq!(extract_body(raw), "Line 1\nLine 2\nLine 3");
    }

    #[test]
    fn test_extract_body_no_status() {
        let raw = "* Task\n:PROPERTIES:\n:ID:       abc\n:END:\nJust a note\n";
        assert_eq!(extract_body(raw), "Just a note");
    }

    #[test]
    fn test_extract_body_whitespace_only() {
        let raw = "* TODO Task\n:PROPERTIES:\n:ID:       abc\n:END:\n   \n  \n";
        assert_eq!(extract_body(raw), "");
    }

    // -----------------------------------------------------------------------
    // run_create — standalone
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_create_standalone_task() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        run_create(&notes, "Test Task", None, None, "TODO")
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
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        run_create(&notes, "Buy milk", Some("Milk, eggs, bread"), None, "TODO")
            .await
            .unwrap();

        let entries: Vec<_> = fs::read_dir(&notes).unwrap().collect();
        let path = entries[0].as_ref().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("Milk, eggs, bread"));
    }

    #[tokio::test]
    async fn test_create_standalone_task_custom_status() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        run_create(&notes, "Urgent fix", None, None, "NEXT")
            .await
            .unwrap();

        let entries: Vec<_> = fs::read_dir(&notes).unwrap().collect();
        let path = entries[0].as_ref().unwrap().path();
        let (_, status, _) = parse_headline(&path);
        assert_eq!(status, "NEXT");
    }

    // -----------------------------------------------------------------------
    // run_create — project
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_create_project_task_creates_project_file() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        run_create(&notes, "Fix login", None, Some("sprint-12"), "TODO")
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
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        run_create(&notes, "Task one", None, Some("sprint-12"), "TODO")
            .await
            .unwrap();
        run_create(&notes, "Task two", None, Some("sprint-12"), "DONE")
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
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        run_create(&notes, "My task", None, None, "TODO")
            .await
            .unwrap();

        // Find the created task's ID
        let path = fs::read_dir(&notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        let id_start = content.find(":ID:").unwrap() + ":ID:       ".len();
        let id = content[id_start..].lines().next().unwrap().trim().to_string();

        run_update(&notes, &id, None, None, Some("DONE"))
            .await
            .unwrap();

        let (_, status, _) = parse_headline(&path);
        assert_eq!(status, "DONE");
    }

    #[tokio::test]
    async fn test_update_standalone_title_and_body() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        run_create(&notes, "Old title", Some("Old body"), None, "TODO")
            .await
            .unwrap();

        let path = fs::read_dir(&notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        let id_start = content.find(":ID:").unwrap() + ":ID:       ".len();
        let id = content[id_start..].lines().next().unwrap().trim().to_string();

        run_update(&notes, &id, Some("New title"), Some("New body"), None)
            .await
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
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        run_create(&notes, "Fix bug", None, Some("sprint-12"), "TODO")
            .await
            .unwrap();

        // Find the task ID from the project file
        let path = fs::read_dir(&notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        // The headline ID is the second occurrence (after the project-level ID)
        let id_marker = ":ID:       ";
        let second_id_start = content.match_indices(id_marker).nth(1).unwrap().0 + id_marker.len();
        let id = content[second_id_start..].lines().next().unwrap().trim().to_string();

        run_update(&notes, &id, None, None, Some("DONE"))
            .await
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
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        let result = run_update(&notes, "nonexistent-uuid", None, None, Some("DONE")).await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // run_delete
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_delete_standalone_task() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        run_create(&notes, "Temp task", None, None, "TODO")
            .await
            .unwrap();

        let path = fs::read_dir(&notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        let id_start = content.find(":ID:").unwrap() + ":ID:       ".len();
        let id = content[id_start..].lines().next().unwrap().trim().to_string();

        run_delete(&notes, &id).await.unwrap();

        // File should be gone
        assert!(fs::read_dir(&notes).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn test_delete_project_task() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        run_create(&notes, "Task one", None, Some("sprint-12"), "TODO")
            .await
            .unwrap();
        run_create(&notes, "Task two", None, Some("sprint-12"), "TODO")
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
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        run_create(&notes, "Only task", None, Some("sprint-12"), "TODO")
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
    // find_or_create_project
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_find_or_create_project_creates_new() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        let path = find_or_create_project(&notes, "my-project").await.unwrap();
        assert!(path.exists());
        assert!(path.to_str().unwrap().contains("--project-my-project"));

        // Should contain a project preamble
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("#+TITLE: my-project"));
        assert!(content.contains("#+FILETAGS: project"));
    }

    #[tokio::test]
    async fn test_find_or_create_project_finds_existing() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        let path1 = find_or_create_project(&notes, "my-project").await.unwrap();
        let path2 = find_or_create_project(&notes, "my-project").await.unwrap();

        assert_eq!(path1, path2, "should return the same existing file");
    }
}
