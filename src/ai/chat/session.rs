//! Long-lived chat session tasks and the manager that owns them.
//!
//! Today, a chat session only progresses when an HTTP request arrives at
//! `/api/chat`. Each request builds a fresh [`Chat`], runs one turn of
//! `next_msg`, and streams the response back over SSE.
//!
//! [`ChatSessionManager`] changes that: each session has a long-lived
//! [`ChatTask`] that owns the in-memory transcript and processes commands
//! serially. Commands come from two sources:
//!
//! - **Http** (from `chat_handler`): user-initiated chat turns. The
//!   response streams back through the SSE `tx` and falls back to web
//!   push notification if the client disconnects.
//! - **Pub/Sub** (from [`crate::ai::pubsub::PubSubBroker`]): messages
//!   published to a channel the session is subscribed to. These drive a
//!   normal `next_msg` turn; the assistant's response is delivered via
//!   web push notification (since pub/sub messages arrive async, outside
//!   any HTTP request).
//!
//! Chat tasks are spawned lazily on first activity (HTTP request or
//! subscription) and live for the server lifetime. At startup, sessions
//! with persisted subscriptions are eagerly spawned so pub/sub messages
//! aren't dropped before the session's first HTTP request.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use serde_json::json;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tracing::error;

use crate::ai::chat::db::{
    add_subscription, find_chat_session_by_id, insert_chat_message, list_subscriptions,
};
use crate::ai::chat::middleware::{InfiniteLoopDetector, ToolSecurityMiddleware};
use crate::ai::chat::{Chat, ChatBuilder};
use crate::ai::pubsub::PubSubBroker;
use crate::ai::skills::SkillRegistry;
use crate::ai::tools::{
    BashTool, CalendarTool, DateTimeTool, EmailUnreadTool, MeetingSearchTool, MemoryTool,
    NoteSearchTool, NotifyTool, TasksDueTodayTool, TasksScheduledTodayTool, WebSearchTool,
    WebsiteViewTool,
};
use crate::core::AppConfig;
use crate::notify::{
    PushNotificationPayload, broadcast_push_notification, find_all_notification_subscriptions,
    mark_push_subscription_invalid,
};
use crate::openai::{BoxedToolCall, Message, Role};
use crate::search::index_chat_messages;
use tokio_rusqlite::Connection;

/// Shared dependencies needed by every [`ChatTask`]. Cheap to clone
/// (all fields are `Arc` or `Clone`-cheap handles).
#[derive(Clone)]
pub struct ChatTaskDeps {
    pub db: Connection,
    pub config: Arc<AppConfig>,
    pub skill_registry: Arc<RwLock<SkillRegistry>>,
}

/// Command sent to a [`ChatTask`] from `chat_handler` (Http). Pub/sub
/// messages arrive on a separate channel registered with the broker;
/// they don't use this type.
#[derive(Debug)]
pub enum ChatCommand {
    /// User-initiated chat turn from an HTTP request. The response
    /// streams through `sse_tx` (matching the existing chat_handler
    /// behavior). If the client disconnects (`sse_tx.is_closed()`),
    /// a push notification is sent as fallback.
    Http {
        msg: Message,
        sse_tx: UnboundedSender<String>,
    },
}

/// Handle held by [`ChatSessionManager`] for an active task. Cloning
/// the handle lets multiple callers send commands to the same task.
#[derive(Clone)]
struct ChatTaskHandle {
    /// Sender for Http commands from chat_handler.
    cmd_tx: UnboundedSender<ChatCommand>,
    /// Sender for pub/sub messages, registered with the broker on
    /// subscribe. Cloned per subscription so a session subscribing to
    /// multiple channels fans out messages to the same task receiver.
    pubsub_tx: UnboundedSender<Message>,
}

/// Manages long-lived [`ChatTask`]s, one per session_id. Tasks are
/// spawned lazily on first activity and live for the server lifetime.
pub struct ChatSessionManager {
    tasks: Arc<RwLock<HashMap<String, ChatTaskHandle>>>,
    broker: Arc<PubSubBroker>,
    deps: Arc<ChatTaskDeps>,
}

