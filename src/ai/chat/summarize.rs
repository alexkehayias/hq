use tokio_rusqlite::Connection;

use crate::ai::chat::{ChatBuilder, InvisibleCharFilter};
use crate::openai::{Message, Role};

/// Generate a title and summary for a chat session from its transcript and
/// persist them to the `session` table.
///
/// Returns `Some((title, summary))` when generated and saved, or `None` when
/// the LLM response couldn't be parsed as the expected JSON. Callers that only
/// care about the side effect (e.g. the background title job) can ignore the
/// value.
pub async fn generate_and_update_session_info(
    db_conn: &Connection,
    api_hostname: &str,
    api_key: &str,
    model: &str,
    session_id: &str,
    transcript: &[Message],
) -> Result<Option<(String, String)>, anyhow::Error> {
    let prompt = create_session_prompt(transcript);

    let system_prompt = "You are an assistant that generates concise titles and summaries for chat sessions based on the conversation content.";

    let mut chat = ChatBuilder::new(api_hostname, api_key, model)
        .transcript(vec![Message::new(Role::System, system_prompt)])
        .middleware(vec![Box::new(InvisibleCharFilter)])
        .build();

    let response = chat.next_msg(Message::new(Role::User, &prompt)).await?;
    let last_msg = response.last().expect("No messages").to_owned();
    let content = last_msg.content.expect("No content");

    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(json_response) => {
            if let (Some(title), Some(summary)) = (
                json_response["title"].as_str(),
                json_response["summary"].as_str(),
            ) {
                let session_id_owned = session_id.to_string();
                let title_owned = title.to_string();
                let summary_owned = summary.to_string();

                db_conn
                    .call(move |conn| {
                        let mut stmt =
                            conn.prepare("UPDATE session SET title = ?, summary = ? WHERE id = ?")?;
                        stmt.execute([title_owned, summary_owned, session_id_owned])?;
                        Ok(())
                    })
                    .await?;

                Ok(Some((title.to_string(), summary.to_string())))
            } else {
                tracing::warn!("LLM response missing title or summary fields: {}", content);
                Ok(None)
            }
        }
        Err(e) => {
            tracing::error!(
                "Failed to parse LLM response as JSON for session {}: {} - Response: {}",
                session_id,
                e,
                content
            );
            Ok(None)
        }
    }
}

fn create_session_prompt(transcript: &[Message]) -> String {
    let mut conversation = String::new();

    // Skip system messages (e.g. the model's system prompt) in the summary
    for message in transcript {
        if message.role() == &Role::System {
            continue;
        }
        if let Some(content) = &message.content {
            conversation.push_str(&format!("{}\n", content));
        }
    }

    format!(
        "Based on this chat conversation, generate a concise title and summary for the session. Return ONLY a JSON object with 'title' and 'summary' fields. The title should be 5-10 words, and the summary should be a short paragraph (2-3 sentences). Do not include any other text, just the JSON object.\n\nConversation:\n{}",
        conversation
    )
}
