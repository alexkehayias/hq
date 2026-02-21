use tokio_rusqlite::Connection;

use crate::ai::chat::ChatBuilder;
use crate::ai::tools::EmailUnreadTool;
use crate::openai::{BoxedToolCall, Message, Role};

/// Email reader and responder agent.
pub async fn email_chat_response(
    db: &Connection,
    api_base_url: &str,
    emails: Vec<String>,
    openai_api_hostname: &str,
    openai_api_key: &str,
    openai_model: &str,
) -> (String, Vec<Message>) {
    let email_unread_tool = EmailUnreadTool::new(api_base_url);
    let tools: Vec<BoxedToolCall> = vec![Box::new(email_unread_tool)];

    let system_msg = format!(
        "You are an email assistant AI. Summarize, search, and analyze emails on behalf of the user for the following users: {}",
        emails.join(", ")
    );
    let user_msg = Message::new(Role::User, "Summarize my unread emails.");

    let mut chat = ChatBuilder::new(openai_api_hostname, openai_api_key, openai_model)
        .transcript(vec![Message::new(Role::System, &system_msg)])
        .database(db, None, Some(vec![String::from("background")]))
        .tools(tools)
        .build();

    let response = chat.next_msg(user_msg).await.expect("Chat session failed");
    (chat.session_id.unwrap(), response)
}