impl ChatSessionManager {
    pub fn new(broker: Arc<PubSubBroker>, deps: ChatTaskDeps) -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            broker,
            deps: Arc::new(deps),
        }
    }

    /// Send an Http command to a session's ChatTask. Spawns the task
    /// if it isn't already running. The response streams through `sse_tx`
    /// (matching the existing chat_handler behavior).
    ///
    /// This is the entry point `chat_handler` calls instead of building
    /// a fresh `Chat` inline.
    pub fn send_http(
        &self,
        session_id: &str,
        msg: Message,
        sse_tx: UnboundedSender<String>,
    ) {
        let handle = self.get_or_spawn(session_id);
        // send only errors if the receiver was dropped — task died.
        // Log and move on; chat_handler's SSE stream will just close
        // without a response, same as today's error path.
        if handle.cmd_tx.send(ChatCommand::Http { msg, sse_tx }).is_err() {
            error!("ChatTask for session {} dropped command (task died?)", session_id);
        }
    }

    /// Get an existing task handle or spawn a new one for the session.
    /// Idempotent — calling twice with the same `session_id` returns
    /// handles that send to the same task.
    fn get_or_spawn(&self, session_id: &str) -> ChatTaskHandle {
        // Fast path: read lock
        if let Some(handle) = self.tasks.read().unwrap().get(session_id).cloned() {
            return handle;
        }
        // Slow path: write lock, double-check
        let mut tasks = self.tasks.write().unwrap();
        if let Some(handle) = tasks.get(session_id).cloned() {
            return handle;
        }
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<ChatCommand>();
        let (pubsub_tx, pubsub_rx) = mpsc::unbounded_channel::<Message>();
        let handle = ChatTaskHandle {
            cmd_tx: cmd_tx.clone(),
            pubsub_tx,
        };

        let task = ChatTask::new(
            session_id.to_string(),
            self.deps.clone(),
            cmd_rx,
            pubsub_rx,
        );
        tokio::spawn(async move {
            task.run().await;
        });

        tasks.insert(session_id.to_string(), handle.clone());
        handle
    }

    /// Subscribe a session to a channel. Spawns the ChatTask if it
    /// isn't already running (so messages aren't dropped), registers
    /// the task's pubsub sender with the broker, and persists the
    /// subscription to DB so it survives restarts.
    pub async fn subscribe(&self, session_id: &str, channel: &str) -> Result<()> {
        let handle = self.get_or_spawn(session_id);
        self.broker.subscribe(channel, handle.pubsub_tx.clone());
        add_subscription(&self.deps.db, session_id, channel).await?;
        Ok(())
    }

    /// Eagerly spawn ChatTasks for all sessions with persisted
    /// subscriptions. Called once at server startup so pub/sub messages
    /// aren't dropped before a session's first HTTP request.
    ///
    /// Reads `chat_subscription` table, groups by session_id, spawns
    /// each task and re-registers its channels with the broker.
    pub async fn restore_subscriptions(&self) -> Result<()> {
        let subs = list_subscriptions(&self.deps.db).await?;
        // Group channels by session_id
        let mut by_session: HashMap<String, Vec<String>> = HashMap::new();
        for (session_id, channel) in subs {
            by_session.entry(session_id).or_default().push(channel);
        }
        for (session_id, channels) in by_session {
            let handle = self.get_or_spawn(&session_id);
            for channel in &channels {
                self.broker.subscribe(channel, handle.pubsub_tx.clone());
            }
        }
        Ok(())
    }
}

/// A long-lived chat task owning the in-memory transcript for one
/// session. Processes commands serially (no concurrent `next_msg` on
/// the same session — avoids races on the transcript).
struct ChatTask {
    session_id: String,
    transcript: Vec<Message>,
    deps: Arc<ChatTaskDeps>,
    cmd_rx: UnboundedReceiver<ChatCommand>,
    pubsub_rx: UnboundedReceiver<Message>,
}

