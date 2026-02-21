mod test_utils;

#[cfg(test)]
mod tests {
    use anyhow::{Error, Result};
    use async_trait::async_trait;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tokio::sync::mpsc;
    use tower::util::ServiceExt;

    use hq::ai::prompt::{self, Prompt};
    use hq::openai;
    use hq::openai::BoxedToolCall;
    use serde::Serialize;
    use serde_json::json;
    use serial_test::serial;

    use crate::test_utils::{body_to_string, test_app};

    #[tokio::test]
    #[serial]
    async fn it_serves_web_ui() {
        let app = test_app().await;

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("Welcome Alex"));
    }

    #[tokio::test]
    #[serial]
    async fn it_searches_full_text() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search?query=test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[derive(Serialize)]
    pub struct DummyProps {
        dummy_arg: openai::Property,
    }

    #[derive(Serialize)]
    pub struct DummyTool {
        pub r#type: openai::ToolType,
        pub function: openai::Function<DummyProps>,
    }

    #[async_trait]
    impl openai::ToolCall for DummyTool {
        async fn call(&self, _args: &str) -> Result<String, Error> {
            Ok(String::from("DummyTool called!"))
        }

        fn function_name(&self) -> String {
            String::from("dummy_tool")
        }
    }

    #[derive(Serialize)]
    pub struct DummyProps2 {
        dummy_arg: openai::Property,
    }

    #[derive(Serialize)]
    pub struct DummyTool2 {
        pub r#type: openai::ToolType,
        pub function: openai::Function<DummyProps2>,
    }

    #[async_trait]
    impl openai::ToolCall for DummyTool2 {
        async fn call(&self, _args: &str) -> Result<String, Error> {
            Ok(String::from("DummyTool2 called!"))
        }

        fn function_name(&self) -> String {
            String::from("dummy_tool_2")
        }
    }

    #[tokio::test]
    #[ignore]
    async fn it_makes_openai_request() {
        let messages = vec![
            openai::Message::new(openai::Role::System, "You are a helpful assistant."),
            openai::Message::new(
                openai::Role::User,
                "Write a haiku that explains the concept of recursion.",
            ),
        ];
        let tools = None;
        let response = openai::completion(
            &messages,
            &tools,
            "https://api.openai.com",
            "test-api-key",
            "gpt-4o",
        )
        .await;
        assert!(response.is_ok());
    }

    #[tokio::test]
    #[ignore]
    async fn it_makes_openai_streaming_request() {
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        let messages = vec![
            openai::Message::new(openai::Role::System, "You are a helpful assistant."),
            openai::Message::new(
                openai::Role::User,
                "Write a haiku that explains the concept of recursion.",
            ),
        ];
        let tools = None;
        let response = openai::completion_stream(
            tx,
            &messages,
            &tools,
            "https://api.openai.com",
            "test-api-key",
            "gpt-4o",
        )
        .await;

        assert!(response.is_ok());
        assert_eq!(response.unwrap(), "Testing");
    }

    #[tokio::test]
    #[ignore]
    async fn it_makes_openai_tool_calls() {
        let messages = vec![
            openai::Message::new(openai::Role::System, "You are a helpful assistant."),
            openai::Message::new(openai::Role::User, "What's the weather in New York?"),
        ];
        let function = openai::Function {
            name: String::from("get_weather"),
            description: String::from("Retrieves current weather for the given location."),
            parameters: openai::Parameters {
                r#type: String::from("object"),
                properties: DummyProps {
                    dummy_arg: openai::Property {
                        r#type: String::from("string"),
                        description: String::from("Location of the weather requested"),
                        r#enum: None,
                    },
                },
                required: vec![String::from("dummy_arg")],
                additional_properties: false,
            },
            strict: true,
        };
        let dummy_tool = DummyTool {
            r#type: openai::ToolType::Function,
            function,
        };

        let function2 = openai::Function {
            name: String::from("get_notes"),
            description: String::from("Retrieves notes the user asks about."),
            parameters: openai::Parameters {
                r#type: String::from("object"),
                properties: DummyProps2 {
                    dummy_arg: openai::Property {
                        r#type: String::from("string"),
                        description: String::from("Some dummy arg"),
                        r#enum: None
                    },
                },
                required: vec![String::from("dummy_arg")],
                additional_properties: false,
            },
            strict: true,
        };
        let dummy_tool_2 = DummyTool2 {
            r#type: openai::ToolType::Function,
            function: function2,
        };
        let tools: Option<Vec<BoxedToolCall>> =
            Some(vec![Box::new(dummy_tool), Box::new(dummy_tool_2)]);
        let response = openai::completion(
            &messages,
            &tools,
            "https://api.openai.com",
            "test-api-key",
            "gpt-4o",
        )
        .await
        .unwrap();
        let tool_calls = response["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();
        assert!(!tool_calls.is_empty());
    }

    #[tokio::test]
    async fn it_renders_a_prompt() -> Result<(), Error> {
        let templates = prompt::templates();
        let actual = templates.render(
            &Prompt::NoteSummary.to_string(),
            &json!({"context": "test test"}),
        )?;
        assert!(actual.contains("CONTEXT:\ntest test"));
        Ok(())
    }

    #[tokio::test]
    #[serial]
    async fn it_gets_chat_sessions() {
        let app = test_app().await;

        // First create some chat sessions by making a request to the chat endpoint
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/chat")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "session_id": "test-session-1",
                            "message": "Hello"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Now test the sessions endpoint
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat/sessions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        // Verify we get valid JSON response
        assert!(body.contains("\"sessions\""));
        assert!(body.contains("\"page\""));
        assert!(body.contains("\"limit\""));
        assert!(body.contains("\"total_sessions\""));
        assert!(body.contains("\"total_pages\""));
    }

    #[tokio::test]
    #[serial]
    async fn it_gets_chat_sessions_with_pagination() {
        let app = test_app().await;

        // Create multiple chat sessions to test pagination
        for i in 1..=5 {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/chat")
                        .method("POST")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            json!({
                                "session_id": format!("test-session-{}", i),
                                "message": format!("Message {}", i)
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
        }

        // Test pagination with limit=2, page=1
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/chat/sessions?page=1&limit=2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        // Verify we get valid JSON response with pagination data
        assert!(body.contains("\"sessions\""));
        // Just check that the response has the basic structure we expect
        assert!(body.contains("\"page\""));
        assert!(body.contains("\"limit\""));
        assert!(body.contains("\"total_sessions\""));
        assert!(body.contains("\"total_pages\""));

        // Test pagination with limit=2, page=2
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat/sessions?page=2&limit=2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        // Verify we get valid JSON response with pagination data for second page
        assert!(body.contains("\"page\""));
        assert!(body.contains("\"limit\""));
        assert!(body.contains("\"total_sessions\""));
        assert!(body.contains("\"total_pages\""));
    }

    #[tokio::test]
    #[serial]
    async fn it_records_metric() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": "token-count",
                            "value": 20,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    #[serial]
    async fn it_receives_blurt_webhook() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhook/blurt")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "id": 12345,
                            "title": "Test Notification",
                            "subtitle": Some("Subtitle"),
                            "body": "This is a test notification body",
                            "date": 1704067200,
                            "bundle_id": Some("com.example.app"),
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
