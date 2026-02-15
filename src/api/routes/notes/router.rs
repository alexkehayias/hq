//! Router for the notes API

use std::convert::Infallible;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, sse::Event, sse::KeepAlive, sse::Sse},
    routing::{get, post},
};
use axum_extra::extract::Query;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::ai::tools::{
    CalendarTool, EmailUnreadTool, NoteSearchTool, TasksDueTodayTool, TasksScheduledTodayTool,
    WebSearchTool, WebsiteViewTool,
};
use crate::api::routes::notes::db as notes_db;
use crate::api::state::AppState;
use crate::core::AppConfig;
use crate::notify::{
    PushNotificationPayload, broadcast_push_notification, find_all_notification_subscriptions,
};
use crate::openai::{BoxedToolCall, Message, Role};
use crate::openai::{
    chat_session_count, chat_session_list, chat_stream, find_chat_session_by_id,
    get_or_create_session, insert_chat_message,
};
use crate::search::aql;
use crate::search::index_all;
use crate::search::search_notes;
use super::public;

type SharedState = Arc<RwLock<AppState>>;

// Note search endpoint
async fn note_search(
    State(state): State<SharedState>,
    Query(params): Query<public::SearchRequest>,
) -> Result<axum::Json<public::SearchResponse>, crate::api::public::ApiError> {
    let raw_query = params.query;
    let query = aql::parse_query(&raw_query).expect("Parsing AQL failed");
    let (db, index_path) = {
        let shared_state = state.read().unwrap();
        (
            shared_state.db.clone(),
            shared_state.config.index_path.clone(),
        )
    };

    let results = search_notes(
        &index_path,
        &db,
        params.include_similarity,
        params.truncate,
        &query,
        params.limit,
    )
    .await?;

    let resp = public::SearchResponse {
        raw_query: raw_query.to_string(),
        parsed_query: format!("{:?}", query),
        results,
    };

    Ok(axum::Json(resp))
}

// Index notes endpoint
async fn index_notes(
    State(state): State<SharedState>,
) -> Result<axum::Json<Value>, crate::api::public::ApiError> {
    let (a_db, index_path, notes_path, deploy_key_path) = {
        let shared_state = state.read().expect("Unable to read share state");
        (
            shared_state.db.clone(),
            shared_state.config.index_path.clone(),
            shared_state.config.notes_path.clone(),
            shared_state.config.deploy_key_path.clone(),
        )
    };
    tokio::spawn(async move {
        crate::core::git::maybe_pull_and_reset_repo(&deploy_key_path, &notes_path).await;
        let diff = crate::core::git::diff_last_commit_files(&deploy_key_path, &notes_path).await;
        let paths: Vec<std::path::PathBuf> = diff
            .iter()
            .map(|f| std::path::PathBuf::from(format!("{}/{}", &notes_path, f)))
            .collect();
        let filter_paths = if paths.is_empty() { None } else { Some(paths) };
        index_all(&a_db, &index_path, &notes_path, true, true, filter_paths)
            .await
            .unwrap();
    });
    Ok(axum::Json(json!({ "success": true })))
}

