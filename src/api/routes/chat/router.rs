//! Router for the chat API

use std::convert::Infallible;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::Json;
use axum::response::Response;
use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, sse::Event, sse::KeepAlive, sse::Sse},
    routing::{get, post},
};
use axum_extra::extract::Query;
use serde_json::json;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::UnboundedReceiverStream;
use uuid::Uuid;

use super::db::{chat_session_count, chat_session_list};
use crate::ai::chat::commands::{SlashCommand, get_help_text};
use crate::ai::chat::models::SessionMode;
use crate::ai::chat::{
    ChatBuilder, find_chat_session_by_id, get_or_create_session, insert_chat_message, set_session_mode
};
use crate::ai::tools::{
    CalendarTool, EmailUnreadTool, MeetingSearchTool, MemoryTool, NoteSearchTool,
    TasksDueTodayTool, TasksScheduledTodayTool, WebSearchTool, WebsiteViewTool,
};
use crate::anthropic::claude::{ClaudeCodeSession, Delta, StreamEvent};
use crate::api::state::AppState;
use crate::api::utils::DetectDisconnect;
use crate::core::AppConfig;
use crate::notify::{
    PushNotificationPayload, broadcast_push_notification, find_all_notification_subscriptions,
    mark_push_subscription_invalid,
};
use crate::openai::{BoxedToolCall, Message, Role};
use crate::search::index_single_chat_message;
// Re-export public types for this module
pub use super::public::{
    ChatRequest, ChatSessionsQuery, ChatSessionsResponse, ChatTranscriptResponse,
};

type SharedState = Arc<RwLock<AppState>>;

/// Get a single chat session by ID
async fn chat_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, crate::api::public::ApiError> {
    let db = state.read().expect("Unable to read share state").db.clone();
    let transcript_with_ids = find_chat_session_by_id(&db, &id).await?;
    let transcript: Vec<Message> = transcript_with_ids
        .into_iter()
        .map(|(_, msg)| msg)
        .collect();

    if transcript.is_empty() {
        return Ok((
            StatusCode::NOT_FOUND,
            format!("Chat session {} not found", id),
        )
            .into_response());
    }

    Ok(axum::Json(ChatTranscriptResponse { transcript }).into_response())
}

/// Get a list of all chat sessions
async fn chat_list(
    State(state): State<SharedState>,
    Query(params): Query<ChatSessionsQuery>,
) -> Result<Json<ChatSessionsResponse>, crate::api::public::ApiError> {
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

    Ok(axum::Json(ChatSessionsResponse {
        sessions: paged_sessions,
        page,
        limit,
        total_sessions,
        total_pages,
    }))
}

