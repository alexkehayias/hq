use std::ops::Range;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Local;
use orgize::export::{Container, Event, TraversalContext, Traverser};
use orgize::rowan::ast::AstNode;
use orgize::SyntaxElement;
use regex::Regex;
use tokio::fs;
use uuid::Uuid;

use crate::org;

pub struct TaskLocation {
    pub path: PathBuf,
    pub range: Range<usize>,
    pub content: String,
    pub current_title: String,
    pub current_body: String,
    pub current_status: String,
    pub current_level: usize,
}

pub fn build_headline(id: &str, title: &str, body: &str, status: &str, level: usize) -> String {
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

/// Recursively collect all .org files in a directory tree.
pub fn collect_org_files(path: &std::path::Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(path) else {
        return files;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            files.extend(collect_org_files(&p));
        } else if p.extension().map_or(false, |e| e == "org")
            && p.file_name().unwrap_or_default() != "config.org"
        {
            files.push(p);
        }
    }
    files
}

/// Build a regex pattern that matches `:ID:` followed by the given value,
/// regardless of the amount of whitespace between the key and value.
fn id_pattern(id: &str) -> Regex {
    Regex::new(&format!(":ID:\\s+{}", regex::escape(id))).unwrap()
}

/// Find a task by UUID within a specific file.
pub async fn find_task_in_file(path: &PathBuf, id: &str) -> Result<TaskLocation> {
    let pattern = id_pattern(id);
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("Cannot read file: {}", path.display()))?;

    if !pattern.is_match(&content) {
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

/// Find a task by UUID across all org files in the notes directory.
pub async fn find_task(notes_path: &str, id: &str) -> Result<TaskLocation> {
    let pattern = id_pattern(id);
    let files = collect_org_files(std::path::Path::new(notes_path));

    for path in files {
        if path.file_name().unwrap_or_default() == "config.org" {
            continue;
        }

        let content = fs::read_to_string(&path).await?;
        if !pattern.is_match(&content) {
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

/// Find a project file by ID (UUID property) or by exact filename match.
pub async fn find_project_file_by_id_or_name(
    notes_path: &str,
    project_ref: &str,
) -> Result<PathBuf> {
    let pattern = id_pattern(project_ref);
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
        if file_name == project_ref
            || file_name.ends_with(&format!("--project-{}.org", project_ref))
        {
            return Ok(path);
        }

        // Match by ID (UUID property in the org file)
        let content = fs::read_to_string(&path).await?;
        if pattern.is_match(&content) {
            return Ok(path);
        }
    }

    anyhow::bail!("Project '{project_ref}' not found by ID or filename");
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
pub fn body_from_headline(headline: &orgize::ast::Headline) -> String {
    let mut extractor = BodyExtractor::default();
    let mut ctx = TraversalContext::default();
    extractor.element(SyntaxElement::Node(headline.syntax().clone()), &mut ctx);
    extractor.finish()
}

/// Update a task's fields in a specific file.
///
/// Finds the task by ID in the given file, rebuilds the headline with the
/// provided field updates, and writes the modified file back to disk.
pub async fn update_task_in_file(
    file_path: &PathBuf,
    id: &str,
    title: Option<&str>,
    body: Option<&str>,
    status: Option<&str>,
) -> Result<()> {
    let location = find_task_in_file(file_path, id).await?;
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

    Ok(())
}

/// Update a task across the notes directory, with optional file/project scope.
///
/// If `file_name` is provided, looks in that file first (falling back to a
/// full search). If `project` is provided, scopes the search to that project
/// file. Otherwise searches all files.
pub async fn update_task(
    notes_path: &str,
    id: &str,
    file_name: Option<&str>,
    project: Option<&str>,
    title: Option<&str>,
    body: Option<&str>,
    status: Option<&str>,
) -> Result<()> {
    let location = if let Some(project_ref) = project {
        let project_path = find_project_file_by_id_or_name(notes_path, project_ref).await?;
        find_task_in_file(&project_path, id).await?
    } else if let Some(fname) = file_name {
        let path = std::path::Path::new(notes_path).join(fname);
        match find_task_in_file(&path, id).await {
            Ok(loc) => loc,
            Err(_) => {
                tracing::warn!(
                    "Task {id} not found in expected file {fname}, searching all files"
                );
                find_task(notes_path, id).await?
            }
        }
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

    Ok(())
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

/// Build a full org-mode document string for a standalone task.
pub fn build_document(id: &str, title: &str, body: &str, status: &str) -> String {
    let headline = org::Headline::builder()
        .level(1)
        .status(status)
        .title(title)
        .property("ID", id);
    let headline = if !body.is_empty() {
        headline.body(body)
    } else {
        headline
    };
    org::Document::builder()
        .property("ID", id)
        .title(title)
        .filetags("task")
        .headline(headline.build())
        .build()
        .to_string()
}

/// Find an existing project file by slug, or create a new one.
pub async fn find_or_create_project(notes_path: &str, project_name: &str) -> Result<PathBuf> {
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
    Ok(PathBuf::from(filename))
}
