//! Router for the email API

use std::sync::{Arc, RwLock};

use axum::{Router, extract::State, response::Json};
use axum_extra::extract::Query;
use tokio::task::JoinSet;

use super::public;
use crate::api::state::AppState;
use crate::core::AppConfig;
use crate::google::gmail::{Thread, extract_body, fetch_thread, list_unread_messages};
use crate::google::oauth::refresh_access_token;

type SharedState = Arc<RwLock<AppState>>;

async fn email_unread_handler(
    State(state): State<SharedState>,
    Query(params): Query<public::EmailUnreadQuery>,
) -> Result<Json<Vec<public::EmailThread>>, crate::api::public::ApiError> {
    let refresh_token: String = {
        let db = state.read().unwrap().db.clone();

        db.call(move |conn| {
            let result = conn
                .prepare("SELECT refresh_token FROM auth WHERE id = ?1")
                .and_then(|mut stmt| stmt.query_row([&params.email], |row| row.get(0)))?;
            Ok(result)
        })
        .await?
    };

    let (client_id, client_secret) = {
        let shared_state = state.read().expect("Unable to read share state");
        let AppConfig {
            gmail_api_client_id,
            gmail_api_client_secret,
            ..
        } = &shared_state.config;
        (gmail_api_client_id.clone(), gmail_api_client_secret.clone())
    };
    let oauth = refresh_access_token(&client_id, &client_secret, &refresh_token).await?;
    let access_token = oauth.access_token;
    let limit = params.limit.unwrap_or(7);

    // Query Gmail for unread messages
    let messages = list_unread_messages(&access_token, limit).await?;

    // Fetch each thread concurrently
    let mut tasks = JoinSet::new();
    for message in messages.into_iter() {
        let access_token = access_token.clone();
        let thread_id = message.thread_id;
        tasks.spawn(fetch_thread(access_token, thread_id));
    }
    let results: Vec<Thread> = tasks
        .join_all()
        .await
        .into_iter()
        .map(|i| i.unwrap())
        .collect();

    // Transform the threads and messages into a simpler format
    let mut threads: Vec<public::EmailThread> = Vec::new();
    for t in results {
        let mut messages: Vec<public::EmailMessage> = Vec::new();
        for m in t.messages {
            let body = extract_body(&m).trim().to_string();
            if body == "Failed to decode" {
                tracing::error!("Decode error: {:?}", m.payload);
            }
            let payload = m.payload.unwrap();
            let headers = payload.headers.unwrap();

            let from = headers
                .iter()
                .find(|h| h.name == "From")
                .map(|h| h.value.clone())
                .unwrap();
            let to = headers
                .iter()
                .find(|h| h.name == "To")
                .map(|h| h.value.clone())
                .unwrap();
            let subject = headers
                .iter()
                .find(|h| h.name == "Subject")
                .map(|h| h.value.clone())
                .unwrap();

            messages.push(public::EmailMessage {
                id: m.id,
                thread_id: m.thread_id,
                received: m.internal_date,
                from,
                to,
                subject,
                body,
            })
        }

        let latest_msg = messages[0].clone();

        threads.push(public::EmailThread {
            id: t.id,
            received: latest_msg.received,
            subject: latest_msg.subject,
            from: latest_msg.from,
            to: latest_msg.to,
            messages,
        });
    }

    threads.sort_by_key(|i| std::cmp::Reverse(i.received.clone()));

    Ok(Json(threads))
}

/// Create the email router
pub fn router() -> Router<SharedState> {
    Router::new().route("/unread", axum::routing::get(email_unread_handler))
}