/// Handle messages in agent mode using Claude Code
async fn handle_agent_mode(
    state: SharedState,
    session_id: String,
    user_msg: Message,
    _tx: mpsc::UnboundedSender<String>,
    mut disconnect_receiver: broadcast::Receiver<()>,
    resume: bool,
) -> Result<Response, crate::api::public::ApiError> {
    let db = state.read().expect("Unable to read share state").db.clone();
    let _vapid_key_path = state
        .read()
        .expect("Unable to read shared state")
        .config
        .vapid_key_path
        .clone();

    // Create UUID from session_id for Claude Code
    let uuid = Uuid::parse_str(&session_id).unwrap_or_else(|_| Uuid::new_v4());

    // Create Claude Code session with default tools (Read, Edit, Bash)
    let claude_session = ClaudeCodeSession::with_default_tools(uuid);

    // Start the agent conversation
    let mut events = if resume {
        claude_session.resume(user_msg.content.as_deref().unwrap_or(""))
    } else {
        claude_session.start(user_msg.content.as_deref().unwrap_or(""))
    };

    let (sse_tx, sse_rx) = mpsc::unbounded_channel::<String>();
    let (disconnect_notifier, _) = broadcast::channel::<()>(1);

    // Spawn task to process events and forward to SSE channel
    tokio::spawn(async move {
        let mut full_response = String::new();

        while let Some(event_result) = tokio_stream::StreamExt::next(&mut events).await {
            match event_result {
                Ok(StreamEvent::ContentBlockDelta { delta }) => {
                    if let Delta::TextDelta { text } = delta {
                        full_response.push_str(&text);
                        // Send in same format as chat API
                        let chunk = json!({
                            "choices": [{
                                "delta": { "content": text }
                            }]
                        });
                        let _ = sse_tx.send(chunk.to_string());
                    }
                }
                Ok(StreamEvent::MessageStop) => {
                    // Store assistant response in transcript
                    let assistant_msg = Message::new(Role::Assistant, &full_response);
                    // Store user message in transcript
                    insert_chat_message(&db, &session_id, &user_msg).await.expect("Inserting user message failed");
                    insert_chat_message(&db, &session_id, &assistant_msg).await.expect("Inserting assistant message failed");
                }
                Ok(_) => {
                    // Other events we don't need to forward
                }
                Err(e) => {
                    tracing::error!("Claude Code error: {}", e);
                    let err_chunk = json!({
                        "choices": [{
                            "finish_reason": "error",
                            "delta": { "content": format!("Error: {}", e) }
                        }]
                    });
                    let _ = sse_tx.send(err_chunk.to_string());
                }
            }
        }

        // Send notification if client disconnected
        if sse_tx.is_closed() {
            let _ = disconnect_receiver.recv().await.map(|()| {
                tracing::info!("Sending notification for agent response!");
            });
        }
    });

    let sse_stream = tokio_stream::StreamExt::map(UnboundedReceiverStream::new(sse_rx), |chunk| {
        Ok::<Event, Infallible>(Event::default().data(chunk))
    });
    let wrapped_sse_stream = DetectDisconnect::new(sse_stream, disconnect_notifier);

    Ok(Sse::new(wrapped_sse_stream)
        .keep_alive(
            KeepAlive::default()
                .text("keep-alive")
                .interval(Duration::from_millis(100)),
        )
        .into_response())
}