// View note endpoint
async fn view_note(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<axum::Json<public::ViewNoteResponse>, crate::api::public::ApiError> {
    let db = state.read().unwrap().db.clone();
    let note_result = notes_db::get_note_by_id(&db, id).await?;
    Ok(axum::Json(note_result))
}

// Get a single chat session by ID
async fn chat_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, crate::api::public::ApiError> {
    let db = state.read().expect("Unable to read share state").db.clone();
    let transcript = find_chat_session_by_id(&db, &id).await?.to_owned();

    if transcript.is_empty() {
        return Ok((
            StatusCode::NOT_FOUND,
            format!("Chat session {} not found", id),
        )
            .into_response());
    }

    Ok(axum::Json(public::ChatTranscriptResponse { transcript }).into_response())
}

/// Get a list of all chat sessions
async fn chat_list(
    State(state): State<SharedState>,
    Query(params): Query<public::ChatSessionsQuery>,
) -> Result<axum::Json<public::ChatSessionsResponse>, crate::api::public::ApiError> {
    let db = state.read().expect("Unable to read share state").db.clone();
    let page = params.page.unwrap_or(1);
    let limit = params.limit.unwrap_or(20);
    let offset = (page - 1) * limit;
    let include_tags = params.tags.unwrap_or(vec![]);
    let exclude_tags = params.exclude_tags.unwrap_or(vec![]);
    let total_sessions = chat_session_count(&db, &include_tags, &exclude_tags).await?;
    let paged_sessions =
        chat_session_list(&db, &include_tags, &exclude_tags, limit, offset).await?;
    let total_pages = (total_sessions as f64 / limit as f64).ceil() as i64;

    Ok(axum::Json(public::ChatSessionsResponse {
        sessions: paged_sessions,
        page,
        limit,
        total_sessions,
        total_pages,
    }))
}

/// Initiate or add to a chat session and stream the response
async fn chat_handler(
    State(state): State<SharedState>,
    axum::Json(payload): axum::Json<public::ChatRequest>,
) -> Result<impl IntoResponse, crate::api::public::ApiError> {
    use crate::api::utils::DetectDisconnect;

    let session_id = payload.session_id;
    let (tx, rx) = mpsc::unbounded_channel::<String>();

    let sse_stream = UnboundedReceiverStream::new(rx)
        .map(|chunk| Ok::<Event, Infallible>(Event::default().data(chunk)));
    let (disconnect_notifier, mut disconnect_receiver) = broadcast::channel::<()>(1);
    let wrapped_sse_stream = DetectDisconnect::new(sse_stream, disconnect_notifier);

    let (
        note_search_tool,
        web_search_tool,
        email_unread_tool,
        calendar_tool,
        website_view_tool,
        tasks_due_today_tool,
        tasks_scheduled_today_tool,
        openai_api_hostname,
        openai_api_key,
        openai_model,
        vapid_key_path,
    ) = {
        let shared_state = state.read().expect("Unable to read share state");
        let AppConfig {
            note_search_api_url,
            openai_api_hostname,
            openai_api_key,
            openai_model,
            vapid_key_path,
            ..
        } = &shared_state.config;
        (
            NoteSearchTool::new(note_search_api_url),
            WebSearchTool::new(note_search_api_url),
            EmailUnreadTool::new(note_search_api_url),
            CalendarTool::new(note_search_api_url),
            WebsiteViewTool::new(),
            TasksDueTodayTool::new(note_search_api_url),
            TasksScheduledTodayTool::new(note_search_api_url),
            openai_api_hostname.clone(),
            openai_api_key.clone(),
            openai_model.clone(),
            vapid_key_path.clone(),
        )
    };

    let tools: Option<Vec<BoxedToolCall>> = Some(vec![
        Box::new(note_search_tool),
        Box::new(web_search_tool),
        Box::new(email_unread_tool),
        Box::new(calendar_tool),
        Box::new(website_view_tool),
        Box::new(tasks_due_today_tool),
        Box::new(tasks_scheduled_today_tool),
    ]);
    let user_msg = Message::new(Role::User, &payload.message);

    let db = state.read().expect("Unable to read share state").db.clone();

    // Create session in database if it doesn't already exist
    get_or_create_session(&db, &session_id, &[]).await?;

    // Try to fetch the session from the db
    let mut transcript = find_chat_session_by_id(&db, &session_id).await?;

    // Initialize a new transcript
    if transcript.is_empty() {
        let shared_state = state.read().expect("Unable to read share state");
        let default_system_msg = Message::new(Role::System, &shared_state.config.system_message);
        transcript.push(default_system_msg.clone());
    }

    // Add the new message to the transcript
    transcript.push(user_msg.clone());

    // Get the next response
    tokio::spawn(async move {
        let result = chat_stream(
            tx.clone(),
            &tools,
            &transcript,
            &openai_api_hostname,
            &openai_api_key,
            &openai_model,
        )
        .await;

        match result {
            Ok(messages) => {
                // Write the user's message to the DB
                insert_chat_message(&db, &session_id, &user_msg).await?;
                // Write new messages that were generated by the chat
                for m in messages {
                    insert_chat_message(&db, &session_id, &m).await?;
                }
                // Send a notification if the client disconnected
                if tx.is_closed() {
                    let _ = disconnect_receiver
                        .recv()
                        .await
                        .map(async |()| {
                            tracing::info!("Sending notification!");
                            let payload = PushNotificationPayload::new(
                                "New chat response",
                                "New response after you disconnected.",
                                Some(&format!("/chat/?session_id={session_id}")),
                                None,
                                None,
                            );
                            let subscriptions =
                                find_all_notification_subscriptions(&db).await.unwrap();
                            broadcast_push_notification(
                                subscriptions,
                                vapid_key_path.to_string(),
                                payload,
                            )
                            .await;
                        })?
                        .await;
                };
            }
            Err(e) => {
                tracing::error!("Chat handler error: {}. Root cause: {}", e, e.root_cause());

                let err_msg = format!("Something went wrong: {}", e);
                let completion_chunk = json!({
                    "id": "error",
                    "choices": [
                        {
                            "finish_reason": "error",
                            "delta": { "content": err_msg }
                        }
                    ]
                })
                .to_string();
                tx.send(completion_chunk)?;
            }
        }

        Ok::<(), anyhow::Error>(())
    });

    let resp = Sse::new(wrapped_sse_stream)
        .keep_alive(
            KeepAlive::default()
                .text("keep-alive")
                .interval(Duration::from_millis(100)),
        )
        .into_response();

    Ok(resp)
}

/// Create the notes router
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/search", get(note_search))
        .route("/index", post(index_notes))
        .route("/{id}/view", get(view_note))
        .route("/chat", post(chat_handler))
        .route("/chat/{id}", get(chat_session))
        .route("/chat/sessions", get(chat_list))
}
