use anyhow::{Error, Result, anyhow, bail};
use futures_util::future::try_join_all;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_rusqlite::Connection;
use uuid::Uuid;

use crate::openai::{
    BoxedToolCall, FunctionCall, FunctionCallFn, Message, Role, completion, completion_stream
};
use super::models::Transcript;
use super::db::{insert_chat_message, get_or_create_session};

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
    // TODO: Skills
    // TODO: MCP
    // TODO: Permissions
}

impl Chat {
    async fn handle_tool_call(
        tools: &Vec<BoxedToolCall>,
        tool_call: &Value,
    ) -> Result<Vec<Message>, Error> {
        let tool_call_id = &tool_call["id"]
            .as_str()
            .ok_or(anyhow!("Tool call missing ID: {}", tool_call))?;
        let tool_call_function = &tool_call["function"];
        let tool_call_args = tool_call_function["arguments"]
            .as_str()
            .ok_or(anyhow!("Tool call missing arguments: {}", tool_call))?;
        let tool_call_name = tool_call_function["name"]
            .as_str()
            .ok_or(anyhow!("Tool call missing name: {}", tool_call))?;

        tracing::debug!(
            "\nTool call: {}\nargs: {}",
            &tool_call_name,
            &tool_call_args
        );

        // Call the tool and get the next completion from the result
        let tool_call_result = tools
            .iter()
            .find(|i| *i.function_name() == *tool_call_name)
            .ok_or(anyhow!(
                "Received tool call that doesn't exist: {}",
                tool_call_name
            ))?
            .call(tool_call_args)
            .await?;

        let tool_call_request = vec![FunctionCall {
            function: FunctionCallFn {
                arguments: tool_call_args.to_string(),
                name: tool_call_name.to_string(),
            },
            id: tool_call_id.to_string(),
            r#type: String::from("function"),
        }];
        let results = vec![
            Message::new_tool_call_request(tool_call_request),
            Message::new_tool_call_response(&tool_call_result, tool_call_id),
        ];

        Ok(results)
    }

    async fn handle_tool_calls(
        tools: &Vec<BoxedToolCall>,
        tool_calls: &[Value],
    ) -> Result<Vec<Message>, Error> {
        // Run each tool call concurrently and return them in order. I'm
        // not sure if ordering really matters for OpenAI compatible API
        // implementations, but better to be safe. This could also be
        // done using a `futures::stream` and `FutureUnordered` which
        // would be more efficient as it runs on the same thread, but that
        // causes lifetime issues that I don't understand how to get
        // around.
        let futures = tool_calls.iter().map(|call| Self::handle_tool_call(tools, call));
        // Flatten the results to match what the API is expecting.
        let results = try_join_all(futures).await?.into_iter().flatten().collect();
        Ok(results)
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
                tx.clone(), &self.tools, &self.transcript, &self.api_hostname, &self.api_key, &self.model
            ).await?
        } else {
            Self::chat(
                &self.tools, &self.transcript, &self.api_hostname, &self.api_key, &self.model
            ).await?
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
            get_or_create_session(db, session_id, tags).await?;

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

    /// Runs the next turn in chat by passing a transcript to the LLM for
    /// the next response. Can return multiple messages when there are
    /// tool calls.
    async fn chat(
        tools: &Option<Vec<BoxedToolCall>>,
        transcript: &Transcript,
        api_hostname: &str,
        api_key: &str,
        model: &str,
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

            let tools_ref = tools
                .as_ref()
                .expect("Received tool call but no tools were specified");

            let tool_call_msgs = Self::handle_tool_calls(tools_ref, tool_calls).await?;
            for m in tool_call_msgs.into_iter() {
                messages.push(m.clone());
                updated_history.push(m);
            }

            // Provide the results of the tool calls back to the chat
            resp = completion(&updated_history, tools, api_hostname, api_key, model).await?;
        }

        if let Some(msg) = resp["choices"][0]["message"]["content"].as_str() {
            messages.push(Message::new(Role::Assistant, msg));
        } else {
            panic!("No message received. Resp:\n\n {}", resp);
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
            let tools_ref = tools
                .as_ref()
                .expect("Received tool call but no tools were specified");

            // TODO: Update this to be streaming
            let tool_call_msgs = Self::handle_tool_calls(tools_ref, tool_calls).await?;
            for m in tool_call_msgs.into_iter() {
                messages.push(m.clone());
                updated_history.push(m);
            }

            // Provide the results of the tool calls back to the chat
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
            bail!("No message received. Resp:\n\n {}", resp);
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
        }
    }

    pub fn database(mut self, db: &Connection, session_id: Option<&str>, tags: Option<Vec<String>>) -> Self {
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

    pub fn skills(self) -> Self {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let messages = vec![
            Message::new(Role::User, "Hello")
        ];

        let builder = ChatBuilder::new("https://api.example.com", "test-key", "gpt-4")
            .transcript(messages);

        assert_eq!(builder.transcript.messages().len(), 1);
    }

    #[test]
    fn test_builder_streaming() {
        let (tx, _rx) = mpsc::unbounded_channel();

        let builder = ChatBuilder::new("https://api.example.com", "test-key", "gpt-4")
            .streaming(tx);

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
        let builder = ChatBuilder::new("https://api.example.com", "test-key", "gpt-4")
            .tools(tools);

        assert!(builder.tools.is_some());
        assert_eq!(builder.tools.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_builder_chaining() {
        let messages = vec![
            Message::new(Role::User, "Hello")
        ];

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

        let builder = ChatBuilder::new("https://api.example.com", "test-key", "gpt-4")
            .database(&db, Some("existing-session-id"), None);

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
        let mut chat = ChatBuilder::new(&url, "test-key", "gpt-4")
            .build();

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
        assert!(chunk_count >= 3, "Expected at least 3 chunks, got {}", chunk_count);
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
}
