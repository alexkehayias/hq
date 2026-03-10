use async_trait::async_trait;
use std::time::Duration;
use tokio_rusqlite::Connection;

use super::PeriodicJob;
use crate::{
    ai::agents::email,
    core::AppConfig,
    google::oauth::find_all_gmail_auth_emails,
    notify::{
        mark_push_subscription_invalid, broadcast_push_notification,
        find_all_notification_subscriptions, PushNotificationPayload,
    },
};

#[derive(Default, Debug)]
pub struct ProcessEmail;

#[async_trait]
impl PeriodicJob for ProcessEmail {
    fn interval(&self) -> Duration {
        Duration::from_secs(60 * 60 * 2)
    }

    async fn run_job(&self, config: &AppConfig, db: &Connection) {
        let AppConfig {
            note_search_api_url,
            vapid_key_path,
            openai_api_hostname,
            openai_api_key,
            openai_model,
            ..
        } = config;
        let emails = { find_all_gmail_auth_emails(db).await.expect("Query failed") };

        let (session_id, messages) = email::email_chat_response(
            db,
            note_search_api_url,
            emails,
            openai_api_hostname,
            openai_api_key,
            openai_model,
        )
        .await;
        let last_msg = messages.last().unwrap();
        let summary = last_msg.content.clone().unwrap();

        // Broadcast push notification to all subscribers, using a new read lock for DB/config each time
        let chat_url = format!("/chat?session_id={}", session_id);
        let payload = PushNotificationPayload::new(
            "Unread Email Summary",
            &format!("Emails processed! {}", summary),
            Some(&chat_url),
            None,
            None,
        );
        let subscriptions = find_all_notification_subscriptions(db).await.unwrap();
        let failed_subscriptions =
            broadcast_push_notification(subscriptions, vapid_key_path.to_string(), payload).await;
        for sub in failed_subscriptions {
            let _ = mark_push_subscription_invalid(db, &sub.endpoint).await;
        }
    }
}