/// Initiate or add to a chat session and stream the response
async fn chat_handler(
    State(state): State<SharedState>,
    Json(payload): Json<ChatRequest>,
) -> Result<impl IntoResponse, crate::api::public::ApiError> {
    let session_id = payload.session_id;
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let (disconnect_notifier, mut disconnect_receiver) = broadcast::channel::<()>(1);

    let db = state.read().expect("Unable to read share state").db.clone();

    let (
        note_search_tool,
        meeting_search_tool,
        web_search_tool,
        email_unread_tool,
        calendar_tool,
        website_view_tool,
        tasks_due_today_tool,
        tasks_scheduled_today_tool,
        memory_tool,
        openai_api_hostname,
        openai_api_key,
        openai_model,
        vapid_key_path,
        index_dir_path,
    ) = {
        let shared_state = state.read().expect("Unable to read share state");
        let AppConfig {
            note_search_api_url,
            storage_path,
            openai_api_hostname,
            openai_api_key,
            openai_model,
            vapid_key_path,
            ..
        } = &shared_state.config;
        let index_dir_path = format!("{}/index", storage_path);
        (
            NoteSearchTool::new(note_search_api_url),
            MeetingSearchTool::new(note_search_api_url),
            WebSearchTool::new(note_search_api_url),
            EmailUnreadTool::new(note_search_api_url),
            CalendarTool::new(db.clone(), note_search_api_url),
            WebsiteViewTool::new(),
            TasksDueTodayTool::new(note_search_api_url),
            TasksScheduledTodayTool::new(note_search_api_url),
            MemoryTool::new(storage_path),
            openai_api_hostname.clone(),
            openai_api_key.clone(),
            openai_model.clone(),
            vapid_key_path.clone(),
            index_dir_path,
        )
    };

    let tools: Vec<BoxedToolCall> = vec![
        Box::new(note_search_tool),
        Box::new(meeting_search_tool),
        Box::new(web_search_tool),
        Box::new(email_unread_tool),
        Box::new(calendar_tool),
        Box::new(website_view_tool),
        Box::new(tasks_due_today_tool),
        Box::new(tasks_scheduled_today_tool),
        Box::new(memory_tool),
    ];
    let user_msg = Message::new(Role::User, &payload.message);

    let db = state.read().expect("Unable to read share state").db.clone();

    // Parse message using slash command system
    let slash_cmd_str = payload.message.as_str();
    let slash_command = SlashCommand::from_str(slash_cmd_str)?;
    let session = get_or_create_session(&db, &session_id, &[], SessionMode::Chat).await?;
    let current_mode = session.mode;

    // Handle mode transitions
    match (&current_mode, &slash_command) {
        (SessionMode::Chat, SlashCommand::Code { prompt }) => {
            // Transition from chat to agent mode
            set_session_mode(&db, &session_id, SessionMode::Code).await?;
            let user_msg = Message::new(Role::User, prompt);
            return handle_agent_mode(state, session_id, user_msg, tx, disconnect_receiver, false)
                .await;
        }
        (SessionMode::Code, SlashCommand::Exit) => {
            // Exit agent mode back to chat
            set_session_mode(&db, &session_id, SessionMode::Chat).await?;

            // Store the user's /exit message
            let user_msg = Message::new(Role::User, "/exit");
            insert_chat_message(&db, &session_id, &user_msg).await?;

            // Store the assistant's exit response
            let exit_response = "Exited agent mode. How else can I help?";
            let assistant_msg = Message::new(Role::Assistant, exit_response);
            insert_chat_message(&db, &session_id, &assistant_msg).await?;

            tx.send(
                json!({
                    "choices": [{
                        "delta": { "content": exit_response }
                    }]
                })
                .to_string(),
            )?;

            // Return early to avoid falling through to chat logic
            let sse_stream =
                StreamExt::map(UnboundedReceiverStream::new(rx), |chunk| {
                    Ok::<Event, Infallible>(Event::default().data(chunk))
                });
            let wrapped_sse_stream = DetectDisconnect::new(sse_stream, disconnect_notifier);
            return Ok(Sse::new(wrapped_sse_stream)
                .keep_alive(
                    KeepAlive::default()
                        .text("keep-alive")
                        .interval(Duration::from_millis(100)),
                )
                .into_response());
        }
        (SessionMode::Code, SlashCommand::None(msg)) => {
            // Continue in agent mode
            let user_msg = Message::new(Role::User, msg);
            return handle_agent_mode(state, session_id, user_msg, tx, disconnect_receiver, true)
                .await;
        }
        (SessionMode::Code, SlashCommand::Code { prompt }) => {
            // Already in agent mode but got another /code command
            let user_msg = Message::new(Role::User, prompt);
            return handle_agent_mode(state, session_id, user_msg, tx, disconnect_receiver, true)
                .await;
        }
        (SessionMode::Chat, SlashCommand::Exit) => {
            // Already in chat mode, /exit is a no-op - store messages for consistency
            let exit_msg = "Already in chat mode. How can I help?";
            tx.send(
                json!({
                    "choices": [{
                        "delta": { "content": exit_msg }
                    }]
                })
                .to_string(),
            )?;
            // Return early - don't fall through to chat logic
            let sse_stream =
                tokio_stream::StreamExt::map(UnboundedReceiverStream::new(rx), |chunk| {
                    Ok::<Event, Infallible>(Event::default().data(chunk))
                });
            let wrapped_sse_stream = DetectDisconnect::new(sse_stream, disconnect_notifier);
            return Ok(Sse::new(wrapped_sse_stream)
                .keep_alive(
                    KeepAlive::default()
                        .text("keep-alive")
                        .interval(Duration::from_millis(100)),
                )
                .into_response());
        }
        (SessionMode::Chat, SlashCommand::None(_)) => {
            // Continue in chat mode - fall through to existing logic
        }
        (_, SlashCommand::Error(err_msg)) => {
            tx.send(
                json!({
                    "choices": [{
                        "delta": { "content": err_msg }
                    }]
                })
                .to_string(),
            )?;
            // Return early - don't fall through
            let sse_stream =
                tokio_stream::StreamExt::map(UnboundedReceiverStream::new(rx), |chunk| {
                    Ok::<Event, Infallible>(Event::default().data(chunk))
                });
            let wrapped_sse_stream = DetectDisconnect::new(sse_stream, disconnect_notifier);
            return Ok(Sse::new(wrapped_sse_stream)
                .keep_alive(
                    KeepAlive::default()
                        .text("keep-alive")
                        .interval(Duration::from_millis(100)),
                )
                .into_response());
        }
        (_, SlashCommand::Help) => {
            // Show help text - same in both modes
            let help_msg = get_help_text();
            tx.send(
                json!({
                    "choices": [{
                        "delta": { "content": help_msg }
                    }]
                })
                .to_string(),
            )?;
            // Return early - don't fall through
            let sse_stream =
                tokio_stream::StreamExt::map(UnboundedReceiverStream::new(rx), |chunk| {
                    Ok::<Event, Infallible>(Event::default().data(chunk))
                });
            let wrapped_sse_stream = DetectDisconnect::new(sse_stream, disconnect_notifier);
            return Ok(Sse::new(wrapped_sse_stream)
                .keep_alive(
                    KeepAlive::default()
                        .text("keep-alive")
                        .interval(Duration::from_millis(100)),
                )
                .into_response());
        }
    }

    // Create SSE stream after mode detection (for non-early-return cases)
    let sse_stream = tokio_stream::StreamExt::map(UnboundedReceiverStream::new(rx), |chunk| {
        Ok::<Event, Infallible>(Event::default().data(chunk))
    });
    let wrapped_sse_stream = DetectDisconnect::new(sse_stream, disconnect_notifier);

    // Create session in database if it doesn't already exist

    // Try to fetch the session from the db
    let transcript_with_ids = find_chat_session_by_id(&db, &session_id).await?;
    let mut transcript: Vec<Message> = transcript_with_ids
        .into_iter()
        .map(|(_, msg)| msg)
        .collect();

    // Initialize a new transcript
    if transcript.is_empty() {
        let shared_state = state.read().expect("Unable to read share state");
        let default_system_msg = Message::new(Role::System, &shared_state.config.system_message);
        transcript.push(default_system_msg.clone());
    }

    let mut chat = ChatBuilder::new(&openai_api_hostname, &openai_api_key, &openai_model)
        .database(&db, Some(&session_id), None)
        .transcript(transcript)
        .tools(tools)
        .streaming(tx.clone())
        .build();

    tokio::spawn(async move {
        let result = chat.next_msg(user_msg.clone()).await;
        match result {
            Ok(messages) => {
                // Index new chat messages for full-text search
                let db_clone = db.clone();
                let index_dir_path_clone = index_dir_path.clone();
                let session_id_clone = session_id.clone();
                for msg in messages.iter() {
                    let db_inner = db_clone.clone();
                    let index_dir_path_inner = index_dir_path_clone.clone();
                    let session_id_inner = session_id_clone.clone();
                    let msg_clone = msg.clone();
                    tokio::spawn(async move {
                        if let Err(e) = index_single_chat_message(
                            &db_inner,
                            &index_dir_path_inner,
                            &session_id_inner,
                            &msg_clone,
                        )
                        .await
                        {
                            tracing::error!("Failed to index chat message: {}", e);
                        }
                    });
                }
                // Send a notification if the client disconnected
                if tx.is_closed() {
                    let _ = disconnect_receiver.recv().await;
                    tracing::info!("Sending notification!");
                    let db_for_notify = db.clone();
                    let vapid_key_path_for_notify = vapid_key_path.to_string();
                    let session_id_for_notify = session_id.clone();
                    tokio::spawn(async move {
                        let payload = PushNotificationPayload::new(
                            "New chat response",
                            "New response after you disconnected.",
                            Some(&format!("/chat/?session_id={session_id_for_notify}")),
                            None,
                            None,
                        );
                        let subscriptions = find_all_notification_subscriptions(&db_for_notify)
                            .await
                            .unwrap();
                        let failed_subscriptions = broadcast_push_notification(
                            subscriptions,
                            vapid_key_path_for_notify,
                            payload,
                        )
                        .await;
                        for sub in failed_subscriptions {
                            let _ =
                                mark_push_subscription_invalid(&db_for_notify, &sub.endpoint).await;
                        }
                    });
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

/// Create the chat router
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/", post(chat_handler))
        .route("/{id}", get(chat_session))
        .route("/sessions", get(chat_list))
}
