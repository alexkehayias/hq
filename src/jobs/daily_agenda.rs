use async_trait::async_trait;
use std::time::Duration;
use tokio_rusqlite::Connection;
use uuid::Uuid;

use super::PeriodicJob;
use crate::{
    ai::agents::agenda, core::AppConfig, google::oauth::find_all_gmail_auth_emails, notify::{
        PushNotificationPayload, broadcast_push_notification, find_all_notification_subscriptions,
    }, openai::{get_or_create_session, insert_chat_message}
};

#[derive(Debug)]
pub struct DailyAgenda;

#[async_trait]
impl PeriodicJob for DailyAgenda {
    fn interval(&self) -> Duration {
        // Every 12 hours
        Duration::from_secs(60 * 60 * 12)
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

        let session_id = Uuid::new_v4().to_string();

        let calendar_emails = find_all_gmail_auth_emails(&db).await.expect("No authenticated calendars for emails found");

        // Create the session with an "agenda" tag
        if let Err(e) = get_or_create_session(db, &session_id, &["background", "agenda"]).await {
            tracing::error!("Failed to create session for daily agenda: {}", e);
            return;
        }

        let history = agenda::daily_agenda_response(
            note_search_api_url,
            calendar_emails,
            openai_api_hostname,
            openai_api_key,
            openai_model,
        )
        .await;

        let last_msg = history.last().unwrap();
        let summary = last_msg.content.clone().unwrap();

        // Store the chat messages so the session can be picked up later
        for m in history {
            if let Err(e) = insert_chat_message(db, &session_id, &m).await {
                tracing::error!("Failed to insert chat message: {}", e);
            }
        }

        // Broadcast push notification to all subscribers with a link
        // to the chat session
        let chat_url = format!("/chat?session_id={}", session_id);
        let payload = PushNotificationPayload::new(
            "Daily Agenda",
            &summary.chars().take(150).collect::<String>(),
            Some(&chat_url),
            None,
            None,
        );

        let subscriptions = match find_all_notification_subscriptions(db).await {
            Ok(subs) => subs,
            Err(e) => {
                tracing::error!("Failed to fetch notification subscriptions: {}", e);
                vec![]
            }
        };

        broadcast_push_notification(subscriptions, vapid_key_path.to_string(), payload).await;
    }
}
