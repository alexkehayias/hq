use super::registry::{Tool, ToolConfig};
use crate::notify::{
    PushNotificationPayload, broadcast_push_notification, find_all_notification_subscriptions,
    mark_push_subscription_invalid,
};
use crate::openai::{
    Function, Parameters, Property, RecoverableToolError, ToolCall, ToolType, parse_tool_args,
};
use anyhow::{Error, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_rusqlite::Connection;

#[derive(Serialize)]
pub struct NotifyProps {
    pub message: Property,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<Property>,
}

#[derive(Deserialize)]
pub struct NotifyArgs {
    pub message: String,
    pub title: Option<String>,
}

#[derive(Serialize)]
pub struct NotifyTool {
    pub r#type: ToolType,
    pub function: Function<NotifyProps>,
    #[serde(skip)]
    db: Connection,
    #[serde(skip)]
    vapid_key_path: String,
    /// The URL to open when the notification is clicked, passed through to the
    /// push payload verbatim.
    #[serde(skip)]
    url: Option<String>,
}

#[async_trait]
impl ToolCall for NotifyTool {
    async fn call(&self, args: &str) -> Result<String, Error> {
        let fn_args: NotifyArgs = parse_tool_args(args)?;

        if fn_args.message.trim().is_empty() {
            return Err(Error::from(RecoverableToolError::new(
                "Notification message cannot be empty.",
            )));
        }

        let subscriptions = find_all_notification_subscriptions(&self.db)
            .await
            .unwrap_or_default();

        if subscriptions.is_empty() {
            return Err(Error::from(RecoverableToolError::new(
                "No push notification subscriptions found. The user may not have the web app open with notifications enabled.",
            )));
        }

        let total = subscriptions.len();
        tracing::info!(
            "Sending notification to {} subscription(s): {}",
            total,
            fn_args.title.as_deref().unwrap_or("Notification"),
        );

        let payload = self.payload(
            fn_args.title.as_deref().unwrap_or("Notification"),
            &fn_args.message,
        );

        let failed_subscriptions =
            broadcast_push_notification(subscriptions, self.vapid_key_path.clone(), payload).await;
        let fail_count = failed_subscriptions.len();
        for sub in &failed_subscriptions {
            let _ = mark_push_subscription_invalid(&self.db, &sub.endpoint).await;
        }

        let status_message = if fail_count == total {
            "Failed to send notification: all subscriptions are invalid and were removed."
                .to_string()
        } else if fail_count > 0 {
            format!(
                "Notification sent to {} of {} device(s). {} invalid subscription(s) removed.",
                total - fail_count,
                total,
                fail_count
            )
        } else {
            "Notification sent successfully.".to_string()
        };

        Ok(status_message)
    }

    fn function_name(&self) -> String {
        Self::NAME.to_string()
    }
}

impl Tool for NotifyTool {
    const NAME: &'static str = "notify";

    fn from_config(conf: &ToolConfig) -> Result<Self> {
        Ok(Self::new(
            conf.db.clone(),
            &conf.vapid_key_path,
            conf.notification_url.as_deref(),
        ))
    }
}

impl NotifyTool {
    pub fn new(db: Connection, vapid_key_path: &str, url: Option<&str>) -> Self {
        let function = Function {
            name: Self::NAME.to_string(),
            description: String::from(
                "Send a push notification to the user's devices. Use this to deliver important information that the user should see outside of the chat, such as completed tasks, alerts, summaries, or time-sensitive updates. The notification will appear on all devices where the web app is open and notifications are enabled. Clicking the notification opens the page this notification is linked to.",
            ),
            parameters: Parameters {
                r#type: String::from("object"),
                properties: NotifyProps {
                    message: Property {
                        r#type: String::from("string"),
                        description: String::from(
                            "The notification message body. Keep it concise and actionable (recommended under 200 characters).",
                        ),
                        r#enum: None,
                    },
                    title: Some(Property {
                        r#type: String::from("string"),
                        description: String::from(
                            "Optional notification title. Defaults to 'Notification' if not provided.",
                        ),
                        r#enum: None,
                    }),
                },
                required: vec![String::from("message")],
                additional_properties: false,
            },
            strict: true,
        };
        Self {
            r#type: ToolType::Function,
            function,
            db,
            vapid_key_path: vapid_key_path.to_string(),
            url: url.map(str::to_string),
        }
    }

    /// The push payload for a notification, linking to this tool's URL.
    fn payload(&self, title: &str, message: &str) -> PushNotificationPayload {
        PushNotificationPayload::new(title, message, self.url.as_deref(), None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use tokio_rusqlite::Connection;
    use tokio_rusqlite::rusqlite;
    use uuid::Uuid;

    use crate::core::db::{async_db, initialize_db};

    /// Create a database with all tables initialized in a temp directory.
    async fn setup_db() -> Connection {
        let temp_dir = env::temp_dir();
        let dir = temp_dir.join(Uuid::new_v4().to_string());
        fs::create_dir_all(&dir).expect("Failed to create base directory");

        let db_path_str = dir.to_str().unwrap();
        let db = async_db(db_path_str)
            .await
            .expect("Failed to connect to async db");
        db.call(|conn| {
            initialize_db(conn).expect("Failed to migrate db");
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .unwrap();
        db
    }

    /// A tool context as the loop supervisor builds it, with `notification_url`
    /// pointing at the current run's chat session.
    fn tool_config(db: Connection, notification_url: Option<&str>) -> ToolConfig {
        ToolConfig {
            db,
            api_base_url: "http://localhost:2222".to_string(),
            storage_path: "/tmp".to_string(),
            vapid_key_path: "/tmp/fake-vapid.pem".to_string(),
            session_id: "loop-workspace".to_string(),
            notification_url: notification_url.map(str::to_string),
            skill_registry: None,
            api_hostname: "https://api.openai.com".to_string(),
            api_key: "test-key".to_string(),
            model: "gpt-4".to_string(),
        }
    }

    #[tokio::test]
    async fn test_notify_no_subscriptions_returns_recoverable_error() {
        let db = setup_db().await;
        let tool = NotifyTool::new(db, "/tmp/fake-vapid.pem", None);

        let result = tool.call(r#"{"message": "test notification"}"#).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let recoverable = err.downcast_ref::<RecoverableToolError>();
        assert!(recoverable.is_some());
        assert!(
            recoverable
                .unwrap()
                .message
                .contains("No push notification subscriptions found")
        );
    }

    #[tokio::test]
    async fn test_notify_empty_message_returns_error() {
        let db = setup_db().await;
        let tool = NotifyTool::new(db, "/tmp/fake-vapid.pem", None);

        let result = tool.call(r#"{"message": ""}"#).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let recoverable = err.downcast_ref::<RecoverableToolError>();
        assert!(recoverable.is_some());
        assert!(
            recoverable
                .unwrap()
                .message
                .contains("Notification message cannot be empty")
        );
    }

    #[tokio::test]
    async fn test_notify_missing_message_returns_error() {
        let db = setup_db().await;
        let tool = NotifyTool::new(db, "/tmp/fake-vapid.pem", None);

        let result = tool.call(r#"{}"#).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_notify_with_title() {
        let db = setup_db().await;
        let tool = NotifyTool::new(db, "/tmp/fake-vapid.pem", None);

        let result = tool
            .call(r#"{"message": "test", "title": "My Title"}"#)
            .await;

        // Should fail with no subscriptions, not with parse error
        assert!(result.is_err());
        let err = result.unwrap_err();
        let recoverable = err.downcast_ref::<RecoverableToolError>();
        assert!(recoverable.is_some());
    }

    #[tokio::test]
    async fn test_notify_tool_function_name() {
        let db = setup_db().await;
        let tool = NotifyTool::new(db, "/tmp/fake-vapid.pem", None);
        assert_eq!(tool.function_name(), "notify");
    }

    #[tokio::test]
    async fn test_payload_passes_url_through() {
        let db = setup_db().await;
        let url = "http://localhost:2222/chat/?session_id=session-123";
        let tool = NotifyTool::new(db, "/tmp/fake-vapid.pem", Some(url));
        let payload = tool.payload("Title", "Body");
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["data"]["url"], url);
    }

    #[tokio::test]
    async fn test_payload_without_url_defaults_to_app_base() {
        let db = setup_db().await;
        let tool = NotifyTool::new(db, "/tmp/fake-vapid.pem", None);
        let payload = tool.payload("Title", "Body");
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["data"]["url"], "/");
    }

    #[tokio::test]
    async fn test_from_config_uses_notification_url() {
        let db = setup_db().await;
        let url = "http://localhost:2222/chat/?session_id=run-1";
        let conf = tool_config(db, Some(url));
        let tool = NotifyTool::from_config(&conf).unwrap();
        let payload = tool.payload("Title", "Body");
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["data"]["url"], url);
    }

    #[tokio::test]
    async fn test_from_config_without_notification_url_defaults_to_app_base() {
        let db = setup_db().await;
        let conf = tool_config(db, None);
        let tool = NotifyTool::from_config(&conf).unwrap();
        let payload = tool.payload("Title", "Body");
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["data"]["url"], "/");
    }

    #[tokio::test]
    async fn test_notify_function_json_has_required_parameters() {
        let db = setup_db().await;
        let tool = NotifyTool::new(db, "/tmp/fake-vapid.pem", None);
        let json = serde_json::to_string(&tool.function).expect("Failed to serialize function");

        let value: serde_json::Value =
            serde_json::from_str(&json).expect("Failed to parse function JSON");

        assert_eq!(value["name"], "notify");

        let params = &value["parameters"];
        let required = params["required"]
            .as_array()
            .expect("Required should be an array");
        assert!(
            required.contains(&serde_json::json!("message")),
            "message should be in required array"
        );

        let properties = &params["properties"];
        let message = &properties["message"];
        assert_eq!(message["type"], "string");

        let title = &properties["title"];
        assert_eq!(title["type"], "string");
    }
}
