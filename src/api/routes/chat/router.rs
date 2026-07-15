//! Router for the chat API

use std::convert::Infallible;
use std::path::PathBuf;
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
    find_chat_session_by_id, get_or_create_session, insert_chat_message, set_session_mode,
};
use crate::ai::tools::run_in_sandbox;
use crate::anthropic::claude::{ClaudeCodeSession, Delta, StreamEvent};
use crate::api::state::AppState;
use crate::api::utils::DetectDisconnect;
use crate::openai::{Message, Role};
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
                        // Skip sending if client disconnected
                        if !sse_tx.is_closed() {
                            let _ = sse_tx.send(chunk.to_string());
                        } else {
                            break;
                        }
                    }
                }
                Ok(StreamEvent::MessageStop) => {
                    // Store assistant response in transcript
                    let assistant_msg = Message::new(Role::Assistant, &full_response);
                    // Store user message in transcript
                    insert_chat_message(&db, &session_id, &user_msg)
                        .await
                        .expect("Inserting user message failed");
                    insert_chat_message(&db, &session_id, &assistant_msg)
                        .await
                        .expect("Inserting assistant message failed");
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
                    // Skip sending if client disconnected
                    if !sse_tx.is_closed() {
                        let _ = sse_tx.send(err_chunk.to_string());
                    }
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
    let (disconnect_notifier, disconnect_receiver) = broadcast::channel::<()>(1);

    let db = state.read().expect("Unable to read share state").db.clone();

    // Variables needed by slash command paths below (skill listing,
    // bash sandbox workspace). Tool construction for chat turns is now
    // handled by ChatTask — see src/ai/chat/session.rs.
    let skill_registry = state
        .read()
        .expect("Unable to read share state")
        .skill_registry
        .clone();
    let storage_path_owned = state
        .read()
        .expect("Unable to read share state")
        .config
        .storage_path
        .clone();
    let user_msg = Message::new(Role::User, &payload.message);

    // Parse message using slash command system
    let slash_cmd = payload.message.parse()?;
    let session = get_or_create_session(&db, &session_id, &[], SessionMode::Chat).await?;
    let current_mode = session.mode;

    // Handle mode transitions
    match (&current_mode, &slash_cmd) {
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

            let _ = tx.send(
                json!({
                    "choices": [{
                        "delta": { "content": exit_response }
                    }]
                })
                .to_string(),
            );

            // Return early to avoid falling through to chat logic
            let sse_stream = StreamExt::map(UnboundedReceiverStream::new(rx), |chunk| {
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
            let _ = tx.send(
                json!({
                    "choices": [{
                        "delta": { "content": exit_msg }
                    }]
                })
                .to_string(),
            );
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
        (SessionMode::Code, SlashCommand::Skill { name: _name }) => {
            tracing::warn!(
                "Attempted to activate a skill within coding agent session mode which is not allowed."
            );
        }
        (SessionMode::Code, SlashCommand::Bash { .. }) => {
            tracing::warn!(
                "Attempted to run /bash within coding agent session mode which is not allowed."
            );
        }
        (SessionMode::Chat, SlashCommand::Skill { name }) => {
            // List all skills or show a specific skill's content
            let (response, persist_skill_msg) = if let Some(skill_name) = name {
                let registry = skill_registry
                    .read()
                    .expect("Unable to read skill registry")
                    .clone();

                match registry.load_skill(&skill_name).await {
                    Ok(skill) => {
                        let skill_msg = format!(
                            "<skill>\n<name>{name}</name>\n<path>{path}</path>\n{content}</skill>",
                            name = skill_name,
                            path = skill.path.display(),
                            content = skill.full_content(),
                        );

                        (skill_msg, true)
                    }
                    Err(e) => (e.to_string(), false),
                }
            } else {
                let registry = skill_registry
                    .read()
                    .expect("Unable to read skill registry");
                let skills = registry.list_skills();
                if skills.is_empty() {
                    ("No skills available.".to_string(), false)
                } else {
                    let skill_list: Vec<String> = skills
                        .iter()
                        .map(|s| format!("- **{}**: {}", s.name, s.description))
                        .collect();
                    (
                        format!(
                            "Available skills:\n\n{}\n\nUse `/skills <name>` to view a specific skill.",
                            skill_list.join("\n")
                        ),
                        false,
                    )
                }
            };

            if persist_skill_msg {
                let user_msg = Message::new(Role::User, &response);
                insert_chat_message(&db, &session_id, &user_msg).await?;
            }
            let _ = tx.send(
                json!({
                    "choices": [{
                        "delta": { "content": response }
                    }]
                })
                .to_string(),
            );
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
        (SessionMode::Chat, SlashCommand::Bash { command }) => {
            // Run a command in the bashkit sandbox and return the output
            let workspace_path =
                PathBuf::from(format!("{}/workspace/{}", storage_path_owned, session_id));
            let response = match run_in_sandbox(&command, &workspace_path).await {
                Ok(output) => {
                    let mut parts = Vec::new();
                    parts.push("```".to_string());
                    parts.push(format!("$ {}\n", command));
                    if !output.stdout.is_empty() {
                        parts.push(output.stdout);
                    }
                    if !output.stderr.is_empty() {
                        parts.push(format!("stderr:\n{}", output.stderr));
                    }
                    parts.push(format!("---\nExit code: {}", output.exit_code));
                    if output.truncated {
                        parts.push("*Output was truncated due to size limits.*".to_string());
                    }
                    parts.push("```".to_string());
                    parts.join("\n")
                }
                Err(e) => format!("```\nError running command: {}\n```", e),
            };
            let _ = tx.send(
                json!({
                    "choices": [{
                        "delta": { "content": response }
                    }]
                })
                .to_string(),
            );
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
        (SessionMode::Chat, SlashCommand::None(_)) => {
            // Route through ChatTask — it owns the long-lived transcript
            // and handles streaming (via tx) + push notification on
            // disconnect. This replaces the inline ChatBuilder + next_msg
            // logic that used to run here.
            let chat_sessions = state
                .read()
                .expect("Unable to read share state")
                .chat_sessions
                .clone();
            // Move tx into the command; rx stays here for the SSE stream.
            chat_sessions.send_http(&session_id, user_msg, tx);

            let sse_stream = tokio_stream::StreamExt::map(
                UnboundedReceiverStream::new(rx),
                |chunk| Ok::<Event, Infallible>(Event::default().data(chunk)),
            );
            let wrapped_sse_stream = DetectDisconnect::new(sse_stream, disconnect_notifier);
            return Ok(Sse::new(wrapped_sse_stream)
                .keep_alive(
                    KeepAlive::default()
                        .text("keep-alive")
                        .interval(Duration::from_millis(100)),
                )
                .into_response());
        }
        (_, SlashCommand::Error(err_msg)) => {
            let _ = tx.send(
                json!({
                    "choices": [{
                        "delta": { "content": err_msg }
                    }]
                })
                .to_string(),
            );
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
            let _ = tx.send(
                json!({
                    "choices": [{
                        "delta": { "content": help_msg }
                    }]
                })
                .to_string(),
            );
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

    // Unreachable — every match arm returns early. The (Chat, None)
    // arm routes through ChatTask and returns its own SSE response.
    unreachable!("chat_handler match should cover all cases")
}

/// Create the chat router
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/", post(chat_handler))
        .route("/{id}", get(chat_session))
        .route("/sessions", get(chat_list))
}