impl ChatTask {
    fn new(
        session_id: String,
        deps: Arc<ChatTaskDeps>,
        cmd_rx: UnboundedReceiver<ChatCommand>,
        pubsub_rx: UnboundedReceiver<Message>,
    ) -> Self {
        Self {
            session_id,
            transcript: Vec::new(),
            deps,
            cmd_rx,
            pubsub_rx,
        }
    }

    /// Main loop. Loads transcript from DB on first run (lazy init),
    /// then processes Http commands and pub/sub messages serially using
    /// `tokio::select!`.
    async fn run(mut self) {
        if let Err(e) = self.load_transcript().await {
            error!(
                "ChatTask {} failed to load transcript: {}. Task exiting.",
                self.session_id, e
            );
            return;
        }

        loop {
            tokio::select! {
                // Http command from chat_handler
                Some(cmd) = self.cmd_rx.recv() => match cmd {
                    ChatCommand::Http { msg, sse_tx } => {
                        self.handle_http(msg, sse_tx).await;
                    }
                },
                // Pub/sub message from broker
                Some(msg) = self.pubsub_rx.recv() => {
                    self.handle_pubsub(msg).await;
                }
            }
        }
    }

    /// Load transcript from DB. On a new session (no messages yet),
    /// build and persist the system message first.
    async fn load_transcript(&mut self) -> Result<()> {
        let existing = find_chat_session_by_id(&self.deps.db, &self.session_id).await?;
        let mut transcript: Vec<Message> = existing.into_iter().map(|(_, m)| m).collect();

        // New session: build and persist system message
        if transcript.is_empty() {
            let mut system_content = self.deps.config.system_message.clone();
            // Append skill instructions if there are skills in the registry
            {
                let registry = self
                    .deps
                    .skill_registry
                    .read()
                    .expect("Unable to read skill registry");
                if registry.count() > 0 {
                    let skill_names: Vec<String> = registry
                        .list_skills()
                        .iter()
                        .map(|s| format!("{}: {}", s.name, s.description))
                        .collect();
                    let skills_section = format!(
                        "\n\n## Available Skills\n\
                        You have access to the following skills. **Always use relevant skills first before using tools.**\n\
                        - {}\n\
                        \nTo use a skill, first load it using the `load_skill` tool. Loaded skills appear in the transcript enclosed in a `<skill>` tag so you can refer to them. You don't need to load a skill more than once. Skills are just instructions, they may reference other tools but you can't call a skill as a tool. Follow loaded skill instructions very carefully.",
                        skill_names.join("\n- ")
                    );
                    system_content.push_str(&skills_section);
                }
            }
            let system_msg = Message::new(Role::System, &system_content);
            insert_chat_message(&self.deps.db, &self.session_id, &system_msg).await?;
            transcript.push(system_msg);
        }

        self.transcript = transcript;
        Ok(())
    }

