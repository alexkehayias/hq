use crate::notify::{
    PushNotificationPayload, broadcast_push_notification, find_all_notification_subscriptions,
    mark_push_subscription_invalid,
};
use crate::openai::{Function, Parameters, Property, RecoverableToolError, ToolCall, ToolType, parse_tool_args};
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

        let payload = PushNotificationPayload::new(
            fn_args.title.as_deref().unwrap_or("Notification"),
            &fn_args.message,
            None,
            None,
            None,
        );

        let failed_subscriptions = broadcast_push_notification(
            subscriptions,
            self.vapid_key_path.clone(),
            payload,
        )
        .await;
        let fail_count = failed_subscriptions.len();
        for sub in &failed_subscriptions {
            let _ = mark_push_subscription_invalid(&self.db, &sub.endpoint).await;
        }

        let status_message = if fail_count == total {
            "Failed to send notification: all subscriptions are invalid and were removed.".to_string()
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
        self.function.name.clone()
    }
}

impl NotifyTool {
    pub fn new(db: Connection, vapid_key_path: &str) -> Self {
        let function = Function {
            name: String::from("notify"),
            description: String::from(
                "Send a push notification to the user's devices. Use this to deliver important information that the user should see outside of the chat, such as completed tasks, alerts, summaries, or time-sensitive updates. The notification will appear on all devices where the web app is open and notifications are enabled.",
            ),
            parameters: Parameters {
                r#type: String::from("object"),
                properties: NotifyProps {
                    message: Property {
                        r#type: String::from("string"),
                        description: String::from(
                            "The notification message body. Keep it concise and actionable (recommended under 200 characters)."
                        ),
                        r#enum: None,
                    },
                    title: Some(Property {
                        r#type: String::from("string"),
                        description: String::from(
                            "Optional notification title. Defaults to 'Notification' if not provided."
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_rusqlite::Connection;

    /// Create an in-memory database with the push_subscription table
    /// and the auth table (needed for the foreign key or other queries).
    async fn setup_db() -> Connection {
        let db = Connection::open_in_memory().await.unwrap();
        db.call(|conn| {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS push_subscription (
                    endpoint TEXT PRIMARY KEY,
                    p256dh TEXT NOT NULL,
                    auth TEXT NOT NULL,
                    encoding TEXT NOT NULL DEFAULT 'Aes126Gcm',
                    is_valid INTEGER NOT NULL DEFAULT 1
                )",
                [],
            )
            .unwrap();
            Ok(())
        })
        .await
        .unwrap();
        db
    }

    #[tokio::test]
    async fn test_notify_no_subscriptions_returns_recoverable_error() {
        let db = setup_db().await;
        let tool = NotifyTool::new(db, "/tmp/fake-vapid.pem");

        let result = tool
            .call(r#"{"message": "test notification"}"#)
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let recoverable = err.downcast_ref::<RecoverableToolError>();
        assert!(recoverable.is_some());
        assert!(recoverable
            .unwrap()
            .message
            .contains("No push notification subscriptions found"));
    }

    #[tokio::test]
    async fn test_notify_empty_message_returns_error() {
        let db = setup_db().await;
        let tool = NotifyTool::new(db, "/tmp/fake-vapid.pem");

        let result = tool.call(r#"{"message": ""}"#).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let recoverable = err.downcast_ref::<RecoverableToolError>();
        assert!(recoverable.is_some());
        assert!(recoverable
            .unwrap()
            .message
            .contains("Notification message cannot be empty"));
    }

    #[tokio::test]
    async fn test_notify_missing_message_returns_error() {
        let db = setup_db().await;
        let tool = NotifyTool::new(db, "/tmp/fake-vapid.pem");

        let result = tool.call(r#"{}"#).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_notify_with_title() {
        let db = setup_db().await;
        let tool = NotifyTool::new(db, "/tmp/fake-vapid.pem");

        let result = tool
            .call(r#"{"message": "test", "title": "My Title"}"#)
            .await;

        // Should fail with no subscriptions, not with parse error
        assert!(result.is_err());
        let err = result.unwrap_err();
        let recoverable = err.downcast_ref::<RecoverableToolError>();
        assert!(recoverable.is_some());
    }

    #[test]
    fn test_notify_tool_function_name() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let db = rt.block_on(setup_db());
        let tool = NotifyTool::new(db, "/tmp/fake-vapid.pem");
        assert_eq!(tool.function_name(), "notify");
    }

    #[test]
    fn test_notify_function_json_has_required_parameters() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let db = rt.block_on(setup_db());
        let tool = NotifyTool::new(db, "/tmp/fake-vapid.pem");
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
