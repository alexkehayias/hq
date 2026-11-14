//! Parse Org files into the note model that the indexer translates into
//! documents.
use anyhow::{Context, Result};
use orgize::rowan::ast::AstNode;
use regex::Regex;
use tokio::fs;

use crate::core::markdown::MarkdownExport;
use crate::core::orgmode::{ORG_ARCHIVE_SUFFIX, is_archive_file};
use crate::org::todo_keywords_config;

#[derive(Debug, Clone)]
pub(crate) struct Task {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) category: String,
    pub(crate) body: String,
    pub(crate) status: String,
    pub(crate) tags: Option<String>,
    pub(crate) scheduled: Option<String>,
    pub(crate) deadline: Option<String>,
    pub(crate) closed: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Meeting {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) category: String,
    pub(crate) body: String,
    pub(crate) tags: Option<String>,
    pub(crate) date: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Heading {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) category: String,
    pub(crate) body: String,
    pub(crate) tags: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Note {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) category: String,
    pub(crate) body: String,
    pub(crate) tags: Option<String>,
    pub(crate) tasks: Vec<Task>,
    pub(crate) meetings: Vec<Meeting>,
    pub(crate) headings: Vec<Heading>,
}

/// Parse the content into a `Note`.
///
/// `file_name` is the note's path relative to the notes root. `source_id` is
/// the document-level `:ID:` of the note an archive file was archived from
/// (e.g. `work.org` for `work.org_archive`); it is used only when the content
/// itself carries no `:ID:`. Resolution is the caller's responsibility so this
/// stays free of file I/O.
pub(crate) fn parse_note(file_name: &str, source_id: Option<&str>, content: &str) -> Result<Note> {
    let config = todo_keywords_config();
    let p = config.parse(content);
    let d = p.document();

    let is_archive = is_archive_file(file_name);

    // Org archive files (`work.org_archive`) carry no document-level `:ID:`;
    // fall back to the source file's ID so archived content groups with the
    // note it was archived from.
    let note_id = document_id(&d)
        .or_else(|| source_id.map(str::to_owned))
        .with_context(|| {
            format!(
                "Missing org-id for note '{file_name}'{}",
                if is_archive {
                    " and unable to resolve the source file's ID"
                } else {
                    ""
                }
            )
        })?;
    // Archive files carry no `#+TITLE:`; fall back to the source filename stem
    // (e.g. `work` from `projects/work.org_archive`) so parsing succeeds.
    let note_title = p
        .title()
        .or_else(|| {
            is_archive.then(|| {
                let stem = file_name.rsplit('/').next().unwrap_or(file_name);
                stem.strip_suffix(ORG_ARCHIVE_SUFFIX)
                    .unwrap_or(stem)
                    .to_string()
            })
        })
        .context("No title found")?;
    let note_category = p
        .keywords()
        .filter_map(|k| match k.key().to_string().as_str() {
            "CATEGORY" => Some(k.value().to_string()),
            _ => None,
        })
        .collect::<Vec<String>>()
        .first()
        .unwrap_or(&note_title.to_lowercase().replace(" ", "_"))
        .trim()
        .to_owned();

    let mut note_body_md = MarkdownExport::default();
    note_body_md.render(d.syntax());
    let note_body = note_body_md.finish();

    let filetags: Vec<Vec<String>> = p
        .keywords()
        .filter_map(|k| match k.key().to_string().as_str() {
            "FILETAGS" => Some(
                k.value()
                    .to_string()
                    .trim()
                    .split(" ")
                    .map(|s| s.to_string())
                    .collect(),
            ),
            _ => None,
        })
        .collect();

    // For now, tags are a comma separated string which should
    // allow it to still be searchable
    let note_tags = if filetags.is_empty() {
        None
    } else {
        Some(filetags[0].to_owned().join(","))
    };

    let mut tasks: Vec<Task> = Vec::new();
    let mut meetings: Vec<Meeting> = Vec::new();
    let mut headings: Vec<Heading> = Vec::new();

    let date_regex = Regex::new(r"(\d{4})-(\d{2})-(\d{2})").unwrap();
    for i in p.document().headlines() {
        let tag_string = i
            .tags()
            .map(|j| j.to_string())
            .collect::<Vec<String>>()
            .join(",");
        // Mark every entry indexed from an archive file as archived so it can
        // be distinguished from live content in search and listings.
        let tags = match (tag_string.is_empty(), is_archive) {
            (true, false) => None,
            (true, true) => Some("archive".to_string()),
            (false, false) => Some(tag_string.clone()),
            (false, true) => Some(format!("{tag_string},archive")),
        };
        let title = i.title_raw().trim().to_string();

        let id = match i.properties().and_then(|p| p.get("ID")) {
            Some(id) => id.to_string(),
            None => {
                tracing::warn!("Skipping heading with missing ID: '{}'", title);
                continue;
            }
        };

        let mut plain_text = MarkdownExport::default();
        plain_text.render(i.syntax());
        let body = plain_text.finish();

        // Handle meetings
        if tag_string.contains("meeting") {
            // Parse it from the headline to get the meeting date
            // since this is always added as part of the org-mode
            // capture template
            let mut dates = vec![];
            for (_, [year, month, day]) in date_regex.captures_iter(&title).map(|c| c.extract()) {
                dates.push(format!("{}-{}-{}", year, month, day));
            }
            let date = dates.first().map(|d| d.to_string()).unwrap_or_else(|| {
                println!(
                    "Meeting missing date! {}, file: {}",
                    title.clone(),
                    note_title.clone()
                );
                String::from("2000-01-01")
            });

            let meeting = Meeting {
                id,
                title,
                category: note_category.clone(),
                body,
                tags,
                date,
            };
            meetings.push(meeting);
            continue;
        }

        // Handle tasks
        if let Some(status) = i.todo_keyword().map(|j| j.to_string().to_lowercase()) {
            let mut scheduled = None;
            let mut deadline = None;
            let mut closed = None;
            if let Some(planning) = i.planning() {
                scheduled = planning.scheduled().map(|t| {
                    format!(
                        "{}-{}-{}",
                        t.year_start().unwrap(),
                        t.month_start().unwrap(),
                        t.day_start().unwrap()
                    )
                });
                deadline = planning.deadline().map(|t| {
                    format!(
                        "{}-{}-{}",
                        t.year_start().unwrap(),
                        t.month_start().unwrap(),
                        t.day_start().unwrap()
                    )
                });
                closed = planning.closed().map(|t| {
                    format!(
                        "{}-{}-{}",
                        t.year_start().unwrap(),
                        t.month_start().unwrap(),
                        t.day_start().unwrap()
                    )
                });
            }

            let task = Task {
                id,
                title,
                category: note_category.clone(),
                body,
                tags,
                status,
                scheduled,
                deadline,
                closed,
            };
            tasks.push(task);
            continue;
        }

        // Handle all other headings
        let heading = Heading {
            id,
            title,
            category: note_category.clone(),
            body,
            tags,
        };
        headings.push(heading);
    }

    Ok(Note {
        id: note_id,
        title: note_title,
        category: note_category,
        body: note_body,
        tags: note_tags,
        tasks,
        meetings,
        headings,
    })
}