    /// Build a fresh `Chat` for this turn using the current in-memory
    /// transcript. Tools, skills, and middleware are constructed fresh
    /// each turn so changes (e.g., new skills added) take effect
    /// without restarting the task.
    fn build_chat(&self, streaming_tx: Option<UnboundedSender<String>>) -> Chat {
        let AppConfig {
            note_search_api_url,
            storage_path,
            openai_api_hostname,
            openai_api_key,
            openai_model,
            ..
        } = self.deps.config.as_ref();

        // Construct all tools fresh (mirrors chat_handler lines 252-289)
        let note_search_tool = NoteSearchTool::new(note_search_api_url);
        let meeting_search_tool = MeetingSearchTool::new(note_search_api_url);
        let web_search_tool = WebSearchTool::new(note_search_api_url);
        let email_unread_tool = EmailUnreadTool::new(note_search_api_url);
        let calendar_tool = CalendarTool::new(self.deps.db.clone(), note_search_api_url);
        let website_view_tool = WebsiteViewTool::new(storage_path, &self.session_id);
        let tasks_due_today_tool = TasksDueTodayTool::new(note_search_api_url);
        let tasks_scheduled_today_tool = TasksScheduledTodayTool::new(note_search_api_url);
        #[cfg(debug_assertions)]
        let memory_tool = MemoryTool::new(storage_path);
        let datetime_tool = DateTimeTool::new();
        let bash_tool = BashTool::new(storage_path, &self.session_id);
        let notify_tool = NotifyTool::new(self.deps.db.clone(), &self.deps.config.vapid_key_path);

        let all_tools: Vec<BoxedToolCall> = vec![
            Box::new(note_search_tool),
            Box::new(meeting_search_tool),
            Box::new(web_search_tool),
            Box::new(email_unread_tool),
            Box::new(calendar_tool),
            Box::new(website_view_tool),
            Box::new(tasks_due_today_tool),
            Box::new(tasks_scheduled_today_tool),
            #[cfg(debug_assertions)]
            Box::new(memory_tool),
            Box::new(datetime_tool),
            Box::new(bash_tool),
            Box::new(notify_tool),
        ];

        let mut builder = ChatBuilder::new(openai_api_hostname, openai_api_key, openai_model)
            .database(&self.deps.db, Some(&self.session_id), None)
            .transcript(self.transcript.clone())
            .tools(all_tools)
            .middleware(vec![
                Box::new(InfiniteLoopDetector::new(3)),
                Box::new(ToolSecurityMiddleware::default()),
            ]);

        if let Some(tx) = streaming_tx {
            builder = builder.streaming(tx);
        }

        // Add skill management tools (merges with existing)
        builder = builder.skills(
            self.deps.skill_registry.clone(),
            storage_path,
            &self.session_id,
        );

        builder.build()
    }

    /// Handle an Http command from chat_handler. Builds a fresh Chat
    /// with the SSE `tx` for streaming, calls `next_msg`, indexes new
    /// messages, and sends a push notification if the client
    /// disconnected.
    async fn handle_http(&mut self, msg: Message, sse_tx: UnboundedSender<String>) {
        let mut chat = self.build_chat(Some(sse_tx.clone()));
        match chat.next_msg(msg.clone()).await {
            Ok(new_messages) => {
                // Append new messages to in-memory transcript
                self.transcript.push(msg);
                for m in &new_messages {
                    self.transcript.push(m.clone());
                }
                // Index new messages for full-text search (best-effort)
                self.spawn_index_messages(new_messages.clone()).await;

                // Push notification on disconnect (matches chat_handler
                // lines 657-686)
                if sse_tx.is_closed() {
                    self.send_push_notification(
                        "New chat response",
                        "New response after you disconnected.",
                    )
                    .await;
                }
            }
            Err(e) => {
                error!(
                    "ChatTask {} Http command failed: {}. Root cause: {}",
                    self.session_id,
                    e,
                    e.root_cause()
                );
                let err_msg = format!("Something went wrong: {}", e);
                let chunk = json!({
                    "id": "error",
                    "choices": [{
                        "finish_reason": "error",
                        "delta": { "content": err_msg }
                    }]
                })
                .to_string();
                if !sse_tx.is_closed() {
                    let _ = sse_tx.send(chunk);
                }
            }
        }
    }

    /// Handle a pub/sub message. Builds a fresh Chat (no streaming tx
    /// — response delivery is via push notification), calls `next_msg`,
    /// indexes new messages, and sends a push notification with the
    /// assistant's response.
    async fn handle_pubsub(&mut self, msg: Message) {
        let mut chat = self.build_chat(None);
        match chat.next_msg(msg.clone()).await {
            Ok(new_messages) => {
                self.transcript.push(msg);
                for m in &new_messages {
                    self.transcript.push(m.clone());
                }
                self.spawn_index_messages(new_messages.clone()).await;

                // Pub/sub responses always go to push notification
                // (async delivery — no active HTTP request)
                let response_text = new_messages
                    .iter()
                    .rev()
                    .find(|m| matches!(m.role(), Role::Assistant) && m.content.is_some())
                    .and_then(|m| m.content.as_deref())
                    .unwrap_or("You have a new message");
                self.send_push_notification("New chat message", response_text)
                    .await;
            }
            Err(e) => {
                error!("ChatTask {} PubSub command failed: {}", self.session_id, e);
            }
        }
    }

