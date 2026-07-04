use anyhow::{Error, Result, anyhow, bail};
use futures_util::future::try_join_all;
use serde_json::Value;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tokio_rusqlite::Connection;
use uuid::Uuid;

use super::db::{get_or_create_session, insert_chat_message};
use super::models::{SessionMode, Transcript};
use crate::ai::chat::middleware::{MiddlewareAction, ToolCallMiddleware};
use crate::ai::skills::SkillRegistry;
use crate::ai::tools::skills::{
    ListSkillsTool, LoadSkillTool, ReadSkillFileTool, SaveSkillTool, SearchSkillsTool,
    WorkOnSkillTool,
};
use crate::openai::{
    BoxedToolCall, FunctionCall, FunctionCallFn, Message, RecoverableToolError, Role, completion,
    completion_stream,
};

/// The core abstraction around interacting with an LLM in a chat
/// completion style using an OpenAI compatible API.
///
/// Supports the following features:
/// - Streaming
/// - Tool calling
/// - Saving to a database
//  - Use local or commercial models
///
/// Use `Chat::builder()` to construct a valid `Chat`.
pub struct Chat {
    api_hostname: String,
    api_key: String,
    model: String,
    db: Option<Connection>,
    streaming: bool,
    tx: Option<mpsc::UnboundedSender<String>>,
    tools: Option<Vec<BoxedToolCall>>,
    transcript: Transcript,
    pub session_id: Option<String>,
    tags: Option<Vec<String>>,
    middleware: Vec<Box<dyn ToolCallMiddleware>>,
    // TODO: Skills
    // TODO: MCP
    // TODO: Permissions
}

impl Chat {
    async fn handle_tool_call(
        tools: &Vec<BoxedToolCall>,
        tool_call: &FunctionCall,
    ) -> Result<Message, Error> {
        let tool_call_name = &tool_call.function.name;
        let tool_call_args = &tool_call.function.arguments;
        let tool_call_id = &tool_call.id;

        tracing::debug!(
            "\nTool call: {}\nargs: {}",
            &tool_call_name,
            &tool_call_args
        );

        // Call the tool and get the next completion from the result.
        // Recoverable errors are returned to the LLM as tool response
        // messages so it can retry or adjust its approach.
        let tool_call_result = match tools
            .iter()
            .find(|i| *i.function_name() == *tool_call_name)
            .ok_or(anyhow!(
                "Received tool call that doesn't exist: {}",
                tool_call_name
            ))?
            .call(tool_call_args)
            .await
        {
            Ok(result) => result,
            Err(e) => match e.downcast_ref::<RecoverableToolError>() {
                Some(recoverable) => recoverable.message.clone(),
                None => return Err(e),
            },
        };

        // TODO: if the tool call result is too large, write it to a
        // file and change the response text to a summary that points
        // to the file to use other tools to inspect if needed.

        Ok(Message::new_tool_call_response(&tool_call_result, tool_call_id))
    }

    async fn handle_tool_calls(
        tools: &Vec<BoxedToolCall>,
        tool_calls: &[FunctionCall],
    ) -> Result<Vec<Message>, Error> {
        // Run each tool call concurrently and return them in order. I'm
        // not sure if ordering really matters for OpenAI compatible API
        // implementations, but better to be safe. This could also be
        // done using a `futures::stream` and `FutureUnordered` which
        // would be more efficient as it runs on the same thread, but that
        // causes lifetime issues that I don't understand how to get
        // around.
        let futures = tool_calls
            .iter()
            .map(|call| Self::handle_tool_call(tools, call));
        try_join_all(futures).await
    }

    /// The inner chat loop that handles sending and receiving the
    /// next response from the LLM, tool calls,
    /// Runs the next turn in chat by passing a transcript to the LLM for
    /// the next response. Can return multiple messages when there are
    /// tool calls.
    pub async fn next_msg(&mut self, msg: Message) -> Result<Vec<Message>, Error> {
        self.transcript.push(msg.clone());

        let messages = if self.streaming {
            // ChatBuilder enforces that `streaming` and `tx` are
            // always set together
            let tx = &self.tx.clone().unwrap();
            Self::chat_stream(
                tx.clone(),
                &self.tools,
                &self.transcript,
                &self.api_hostname,
                &self.api_key,
                &self.model,
                &self.middleware,
            )
            .await?
        } else {
            Self::chat(
                &self.tools,
                &self.transcript,
                &self.api_hostname,
                &self.api_key,
                &self.model,
                &self.middleware,
            )
            .await?
        };

        // Store the new messages in the DB
        // ChatBuilder enforces that these are always set together
        if let (Some(db), Some(session_id), Some(tags)) = (&self.db, &self.session_id, &self.tags) {
            // Convert tags into a slice
            let tags: &[&str] = &tags.iter().map(String::as_str).collect::<Vec<&str>>();
            // Ensure that the session exists in the DB
            // NOTE: While it isn't great that this gets called repeatedly
            // for each turn in the chat, it avoids filling up the DB
            // with sessions that have no messages e.g. a chat that
            // resulted in an error on the first turn.
            get_or_create_session(db, session_id, tags, SessionMode::Chat).await?;

            // Save the input message
            insert_chat_message(db, session_id, &msg).await?;

            // Save each message
            for m in messages.iter() {
                self.transcript.push(m.clone());
                insert_chat_message(db, session_id, m).await?;
            }
        } else {
            for m in messages.iter() {
                self.transcript.push(m.clone());
            }
        }

        Ok(messages)
    }

