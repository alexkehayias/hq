use std::ops::Range;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Local;
use once_cell::sync::Lazy;
use orgize::export::{Container, Event, TraversalContext, Traverser};
use orgize::rowan::ast::AstNode;
use orgize::SyntaxElement;
use regex::Regex;
use tokio::fs;

use crate::org;

/// Done-state keywords that count as "closed" for setting CLOSED:.
///
/// SOMEDAY is intentionally excluded — only DONE and CANCELED mark a task as
/// closed (per user requirement).
const CLOSED_STATUSES: &[&str] = &["DONE", "CANCELED"];

/// Format the current local time as an inactive org timestamp:
/// `[YYYY-MM-DD Ddd HH:MM]`.
fn now_timestamp() -> String {
    Local::now().format("[%Y-%m-%d %a %H:%M]").to_string()
}

/// Format a state-change logbook entry matching emacs' `org-log-into-drawer`:
///
/// ```text
/// - State "DONE"       from "TODO"       [2026-05-23 Sat 10:59]
/// ```
///
/// The seven-space alignment between the quoted state keyword and `from`, and
/// again before the timestamp, matches emacs' default formatting so logs
/// produced by hq round-trip cleanly with logs added by emacs.
fn format_state_change(from: &str, to: &str) -> String {
    format!("- State \"{to}\"       from \"{from}\"       {}", now_timestamp())
}

/// Regex for the CLOSED: planning line, capturing the bracketed timestamp.
/// Matches `CLOSED: [2026-05-23 Sat 10:59]` with any whitespace between
/// `CLOSED:` and the bracket. First capture group is the timestamp (with
/// brackets).
static RE_CLOSED: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^CLOSED:\s*(\[[^\]]+\])").unwrap()
});

/// Regex for the boundaries of a `:LOGBOOK:` drawer so we can scope
/// state-change line extraction to within the drawer. Captures the content
/// between `:LOGBOOK:` and the matching `:END:` on its own line.
static RE_LOGBOOK_BLOCK: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?ms)^:LOGBOOK:\n(.*?)(?:^:END:\n?)").unwrap()
});

/// Regex for individual state-change lines inside a LOGBOOK drawer. Captures
/// the full line so it can be preserved verbatim.
///
/// Trailing whitespace is restricted to horizontal (`[ \t]`) so the match
/// does not consume the newline at end of line — that would otherwise cause
/// `writeln!` to emit a spurious blank line between consecutive entries.
static RE_STATE_CHANGE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?m)^- State "(\w+)"\s+from "(\w+)"\s+(\[[^\]]+\])[ \t]*$"#).unwrap()
});

/// Extract the CLOSED: timestamp from a headline's raw text, if present.
///
/// `raw` should be the slice of file content covering one headline (the
/// `content[range.start..range.end]` slice stored in `TaskLocation::content`
/// and indexed by `TaskLocation::range`).
fn extract_closed(raw: &str) -> Option<String> {
    RE_CLOSED
        .captures(raw)
        .map(|caps| caps.get(1).unwrap().as_str().to_string())
}

/// Extract state-change logbook entries from a headline's raw text.
///
/// Returns each `- State "X"       from "Y"       [TS]` line found inside a
/// `:LOGBOOK:` drawer, preserving the original text verbatim. Entries outside
/// a LOGBOOK drawer are ignored.
fn extract_logbook(raw: &str) -> Vec<String> {
    let mut entries = Vec::new();
    for block in RE_LOGBOOK_BLOCK.captures_iter(raw) {
        let content = block.get(1).unwrap().as_str();
        for caps in RE_STATE_CHANGE.captures_iter(content) {
            entries.push(caps.get(0).unwrap().as_str().to_string());
        }
    }
    entries
}

