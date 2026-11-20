//! Router for the email API

use std::sync::{Arc, RwLock};

use axum::{Router, extract::State, response::Json};
use axum_extra::extract::Query;
use tokio::task::JoinSet;

use super::public;
use crate::state::AppState;
use hq::core::AppConfig;
use hq::google::gmail::{
    Thread, extract_body, fetch_thread, list_messages_by_query, list_unread_messages,
};
use hq::google::oauth::{find_refresh_token, refresh_access_token};

type SharedState = Arc<RwLock<AppState>>;

async fn email_unread_handler(
    State(state): State<SharedState>,
    Query(params): Query<public::EmailUnreadQuery>,
) -> Result<Json<Vec<public::EmailThread>>, crate::public::ApiError> {
    let db = state.read().unwrap().db.clone();
    let refresh_token = find_refresh_token(&db, &params.email).await?;

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

    let threads = fetch_and_transform_threads(&access_token, messages).await?;
    Ok(Json(threads))
}

async fn email_search_handler(
    State(state): State<SharedState>,
    Query(params): Query<public::EmailSearchQuery>,
) -> Result<Json<Vec<public::EmailThread>>, crate::public::ApiError> {
    let db = state.read().unwrap().db.clone();
    let refresh_token = find_refresh_token(&db, &params.email).await?;

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

    // Query Gmail for messages matching the user-provided search query
    let messages = list_messages_by_query(&access_token, &params.query).await?;

    let threads = fetch_and_transform_threads(&access_token, messages).await?;
    Ok(Json(threads))
}

/// Create the email router
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/unread", axum::routing::get(email_unread_handler))
        .route("/search", axum::routing::get(email_search_handler))
}

/// Fetch the full thread for each message concurrently and transform
/// them into the public `EmailThread` shape. Shared by both the
/// unread and search handlers.
async fn fetch_and_transform_threads(
    access_token: &str,
    messages: Vec<hq::google::gmail::MessageResponse>,
) -> Result<Vec<public::EmailThread>, crate::public::ApiError> {
    let mut tasks = JoinSet::new();
    for message in messages.into_iter() {
        let access_token = access_token.to_string();
        let thread_id = message.thread_id;
        tasks.spawn(fetch_thread(access_token, thread_id));
    }
    let results: Vec<Thread> = tasks
        .join_all()
        .await
        .into_iter()
        .map(|i| i.unwrap())
        .collect();

    Ok(transform_threads(results))
}

/// Transform fetched Gmail threads into the public `EmailThread` shape.
fn transform_threads(results: Vec<Thread>) -> Vec<public::EmailThread> {
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

        // Gmail returns thread messages oldest first, so the thread's
        // headline (subject/from/received) is the newest message.
        let Some(latest) = messages.last() else {
            tracing::warn!("Thread {} has no messages; skipping", t.id);
            continue;
        };
        let received = latest.received.clone();
        let subject = latest.subject.clone();
        let from = latest.from.clone();
        let to = latest.to.clone();

        threads.push(public::EmailThread {
            id: t.id,
            received,
            subject,
            from,
            to,
            messages,
        })
    }

    threads.sort_by_key(|i| std::cmp::Reverse(i.received.clone()));
    threads
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq::google::gmail::{Message, MessageHeader, MessagePayload};

    fn message(id: &str, subject: &str, from: &str, to: &str, received: &str) -> Message {
        Message {
            id: id.to_string(),
            thread_id: "thr_1".to_string(),
            snippet: Some(format!("body of {id}")),
            payload: Some(MessagePayload {
                headers: Some(vec![
                    MessageHeader {
                        name: "From".to_string(),
                        value: from.to_string(),
                    },
                    MessageHeader {
                        name: "To".to_string(),
                        value: to.to_string(),
                    },
                    MessageHeader {
                        name: "Subject".to_string(),
                        value: subject.to_string(),
                    },
                ]),
                mimetype: "text/plain".to_string(),
                body: None,
                parts: None,
            }),
            label_ids: None,
            internal_date: received.to_string(),
        }
    }

    #[test]
    fn thread_headline_comes_from_newest_message() {
        let threads = transform_threads(vec![Thread {
            id: "thr_1".to_string(),
            messages: vec![
                message(
                    "m1",
                    "Original",
                    "alice@example.com",
                    "bob@example.com",
                    "1000",
                ),
                message(
                    "m2",
                    "Re: Original",
                    "bob@example.com",
                    "alice@example.com",
                    "2000",
                ),
            ],
        }]);

        assert_eq!(threads.len(), 1);
        let thread = &threads[0];
        assert_eq!(thread.subject, "Re: Original");
        assert_eq!(thread.from, "bob@example.com");
        assert_eq!(thread.to, "alice@example.com");
        assert_eq!(thread.received, "2000");
        assert_eq!(thread.messages.len(), 2);
    }

    #[test]
    fn threads_sort_by_newest_message() {
        let threads = transform_threads(vec![
            Thread {
                id: "old".to_string(),
                messages: vec![message(
                    "m1",
                    "Old",
                    "a@example.com",
                    "b@example.com",
                    "1000",
                )],
            },
            Thread {
                id: "new".to_string(),
                messages: vec![message(
                    "m2",
                    "New",
                    "a@example.com",
                    "b@example.com",
                    "2000",
                )],
            },
        ]);

        let ids: Vec<&str> = threads.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["new", "old"]);
    }
}
