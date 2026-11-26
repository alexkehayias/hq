use super::registry::{Tool, ToolConfig};
use crate::openai::{
    Function, Parameters, Property, RecoverableToolError, ToolCall, ToolType, parse_tool_args,
};
use crate::search::aql;
use crate::types::notes::{SearchResponse, SearchResult};
use anyhow::{Error, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct SearchProps {
    pub query: Property,
}

#[derive(Deserialize)]
pub struct SearchArgs {
    pub query: String,
}

/// Searches notes, meetings, tasks, and chat history through the notes API
/// using AQL. This replaces the separate note/meeting/task tools: the model
/// composes a query, and skills provide the specialized recipes.
#[derive(Serialize)]
pub struct SearchTool {
    pub r#type: ToolType,
    pub function: Function<SearchProps>,
    api_base_url: String,
}

#[async_trait]
impl ToolCall for SearchTool {
    async fn call(&self, args: &str) -> Result<String, Error> {
        let fn_args: SearchArgs = parse_tool_args(args)?;

        // Validate locally so the model gets a precise, actionable error
        // without a round trip. The API validates again for direct callers.
        let _query = aql::parse_and_validate_query(&fn_args.query).map_err(|e| {
            Error::from(RecoverableToolError::new(&format!(
                "Invalid AQL query: {e}"
            )))
        })?;

        let mut url = reqwest::Url::parse(&format!("{}/api/notes/search", self.api_base_url))
            .expect("Invalid URL");
        url.query_pairs_mut()
            .append_pair("query", &fn_args.query)
            .append_pair("include_similarity", "false");

        let resp = reqwest::Client::new()
            .get(url.as_str())
            .header("Content-Type", "application/json")
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(Error::from(RecoverableToolError::new(&format!(
                "Search failed ({}): {}",
                status,
                body.trim()
            ))));
        }

        let search_resp: SearchResponse = serde_json::from_str(&body)?;
        Ok(format_results(&search_resp.results))
    }

    fn function_name(&self) -> String {
        Self::NAME.to_string()
    }
}

impl Tool for SearchTool {
    const NAME: &'static str = "search";

    fn from_config(conf: &ToolConfig) -> Result<Self> {
        Ok(Self::new(&conf.api_base_url))
    }
}

impl SearchTool {
    pub fn new(api_base_url: &str) -> Self {
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
            api_base_url: api_base_url.to_string(),
        }
    }
}

impl Default for SearchTool {
    fn default() -> Self {
        Self::new("http://localhost:2222")
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
    use std::fs;

    #[tokio::test]
    async fn it_searches_with_aql() -> Result<()> {
        let mut server = mockito::Server::new_async().await;
        let url = server.url();

        let mock_resp = fs::read_to_string("./tests/data/tasks_search_response.json").unwrap();
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/api/notes/search\?query=.*".into()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_resp)
            .create();

        let tool = SearchTool::new(&url);
        let result = tool
            .call(r#"{"query":"type:task status:todo deadline:<=2025-01-31"}"#)
            .await;

        let output = result.expect("search should succeed");
        assert!(output.contains("## Complete project report"));
        assert!(output.contains("id: note-123"));
        assert!(output.contains("type: task"));
        assert!(output.contains("status: todo"));

        Ok(())
    }

    #[tokio::test]
    async fn it_reports_no_results() -> Result<()> {
        let mut server = mockito::Server::new_async().await;
        let url = server.url();

        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/api/notes/search\?query=.*".into()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"raw_query": "", "parsed_query": "", "results": []}"#)
            .create();

        let tool = SearchTool::new(&url);
        let output = tool.call(r#"{"query":"nothing-matches"}"#).await?;
        assert_eq!(output, "No results found.");

        Ok(())
    }

    #[tokio::test]
    async fn it_returns_recoverable_error_for_unknown_field() {
        let tool = SearchTool::default();
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
        let tool = SearchTool::default();
        let err = tool
            .call(r#"{"query":"deadline:>tomorrow"}"#)
            .await
            .expect_err("bad date should fail");

        let recoverable = err
            .downcast_ref::<RecoverableToolError>()
            .expect("error should be recoverable");
        assert!(recoverable.message.contains("invalid date `tomorrow`"));
    }

    #[tokio::test]
    async fn it_returns_recoverable_error_for_server_rejection() -> Result<()> {
        let mut server = mockito::Server::new_async().await;
        let url = server.url();

        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/api/notes/search\?query=.*".into()),
            )
            .with_status(400)
            .with_body("invalid query")
            .create();

        let tool = SearchTool::new(&url);
        let err = tool
            .call(r#"{"query":"meeting"}"#)
            .await
            .expect_err("400 should fail");

        let recoverable = err
            .downcast_ref::<RecoverableToolError>()
            .expect("error should be recoverable");
        assert!(recoverable.message.contains("invalid query"));

        Ok(())
    }

    #[test]
    fn test_search_default() {
        let tool = SearchTool::default();
        assert_eq!(tool.api_base_url, "http://localhost:2222");
        assert_eq!(tool.function_name(), "search");
    }
}