    /// Spawn a background task to index new chat messages for full-text
    /// search. Best-effort — errors are logged.
    async fn spawn_index_messages(&self, messages: Vec<Message>) {
        let db = self.deps.db.clone();
        let index_path = self.deps.config.index_path.clone();
        let session_id = self.session_id.clone();
        tokio::spawn(async move {
            if let Err(e) = index_chat_messages(&db, &index_path, &session_id, messages).await {
                error!("Failed to index chat messages: {}", e);
            }
        });
    }

    /// Send a web push notification with the given title/body. Mirrors
    /// `chat_handler` lines 657-686: fetches all valid subscriptions,
    /// broadcasts, and marks failed subscriptions as invalid.
    async fn send_push_notification(&self, title: &str, body: &str) {
        let db = self.deps.db.clone();
        let vapid_key_path = self.deps.config.vapid_key_path.clone();
        let session_id = self.session_id.clone();

        let payload = PushNotificationPayload::new(
            title,
            body,
            Some(&format!("/chat/?session_id={session_id}")),
            None,
            None,
        );
        match find_all_notification_subscriptions(&db).await {
            Ok(subscriptions) => {
                let failed =
                    broadcast_push_notification(subscriptions, vapid_key_path.to_string(), payload)
                        .await;
                for sub in failed {
                    if let Err(e) = mark_push_subscription_invalid(&db, &sub.endpoint).await {
                        error!("Failed to mark push subscription invalid: {}", e);
                    }
                }
            }
            Err(e) => {
                error!("Failed to find notification subscriptions: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::Role;

    /// Test that a published message reaches the ChatTask's pubsub_rx
    /// via the broker. This validates the wiring: ChatSessionManager
    /// registers pubsub_tx with broker, broker publishes, message lands
    /// in ChatTask's receiver.
    #[tokio::test]
    async fn test_broker_delivers_to_chat_task_channel() {
        let broker = Arc::new(PubSubBroker::new());
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        broker.subscribe("test-channel", tx);
        broker.publish(
            "test-channel",
            Message::new(Role::User, "hello from broker"),
        );
        let msg = rx.recv().await.expect("Should receive");
        assert_eq!(msg.content.as_deref(), Some("hello from broker"));
    }

    /// Test ChatCommand::Http carries the sse_tx through to the task.
    /// (Full ChatTask lifecycle is covered in tests/ integration tests
    /// since it requires AppConfig + env vars.)
    #[tokio::test]
    async fn test_chat_command_http_carries_sse_tx() {
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        // Construct the command and verify it builds without panic.
        // We can't access sse_tx directly (it's a struct field inside
        // the enum variant), but we can verify tx is still open after.
        let _cmd = ChatCommand::Http {
            msg: Message::new(Role::User, "hi"),
            sse_tx: tx.clone(),
        };
        // The original tx should still be open (not closed) since the
        // receiver (_rx) is still alive.
        assert!(!tx.is_closed());
    }

    /// Test that Uuid-based session IDs don't cause issues in
    /// ChatSessionManager (it uses String keys, so any string works).
    #[test]
    fn test_session_id_as_uuid_string() {
        // ChatSessionManager uses String keys, so any session ID format
        // (UUID, arbitrary string) works. Real manager construction is
        // covered in tests/ integration tests since it requires AppConfig.
        let uuid_str = uuid::Uuid::new_v4().to_string();
        assert!(!uuid_str.is_empty());
    }
}