use super::registry::{Tool, ToolConfig};
use crate::openai::{
    Function, Parameters, Property, RecoverableToolError, ToolCall, ToolType, parse_tool_args,
};
use crate::search::{aql, search_notes};
use crate::types::notes::SearchResult;
use anyhow::{Error, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_rusqlite::Connection;

/// The search tool searches the index directly rather than looping back
/// through the HTTP API. These mirror the API defaults.
const INCLUDE_SIMILARITY: bool = false;
const TRUNCATE: bool = true;
const DEFAULT_LIMIT: usize = 20;

#[derive(Serialize)]
pub struct SearchProps {
    pub query: Property,
}

#[derive(Deserialize)]
pub struct SearchArgs {
    pub query: String,
}

/// Searches notes, meetings, tasks, and chat history using AQL. This
/// replaces the separate note/meeting/task tools: the model composes a
/// query, and skills provide the specialized recipes.
#[derive(Serialize)]
pub struct SearchTool {
    pub r#type: ToolType,
    pub function: Function<SearchProps>,
    #[serde(skip)]
    db: Connection,
    #[serde(skip)]
    index_path: String,
}

#[async_trait]
impl ToolCall for SearchTool {
    async fn call(&self, args: &str) -> Result<String, Error> {
        let fn_args: SearchArgs = parse_tool_args(args)?;

        let query = aql::parse_and_validate_query(&fn_args.query).map_err(|e| {
            Error::from(RecoverableToolError::new(&format!(
                "Invalid AQL query: {e}"
            )))
        })?;

        let results = search_notes(
            &self.index_path,
            &self.db,
            INCLUDE_SIMILARITY,
            TRUNCATE,
            &query,
            DEFAULT_LIMIT,
        )
        .await
        .map_err(|e| Error::from(RecoverableToolError::new(&format!("Search failed: {e}"))))?;

        Ok(format_results(&results))
    }

    fn function_name(&self) -> String {
        Self::NAME.to_string()
    }
}

impl Tool for SearchTool {
    const NAME: &'static str = "search";

    fn from_config(conf: &ToolConfig) -> Result<Self> {
        Ok(Self::new(conf.db.clone(), &conf.index_path))
    }
}

impl SearchTool {
    pub fn new(db: Connection, index_path: &str) -> Self {
        let function = Function {
            name: Self::NAME.to_string(),
            description: String::from(
                "Search the user's notes, meetings, tasks, and chat history with AQL. \
                 Examples: `meeting`, `title:rust`, `type:meeting tags:standup`, \
                 `type:task status:todo deadline:<=2025-01-31`, `-status:done`, \
                 `title:\"rust programming\"`, `status:done OR status:canceled`.\n\n\
                 Fields: title, body, tags, status (alias `todo`), category (alias \
                 `project`), type (note|task|meeting|heading|chat), date, scheduled, \
                 deadline, closed, file_name, id, chat_role.\n\n\
                 Syntax: space-separated terms are AND-ed; comma-separated values are \
                 OR-ed (`tags:work,urgent`); prefix with `-` or `NOT` to negate; \
                 `field:` matches documents that have the field; ranges (`>`, `>=`, \
                 `<`, `<=`) work on date fields with `YYYY-MM-DD` values. Load a skill \
                 for task-, meeting-, or note-specific query recipes.",
            ),
            parameters: Parameters {
                r#type: String::from("object"),
                properties: SearchProps {
                    query: Property {
                        r#type: String::from("string"),
                        description: String::from(
                            "The AQL query. Use fielded terms like `type:task status:todo` \
                             or a plain phrase like `quarterly review`.",
                        ),
                        r#enum: None,
                    },
                },
                required: vec![String::from("query")],
                additional_properties: false,
            },
            strict: true,
        };
        Self {
            r#type: ToolType::Function,
            function,
            db,
            index_path: index_path.to_string(),
        }
    }
}

fn format_results(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "No results found.".to_string();
    }
    results
        .iter()
        .map(format_result)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn format_result(result: &SearchResult) -> String {
    let mut meta = vec![
        format!("id: {}", result.id),
        format!("type: {}", result.r#type),
    ];

    if !result.category.is_empty() {
        meta.push(format!("category: {}", result.category));
    }
    if let Some(tags) = result.tags.as_deref().filter(|tags| !tags.is_empty()) {
        meta.push(format!("tags: {tags}"));
    }
    if let Some(status) = &result.task_status {
        meta.push(format!("status: {status}"));
    }
    if let Some(scheduled) = &result.task_scheduled {
        meta.push(format!("scheduled: {scheduled}"));
    }
    if let Some(deadline) = &result.task_deadline {
        meta.push(format!("deadline: {deadline}"));
    }
    if let Some(closed) = &result.task_closed {
        meta.push(format!("closed: {closed}"));
    }
    if let Some(date) = &result.meeting_date {
        meta.push(format!("date: {date}"));
    }
    if let Some(role) = &result.chat_role {
        meta.push(format!("chat_role: {role}"));
    }
    if let Some(session_id) = &result.session_id {
        meta.push(format!("session_id: {session_id}"));
    }

    format!("## {}\n{}\n{}", result.title, meta.join(" · "), result.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db::{async_db, initialize_db};
    use hq_search::{DocumentType, Index, IndexDocument, recreate_index};
    use tokio_rusqlite::rusqlite;

    /// A tool backed by a migrated, empty database. Enough to exercise
    /// argument parsing and query validation.
    async fn test_tool() -> (tempfile::TempDir, SearchTool) {
        let dir = tempfile::tempdir().unwrap();
        let db = async_db(dir.path().to_str().unwrap()).await.unwrap();
        db.call(|conn| {
            initialize_db(conn).expect("Failed to migrate db");
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .unwrap();

        let tool = SearchTool::new(db, dir.path().join("index").to_str().unwrap());
        (dir, tool)
    }

    /// A tool backed by a database and a full-text index containing one task.
    async fn indexed_tool() -> (tempfile::TempDir, SearchTool) {
        let (dir, tool) = test_tool().await;

        std::fs::create_dir_all(&tool.index_path).unwrap();
        recreate_index(&tool.index_path).await;

        let index = Index::open(
            tool.db.clone(),
            &tool.index_path,
            dir.path().join("embeddings"),
        );
        index
            .upsert(
                &[IndexDocument {
                    id: "note-123".to_string(),
                    document_type: DocumentType::Task,
                    title: "Complete project report".to_string(),
                    category: "work".to_string(),
                    body: "Write the final report for the quarterly project review".to_string(),
                    tags: Some("urgent,priority".to_string()),
                    status: Some("todo".to_string()),
                    scheduled: None,
                    deadline: Some("2026-02-06".to_string()),
                    closed: None,
                    date: None,
                    file_name: "work.org".to_string(),
                    chat_role: None,
                    embed: false,
                }],
                true,
                false,
            )
            .await
            .unwrap();

        (dir, tool)
    }

    #[tokio::test]
    async fn it_searches_with_aql() {
        let (_dir, tool) = indexed_tool().await;

        let output = tool
            .call(r#"{"query":"type:task status:todo deadline:<=2026-02-06"}"#)
            .await
            .expect("search should succeed");

        assert!(output.contains("## Complete project report"));
        assert!(output.contains("id: note-123"));
        assert!(output.contains("type: task"));
        assert!(output.contains("status: todo"));
        assert!(output.contains("deadline: 2026-02-06"));
    }

    #[tokio::test]
    async fn it_reports_no_results() {
        let (_dir, tool) = indexed_tool().await;

        let output = tool
            .call(r#"{"query":"nonexistentterm123"}"#)
            .await
            .expect("search should succeed");

        assert_eq!(output, "No results found.");
    }

    #[tokio::test]
    async fn it_returns_recoverable_error_for_unknown_field() {
        let (_dir, tool) = test_tool().await;

        let err = tool
            .call(r#"{"query":"titel:rust"}"#)
            .await
            .expect_err("unknown field should fail");

        let recoverable = err
            .downcast_ref::<RecoverableToolError>()
            .expect("error should be recoverable");
        assert!(recoverable.message.contains("Invalid AQL query"));
        assert!(recoverable.message.contains("unknown field `titel`"));
    }

    #[tokio::test]
    async fn it_returns_recoverable_error_for_bad_date() {
        let (_dir, tool) = test_tool().await;

        let err = tool
            .call(r#"{"query":"deadline:>tomorrow"}"#)
            .await
            .expect_err("bad date should fail");

        let recoverable = err
            .downcast_ref::<RecoverableToolError>()
            .expect("error should be recoverable");
        assert!(recoverable.message.contains("invalid date `tomorrow`"));
    }

    #[test]
    fn test_search_name() {
        assert_eq!(SearchTool::NAME, "search");
    }
}
