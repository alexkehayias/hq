use anyhow::{Context, Result};
use chrono::Local;
use orgize::export::{Container, Event, TraversalContext, Traverser};
use orgize::rowan::ast::AstNode;
use orgize::SyntaxElement;
use std::ops::Range;
use std::path::PathBuf;
use tokio::fs;
use uuid::Uuid;

use crate::org;
use crate::search::{index_single_file, remove_task_from_indexes};

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

fn build_document(id: &str, title: &str, body: &str, status: &str) -> String {
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

/// Traverses a headline's syntax subtree and extracts body text,
/// skipping the headline title and property drawer.
#[derive(Default)]
struct BodyExtractor {
    output: String,
    in_headline_title: bool,
}

impl BodyExtractor {
    fn finish(self) -> String {
        self.output.trim().to_string()
    }
}

impl Traverser for BodyExtractor {
    fn event(&mut self, event: Event, ctx: &mut TraversalContext) {
        match event {
            Event::Enter(Container::Headline(_)) => {
                self.in_headline_title = true;
            }
            Event::Leave(Container::Headline(_)) => {
                self.in_headline_title = false;
            }
            // Entering a Section means we've passed the headline title
            // and are now in the body area.
            Event::Enter(Container::Section(_)) => {
                self.in_headline_title = false;
            }
            // Skip property drawers entirely
            Event::Enter(Container::PropertyDrawer(_)) => {
                ctx.skip();
            }
            Event::Leave(Container::PropertyDrawer(_)) => {}
            // Add newline between paragraphs
            Event::Leave(Container::Paragraph(_)) => {
                if !self.in_headline_title {
                    self.output.push('\n');
                }
            }
            Event::Text(text) => {
                if !self.in_headline_title {
                    self.output.push_str(&text);
                }
            }
            _ => {}
        }
    }
}

/// Extract body text from a headline's syntax node using the orgize Traverser.
fn body_from_headline(headline: &orgize::ast::Headline) -> String {
    let mut extractor = BodyExtractor::default();
    let mut ctx = TraversalContext::default();
    extractor.element(SyntaxElement::Node(headline.syntax().clone()), &mut ctx);
    extractor.finish()
}

struct TaskLocation {
    path: PathBuf,
    range: Range<usize>,
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

        // Parse to find the matching headline
        let config = org::todo_keywords_config();
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
                    let current_body = body_from_headline(&headline);

                    return Ok(TaskLocation {
                        path,
                        range: usize_range,
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

/// Find a task by UUID within a specific file (not across all notes).
async fn find_task_in_file(path: &PathBuf, id: &str) -> Result<TaskLocation> {
    let id_pattern = format!(":ID:       {id}");
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("Cannot read file: {}", path.display()))?;

    if !content.contains(&id_pattern) {
        anyhow::bail!("Task with ID {id} not found in {}", path.display());
    }

    let config = org::todo_keywords_config();
    let org = config.parse(&content);
    for headline in org.document().headlines() {
        if let Some(props) = headline.properties() {
            if props.get("ID").is_some_and(|v| v == id) {
                let range = headline.syntax().text_range();
                let usize_range = u32::from(range.start()) as usize..u32::from(range.end()) as usize;
                let current_status = headline
                    .todo_keyword()
                    .map(|k| k.to_string())
                    .unwrap_or_else(|| "TODO".to_string());
                let current_title = headline.title_raw().trim().to_string();
                let current_level = headline.level();
                let current_body = body_from_headline(&headline);

                return Ok(TaskLocation {
                    path: path.clone(),
                    range: usize_range,
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

async fn find_or_create_project(notes_path: &str, project_name: &str) -> Result<PathBuf> {
    let slug = slugify(project_name)?;
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
        .category(&slug)
        .date(&today.to_string())
        .filetags("private project")
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
    index_path: &str,
    vec_db_path: &str,
) -> Result<()> {
    let id = Uuid::new_v4().to_string();
    let body = body.unwrap_or_default();
    let status_upper = status.to_uppercase();

    let file_path = if let Some(project_name) = project {
        let project_path = find_or_create_project(notes_path, project_name).await?;
        let headline = build_headline(&id, title, body, &status_upper, 1);
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
        project_path
    } else {
        let slug = slugify(title)?;
        let today = Local::now().format("%Y-%m-%d");
        let filename = format!("{notes_path}/{today}--{slug}.org");
        let content = build_document(&id, title, body, &status_upper);
        fs::write(&filename, &content)
            .await
            .context("Failed to write task file")?;
        println!("Created task '{title}' (id: {id}, file: {filename})");
        PathBuf::from(filename)
    };

    // Index the new/modified file so the task is immediately searchable
    let db = crate::core::db::async_db(vec_db_path)
        .await
        .context("Failed to connect to async db for indexing")?;
    index_single_file(&db, index_path, file_path).await?;

    Ok(())
}

/// Find a project file by ID (UUID property) or by exact filename match.
/// Unlike `find_or_create_project`, this does NOT slugify the input.
async fn find_project_file_by_id_or_name(notes_path: &str, project_ref: &str) -> Result<PathBuf> {
    let mut dir = fs::read_dir(notes_path)
        .await
        .with_context(|| format!("Cannot read notes directory: {notes_path}"))?;

    while let Some(entry) = dir.next_entry().await? {
        let path = entry.path();
        let file_name = path.file_name().unwrap().to_str().unwrap().to_string();

        // Skip non-org files
        if path.extension().map_or(true, |e| e != "org") {
            continue;
        }

        // Match by filename (exact or project-name suffix)
        if file_name == project_ref || file_name.ends_with(&format!("--project-{}.org", project_ref)) {
            return Ok(path);
        }

        // Match by ID (UUID property in the org file)
        let content = fs::read_to_string(&path).await?;
        if content.contains(&format!(":ID:       {project_ref}")) {
            return Ok(path);
        }
    }

    anyhow::bail!("Project '{project_ref}' not found by ID or filename");
}

pub async fn run_update(
    notes_path: &str,
    id: &str,
    title: Option<&str>,
    body: Option<&str>,
    status: Option<&str>,
    project: Option<&str>,
    index_path: &str,
    vec_db_path: &str,
) -> Result<()> {
    let location = if let Some(project_ref) = project {
        let project_path = find_project_file_by_id_or_name(notes_path, project_ref).await?;
        find_task_in_file(&project_path, id).await?
    } else {
        find_task(notes_path, id).await?
    };
    let new_title = title.unwrap_or(&location.current_title);
    let new_body = body.unwrap_or(&location.current_body);
    let status = status.map(|s| s.to_uppercase());
    let new_status = status.as_deref().unwrap_or(&location.current_status);

    let new_headline = build_headline(id, new_title, new_body, new_status, location.current_level);
    let new_content = format!(
        "{before}{new_headline}{after}",
        before = &location.content[..location.range.start],
        after = &location.content[location.range.end..]
    );
    fs::write(&location.path, &new_content)
        .await
        .context("Failed to write updated task file")?;

    println!("Task {id} updated");

    // Re-index the modified file so the task changes are searchable
    let db = crate::core::db::async_db(vec_db_path)
        .await
        .context("Failed to connect to async db for indexing")?;
    index_single_file(&db, index_path, location.path).await?;

    Ok(())
}

pub async fn run_delete(
    notes_path: &str,
    id: &str,
    index_path: &str,
    vec_db_path: &str,
) -> Result<()> {
    let location = find_task(notes_path, id).await?;

    let before = &location.content[..location.range.start];
    let after = &location.content[location.range.end..];
    // Remove one trailing newline if present to avoid blank-line gaps
    let after = after.strip_prefix('\n').unwrap_or(after);
    let new_content = format!("{before}{after}");
    fs::write(&location.path, &new_content)
        .await
        .context("Failed to write project file after deletion")?;
    println!("Deleted task {id} from {}", location.path.display());

    // Remove the task from all search indexes
    let db = crate::core::db::async_db(vec_db_path)
        .await
        .context("Failed to connect to async db for indexing")?;
    remove_task_from_indexes(&db, index_path, id).await?;

    Ok(())
}

/// Find an existing project file by slug, returning `None` if no match exists.
async fn find_project_file(notes_path: &str, slug: &str) -> Result<Option<PathBuf>> {
    let pattern = format!("--project-{slug}.org");
    let mut dir = fs::read_dir(notes_path).await?;
    while let Some(entry) = dir.next_entry().await? {
        let path = entry.path();
        if path
            .file_name()
            .unwrap()
            .to_str()
            .map_or(false, |n| n.ends_with(&pattern))
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

pub async fn run_list(
    notes_path: &str,
    project: Option<&str>,
    status: Option<&str>,
) -> Result<()> {
    let config = org::todo_keywords_config();

    // Determine which file to read.
    let (target_path, project_display) = if let Some(project_name) = project {
        let slug = slugify(project_name)?;
        let path = find_project_file(notes_path, &slug)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Project '{project_name}' not found"))?;
        (path, project_name.to_string())
    } else {
        let path = PathBuf::from(format!("{notes_path}/refile.org"));
        (path, "refile".to_string())
    };

    // If the file doesn't exist, there are no tasks.
    if !target_path.exists() {
        println!("No tasks found matching the given criteria.");
        return Ok(());
    }

    let content = fs::read_to_string(&target_path).await?;
    let org = config.parse(&content);
    let doc = org.document();

    let mut tasks: Vec<(String, String, String)> = Vec::new();

    for headline in doc.headlines() {
        let kw = headline.todo_keyword().map(|k| k.to_string());
        let task_status = match kw {
            Some(ref s) => s.clone(),
            None => continue,
        };

        // Filter by status
        if let Some(ref filter_status) = status {
            if !task_status.eq_ignore_ascii_case(filter_status) {
                continue;
            }
        }

        let title = headline.title_raw().trim().to_string();
        let id = headline
            .properties()
            .and_then(|p| p.get("ID").map(|s| s.to_string()))
            .unwrap_or_default();

        tasks.push((id, task_status, title));
    }

    if tasks.is_empty() {
        println!("No tasks found matching the given criteria.");
        return Ok(());
    }

    // Sort tasks by status, then title
    tasks.sort_by(|a, b| a.1.cmp(&b.1).then(a.2.cmp(&b.2)));

    println!("{:<40} {:<10} {:<24} {}", "ID", "Status", "Project", "Title");
    println!("{}", "-".repeat(100));
    for (id, task_status, title) in &tasks {
        let short_id = if id.len() > 8 { &id[..8] } else { id };
        println!("{short_id:<40} {task_status:<10} {project_display:<24} {title}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db::{async_db, initialize_db};
    use orgize::ParseConfig;
    use std::fs;
    use tempfile::TempDir;

    /// Holds temporary directories for testing task CRUD with indexing.
    struct TestEnv {
        notes: String,
        index: String,
        db: String,
        _dir: TempDir,
    }

    impl TestEnv {
        async fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let notes = dir.path().join("notes").to_str().unwrap().to_string();
            let index = dir.path().join("index").to_str().unwrap().to_string();
            let db_path = dir.path().join("db").to_str().unwrap().to_string();

            std::fs::create_dir_all(&notes).unwrap();
            std::fs::create_dir_all(&index).unwrap();
            std::fs::create_dir_all(&db_path).unwrap();

            let connection = async_db(&db_path).await.unwrap();
            connection
                .call(|conn| {
                    initialize_db(conn).unwrap();
                    Ok(())
                })
                .await
                .unwrap();

            TestEnv {
                notes,
                index,
                db: db_path,
                _dir: dir,
            }
        }
    }

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
        let env = TestEnv::new().await;

        run_create(
            &env.notes,
            "Test Task",
            None,
            None,
            "TODO",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        // Should create a single .org file
        let entries: Vec<_> = fs::read_dir(&env.notes).unwrap().collect();
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
        let env = TestEnv::new().await;

        run_create(
            &env.notes,
            "Buy milk",
            Some("Milk, eggs, bread"),
            None,
            "TODO",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        let entries: Vec<_> = fs::read_dir(&env.notes).unwrap().collect();
        let path = entries[0].as_ref().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("Milk, eggs, bread"));
    }

    #[tokio::test]
    async fn test_create_standalone_task_custom_status() {
        let env = TestEnv::new().await;

        run_create(
            &env.notes,
            "Urgent fix",
            None,
            None,
            "NEXT",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        let entries: Vec<_> = fs::read_dir(&env.notes).unwrap().collect();
        let path = entries[0].as_ref().unwrap().path();
        let (_, status, _) = parse_headline(&path);
        assert_eq!(status, "NEXT");
    }

    // -----------------------------------------------------------------------
    // run_create — with named project
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_create_project_task_creates_project_file() {
        let env = TestEnv::new().await;

        run_create(
            &env.notes,
            "Fix login",
            None,
            Some("sprint-12"),
            "TODO",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        // Should create project file with one headline
        let entries: Vec<_> = fs::read_dir(&env.notes).unwrap().collect();
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
        let env = TestEnv::new().await;

        run_create(
            &env.notes,
            "Task one",
            None,
            Some("sprint-12"),
            "TODO",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();
        run_create(
            &env.notes,
            "Task two",
            None,
            Some("sprint-12"),
            "DONE",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        // Single project file with two headlines
        let entries: Vec<_> = fs::read_dir(&env.notes).unwrap().collect();
        assert_eq!(entries.len(), 1);
        let path = entries[0].as_ref().unwrap().path();
        assert_eq!(headline_count(&path), 2);
    }

    // -----------------------------------------------------------------------
    // run_update
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_update_standalone_status() {
        let env = TestEnv::new().await;

        run_create(
            &env.notes,
            "My task",
            None,
            None,
            "TODO",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        // Find the created task's ID
        let path = fs::read_dir(&env.notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        let id_start = content.find(":ID:").unwrap() + ":ID:       ".len();
        let id = content[id_start..].lines().next().unwrap().trim().to_string();

        run_update(&env.notes, &id, None, None, Some("DONE"), None, &env.index, &env.db)
            .await
            .unwrap();

        let (_, status, _) = parse_headline(&path);
        assert_eq!(status, "DONE");
    }

    #[tokio::test]
    async fn test_update_standalone_title_and_body() {
        let env = TestEnv::new().await;

        run_create(
            &env.notes,
            "Old title",
            Some("Old body"),
            None,
            "TODO",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        let path = fs::read_dir(&env.notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        let id_start = content.find(":ID:").unwrap() + ":ID:       ".len();
        let id = content[id_start..].lines().next().unwrap().trim().to_string();

        run_update(
            &env.notes,
            &id,
            Some("New title"),
            Some("New body"),
            None,
            None,
            &env.index,
            &env.db,
        )
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
        let env = TestEnv::new().await;

        run_create(
            &env.notes,
            "Fix bug",
            None,
            Some("sprint-12"),
            "TODO",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        // Find the task ID from the project file
        let path = fs::read_dir(&env.notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        // The headline ID is the second occurrence (after the project-level ID)
        let id_marker = ":ID:       ";
        let second_id_start = content.match_indices(id_marker).nth(1).unwrap().0 + id_marker.len();
        let id = content[second_id_start..].lines().next().unwrap().trim().to_string();

        run_update(&env.notes, &id, None, None, Some("DONE"), None, &env.index, &env.db)
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
        let env = TestEnv::new().await;

        let result = run_update(&env.notes, "nonexistent-uuid", None, None, Some("DONE"), None, &env.index, &env.db).await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // run_delete
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_delete_standalone_task() {
        let env = TestEnv::new().await;

        run_create(
            &env.notes,
            "Temp task",
            None,
            None,
            "TODO",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        let path = fs::read_dir(&env.notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        let id_start = content.find(":ID:").unwrap() + ":ID:       ".len();
        let id = content[id_start..].lines().next().unwrap().trim().to_string();

        run_delete(&env.notes, &id, &env.index, &env.db)
            .await
            .unwrap();

        // File should still exist (as a note with no task headlines)
        assert!(path.exists());
        assert_eq!(headline_count(&path), 0);
    }

    #[tokio::test]
    async fn test_delete_project_task() {
        let env = TestEnv::new().await;

        run_create(
            &env.notes,
            "Task one",
            None,
            Some("sprint-12"),
            "TODO",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();
        run_create(
            &env.notes,
            "Task two",
            None,
            Some("sprint-12"),
            "TODO",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        let path = fs::read_dir(&env.notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        // Find the first headline's ID (second :ID: in file)
        let id_marker = ":ID:       ";
        let second_id_start = content.match_indices(id_marker).nth(1).unwrap().0 + id_marker.len();
        let id = content[second_id_start..].lines().next().unwrap().trim().to_string();

        run_delete(&env.notes, &id, &env.index, &env.db)
            .await
            .unwrap();

        // One headline should remain
        assert_eq!(headline_count(&path), 1);
    }

    #[tokio::test]
    async fn test_delete_last_project_task_leaves_project_file() {
        let env = TestEnv::new().await;

        run_create(
            &env.notes,
            "Only task",
            None,
            Some("sprint-12"),
            "TODO",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        let path = fs::read_dir(&env.notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        let id_marker = ":ID:       ";
        let second_id_start = content.match_indices(id_marker).nth(1).unwrap().0 + id_marker.len();
        let id = content[second_id_start..].lines().next().unwrap().trim().to_string();

        run_delete(&env.notes, &id, &env.index, &env.db)
            .await
            .unwrap();

        // Project file should still exist (preamble remains)
        assert!(path.exists());
        assert_eq!(headline_count(&path), 0);
    }

    #[tokio::test]
    async fn test_delete_nonexistent_task() {
        let env = TestEnv::new().await;

        let result = run_delete(&env.notes, "nonexistent-uuid", &env.index, &env.db).await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // run_list
    // -----------------------------------------------------------------------

    /// Helper: parse an org file and return (id, status, title) for each headline
    /// that has a TODO keyword.
    fn tasks_in_file(path: &std::path::Path) -> Vec<(String, String, String)> {
        let content = fs::read_to_string(path).unwrap();
        let config = parsing_config();
        let org = config.parse(&content);
        org.document()
            .headlines()
            .filter_map(|h| {
                let status = h.todo_keyword().map(|k| k.to_string())?;
                let title = h.title_raw().trim().to_string();
                let id = h
                    .properties()
                    .and_then(|p| p.get("ID").map(|s| s.to_string()))
                    .unwrap_or_default();
                Some((id, status, title))
            })
            .collect()
    }

    #[tokio::test]
    async fn test_list_refile_missing() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        // No refile.org exists
        let result = run_list(&notes, None, None).await;
        assert!(result.is_ok(), "missing refile.org should not error");
    }

    #[tokio::test]
    async fn test_list_refile_empty() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        // refile.org exists but has no headlines with TODO keywords
        let refile_path = dir.path().join("refile.org");
        fs::write(
            &refile_path,
            ":PROPERTIES:\n:ID:       inbox\n:END:\n#+TITLE: Refile\n",
        )
        .unwrap();

        let result = run_list(&notes, None, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_refile_with_tasks() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        let refile_path = dir.path().join("refile.org");
        fs::write(
            &refile_path,
            "\
* TODO Buy groceries
:PROPERTIES:
:ID:       task-1
:END:
Milk, eggs

* DONE Fix login
:PROPERTIES:
:ID:       task-2
:END:

* NEXT Research API
:PROPERTIES:
:ID:       task-3
:END:
",
        )
        .unwrap();

        let result = run_list(&notes, None, None).await;
        assert!(result.is_ok());

        let tasks = tasks_in_file(&refile_path);
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].1, "TODO");
        assert_eq!(tasks[0].2, "Buy groceries");
        assert_eq!(tasks[1].1, "DONE");
        assert_eq!(tasks[1].2, "Fix login");
        assert_eq!(tasks[2].1, "NEXT");
        assert_eq!(tasks[2].2, "Research API");
    }

    #[tokio::test]
    async fn test_list_refile_filter_by_status() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        let refile_path = dir.path().join("refile.org");
        fs::write(
            &refile_path,
            "\
* TODO Task one
:PROPERTIES:
:ID:       t1
:END:

* DONE Task two
:PROPERTIES:
:ID:       t2
:END:

* TODO Task three
:PROPERTIES:
:ID:       t3
:END:
",
        )
        .unwrap();

        let result = run_list(&notes, None, Some("TODO")).await;
        assert!(result.is_ok());

        // Verify: only TODO tasks are in the file (run_list reads refile.org)
        let tasks = tasks_in_file(&refile_path);
        let todo_tasks: Vec<_> = tasks.iter().filter(|t| t.1 == "TODO").collect();
        assert_eq!(todo_tasks.len(), 2);
    }

    #[tokio::test]
    async fn test_list_project_not_found() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        let result = run_list(&notes, Some("nonexistent"), None).await;
        assert!(result.is_err(), "missing project should error");
    }

    #[tokio::test]
    async fn test_list_project_file() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        // Create a project file
        let project_path = dir.path().join("2026-05-31--project-my-project.org");
        fs::write(
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

        let result = run_list(&notes, Some("my-project"), None).await;
        assert!(result.is_ok());

        let tasks = tasks_in_file(&project_path);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].2, "First task");
        assert_eq!(tasks[1].2, "Second task");
    }

    #[tokio::test]
    async fn test_list_project_filter_by_status() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        let project_path = dir.path().join("2026-05-31--project-sprint-12.org");
        fs::write(
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

        let result = run_list(&notes, Some("sprint-12"), Some("DONE")).await;
        assert!(result.is_ok());

        let tasks = tasks_in_file(&project_path);
        let done_tasks: Vec<_> = tasks.iter().filter(|t| t.1 == "DONE").collect();
        assert_eq!(done_tasks.len(), 1);
        assert_eq!(done_tasks[0].2, "Task B");
    }

    // -----------------------------------------------------------------------
    // find_project_file
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_find_project_file_found() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        let path = dir.path().join("2026-01-01--project-my-project.org");
        fs::write(&path, "").unwrap();

        let result = find_project_file(&notes, "my-project").await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), path);
    }

    #[tokio::test]
    async fn test_find_project_file_not_found() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        let result = find_project_file(&notes, "my-project").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_find_project_file_ignores_other_files() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        // Create a standalone task file and refile.org — neither should match
        fs::write(dir.path().join("2026-01-01--standalone.org"), "").unwrap();
        fs::write(dir.path().join("refile.org"), "").unwrap();

        let result = find_project_file(&notes, "my-project").await.unwrap();
        assert!(result.is_none());
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
        assert!(content.contains("#+CATEGORY: my-project"));
        assert!(content.contains("#+DATE:"));
        assert!(content.contains("#+FILETAGS: private project"));
    }

    #[tokio::test]
    async fn test_find_or_create_project_finds_existing() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        let path1 = find_or_create_project(&notes, "my-project").await.unwrap();
        let path2 = find_or_create_project(&notes, "my-project").await.unwrap();

        assert_eq!(path1, path2, "should return the same existing file");
    }

    // -----------------------------------------------------------------------
    // find_project_file_by_id_or_name
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_find_project_by_name_exact_filename() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        let path = dir.path().join("my-project.org");
        fs::write(&path, ":PROPERTIES:\n:ID:       abc-123\n:END:\n").unwrap();

        let result = find_project_file_by_id_or_name(&notes, "my-project.org")
            .await
            .unwrap();
        assert_eq!(result, path);
    }

    #[tokio::test]
    async fn test_find_project_by_name_suffix() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        let path = dir.path().join("2026-06-01--project-my-project.org");
        fs::write(&path, ":PROPERTIES:\n:ID:       abc-123\n:END:\n").unwrap();

        let result = find_project_file_by_id_or_name(&notes, "my-project")
            .await
            .unwrap();
        assert_eq!(result, path);
    }

    #[tokio::test]
    async fn test_find_project_by_id() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        let path = dir.path().join("2026-06-01--project-sprint-12.org");
        fs::write(
            &path,
            "\
:PROPERTIES:
:ID:       proj-uuid-42
:END:
#+TITLE: sprint-12
#+FILETAGS: private project
",
        )
        .unwrap();

        let result = find_project_file_by_id_or_name(&notes, "proj-uuid-42")
            .await
            .unwrap();
        assert_eq!(result, path);
    }

    #[tokio::test]
    async fn test_find_project_by_id_not_found() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        let result = find_project_file_by_id_or_name(&notes, "nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_find_project_by_id_ignores_standalone_tasks() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().to_str().unwrap().to_string();

        // Standalone task file — should not match
        fs::write(
            dir.path().join("2026-06-01--standalone.org"),
            ":PROPERTIES:\n:ID:       task-1\n:END:\n",
        )
        .unwrap();

        let result = find_project_file_by_id_or_name(&notes, "project-1").await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // find_task_in_file
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

        let location = find_task_in_file(&path, "task-1").await.unwrap();
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

        let result = find_task_in_file(&path, "nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_find_task_in_file_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.org");
        fs::write(&path, "").unwrap();

        let result = find_task_in_file(&path, "task-1").await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // run_update — with --project flag
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_update_project_task_by_filename() {
        let env = TestEnv::new().await;

        run_create(
            &env.notes,
            "Fix bug",
            None,
            Some("sprint-12"),
            "TODO",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        // Find the task ID from the project file
        let path = fs::read_dir(&env.notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        let id_marker = ":ID:       ";
        let second_id_start = content.match_indices(id_marker).nth(1).unwrap().0 + id_marker.len();
        let id = content[second_id_start..].lines().next().unwrap().trim().to_string();

        // Update using --project with filename
        let filename = path.file_name().unwrap().to_str().unwrap().to_string();
        run_update(
            &env.notes,
            &id,
            Some("Fixed bug"),
            None,
            Some("DONE"),
            Some(&filename),
            &env.index,
            &env.db,
        )
        .await
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
        let env = TestEnv::new().await;

        run_create(
            &env.notes,
            "Add tests",
            None,
            Some("sprint-12"),
            "TODO",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        // Find the project file's ID (first :ID: in the file)
        let path = fs::read_dir(&env.notes).unwrap().next().unwrap().unwrap().path();
        let content = fs::read_to_string(&path).unwrap();
        let id_marker = ":ID:       ";
        let first_id_start = content.find(id_marker).unwrap() + id_marker.len();
        let project_id = content[first_id_start..].lines().next().unwrap().trim().to_string();

        // Find the task ID (second :ID:)
        let second_id_start = content.match_indices(id_marker).nth(1).unwrap().0 + id_marker.len();
        let task_id = content[second_id_start..].lines().next().unwrap().trim().to_string();

        // Update using --project with project ID
        run_update(
            &env.notes,
            &task_id,
            None,
            None,
            Some("DONE"),
            Some(&project_id),
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        let config = parsing_config();
        let org = config.parse(&fs::read_to_string(&path).unwrap());
        let headlines: Vec<_> = org.document().headlines().collect();
        assert_eq!(headlines.len(), 1);
        assert_eq!(headlines[0].todo_keyword().unwrap().to_string(), "DONE");
    }

    #[tokio::test]
    async fn test_update_project_task_not_found_in_project() {
        let env = TestEnv::new().await;

        // Create a standalone task (not in any project)
        run_create(
            &env.notes,
            "Standalone",
            None,
            None,
            "TODO",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        // Create a project with a different task
        run_create(
            &env.notes,
            "Project task",
            None,
            Some("my-project"),
            "TODO",
            &env.index,
            &env.db,
        )
        .await
        .unwrap();

        // Find the standalone task's ID
        let standalone_path = env._dir.path().join(
            fs::read_dir(&env.notes)
                .unwrap()
                .filter_map(|e| {
                    let p = e.unwrap().path();
                    let n = p.file_name().unwrap().to_str().unwrap().to_string();
                    if n.contains("--project-") { None } else { Some(p) }
                })
                .next()
                .unwrap(),
        );
        let content = fs::read_to_string(&standalone_path).unwrap();
        let id_start = content.find(":ID:").unwrap() + ":ID:       ".len();
        let task_id = content[id_start..].lines().next().unwrap().trim().to_string();

        // Try to update it scoped to the project — should fail
        let result = run_update(
            &env.notes,
            &task_id,
            None,
            None,
            Some("DONE"),
            Some("my-project"),
            &env.index,
            &env.db,
        )
        .await;
        assert!(result.is_err(), "should not find standalone task in project file");
    }

    #[tokio::test]
    async fn test_update_project_task_nonexistent_project() {
        let env = TestEnv::new().await;

        let result = run_update(
            &env.notes,
            "some-id",
            None,
            None,
            Some("DONE"),
            Some("nonexistent"),
            &env.index,
            &env.db,
        )
        .await;
        assert!(result.is_err(), "should error for nonexistent project");
    }
}
