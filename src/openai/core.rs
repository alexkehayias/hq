use std::{collections::HashMap, time::Duration};
use tokio::sync::mpsc;

use anyhow::{Error, Result};
use async_trait::async_trait;
use erased_serde;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum Role {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "assistant")]
    Assistant,
    #[serde(rename = "user")]
    User,
    #[serde(rename = "tool")]
    Tool,
}

// Object {
//     "content": Null,
//     "refusal": Null,
//     "role": String("assistant"),
//     "tool_calls": Array [
//         Object {
//             "function": Object {
//                 "arguments": String("{\"query\":\"books\"}"),
//                 "name": String("search_notes")
//             },
//             "id": String("call_KCg5V0N5E7hHHrUwdefHBfgL"),
//             "type": String("function")
//         }
//     ]
// }
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct FunctionCallFn {
    pub arguments: String,
    pub name: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct FunctionCall {
    pub function: FunctionCallFn,
    pub id: String,
    pub r#type: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Message {
    role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    refusal: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<FunctionCall>>,
}

impl Message {
    pub fn new(role: Role, content: &str) -> Self {
        Message {
            role,
            refusal: None,
            content: Some(content.to_string()),
            tool_call_id: None,
            tool_calls: None,
        }
    }
    pub fn new_tool_call_request(tool_calls: Vec<FunctionCall>) -> Self {
        Message {
            role: Role::Assistant,
            refusal: None,
            content: None,
            tool_call_id: None,
            tool_calls: Some(tool_calls),
        }
    }
    pub fn new_tool_call_response(content: &str, tool_call_id: &str) -> Self {
        Message {
            role: Role::Tool,
            refusal: None,
            content: Some(content.to_string()),
            tool_call_id: Some(tool_call_id.to_string()),
            tool_calls: None,
        }
    }
}

#[derive(Serialize)]
pub struct Property {
    pub r#type: String,
    pub description: String,
}

#[derive(Serialize)]
pub struct Parameters<Props: Serialize> {
    pub r#type: String,
    pub properties: Props,
    pub required: Vec<String>,
    #[serde(rename = "additionalProperties")]
    pub additional_properties: bool,
}

#[derive(Serialize)]
pub struct Function<Props: Serialize> {
    pub name: String,
    pub description: String,
    pub parameters: Parameters<Props>,
    pub strict: bool,
}

#[derive(Serialize)]
pub enum ToolType {
    #[serde(rename = "function")]
    Function,
}

// Ugh. In order to pass around a collection of `Function` structs
// that can be dynamically dispatched using this trait, the trait
// object needs to implement `Serialize` but `serde` is not object
// safe so it will cause a compile error. Instead, we need to use
// `erased_serde` which _is_ object safe and can be used along with
// dynamic dispatch such that the calls to `serde::json` won't
// complain. Another way to do this is to use `typetag` which uses
// `erased_serde` and has somewhat nicer ergonomics. Still, the fact
// that you have to do these things and resolving the error is
// impossible without a good amount of Googling and ChatGPT'ing is
// annoying.
#[async_trait]
pub trait ToolCall: erased_serde::Serialize {
    async fn call(&self, args: &str) -> Result<String, Error>;
    fn function_name(&self) -> String;
}
erased_serde::serialize_trait_object!(ToolCall);

pub type BoxedToolCall = Box<dyn ToolCall + Send + Sync + 'static>;

pub async fn completion(
    messages: &Vec<Message>,
    tools: &Option<Vec<BoxedToolCall>>,
    api_hostname: &str,
    api_key: &str,
    model: &str,
) -> Result<Value, Error> {
    let mut payload = json!({
        "model": model,
        "messages": messages,
    });
    if let Some(tools) = tools {
        payload["tools"] = json!(tools);
    }
    let url = format!("{}/v1/chat/completions", api_hostname.trim_end_matches("/"));
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .timeout(Duration::from_secs(60 * 10))
        .json(&payload)
        .send()
        .await?
        .json()
        .await?;

    Ok(response)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FunctionInitDelta {
    name: String,
    arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FunctionArgsDelta {
    arguments: String,
}

// OpenAI has two different deltas to handle for tool calls that are
// slightly different and hard to notice, one with initial fields and
// then subsequent deltas for streaming the function arguments.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum ToolCallChunk {
    Init {
        id: String,
        index: usize,
        function: FunctionInitDelta,
        r#type: String,
    },
    ArgsDelta {
        index: usize,
        function: FunctionArgsDelta,
        r#type: String,
    },
}

// HACK: Streaming tool calls results in an incomplete struct until
// all the deltas are streamed so we need this "final" version of the
// tool call data even though it's largely a duplicate of the other
// tool call related structs
#[derive(Debug, Serialize, Deserialize)]
struct FunctionFinal {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ToolCallFinal {
    id: String,
    index: usize,
    function: FunctionFinal,
    r#type: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Delta {
    Content { content: String },

    Reasoning { reasoning: String },

    ToolCall { tool_calls: Vec<ToolCallChunk> },

    Stop {},
}

#[derive(Debug, Deserialize)]
struct CompletionChunkChoice {
    #[allow(dead_code)]
    index: usize,
    delta: Delta,
    finish_reason: Option<String>,
    #[allow(dead_code)]
    logprobs: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CompletionChunk {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    created: usize,
    #[allow(dead_code)]
    model: String,
    #[allow(dead_code)]
    system_fingerprint: String,
    choices: Vec<CompletionChunkChoice>,
}

pub async fn completion_stream(
    tx: mpsc::UnboundedSender<String>,
    messages: &Vec<Message>,
    tools: &Option<Vec<BoxedToolCall>>,
    api_hostname: &str,
    api_key: &str,
    model: &str,
) -> Result<Value, Error> {
    let mut payload = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true}
    });
    if let Some(tools) = tools {
        payload["tools"] = json!(tools);
    }
    let url = format!("{}/v1/chat/completions", api_hostname.trim_end_matches("/"));
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .timeout(Duration::from_secs(60 * 5))
        .json(&payload)
        .send()
        .await?;

    let mut stream = response.bytes_stream();

    let mut content_buf = String::from("");
    let mut reasoning_buf: String = String::from("");
    let mut tool_calls: HashMap<usize, ToolCallFinal> = HashMap::new();
    let mut buffer = String::new();

    'outer: while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("Invalid chunk");
        let chunk_str = std::str::from_utf8(&chunk)?;

        // Append new data to buffer. This is necessary to handle SSE
        // fragmentation over HTTP/2 frames.
        buffer.push_str(chunk_str);

        // Process all complete SSE events from the buffer
        while let Some(event_end) = buffer.find("\n\n") {
            let event_data = buffer[..event_end].to_string();
            buffer = buffer[event_end + 2..].to_string();

            // Skip empty events
            let event_data = event_data.trim();
            if event_data.is_empty() {
                continue;
            }

            // Parse SSE events
            if !event_data.starts_with("data: ") {
                continue;
            }

            // Extract the JSON payload (after "data: ")
            let data = event_data[6..].trim();

            // Data can sometimes be empty. Not sure why.
            if data.is_empty() {
                continue;
            }

            // Forward the chunk to the receiver channel
            // (The result is ignored here because we want to complete
            // processing the response)
            let _ = tx.send(data.to_string());

            // Handle the end of the stream
            if data == "[DONE]" {
                break 'outer;
            }

            // Process the delta
            let chunk = serde_json::from_str::<CompletionChunk>(data).inspect_err(|e| {
                tracing::error!("Parsing completion chunk failed for {}\nError:{}", data, e)
            })?;
            let choice = chunk.choices.first().expect("Missing choices field");

            match &choice.delta {
                Delta::Reasoning { reasoning } => {
                    if choice.finish_reason.is_some() {
                        break 'outer;
                    }
                    reasoning_buf += &reasoning.clone();
                }
                Delta::Content { content } => {
                    if choice.finish_reason.is_some() {
                        break 'outer;
                    }

                    content_buf += &content.clone();
                }
                Delta::ToolCall {
                    tool_calls: tool_call_deltas,
                } => {
                    if choice.finish_reason.is_some() {
                        break 'outer;
                    }
                    for tool_call_delta in tool_call_deltas.iter() {
                        match tool_call_delta {
                            ToolCallChunk::Init {
                                id,
                                index,
                                function,
                                r#type,
                            } => {
                                let init_tool_call = ToolCallFinal {
                                    index: *index,
                                    id: id.clone(),
                                    function: FunctionFinal {
                                        name: function.name.clone(),
                                        arguments: function.arguments.clone(),
                                    },
                                    r#type: r#type.clone(),
                                };
                                tool_calls.insert(*index, init_tool_call);
                            }
                            ToolCallChunk::ArgsDelta {
                                index, function, ..
                            } => {
                                tool_calls.entry(*index).and_modify(|v| {
                                    let args = function.arguments.clone();
                                    v.function.arguments += &args;
                                });
                            }
                        }
                    }
                }
                Delta::Stop {} => {
                    break 'outer;
                }
            }
        }
    }

    // Handle if this is a tool call or a content message
    if !tool_calls.is_empty() {
        let tool_call_message = tool_calls.values().collect::<Vec<_>>();
        let out = json!({
            "choices": [{"message": {"tool_calls": tool_call_message}}]
        });
        return Ok(out);
    }

    let out = json!({
        "choices": [
            {"message": {"content": content_buf}}
        ]
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role_serialization() {
        assert_eq!(serde_json::to_string(&Role::System).unwrap(), r#""system""#);
        assert_eq!(
            serde_json::to_string(&Role::Assistant).unwrap(),
            r#""assistant""#
        );
        assert_eq!(serde_json::to_string(&Role::User).unwrap(), r#""user""#);
        assert_eq!(serde_json::to_string(&Role::Tool).unwrap(), r#""tool""#);
    }

    #[test]
    fn test_role_deserialization() {
        let json = r#""system""#;
        assert_eq!(serde_json::from_str::<Role>(json).unwrap(), Role::System);

        let json = r#""assistant""#;
        assert_eq!(serde_json::from_str::<Role>(json).unwrap(), Role::Assistant);

        let json = r#""user""#;
        assert_eq!(serde_json::from_str::<Role>(json).unwrap(), Role::User);

        let json = r#""tool""#;
        assert_eq!(serde_json::from_str::<Role>(json).unwrap(), Role::Tool);
    }

    #[test]
    fn test_message_new() {
        let msg = Message::new(Role::User, "Hello world");
        assert_eq!(
            serde_json::to_string(&msg).unwrap(),
            r#"{"role":"user","content":"Hello world"}"#
        );

        let msg = Message::new(Role::Assistant, "I can help!");
        assert_eq!(
            serde_json::to_string(&msg).unwrap(),
            r#"{"role":"assistant","content":"I can help!"}"#
        );
    }

    #[test]
    fn test_message_new_tool_call_request() {
        let tool_calls = vec![FunctionCall {
            function: FunctionCallFn {
                arguments: r#"{"query":"books"}"#.to_string(),
                name: "search_notes".to_string(),
            },
            id: "call_test123".to_string(),
            r#type: "function".to_string(),
        }];

        let msg = Message::new_tool_call_request(tool_calls);
        assert_eq!(
            serde_json::to_string(&msg).unwrap(),
            r#"{"role":"assistant","tool_calls":[{"function":{"arguments":"{\"query\":\"books\"}","name":"search_notes"},"id":"call_test123","type":"function"}]}"#
        );
    }

    #[test]
    fn test_message_new_tool_call_response() {
        let msg = Message::new_tool_call_response("Found 3 books", "call_test123");
        assert_eq!(
            serde_json::to_string(&msg).unwrap(),
            r#"{"role":"tool","content":"Found 3 books","tool_call_id":"call_test123"}"#
        );
    }

    #[test]
    fn test_function_call_serialization() {
        let fc = FunctionCallFn {
            arguments: r#"{"query":"test"}"#.to_string(),
            name: "my_function".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&fc).unwrap(),
            r#"{"arguments":"{\"query\":\"test\"}","name":"my_function"}"#
        );
    }

    #[test]
    fn test_function_call_deserialization() {
        let json = r#"{"arguments":"{\"query\":\"test\"}","name":"my_function"}"#;
        let fc: FunctionCallFn = serde_json::from_str(json).unwrap();
        assert_eq!(fc.name, "my_function");
        assert_eq!(fc.arguments, r#"{"query":"test"}"#);
    }

    #[test]
    fn test_function_call_full_serialization() {
        let fc = FunctionCall {
            function: FunctionCallFn {
                arguments: r#"{"query":"books"}"#.to_string(),
                name: "search_notes".to_string(),
            },
            id: "call_test123".to_string(),
            r#type: "function".to_string(),
        };
        let json = serde_json::to_string(&fc).unwrap();
        assert!(json.contains(r#""arguments":"{\"query\":\"books\"}"#));
        assert!(json.contains(r#""name":"search_notes""#));
        assert!(json.contains(r#""id":"call_test123""#));
        assert!(json.contains(r#""type":"function""#));
    }

    #[test]
    fn test_function_call_full_deserialization() {
        let json = r#"{
            "function": {"arguments":"{\"query\":\"books\"}","name":"search_notes"},
            "id":"call_test123",
            "type":"function"
        }"#;
        let fc: FunctionCall = serde_json::from_str(json).unwrap();
        assert_eq!(fc.id, "call_test123");
        assert_eq!(fc.r#type, "function");
        assert_eq!(fc.function.name, "search_notes");
        assert_eq!(fc.function.arguments, r#"{"query":"books"}"#);
    }

    #[test]
    fn test_tool_type_serialization() {
        assert_eq!(
            serde_json::to_string(&ToolType::Function).unwrap(),
            r#""function""#
        );
    }

    #[test]
    fn test_property_serialization() {
        let prop = Property {
            r#type: "string".to_string(),
            description: "The search query".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&prop).unwrap(),
            r#"{"type":"string","description":"The search query"}"#
        );
    }

    #[test]
    fn test_parameters_serialization() {
        let props = serde_json::json!({"query": {"type": "string", "description": "Search query"}});
        let params = Parameters {
            r#type: "object".to_string(),
            properties: props.clone(),
            required: vec!["query".to_string()],
            additional_properties: false,
        };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["type"], "object");
        assert_eq!(json["required"].as_array().unwrap()[0], "query");
        assert_eq!(json["additionalProperties"], false);
    }

    #[test]
    fn test_function_serialization() {
        let props = serde_json::json!({"query": {"type": "string", "description": "Search query"}});
        let params = Parameters {
            r#type: "object".to_string(),
            properties: props,
            required: vec!["query".to_string()],
            additional_properties: false,
        };
        let func = Function {
            name: "search_notes".to_string(),
            description: "Search through notes".to_string(),
            parameters: params,
            strict: true,
        };
        let json = serde_json::to_value(&func).unwrap();
        assert_eq!(json["name"], "search_notes");
        assert_eq!(json["description"], "Search through notes");
    }

    #[test]
    fn test_delta_content_deserialization() {
        let json = r#"{"content":"Hello"}"#;
        let delta: Delta = serde_json::from_str(json).unwrap();
        match delta {
            Delta::Content { content } => assert_eq!(content, "Hello"),
            _ => panic!("Expected Content variant"),
        }
    }

    #[test]
    fn test_delta_reasoning_deserialization() {
        let json = r#"{"reasoning":"Thinking..."}"#;
        let delta: Delta = serde_json::from_str(json).unwrap();
        match delta {
            Delta::Reasoning { reasoning } => assert_eq!(reasoning, "Thinking..."),
            _ => panic!("Expected Reasoning variant"),
        }
    }

    #[test]
    fn test_delta_stop_deserialization() {
        let json = r#"{}"#;
        let delta: Delta = serde_json::from_str(json).unwrap();
        match delta {
            Delta::Stop {} => {}
            _ => panic!("Expected Stop variant"),
        }
    }

    #[test]
    fn test_delta_tool_call_deserialization() {
        let json = r#"{
            "tool_calls": [{
                "id":"call_abc",
                "index":0,
                "function":{"name":"search","arguments":"{\"q\":\"test\"}"},
                "type":"function"
            }]
        }"#;
        let delta: Delta = serde_json::from_str(json).unwrap();
        match delta {
            Delta::ToolCall { tool_calls } => {
                assert_eq!(tool_calls.len(), 1);
            }
            _ => panic!("Expected ToolCall variant"),
        }
    }

    #[test]
    fn test_tool_call_chunk_init_deserialization() {
        let json = r#"{
            "id":"call_abc",
            "index":0,
            "function":{"name":"search","arguments":"{"},
            "type":"function"
        }"#;
        let chunk: ToolCallChunk = serde_json::from_str(json).unwrap();
        match chunk {
            ToolCallChunk::Init {
                id,
                index,
                function,
                r#type,
            } => {
                assert_eq!(id, "call_abc");
                assert_eq!(index, 0);
                assert_eq!(function.name, "search");
                assert_eq!(r#type, "function");
            }
            _ => panic!("Expected Init variant"),
        }
    }

    #[test]
    fn test_tool_call_chunk_args_delta_deserialization() {
        let json = r#"{
            "index":0,
            "function":{"arguments":"\"q\":\"test\"}"},
            "type":"function"
        }"#;
        let chunk: ToolCallChunk = serde_json::from_str(json).unwrap();
        match chunk {
            ToolCallChunk::ArgsDelta {
                index,
                function,
                r#type,
            } => {
                assert_eq!(index, 0);
                assert_eq!(function.arguments, r#""q":"test"}"#);
                assert_eq!(r#type, "function");
            }
            _ => panic!("Expected ArgsDelta variant"),
        }
    }

    #[test]
    fn test_completion_chunk_deserialization() {
        let json = r#"{
            "id":"chunk_123",
            "created":1234567890,
            "model":"gpt-4",
            "system_fingerprint":"fp_abc123",
            "choices":[{
                "index":0,
                "delta":{"content":"Hello"},
                "finish_reason":null
            }]
        }"#;
        let chunk: CompletionChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.id, "chunk_123");
        assert_eq!(chunk.created, 1234567890);
        assert_eq!(chunk.model, "gpt-4");
    }

    #[tokio::test]
    async fn test_completion_basic() {
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

        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(response_body)
            .create();

        let messages = vec![Message::new(Role::User, "Hi")];
        let result = completion(&messages, &None, server.url().as_str(), "test-key", "gpt-4").await;

        mock.assert();
        assert!(result.is_ok());

        let json = result.unwrap();
        assert_eq!(json["choices"][0]["message"]["content"], "Hello!");
    }

    #[tokio::test]
    async fn test_completion_with_tools() {
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
                    "tool_calls": [{
                        "id": "call_abc123",
                        "type": "function",
                        "function": {
                            "name": "search_notes",
                            "arguments": "{\"query\":\"test\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }"#;

        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(response_body)
            .create();

        let messages = vec![Message::new(Role::User, "Search for test")];

        // Create a mock tool
        #[derive(serde::Serialize)]
        struct MockTool;
        #[async_trait]
        impl ToolCall for MockTool {
            async fn call(&self, _args: &str) -> Result<String, Error> {
                Ok("mock result".to_string())
            }
            fn function_name(&self) -> String {
                "search_notes".to_string()
            }
        }

        let tools = Some(vec![Box::new(MockTool) as BoxedToolCall]);

        let result = completion(
            &messages,
            &tools,
            server.url().as_str(),
            "test-key",
            "gpt-4",
        )
        .await;

        mock.assert();
        assert!(result.is_ok());

        let json = result.unwrap();
        assert!(json["choices"][0]["message"]["tool_calls"].is_array());
    }

    #[tokio::test]
    async fn test_completion_stream_content() {
        let mut server = mockito::Server::new_async().await;

        // SSE response with content chunks
        let sse_response = r#"data: {"id":"chunk1","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"id":"chunk2","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"content":" World"},"finish_reason":null}]}

data: {"id":"chunk3","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"content":"!"},"finish_reason":"stop"}]}

data: [DONE]

"#;

        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(sse_response)
            .create();

        let messages = vec![Message::new(Role::User, "Say hello")];
        let (tx, mut rx) = mpsc::unbounded_channel();
        let server_url = server.url();

        // Run completion_stream in a separate task
        let handle = tokio::spawn(async move {
            completion_stream(
                tx,
                &messages,
                &None,
                server_url.as_str(),
                "test-key",
                "gpt-4",
            )
            .await
        });

        // Wait for the task to complete
        let result = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;

        mock.assert();
        assert!(result.is_ok());
        assert!(result.unwrap().unwrap().is_ok());

        // The channel should have received the raw JSON chunks
        let mut chunk_count = 0;
        while let Ok(_) = rx.try_recv() {
            chunk_count += 1;
        }
        assert!(chunk_count >= 3);
    }

    #[tokio::test]
    async fn test_completion_stream_tool_call() {
        let mut server = mockito::Server::new_async().await;

        // SSE response with tool call chunks
        let sse_response = r#"data: {"id":"chunk1","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"tool_calls":[{"id":"call_abc123","index":0,"function":{"name":"search_notes","arguments":"{\"query\":"},"type":"function"}]},"finish_reason":null}]}

data: {"id":"chunk2","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"test\"}"}}]},"finish_reason":null}]}

data: {"id":"chunk3","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":""}}]},"finish_reason":"stop"}]}

data: [DONE]

"#;

        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(sse_response)
            .create();

        let messages = vec![Message::new(Role::User, "Search for test")];
        let (tx, _rx) = mpsc::unbounded_channel();
        let server_url = server.url();

        // Run completion_stream in a separate task
        let handle = tokio::spawn(async move {
            completion_stream(
                tx,
                &messages,
                &None,
                server_url.as_str(),
                "test-key",
                "gpt-4",
            )
            .await
        });

        // Wait for the task to complete
        let result = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;

        mock.assert();
        assert!(result.is_ok());
        assert!(result.unwrap().unwrap().is_ok());
    }

    #[tokio::test]
    async fn test_completion_stream_reasoning() {
        let mut server = mockito::Server::new_async().await;

        // SSE response with reasoning chunks
        let sse_response = r#"data: {"id":"chunk1","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"reasoning":"Thinking"},"finish_reason":null}]}

data: {"id":"chunk2","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"reasoning":"..."},"finish_reason":null}]}

data: {"id":"chunk3","created":1234567890,"model":"gpt-4","system_fingerprint":"fp1","choices":[{"index":0,"delta":{"content":"Done!"},"finish_reason":"stop"}]}

data: [DONE]

"#;

        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(sse_response)
            .create();

        let messages = vec![Message::new(Role::User, "Think about this")];
        let (tx, _rx) = mpsc::unbounded_channel();
        let server_url = server.url();

        // Run completion_stream in a separate task
        let handle = tokio::spawn(async move {
            completion_stream(
                tx,
                &messages,
                &None,
                server_url.as_str(),
                "test-key",
                "gpt-4",
            )
            .await
        });

        // Wait for the task to complete
        let result = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;

        mock.assert();
        assert!(result.is_ok());
        assert!(result.unwrap().unwrap().is_ok());
    }
}