    /// Parse raw JSON tool calls from the API response into typed
    /// `FunctionCall` structs.
    fn parse_tool_calls(tool_calls: &[Value]) -> Vec<FunctionCall> {
        tool_calls
            .iter()
            .map(|tc| {
                let function = &tc["function"];
                FunctionCall {
                    function: FunctionCallFn {
                        arguments: function["arguments"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        name: function["name"].as_str().unwrap_or_default().to_string(),
                    },
                    id: tc["id"].as_str().unwrap_or_default().to_string(),
                    r#type: "function".to_string(),
                }
            })
            .collect()
    }

    /// Run the middleware chain. Returns the first non-`Continue` action,
    /// or `Continue` if all middleware continued.
    async fn run_middleware(
        middleware: &[Box<dyn ToolCallMiddleware>],
        transcript: &[Message],
    ) -> MiddlewareAction {
        for mw in middleware {
            match mw.before_tool_calls(transcript).await {
                MiddlewareAction::Continue => {}
                other => return other,
            }
        }
        MiddlewareAction::Continue
    }

    /// Runs the next turn in chat by passing a transcript to the LLM for
    /// the next response. Can return multiple messages when there are
    /// tool calls.
    async fn chat(
        tools: &Option<Vec<BoxedToolCall>>,
        transcript: &Transcript,
        api_hostname: &str,
        api_key: &str,
        model: &str,
        middleware: &[Box<dyn ToolCallMiddleware>],
    ) -> Result<Vec<Message>, Error> {
        let history = transcript.messages();
        let mut updated_history = history.to_owned();
        let mut messages = Vec::new();

        let mut resp = completion(&history, tools, api_hostname, api_key, model).await?;

        // Tool calls need to be handled for the chat to proceed
        while let Some(tool_calls) = resp["choices"][0]["message"]["tool_calls"].as_array() {
            if tool_calls.is_empty() {
                break;
            }

            // Parse tool calls into typed structs immediately so
            // middleware and tool handling always operate on `Message`
            // and `FunctionCall` types, not raw JSON `Value`.
            let calls = Self::parse_tool_calls(tool_calls);

            // Build the tool call request message and add to transcript
            // before middleware runs, so middleware sees the full picture.
            let tool_call_msg = Message::new_tool_call_request(calls.clone());
            messages.push(tool_call_msg.clone());
            updated_history.push(tool_call_msg);

            // Run middleware before executing tool calls
            match Self::run_middleware(middleware, &updated_history).await {
                MiddlewareAction::Continue => {
                    let tools_ref = tools
                        .as_ref()
                        .expect("Received tool call but no tools were specified");

                    let tool_call_msgs = Self::handle_tool_calls(tools_ref, &calls).await?;
                    for m in tool_call_msgs.into_iter() {
                        messages.push(m.clone());
                        updated_history.push(m);
                    }
                }
                MiddlewareAction::StopWithError(err) => {
                    return Err(err);
                }
                MiddlewareAction::Reject(rejection_msgs) => {
                    for m in rejection_msgs {
                        messages.push(m.clone());
                        updated_history.push(m);
                    }
                }
            }

            // Provide the results of the tool calls (or rejections) back
            // to the chat
            resp = completion(&updated_history, tools, api_hostname, api_key, model).await?;
        }

        if let Some(msg) = resp["choices"][0]["message"]["content"].as_str() {
            messages.push(Message::new(Role::Assistant, msg));
        } else {
            return Err(anyhow!("No message received. Resp:\n{}", resp));
        }

        Ok(messages)
    }

    /// Runs the next turn in chat by passing a transcript to the LLM and
    /// the next response is streamed via the transmitter channel
    /// `tx`. Also returns the next messages so they can be processed
    /// further. Can return multiple messages when there are tool calls.
    async fn chat_stream(
        tx: mpsc::UnboundedSender<String>,
        tools: &Option<Vec<BoxedToolCall>>,
        transcript: &Transcript,
        api_hostname: &str,
        api_key: &str,
        model: &str,
        middleware: &[Box<dyn ToolCallMiddleware>],
    ) -> Result<Vec<Message>, Error> {
        let history = transcript.messages();
        let mut updated_history = history.to_owned();
        let mut messages = Vec::new();

        let mut resp =
            completion_stream(tx.clone(), &history, tools, api_hostname, api_key, model).await?;

        // Tool calls need to be handled for the chat to proceed
        while let Some(tool_calls) = resp["choices"][0]["message"]["tool_calls"].as_array() {
            if tool_calls.is_empty() {
                break;
            }

            // Parse tool calls into typed structs immediately so
            // middleware and tool handling always operate on `Message`
            // and `FunctionCall` types, not raw JSON `Value`.
            let calls = Self::parse_tool_calls(tool_calls);

            // Build the tool call request message and add to transcript
            // before middleware runs, so middleware sees the full picture.
            let tool_call_msg = Message::new_tool_call_request(calls.clone());
            messages.push(tool_call_msg.clone());
            updated_history.push(tool_call_msg);

            // Run middleware before executing tool calls
            match Self::run_middleware(middleware, &updated_history).await {
                MiddlewareAction::Continue => {
                    let tools_ref = tools
                        .as_ref()
                        .expect("Received tool call but no tools were specified");

                    // TODO: Update this to be streaming
                    let tool_call_msgs = Self::handle_tool_calls(tools_ref, &calls).await?;
                    for m in tool_call_msgs.into_iter() {
                        messages.push(m.clone());
                        updated_history.push(m);
                    }
                }
                MiddlewareAction::StopWithError(err) => {
                    return Err(err);
                }
                MiddlewareAction::Reject(rejection_msgs) => {
                    for m in rejection_msgs {
                        messages.push(m.clone());
                        updated_history.push(m);
                    }
                }
            }

            // Provide the results of the tool calls (or rejections) back
            // to the chat
            resp = completion_stream(
                tx.clone(),
                &updated_history,
                tools,
                api_hostname,
                api_key,
                model,
            )
            .await?;
        }

        if let Some(msg) = resp["choices"][0]["message"]["content"].as_str() {
            messages.push(Message::new(Role::Assistant, msg));
        } else {
            bail!("No message received. Resp:\n{}", resp);
        }

        Ok(messages)
    }
}

#[derive(Default)]
pub struct ChatBuilder {
    api_hostname: String,
    api_key: String,
    model: String,
    db: Option<Connection>,
    session_id: Option<String>,
    tools: Option<Vec<BoxedToolCall>>,
    transcript: Transcript,
    streaming: bool,
    tx: Option<mpsc::UnboundedSender<String>>,
    tags: Option<Vec<String>>,
    middleware: Vec<Box<dyn ToolCallMiddleware>>,
}

impl ChatBuilder {
    pub fn new(api_hostname: &str, api_key: &str, model: &str) -> Self {
        let transcript = Transcript::new();

        Self {
            api_hostname: api_hostname.to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            transcript,
            db: None,
            session_id: None,
            tx: None,
            tools: None,
            streaming: false,
            tags: None,
            middleware: Vec::new(),
        }
    }

