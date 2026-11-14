use anyhow::Result;
use hq_search::{Expr, Index, SearchResult};
use tokio_rusqlite::Connection;

use crate::ai::chat::db::get_chat_messages_by_ids;
use crate::core::fastembed_cache_dir;
use crate::openai::Role;

/// Search notes and chat messages.
///
/// Full-text and vector hits are resolved by `hq-search`; note-like documents
/// come back hydrated, while chat messages are hydrated here from the chat
/// store.
pub async fn search_notes(
    index_path: &str,
    db: &Connection,
    include_similarity: bool,
    truncate: bool,
    query: &Expr,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    let index = Index::open(db.clone(), index_path, fastembed_cache_dir());
    let results = index
        .search(query, include_similarity, truncate, limit)
        .await?;

    let mut documents = results.documents;

    // Query for chat results in a single batch
    if !results.chat_message_ids.is_empty()
        && let Ok(messages) = get_chat_messages_by_ids(db, &results.chat_message_ids).await
    {
        for (message_id, msg, session_id, session_title) in messages {
            let role = msg.role();
            // Only include user and assistant messages (not system/tool)
            if *role == Role::User || *role == Role::Assistant {
                let title = session_title.unwrap_or_default();
                let body = msg.text().unwrap_or_default().to_string();

                documents.push(SearchResult {
                    id: message_id,
                    r#type: "chat".to_string(),
                    category: String::new(),
                    file_name: String::new(),
                    title: if truncate {
                        title.chars().take(140).collect()
                    } else {
                        title
                    },
                    tags: None,
                    body: if truncate {
                        body.chars().take(240).collect()
                    } else {
                        body
                    },
                    is_task: false,
                    task_status: None,
                    task_scheduled: None,
                    task_deadline: None,
                    task_closed: None,
                    meeting_date: None,
                    chat_role: Some(role.as_str().to_string()),
                    session_id: Some(session_id),
                });
            }
        }
    }

    Ok(documents)
}