/// For `.org_archive` files (Org mode's archive location), resolve the
/// document-level `:ID:` of the source file the entries were archived from
/// (e.g. `work.org_archive` -> `work.org`). Returns `None` when the file isn't
/// an archive or the source's ID can't be determined.
pub(crate) async fn archive_source_id(notes_dir_path: &str, file_name: &str) -> Option<String> {
    let source_name = format!("{}.org", file_name.strip_suffix(ORG_ARCHIVE_SUFFIX)?);
    let source_path = if file_name.starts_with('/') {
        source_name
    } else {
        format!("{notes_dir_path}/{source_name}")
    };
    let content = fs::read_to_string(&source_path).await.ok()?;
    let config = todo_keywords_config();
    document_id(&config.parse(&content).document())
}

/// The document-level `:ID:` property of a parsed Org document.
fn document_id(d: &orgize::ast::Document) -> Option<String> {
    d.properties()
        .and_then(|props| props.get("ID").map(|id| id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Verify that all example .org notes parse successfully.
    ///
    /// Every note must have a document-level property drawer with an :ID:,
    /// and every headline must also have its own property drawer with an :ID:.
    #[test]
    fn test_example_notes_parse_successfully() {
        let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/notes");
        let mut tested = 0;

        for entry in fs::read_dir(&examples_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "org") {
                let content = fs::read_to_string(&path).unwrap();
                let result = parse_note("", None, &content);
                assert!(
                    result.is_ok(),
                    "Failed to parse example note {:?}: {}",
                    path.file_name().unwrap(),
                    result.unwrap_err()
                );
                tested += 1;
            }
        }

        assert!(tested > 0, "No .org files found in examples/notes/");
    }

    /// Parsing a `.org_archive` file resolves the source note's document-level
    /// `:ID:` (so archived content groups with its source note) and tags every
    /// indexed entry with `archive`. The archive itself carries no document-level
    /// `:ID:`, matching Org mode's archive files.
    #[tokio::test]
    async fn test_parse_archive_resolves_source_id_and_tags() {
        let dir = TempDir::new().unwrap();
        let notes_root = dir.path();

        let projects_dir = notes_root.join("projects");
        fs::create_dir_all(&projects_dir).unwrap();

        fs::write(
            projects_dir.join("work.org"),
            ":PROPERTIES:\n:ID: source-work\n:END:\n#+TITLE: Work\n\n* TODO Live task\n:PROPERTIES:\n:ID: live-task\n:END:\n",
        )
        .unwrap();

        fs::write(
            projects_dir.join("work.org_archive"),
            "Archived entries from file work.org\n\n* DONE Archived task\n:PROPERTIES:\n:ARCHIVE_TIME: 2022-10-16 Sun 09:37\n:ID: archived-task\n:END:\nCLOSED: [2022-10-16 Sun 09:37]\n\n* Archived heading\n:PROPERTIES:\n:ID: archived-heading\n:END:\n",
        )
        .unwrap();

        let file_name = "projects/work.org_archive";
        let archive = fs::read_to_string(projects_dir.join("work.org_archive")).unwrap();
        let source_id = archive_source_id(notes_root.to_str().unwrap(), file_name).await;
        let note = parse_note(file_name, source_id.as_deref(), &archive).unwrap();

        // Same document-level ID as the source note.
        assert_eq!(note.id, "source-work");

        let mut tagged = 0;
        for t in &note.tasks {
            assert!(
                t.tags.as_deref().unwrap_or("").contains("archive"),
                "task missing archive tag: {:?}",
                t.tags
            );
            tagged += 1;
        }
        for h in &note.headings {
            assert!(
                h.tags.as_deref().unwrap_or("").contains("archive"),
                "heading missing archive tag: {:?}",
                h.tags
            );
            tagged += 1;
        }
        assert!(tagged > 0, "expected at least one tagged archived entry");
    }
}
