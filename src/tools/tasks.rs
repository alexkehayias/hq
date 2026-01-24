use crate::openai::{Function, Parameters, ToolCall, ToolType};
use crate::public::SearchResponse;
use anyhow::{Error, Result};
use async_trait::async_trait;
use chrono::Utc;
use reqwest;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct TasksDueTodayProps {}

#[derive(Deserialize)]
pub struct TasksDueTodayArgs {}

#[derive(Serialize)]
pub struct TasksDueTodayTool {
    pub r#type: ToolType,
    pub function: Function<TasksDueTodayProps>,
    api_base_url: String,
}

#[async_trait]
impl ToolCall for TasksDueTodayTool {
    async fn call(&self, _args: &str) -> Result<String, Error> {
        let today = Utc::now().format("%Y-%m-%d").to_string();

        // Build query: deadline:<TODAY> -status:done -status:canceled -title:journal
        let query = format!("deadline:<={} -status:done -status:canceled", today);

        let mut url = reqwest::Url::parse(&format!("{}/notes/search", self.api_base_url))
            .expect("Invalid URL");
        url.query_pairs_mut()
            .append_pair("query", &query)
            .append_pair("include_similarity", "false");

        let resp = reqwest::Client::new()
            .get(url.as_str())
            .header("Content-Type", "application/json")
            .send()
            .await?
            .error_for_status()?;

        let search_resp: SearchResponse = resp.json().await?;

        if search_resp.results.is_empty() {
            return Ok("No results found".to_string());
        }

        let mut accum = vec![];
        for r in search_resp.results.iter() {
            accum.push(format!("## {}\n{}\n{}", r.title, r.id, r.body))
        }

        Ok(accum.join("\n\n"))
    }

    fn function_name(&self) -> String {
        self.function.name.clone()
    }
}

impl TasksDueTodayTool {
    pub fn new(api_base_url: &str) -> Self {
        let function = Function {
            name: String::from("tasks_due_today"),
            description: String::from(
                "Get a list of tasks that are due today, excluding done and canceled tasks.",
            ),
            parameters: Parameters {
                r#type: String::from("object"),
                properties: TasksDueTodayProps {},
                required: vec![],
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

impl Default for TasksDueTodayTool {
    fn default() -> Self {
        Self::new("http://localhost:2222")
    }
}

#[derive(Serialize)]
pub struct TasksScheduledTodayProps {}

#[derive(Deserialize)]
pub struct TasksScheduledTodayArgs {}

#[derive(Serialize)]
pub struct TasksScheduledTodayTool {
    pub r#type: ToolType,
    pub function: Function<TasksScheduledTodayProps>,
    api_base_url: String,
}

#[async_trait]
impl ToolCall for TasksScheduledTodayTool {
    async fn call(&self, _args: &str) -> Result<String, Error> {
        let today = Utc::now().format("%Y-%m-%d").to_string();

        // Build query: scheduled:<TODAY> -status:done -status:canceled -title:journal
        let query = format!("scheduled:<={} -status:done -status:canceled", today);

        let mut url = reqwest::Url::parse(&format!("{}/notes/search", self.api_base_url))
            .expect("Invalid URL");
        url.query_pairs_mut()
            .append_pair("query", &query)
            .append_pair("include_similarity", "false");

        let resp = reqwest::Client::new()
            .get(url.as_str())
            .header("Content-Type", "application/json")
            .send()
            .await?
            .error_for_status()?;

        let search_resp: SearchResponse = resp.json().await?;

        if search_resp.results.is_empty() {
            return Ok("No results found".to_string());
        }

        let mut accum = vec![];
        for r in search_resp.results.iter() {
            accum.push(format!("## {}\n{}\n{}", r.title, r.id, r.body))
        }

        Ok(accum.join("\n\n"))
    }

    fn function_name(&self) -> String {
        self.function.name.clone()
    }
}

impl TasksScheduledTodayTool {
    pub fn new(api_base_url: &str) -> Self {
        let function = Function {
            name: String::from("tasks_scheduled_today"),
            description: String::from(
                "Get a list of tasks that are scheduled for today, excluding done and canceled tasks.",
            ),
            parameters: Parameters {
                r#type: String::from("object"),
                properties: TasksScheduledTodayProps {},
                required: vec![],
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

impl Default for TasksScheduledTodayTool {
    fn default() -> Self {
        Self::new("http://localhost:2222")
    }
}