    pub fn build(self) -> Chat {
        Chat {
            api_hostname: self.api_hostname,
            api_key: self.api_key,
            model: self.model,
            db: self.db,
            streaming: self.streaming,
            tx: self.tx,
            tools: self.tools,
            transcript: self.transcript,
            session_id: self.session_id,
            tags: self.tags,
            middleware: self.middleware,
        }
    }

    pub fn database(
        mut self,
        db: &Connection,
        session_id: Option<&str>,
        tags: Option<Vec<String>>,
    ) -> Self {
        // Always sets a session ID, tags, and DB connection
        if let Some(id) = session_id {
            self.session_id = Some(id.to_string());
        } else {
            self.session_id = Some(Uuid::new_v4().to_string());
        }
        if let Some(tags) = tags {
            self.tags = Some(tags);
        } else {
            self.tags = Some(Vec::new())
        }
        self.db = Some(db.clone());
        self
    }

    pub fn transcript(mut self, messages: Vec<Message>) -> Self {
        self.transcript = Transcript::new_with_messages(messages);
        self
    }

    pub fn streaming(mut self, transmitter: mpsc::UnboundedSender<String>) -> Self {
        // Set the streaming flag and the transmitter
        self.streaming = true;
        self.tx = Some(transmitter);
        self
    }