pub struct TaskLocation {
    pub path: PathBuf,
    pub range: Range<usize>,
    pub content: String,
    pub current_title: String,
    pub current_body: String,
    pub current_status: String,
    pub current_level: usize,
    /// Existing `CLOSED:` timestamp bracket string (e.g.
    /// `"[2026-05-23 Sat 10:59]"`) parsed from the headline, if any.
    pub current_closed: Option<String>,
    /// Existing state-change lines from the headline's `:LOGBOOK:` drawer,
    /// preserved verbatim. Empty if no LOGBOOK drawer is present.
    pub current_logbook: Vec<String>,
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

/// Build an updated headline string from current and new field values,
/// including the `CLOSED:` planning line and `:LOGBOOK:` drawer entries.
///
/// This is used by `update_task` / `update_task_in_file` to rebuild a
/// headline in place. New headlines (created via `run_create`) use
/// `build_headline` instead, since they have no prior state to preserve.
fn build_updated_headline(
    id: &str,
    title: &str,
    body: &str,
    status: &str,
    level: usize,
    closed: Option<&str>,
    logbook: &[String],
) -> String {
    let mut h = org::Headline::builder()
        .level(level)
        .status(status)
        .title(title)
        .property("ID", id);
    if !body.is_empty() {
        h = h.body(body);
    }
    if let Some(ts) = closed {
        h = h.closed(ts);
    }
    for entry in logbook {
        h = h.logbook_entry(entry);
    }
    h.build().to_string()
}

/// Given a task's current state (`location`) and the new status being applied,
/// compute the resulting `CLOSED:` timestamp and logbook entries.
///
/// Rules:
/// - If status unchanged: preserve existing CLOSED/logbook as-is (no new entry).
/// - If status changed: append a `- State "NEW"       from "OLD"       [TS]`
///   entry to the logbook.
/// - If transitioning into DONE/CANCELED (from a non-closed state): set
///   CLOSED to the current time.
/// - If transitioning out of DONE/CANCELED (reopening): clear CLOSED.
/// - SOMEDAY is not treated as closed — transitions to SOMEDAY log a state
///   change but do not set CLOSED.
fn compute_state_transition(
    location: &TaskLocation,
    new_status: &str,
) -> (Option<String>, Vec<String>) {
    let old_status = &location.current_status;
    let status_changed = old_status != new_status;

    let new_is_closed = CLOSED_STATUSES.contains(&new_status);
    let old_was_closed = CLOSED_STATUSES.contains(&old_status.as_str());

    let mut new_logbook = location.current_logbook.clone();
    let mut new_closed = location.current_closed.clone();

    if status_changed {
        new_logbook.push(format_state_change(old_status, new_status));

        if new_is_closed && !old_was_closed {
            // Closing: set CLOSED to now.
            new_closed = Some(now_timestamp());
        } else if !new_is_closed && old_was_closed {
            // Reopening: clear CLOSED.
            new_closed = None;
        }
    }

    (new_closed, new_logbook)
}

/// Recursively collect all .org files in a directory tree.
pub async fn collect_org_files(path: &std::path::Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(path).await else {
        return files;
    };
    while let Some(entry) = entries.next_entry().await.unwrap_or(None) {
        let p = entry.path();
        if p.is_dir() {
            files.extend(Box::pin(collect_org_files(&p)).await);
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
                let raw = &content[usize_range.clone()];
                let current_closed = extract_closed(raw);
                let current_logbook = extract_logbook(raw);

                return Ok(TaskLocation {
                    path: path.clone(),
                    range: usize_range,
                    content,
                    current_title,
                    current_body,
                    current_status,
                    current_level,
                    current_closed,
                    current_logbook,
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
    let files = collect_org_files(std::path::Path::new(notes_path)).await;

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
                    let raw = &content[usize_range.clone()];
                    let current_closed = extract_closed(raw);
                    let current_logbook = extract_logbook(raw);

                    return Ok(TaskLocation {
                        path,
                        range: usize_range,
                        content,
                        current_title,
                        current_body,
                        current_status,
                        current_level,
                        current_closed,
                        current_logbook,
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
            // Skip generic drawers (e.g. :LOGBOOK:) so their content doesn't
            // leak into the extracted body. State-change entries are read
            // separately via `extract_logbook` on the raw headline text.
            Event::Enter(Container::Drawer(_)) => {
                ctx.skip();
            }
            Event::Leave(Container::Drawer(_)) => {}
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
///
/// When `status` changes, a state-change entry is appended to the headline's
/// `:LOGBOOK:` drawer. When transitioning into DONE or CANCELED, a `CLOSED:`
/// planning timestamp is set; when reopening (DONE/CANCELED → non-closed),
/// `CLOSED:` is cleared. Existing logbook entries are preserved verbatim.
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

    let (new_closed, new_logbook) =
        compute_state_transition(&location, new_status);

    let new_headline = build_updated_headline(
        id,
        new_title,
        new_body,
        new_status,
        location.current_level,
        new_closed.as_deref(),
        &new_logbook,
    );
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

/// Update a task by UUID, optionally scoped to a specific file.
///
/// If `file_name` is provided, looks only in that file. If not found there,
/// returns an error (no filesystem fallback). If `file_name` is `None`,
/// searches all org files in the notes directory.
///
/// When `status` changes, a state-change entry is appended to the headline's
/// `:LOGBOOK:` drawer. When transitioning into DONE or CANCELED, a `CLOSED:`
/// planning timestamp is set; when reopening (DONE/CANCELED → non-closed),
/// `CLOSED:` is cleared. Existing logbook entries are preserved verbatim.
pub async fn update_task(
    notes_path: &str,
    id: &str,
    file_name: Option<&str>,
    title: Option<&str>,
    body: Option<&str>,
    status: Option<&str>,
) -> Result<()> {
    let location = if let Some(fname) = file_name {
        let path = std::path::Path::new(notes_path).join(fname);
        find_task_in_file(&path, id).await.with_context(|| {
            format!("Task {id} not found in scoped file {fname}")
        })?
    } else {
        find_task(notes_path, id).await?
    };
    let new_title = title.unwrap_or(&location.current_title);
    let new_body = body.unwrap_or(&location.current_body);
    let status = status.map(|s| s.to_uppercase());
    let new_status = status.as_deref().unwrap_or(&location.current_status);

    let (new_closed, new_logbook) = compute_state_transition(&location, new_status);

    let new_headline = build_updated_headline(
        id,
        new_title,
        new_body,
        new_status,
        location.current_level,
        new_closed.as_deref(),
        &new_logbook,
    );
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

