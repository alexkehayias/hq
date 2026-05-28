use anyhow::{Context, Result};
use chrono::Local;
use orgize::ParseConfig;
use orgize::rowan::ast::AstNode;
use std::fs;
use std::ops::Range;
use std::path::PathBuf;
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
}

fn find_task(notes_path: &str, id: &str) -> Result<TaskLocation> {
    let dir = fs::read_dir(notes_path)
        .with_context(|| format!("Cannot read notes directory: {notes_path}"))?;
    let id_pattern = format!(":ID:       {id}");

    for entry in dir {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map_or(true, |e| e != "org") {
            continue;
        }
        if path.file_name().unwrap_or_default() == "config.org" {
            continue;
        }

        let content = fs::read_to_string(&path)?;
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

fn find_or_create_project(notes_path: &str, project_name: &str) -> Result<PathBuf> {
    let slug = slugify(project_name);
    let pattern = format!("--project-{slug}.org");

    // Look for existing project file
    let dir = fs::read_dir(notes_path)?;
    for entry in dir {
        let entry = entry?;
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
    fs::write(&filename, &content).context("Failed to create project file")?;
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
        let project_path = find_or_create_project(notes_path, project_name)?;
        let headline = build_headline(&id, title, body, status, 1);
        let mut project_content = fs::read_to_string(&project_path)?;
        if !project_content.ends_with('\n') {
            project_content.push('\n');
        }
        project_content.push_str(&headline);
        project_content.push('\n');
        fs::write(&project_path, &project_content).context("Failed to write project file")?;
        println!("Created task '{title}' in project '{project_name}' (id: {id})");
    } else {
        let slug = slugify(title);
        let today = Local::now().format("%Y-%m-%d");
        let filename = format!("{notes_path}/{today}--{slug}.org");
        let content = build_standalone_org(&id, title, body, status);
        fs::write(&filename, &content).context("Failed to write task file")?;
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
    let location = find_task(notes_path, id)?;
    let new_title = title.unwrap_or(&location.current_title);
    let new_body = body.unwrap_or(&location.current_body);
    let new_status = status.as_deref().unwrap_or(&location.current_status);

    if location.is_standalone {
        let new_content = build_standalone_org(id, new_title, new_body, new_status);
        fs::write(&location.path, new_content).context("Failed to write updated task file")?;
    } else {
        let range = location.range.as_ref().unwrap();
        let new_headline = build_headline(id, new_title, new_body, new_status, 1);
        let new_content = format!(
            "{before}{new_headline}{after}",
            before = &location.content[..range.start],
            after = &location.content[range.end..]
        );
        fs::write(&location.path, new_content).context("Failed to write updated project file")?;
    }

    println!("Task {id} updated");
    Ok(())
}

pub async fn run_delete(notes_path: &str, id: &str) -> Result<()> {
    let location = find_task(notes_path, id)?;

    if location.is_standalone {
        fs::remove_file(&location.path).context("Failed to delete task file")?;
        println!("Deleted task file: {}", location.path.display());
    } else {
        let range = location.range.as_ref().unwrap();
        let before = &location.content[..range.start];
        let after = &location.content[range.end..];
        // Remove one trailing newline if present to avoid blank-line gaps
        let after = after.strip_prefix('\n').unwrap_or(after);
        let new_content = format!("{before}{after}");
        fs::write(&location.path, new_content).context("Failed to write project file after deletion")?;
        println!("Deleted task {id} from {}", location.path.display());
    }

    Ok(())
}