    pub fn tools(mut self, tools: Vec<BoxedToolCall>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Add tool call middleware.
    ///
    /// Middleware runs before each batch of tool calls in order of
    /// registration. If any middleware returns `StopWithError`, the
    /// remaining middleware is skipped and the chat returns the error.
    pub fn middleware(mut self, middleware: Vec<Box<dyn ToolCallMiddleware>>) -> Self {
        self.middleware = middleware;
        self
    }

    /// Add skill management tools to the chat.
    ///
    /// Takes an `Arc<RwLock<SkillRegistry>>` so that `save_skill` can
    /// reload the registry through the same shared handle the rest of
    /// the system uses.
    ///
    /// Adds the following tool calls:
    /// - `work_on_skill`: Prepare a skill for editing in the workspace
    /// - `save_skill`: Save a skill from workspace back to global directory
    ///
    /// If the registry has skills loaded, also adds:
    /// - `list_skills`: List all available skills
    /// - `search_skills`: Search for skills by keyword
    /// - `load_skill`: Load the full content of a skill
    /// - `read_skill_file`: Read files from within a skill's directory
    ///
    /// Requires `storage_path` and `session_id` for workspace tools.
    /// The workspace tools are always added even when no skills are
    /// registered, so the agent can create new skills.
    pub fn skills(
        mut self,
        registry_handle: Arc<RwLock<SkillRegistry>>,
        storage_path: &str,
        session_id: &str,
    ) -> Self {
        let registry = match registry_handle.read() {
            Ok(guard) => guard.clone(),
            Err(_) => return self,
        };

        let skills_dir = registry.dir_path().to_string_lossy().to_string();
        let count = registry.count();

        let mut skill_tools: Vec<BoxedToolCall> = vec![
            Box::new(WorkOnSkillTool::new(&skills_dir, storage_path, session_id)),
            Box::new(SaveSkillTool::new(
                &skills_dir,
                storage_path,
                session_id,
                registry_handle,
            )),
        ];

        if count > 0 {
            skill_tools.push(Box::new(ListSkillsTool::new(registry.clone())));
            skill_tools.push(Box::new(SearchSkillsTool::new(registry.clone())));
            skill_tools.push(Box::new(LoadSkillTool::new(registry.clone())));
            skill_tools.push(Box::new(ReadSkillFileTool::new(registry)));
        }

        // Merge with existing tools if any
        match self.tools {
            Some(mut existing) => {
                existing.extend(skill_tools);
                self.tools = Some(existing);
            }
            None => {
                self.tools = Some(skill_tools);
            }
        }

        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat::InfiniteLoopDetector;
    use crate::openai::{Message, Role};
    use tokio::sync::mpsc;

    #[test]
    fn test_builder_new() {
        let builder = ChatBuilder::new("https://api.example.com", "test-key", "gpt-4");

        assert_eq!(builder.api_hostname, "https://api.example.com");
        assert_eq!(builder.api_key, "test-key");
        assert_eq!(builder.model, "gpt-4");
        assert!(builder.db.is_none());
        assert_eq!(builder.session_id, None);
        assert!(builder.tools.is_none());
        assert!(!builder.streaming);
        assert!(builder.tx.is_none());
    }

    #[test]
    fn test_builder_build() {
        let builder = ChatBuilder::new("https://api.example.com", "test-key", "gpt-4");
        let chat = builder.build();

        assert_eq!(chat.api_hostname, "https://api.example.com");
        assert_eq!(chat.api_key, "test-key");
        assert_eq!(chat.model, "gpt-4");
        assert!(chat.db.is_none());
        assert_eq!(chat.session_id, None);
        assert!(chat.tools.is_none());
        assert!(!chat.streaming);
        assert!(chat.tx.is_none());
    }

    #[test]
    fn test_builder_transcript() {
        let messages = vec![Message::new(Role::User, "Hello")];

        let builder =
            ChatBuilder::new("https://api.example.com", "test-key", "gpt-4").transcript(messages);

        assert_eq!(builder.transcript.messages().len(), 1);
    }

    #[test]
    fn test_builder_streaming() {
        let (tx, _rx) = mpsc::unbounded_channel();

        let builder =
            ChatBuilder::new("https://api.example.com", "test-key", "gpt-4").streaming(tx);

        assert!(builder.streaming);
        assert!(builder.tx.is_some());

        let chat = builder.build();
        assert!(chat.streaming);
        assert!(chat.tx.is_some());
    }

    #[test]
    fn test_builder_tools() {
        // Create a mock tool for testing
        #[derive(serde::Serialize)]
        struct MockTool;
        #[async_trait::async_trait]
        impl crate::openai::ToolCall for MockTool {
            async fn call(&self, _args: &str) -> anyhow::Result<String> {
                Ok("mock result".to_string())
            }
            fn function_name(&self) -> String {
                "mock_tool".to_string()
            }
        }

        let tools = vec![Box::new(MockTool) as crate::openai::BoxedToolCall];
        let builder = ChatBuilder::new("https://api.example.com", "test-key", "gpt-4").tools(tools);

        assert!(builder.tools.is_some());
        assert_eq!(builder.tools.as_ref().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_builder_skills() {
        use crate::ai::skills::SkillRegistry;
        use std::fs;
        use tempfile::TempDir;

        // Create a temp directory with a test skill
        let temp = TempDir::new().unwrap();
        let skill_dir = temp.path().join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let skill_content = r#"---
name: test-skill
description: A test skill for unit testing.
---

Test skill body content."#;
        fs::write(skill_dir.join("SKILL.md"), skill_content).unwrap();

        let registry = SkillRegistry::new(temp.path()).await.unwrap();
        assert_eq!(registry.count(), 1);

        // Test with empty tools
        let storage_path = temp.path().to_string_lossy().to_string();
        let handle = Arc::new(RwLock::new(registry.clone()));
        let builder = ChatBuilder::new("https://api.example.com", "test-key", "gpt-4").skills(
            handle,
            &storage_path,
            "test-session",
        );
        assert!(builder.tools.is_some());
        let tools = builder.tools.unwrap();
        // Should have 6 skill tools: list, search, load, read_file, work_on_skill, save_skill
        assert_eq!(tools.len(), 6);
    }

    #[tokio::test]
    async fn test_builder_skills_merges_with_existing_tools() {
        use crate::ai::skills::SkillRegistry;
        use std::fs;
        use tempfile::TempDir;

        // Create a temp directory with a test skill
        let temp = TempDir::new().unwrap();
        let skill_dir = temp.path().join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let skill_content = r#"---
name: test-skill
description: A test skill for unit testing.
---

Test skill body content."#;
        fs::write(skill_dir.join("SKILL.md"), skill_content).unwrap();

        let registry = SkillRegistry::new(temp.path()).await.unwrap();
        assert_eq!(registry.count(), 1);

        // Create a mock tool
        #[derive(serde::Serialize)]
        struct MockTool;
        #[async_trait::async_trait]
        impl crate::openai::ToolCall for MockTool {
            async fn call(&self, _args: &str) -> anyhow::Result<String> {
                Ok("mock result".to_string())
            }
            fn function_name(&self) -> String {
                "mock_tool".to_string()
            }
        }

        // Test that skills merge with existing tools
        let storage_path = temp.path().to_string_lossy().to_string();
        let handle = Arc::new(RwLock::new(registry));
        let builder = ChatBuilder::new("https://api.example.com", "test-key", "gpt-4")
            .tools(vec![Box::new(MockTool) as crate::openai::BoxedToolCall])
            .skills(handle, &storage_path, "test-session");

        let tools = builder.tools.unwrap();
        // Should have 1 mock tool + 6 skill tools
        assert_eq!(tools.len(), 7);
    }

    #[tokio::test]
    async fn test_builder_skills_empty_registry() {
        use tempfile::TempDir;

        // Create an empty temp directory
        let temp = TempDir::new().unwrap();

        let registry = SkillRegistry::new(temp.path()).await.unwrap();
        let storage_path = temp.path().to_string_lossy().to_string();
        let handle = Arc::new(RwLock::new(registry));
        let builder = ChatBuilder::new("https://api.example.com", "test-key", "gpt-4").skills(
            handle,
            &storage_path,
            "test-session",
        );
        // With empty registry, should still add workspace tools
        // (work_on_skill, save_skill) so the agent can create new skills
        assert!(builder.tools.is_some());
        assert_eq!(builder.tools.unwrap().len(), 2);
    }

    #[test]
    fn test_builder_chaining() {
        let messages = vec![Message::new(Role::User, "Hello")];

        let (tx, _rx) = mpsc::unbounded_channel();

        #[derive(serde::Serialize)]
        struct MockTool;
        #[async_trait::async_trait]
        impl crate::openai::ToolCall for MockTool {
            async fn call(&self, _args: &str) -> anyhow::Result<String> {
                Ok("mock result".to_string())
            }
            fn function_name(&self) -> String {
                "mock_tool".to_string()
            }
        }

        let tools = vec![Box::new(MockTool) as crate::openai::BoxedToolCall];

        let chat = ChatBuilder::new("https://api.example.com", "test-key", "gpt-4")
            .transcript(messages)
            .streaming(tx)
            .tools(tools)
            .build();

        assert_eq!(chat.api_hostname, "https://api.example.com");
        assert_eq!(chat.api_key, "test-key");
        assert_eq!(chat.model, "gpt-4");
        assert_eq!(chat.session_id, None);
        assert_eq!(chat.transcript.messages().len(), 1);
        assert!(chat.streaming);
        assert!(chat.tools.is_some());
    }

    #[test]
    fn test_builder_default_empty_transcript() {
        let builder = ChatBuilder::new("https://api.example.com", "test-key", "gpt-4");
        assert_eq!(builder.transcript.messages().len(), 0);

        let chat = builder.build();
        assert_eq!(chat.transcript.messages().len(), 0);
    }

    #[test]
    fn test_builder_default_streaming_disabled() {
        let builder = ChatBuilder::new("https://api.example.com", "test-key", "gpt-4");
        assert!(!builder.streaming);
        assert!(builder.tx.is_none());

        let chat = builder.build();
        assert!(!chat.streaming);
        assert!(chat.tx.is_none());
    }

    #[tokio::test]
    async fn test_builder_database() {
        let db = tokio_rusqlite::Connection::open_in_memory().await.unwrap();

        let builder = ChatBuilder::new("https://api.example.com", "test-key", "gpt-4")
            .database(&db, None, None);

        // db, session_id, tags should always be set together and
        // should never by None
        assert!(builder.db.is_some());
        assert!(builder.session_id.is_some());
        assert!(builder.tags.is_some());

        let chat = builder.build();
        assert!(chat.db.is_some());
        assert!(chat.session_id.is_some());
        assert!(chat.tags.is_some());
    }

    #[tokio::test]
    async fn test_builder_database_with_existing_session_id() {
        let db = tokio_rusqlite::Connection::open_in_memory().await.unwrap();

        let builder = ChatBuilder::new("https://api.example.com", "test-key", "gpt-4").database(
            &db,
            Some("existing-session-id"),
            None,
        );

        // db and session_id should always be set together
        assert!(builder.db.is_some());
        assert_eq!(builder.session_id, Some("existing-session-id".to_string()));

        let chat = builder.build();
        assert!(chat.db.is_some());
        assert_eq!(chat.session_id, Some("existing-session-id".to_string()));
    }

    // Tests for Chat::chat method (tested through next_msg)
    #[tokio::test]
    async fn test_chat_basic_response() {
        let mut server = mockito::Server::new_async().await;

        // Mock response for a basic chat completion (no tools)
        let response_body = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1694268190,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello! How can I help you today?"
                },
                "finish_reason": "stop"
            }]
        }"#;

        let _mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(response_body)
            .create();

        // No tools provided - this should work fine when there are no tool calls
        let url = server.url();
        let mut chat = ChatBuilder::new(&url, "test-key", "gpt-4").build();

        let msg = Message::new(Role::User, "Hi");
        let result = chat.next_msg(msg).await;

        assert!(result.is_ok());
        let messages = result.unwrap();
        // Should return the assistant's response
        assert_eq!(messages.len(), 1);
        let content = messages[0].content.as_ref().expect("Should have content");
        assert_eq!(content, "Hello! How can I help you today?");
    }

