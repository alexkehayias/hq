//! Loop: subscribe to one or more channels and run an LLM chat on events.
//!
//! ## What it does
//!
//! `hq loop --channel <name> [--channel <other>]...` connects to one or more
//! channel publishers (see [`crate::cli::channel`]), merges their event streams,
//! and feeds each event into a single LLM chat conversation. The chat has
//! access to the **bash tool** (sandboxed to a fresh workspace per loop
//! invocation) and the **notify tool** (push notifications) — not to the full
//! tool set used by `hq chat`.
//!
//! ## Event format
//!
//! Each event is tagged with its source channel: `[channel-id] event`. The
//! LLM sees all events in one conversation, so it has context across channels.
//!
//! ## Trust boundary
//!
//! Same as the channel publisher: any process owned by the same user can
//! publish. The loop subscriber trusts events from channels it connects to —
//! do not subscribe to untrusted publishers if the LLM's bash workspace must
//! stay isolated.

use anyhow::{anyhow, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::ai::chat::{
    ChatBuilder, InfiniteLoopDetector, InvisibleCharFilter, ToolSecurityMiddleware,
};
use crate::ai::tools::{BashTool, NotifyTool};
use crate::cli::channel::{sigterm, socket_path};
use crate::openai::{BoxedToolCall, Message, Role};
use tokio_rusqlite::Connection;

/// Run the loop: subscribe to `channels`, feed events into an LLM chat.
///
/// One reader task per channel merges events into a single mpsc receiver. The
/// main loop reads `(channel_id, event)` pairs and sends `"[channel-id] event"`
/// as a user message to the chat. The LLM's response is printed to stdout.
///
/// All config (storage path, LLM endpoint/key/model) is passed in by the
/// caller (`mod.rs` run_dispatch); this module does not parse env vars.
pub async fn run(
    db: Connection,
    storage_path: &str,
    api_hostname: &str,
    api_key: &str,
    model: &str,
    vapid_key_path: &str,
    channels: &[String],
    system_prompt: Option<&str>,
) -> Result<()> {
    if channels.is_empty() {
        return Err(anyhow!("at least one --channel is required"));
    }

    // Merge event streams from all channels into one receiver.
    let (event_tx, mut event_rx) = mpsc::channel::<(String, String)>(100);

    for channel_id in channels {
        let path = socket_path(storage_path, channel_id)?;
        let stream = UnixStream::connect(&path)
            .await
            .map_err(|_| anyhow!("channel '{}' not found — is the publisher running?", channel_id))?;
        println!("Subscribed to channel '{}'", channel_id);

        let ch_id = channel_id.clone();
        let tx = event_tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stream);
            let mut buf = String::new();
            loop {
                buf.clear();
                match reader.read_line(&mut buf).await {
                    Ok(0) => return, // channel closed (publisher exited)
                    Ok(_) => {
                        let event = buf.strip_suffix('\n').unwrap_or(&buf).to_string();
                        if tx.send((ch_id.clone(), event)).await.is_err() {
                            return; // main loop exited
                        }
                    }
                    Err(_) => return,
                }
            }
        });
    }
    drop(event_tx); // close our sender so event_rx drains then returns None

    // Set up chat with bash and notify tools (not the full tool set from hq chat).
    let session_id = Uuid::new_v4().to_string();
    let bash_tool = BashTool::new(storage_path, &session_id);
    let notify_tool = NotifyTool::new(db, vapid_key_path);
    let tools: Vec<BoxedToolCall> = vec![
        Box::new(bash_tool) as BoxedToolCall,
        Box::new(notify_tool) as BoxedToolCall,
    ];

    let default_prompt = "You are a helpful assistant. You receive events from one or more \
         channels, each tagged as [channel-name] event. Respond to each \
         event appropriately. You have access to a bash tool for running commands \
         and a notify tool for sending push notifications.";
    let system_prompt = system_prompt.unwrap_or(default_prompt);

    let mut chat = ChatBuilder::new(api_hostname, api_key, model)
        .transcript(vec![Message::new(Role::System, system_prompt)])
        .tools(tools)
        .middleware(vec![
            Box::new(InfiniteLoopDetector::new(3)),
            Box::new(ToolSecurityMiddleware::default()),
            Box::new(InvisibleCharFilter),
        ])
        .build();

    // Main loop: read merged events, send to chat, print responses.
    // Ctrl-C or SIGTERM breaks out and exits cleanly.
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nShutting down...");
                return Ok(());
            }
            _ = sigterm() => {
                eprintln!("\nShutting down (SIGTERM)...");
                return Ok(());
            }
            event = event_rx.recv() => {
                let Some((channel_id, event)) = event else { break; };
                let user_msg = format!("[{}] {}", channel_id, event);
                match chat.next_msg(Message::new(Role::User, &user_msg)).await {
                    Ok(resp) => {
                        if let Some(msg) = resp.last() {
                            if let Some(content) = &msg.content {
                                println!("{}", content);
                            }
                        }
                    }
                    Err(e) => eprintln!("chat error: {}", e),
                }
            }
        }
    }

    Ok(())
}