    #[tokio::test]
    async fn test_chat_with_tool_calls() {
        let mut server = mockito::Server::new_async().await;

        // First response: model makes a tool call
        let tool_call_response = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1694268190,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_abc123",
                        "type": "function",
                        "function": {
                            "name": "mock_tool",
                            "arguments": "{\"query\":\"test\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }"#;

        // Second response: model responds after tool result
        let final_response = r#"{
            "id": "chatcmpl-124",
            "object": "chat.completion",
            "created": 1694268191,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "I found some results for your query."
                },
                "finish_reason": "stop"
            }]
        }"#;

        // Create two mocks - first for tool call, second for final response
        let mock1 = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(tool_call_response)
            .create();

        let mock2 = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(final_response)
            .create();

        // Create a mock tool that will be called when the model requests it
        #[derive(serde::Serialize)]
        struct MockTool;
        #[async_trait::async_trait]
        impl crate::openai::ToolCall for MockTool {
            async fn call(&self, _args: &str) -> anyhow::Result<String> {
                Ok("mock result".to_string())
            }
            fn function_name(&self) -> String {
                "mock_tool".to_string()
            }
        }

        let url = server.url();
        let tools = vec![Box::new(MockTool) as crate::openai::BoxedToolCall];
        let mut chat = ChatBuilder::new(&url, "test-key", "gpt-4")
            .tools(tools)
            .build();

        let msg = Message::new(Role::User, "Search for test");
        let result = chat.next_msg(msg).await;

        mock1.assert();
        mock2.assert();

        assert!(result.is_ok());
        let messages = result.unwrap();
        // Should return 3 messages:
        // 1. Tool call request
        // 2. Tool call response
        // 3. Assistant's final content
        assert_eq!(messages.len(), 3);
    }

    // Tests for Chat::chat_stream (tested through next_msg with streaming enabled)
    #[tokio::test]
    async fn test_chat_stream_basic() {
        let mut server = mockito::Server::new_async().await;

        // SSE response with content chunks
        let sse_response = r#"data: {"id":"chunk1","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"id":"chunk2","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"content":" World"},"finish_reason":null}]}

data: {"id":"chunk3","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"content":"!"},"finish_reason":"stop"}]}

data: [DONE]

"#;

        let _mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(sse_response)
            .create();

        let url = server.url();
        let (tx, mut rx) = mpsc::unbounded_channel();

        // No tools provided - streaming should work without tools when no tool calls needed
        let mut chat = ChatBuilder::new(&url, "test-key", "gpt-4")
            .streaming(tx)
            .build();

        let msg = Message::new(Role::User, "Say hello");
        let result = chat.next_msg(msg).await;

        assert!(result.is_ok());
        let messages = result.unwrap();

        // Should return the assistant's response (assembled from streamed chunks)
        // Note: The last chunk with finish_reason="stop" doesn't add content,
        // so only "Hello World" (not the "!") is assembled
        assert_eq!(messages.len(), 1);
        let content = messages[0].content.as_ref().expect("Should have content");
        assert_eq!(content, "Hello World");

        // Verify the raw chunks were also sent to the streaming channel
        let mut chunk_count = 0;
        while rx.try_recv().is_ok() {
            chunk_count += 1;
        }
        assert!(
            chunk_count >= 3,
            "Expected at least 3 chunks, got {}",
            chunk_count
        );
    }

    #[tokio::test]
    async fn test_chat_stream_with_tool_calls() {
        let mut server = mockito::Server::new_async().await;

        // First response: streaming tool call chunks
        let sse_tool_call = r#"data: {"id":"chunk1","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"tool_calls":[{"id":"call_abc123","index":0,"function":{"name":"mock_tool","arguments":"{\"query\":"},"type":"function"}]},"finish_reason":null}]}

data: {"id":"chunk2","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"test\"}"}}]},"finish_reason":null}]}

data: {"id":"chunk3","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":""}}]},"finish_reason":"stop"}]}

data: [DONE]

"#;

        // Second response: final content after tool result
        let sse_final = r#"data: {"id":"chunk4","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"content":"Found results!"},"finish_reason":"stop"}]}

data: [DONE]

"#;

        // Create two mocks - first for tool call stream, second for final response
        let mock1 = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(sse_tool_call)
            .create();

        let mock2 = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(sse_final)
            .create();

        // Create a mock tool that will be called when the model requests it
        #[derive(serde::Serialize)]
        struct MockTool;
        #[async_trait::async_trait]
        impl crate::openai::ToolCall for MockTool {
            async fn call(&self, _args: &str) -> anyhow::Result<String> {
                Ok("mock result".to_string())
            }
            fn function_name(&self) -> String {
                "mock_tool".to_string()
            }
        }

        let url = server.url();
        let (tx, _rx) = mpsc::unbounded_channel();
        let tools = vec![Box::new(MockTool) as crate::openai::BoxedToolCall];

        let mut chat = ChatBuilder::new(&url, "test-key", "gpt-4")
            .streaming(tx)
            .tools(tools)
            .build();

        let msg = Message::new(Role::User, "Search for test");
        let result = chat.next_msg(msg).await;

        mock1.assert();
        mock2.assert();

        assert!(result.is_ok());
        let messages = result.unwrap();
        // Should return 3 messages:
        // 1. Tool call request
        // 2. Tool call response
        // 3. Assistant's final content
        assert_eq!(messages.len(), 3);
    }

    #[tokio::test]
    async fn test_chat_with_recoverable_tool_error() {
        let mut server = mockito::Server::new_async().await;

        // First response: model makes a tool call to a tool that will
        // fail with a recoverable error
        let tool_call_response = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1694268190,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_recoverable",
                        "type": "function",
                        "function": {
                            "name": "failing_tool",
                            "arguments": "{}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }"#;

        // Second response: model responds after receiving the
        // recoverable error message
        let final_response = r#"{
            "id": "chatcmpl-124",
            "object": "chat.completion",
            "created": 1694268191,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "The tool failed with a recoverable error. Let me try a different approach."
                },
                "finish_reason": "stop"
            }]
        }"#;

        let mock1 = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(tool_call_response)
            .create();

        let mock2 = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(final_response)
            .create();

        // Create a mock tool that returns a RecoverableToolError
        #[derive(serde::Serialize)]
        struct FailingMockTool;
        #[async_trait::async_trait]
        impl crate::openai::ToolCall for FailingMockTool {
            async fn call(&self, _args: &str) -> anyhow::Result<String> {
                Err(crate::openai::RecoverableToolError::new(
                    "Website is temporarily unavailable (HTTP 503). Try again later.",
                )
                .into())
            }
            fn function_name(&self) -> String {
                "failing_tool".to_string()
            }
        }

        let url = server.url();
        let tools = vec![Box::new(FailingMockTool) as crate::openai::BoxedToolCall];
        let mut chat = ChatBuilder::new(&url, "test-key", "gpt-4")
            .tools(tools)
            .build();

        let msg = Message::new(Role::User, "Fetch the website");
        let result = chat.next_msg(msg).await;

        mock1.assert();
        mock2.assert();

        assert!(result.is_ok());
        let messages = result.unwrap();
        // Should return 3 messages:
        // 1. Tool call request
        // 2. Tool call response (containing the recoverable error message)
        // 3. Assistant's final content
        assert_eq!(messages.len(), 3);

        // The tool call response should contain the recoverable error
        // message, not crash the chat
        let tool_response = &messages[1];
        assert_eq!(*tool_response.role(), crate::openai::Role::Tool);
        assert_eq!(
            tool_response.content.as_ref().unwrap(),
            "Website is temporarily unavailable (HTTP 503). Try again later."
        );
    }

    // --- Middleware integration tests ---

    /// A mock middleware that always stops with an error.
    struct StopMiddleware;
    #[async_trait::async_trait]
    impl ToolCallMiddleware for StopMiddleware {
        async fn before_tool_calls(
            &self,
            _transcript: &[Message],
        ) -> MiddlewareAction {
            MiddlewareAction::StopWithError(anyhow!(
                "Middleware stopped"
            ))
        }
    }

    /// A mock middleware that always continues.
    struct ContinueMiddleware;
    #[async_trait::async_trait]
    impl ToolCallMiddleware for ContinueMiddleware {
        async fn before_tool_calls(
            &self,
            _transcript: &[Message],
        ) -> MiddlewareAction {
            MiddlewareAction::Continue
        }
    }

    #[tokio::test]
    async fn test_builder_middleware() {
        let mw: Vec<Box<dyn ToolCallMiddleware>> =
            vec![Box::new(StopMiddleware)];

        let builder =
            ChatBuilder::new("https://api.example.com", "test-key", "gpt-4")
                .middleware(mw);

        assert_eq!(builder.middleware.len(), 1);
    }

    #[tokio::test]
    async fn test_middleware_continues_does_not_interfere() {
        let mut server = mockito::Server::new_async().await;

        let response_body = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1694268190,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello!"
                },
                "finish_reason": "stop"
            }]
        }"#;

        let _mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(response_body)
            .create();

        let url = server.url();
        let mut chat = ChatBuilder::new(&url, "test-key", "gpt-4")
            .middleware(vec![Box::new(ContinueMiddleware)])
            .build();

        let msg = Message::new(Role::User, "Hi");
        let result = chat.next_msg(msg).await;

        assert!(result.is_ok());
        let messages = result.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].content.as_ref().unwrap(),
            "Hello!"
        );
    }

    #[tokio::test]
    async fn test_middleware_stops_tool_calls() {
        let mut server = mockito::Server::new_async().await;

        let tool_call_response = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1694268190,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_abc123",
                        "type": "function",
                        "function": {
                            "name": "mock_tool",
                            "arguments": "{\"query\":\"test\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }"#;

        let _mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(tool_call_response)
            .create();

        #[derive(serde::Serialize)]
        struct MockTool;
        #[async_trait::async_trait]
        impl crate::openai::ToolCall for MockTool {
            async fn call(&self, _args: &str) -> anyhow::Result<String> {
                Ok("mock result".to_string())
            }
            fn function_name(&self) -> String {
                "mock_tool".to_string()
            }
        }

        let url = server.url();
        let tools = vec![Box::new(MockTool) as crate::openai::BoxedToolCall];
        let mut chat = ChatBuilder::new(&url, "test-key", "gpt-4")
            .tools(tools)
            .middleware(vec![Box::new(StopMiddleware)])
            .build();

        let msg = Message::new(Role::User, "Search");
        let result = chat.next_msg(msg).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Middleware stopped"),
            "Expected 'Middleware stopped', got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_middleware_chain_stops_at_first_error() {
        let mut server = mockito::Server::new_async().await;

        let tool_call_response = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1694268190,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_abc123",
                        "type": "function",
                        "function": {
                            "name": "mock_tool",
                            "arguments": "{}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }"#;

        let _mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(tool_call_response)
            .create();

        #[derive(serde::Serialize)]
        struct MockTool;
        #[async_trait::async_trait]
        impl crate::openai::ToolCall for MockTool {
            async fn call(&self, _args: &str) -> anyhow::Result<String> {
                Ok("result".to_string())
            }
            fn function_name(&self) -> String {
                "mock_tool".to_string()
            }
        }

        let url = server.url();
        let tools = vec![Box::new(MockTool) as crate::openai::BoxedToolCall];
        let mut chat = ChatBuilder::new(&url, "test-key", "gpt-4")
            .tools(tools)
            .middleware(vec![
                Box::new(ContinueMiddleware),
                Box::new(StopMiddleware),
            ])
            .build();

        let msg = Message::new(Role::User, "Search");
        let result = chat.next_msg(msg).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Middleware stopped"),
            "Expected 'Middleware stopped', got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_infinite_loop_detector_integration() {
        let mut server = mockito::Server::new_async().await;

        let tool_call_response = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1694268190,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_abc123",
                        "type": "function",
                        "function": {
                            "name": "mock_tool",
                            "arguments": "{\"query\":\"test\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }"#;

        // After rejection, the model responds with a normal message
        let final_response = r#"{
            "id": "chatcmpl-124",
            "object": "chat.completion",
            "created": 1694268191,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "I see the tool is looping. Let me try a different approach."
                },
                "finish_reason": "stop"
            }]
        }"#;

        let _mock1 = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(tool_call_response)
            .create();
        let _mock2 = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(tool_call_response)
            .create();
        let _mock3 = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(tool_call_response)
            .create();
        let _mock4 = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(final_response)
            .create();

        #[derive(serde::Serialize)]
        struct MockTool;
        #[async_trait::async_trait]
        impl crate::openai::ToolCall for MockTool {
            async fn call(&self, _args: &str) -> anyhow::Result<String> {
                Ok("same result every time".to_string())
            }
            fn function_name(&self) -> String {
                "mock_tool".to_string()
            }
        }

        let url = server.url();
        let tools = vec![Box::new(MockTool) as crate::openai::BoxedToolCall];
        let detector = InfiniteLoopDetector::new(3);
        let mut chat = ChatBuilder::new(&url, "test-key", "gpt-4")
            .tools(tools)
            .middleware(vec![Box::new(detector)])
            .build();

        let msg = Message::new(Role::User, "Keep searching");
        let result = chat.next_msg(msg).await;

        assert!(result.is_ok(), "Expected OK (model recovers from rejection), got: {:?}", result.err());
        let messages = result.unwrap();
        // Messages should include:
        // 1. Tool call request (iter 1)
        // 2. Tool call response (iter 1)
        // 3. Tool call request (iter 2)
        // 4. Tool call response (iter 2)
        // 5. Tool call request (iter 3)
        // 6. Tool call rejection response
        // 7. Final assistant content
        assert_eq!(messages.len(), 7);
        // The rejection response should contain the loop detection message
        let rejection = &messages[5];
        assert_eq!(*rejection.role(), crate::openai::Role::Tool);
        assert!(
            rejection.content.as_ref().unwrap().contains("Tool call rejected due to infinite loop"),
            "Expected rejection message, got: {:?}",
            rejection.content
        );
    }

    #[tokio::test]
    async fn test_middleware_streaming_stops_tool_calls() {
        let mut server = mockito::Server::new_async().await;

        let sse_tool_call = r#"data: {"id":"chunk1","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"tool_calls":[{"id":"call_abc123","index":0,"function":{"name":"mock_tool","arguments":"{}"},"type":"function"}]},"finish_reason":null}]}

data: {"id":"chunk2","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":""}}]},"finish_reason":"stop"}]}

data: [DONE]

"#;

        let _mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(sse_tool_call)
            .create();

        #[derive(serde::Serialize)]
        struct MockTool;
        #[async_trait::async_trait]
        impl crate::openai::ToolCall for MockTool {
            async fn call(&self, _args: &str) -> anyhow::Result<String> {
                Ok("result".to_string())
            }
            fn function_name(&self) -> String {
                "mock_tool".to_string()
            }
        }

        let url = server.url();
        let (tx, _rx) = mpsc::unbounded_channel();
        let tools = vec![Box::new(MockTool) as crate::openai::BoxedToolCall];
        let mut chat = ChatBuilder::new(&url, "test-key", "gpt-4")
            .streaming(tx)
            .tools(tools)
            .middleware(vec![Box::new(StopMiddleware)])
            .build();

        let msg = Message::new(Role::User, "Search");
        let result = chat.next_msg(msg).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Middleware stopped"),
            "Expected 'Middleware stopped', got: {}",
            err
        );
    }
}
