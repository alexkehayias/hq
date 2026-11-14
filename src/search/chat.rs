use anyhow::Result;
use hq_search::{DocumentType, Index, IndexDocument};
use tokio_rusqlite::{Connection, rusqlite};

use crate::ai::chat::db::{
    find_chat_session_by_id, get_non_background_sessions, session_has_background_tag,
};
use crate::core::fastembed_cache_dir;
use crate::openai::{Message, Role};

fn chat_document(message_id: &str, title: Option<&str>, msg: &Message) -> IndexDocument {
    IndexDocument {
        id: message_id.to_string(),
        document_type: DocumentType::Chat,
        title: title.unwrap_or("").to_string(),
        category: String::new(),
        body: msg.text().unwrap_or("").to_string(),
        tags: None,
        status: None,
        scheduled: None,
        deadline: None,
        closed: None,
        date: None,
        file_name: String::new(),
        chat_role: Some(msg.role().as_str().to_string()),
        embed: false,
    }
}

/// Index multiple chat messages in a single task with one writer and commit.
///
/// This avoids the LockFailure error that occurs when multiple IndexWriters
/// try to access the same index simultaneously.
pub async fn index_chat_messages(
    db: &Connection,
    index_dir_path: &str,
    session_id: &str,
    messages: Vec<Message>,
) -> Result<()> {
    // Skip if session has background tag
    if session_has_background_tag(db, session_id).await? {
        return Ok(());
    }

    let messages_len = messages.len();

    // Get session title and message IDs for this session
    let session_id_owned = session_id.to_string();
    let (title, message_ids): (Option<String>, Vec<String>) = db
        .call(move |conn| {
            let title: Option<String> = conn
                .query_row(
                    "SELECT title FROM session WHERE id = ?",
                    [&session_id_owned],
                    |row| row.get(0),
                )
                .ok();

            // Get all message IDs for this session (we'll index the new ones)
            let mut stmt = conn.prepare(
                "SELECT id FROM chat_message WHERE session_id = ? ORDER BY rowid DESC LIMIT ?",
            )?;
            let message_ids: Vec<String> = stmt
                .query_map([&session_id_owned, &messages_len.to_string()], |row| {
                    row.get(0)
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok::<_, rusqlite::Error>((title, message_ids))
        })
        .await?;

    // Reverse to get oldest first (to match message order)
    let mut message_ids: Vec<String> = message_ids.into_iter().rev().collect();
    // Pad with placeholder IDs if needed
    while message_ids.len() < messages_len {
        message_ids.push(format!("msg_{}", message_ids.len()));
    }

    // Filter messages to index (skip system and tool)
    let documents: Vec<IndexDocument> = messages
        .into_iter()
        .filter(|msg| *msg.role() != Role::System && *msg.role() != Role::Tool)
        .zip(message_ids.iter())
        .map(|(msg, id)| chat_document(id, title.as_deref(), &msg))
        .collect();

    if documents.is_empty() {
        return Ok(());
    }

    let index = Index::open(db.clone(), index_dir_path, fastembed_cache_dir());
    index.upsert(&documents, true, false).await?;

    Ok(())
}

/// Index all chat messages from non-background sessions
pub async fn index_all_chat_sessions(db: &Connection, index_dir_path: &str) -> Result<()> {
    let sessions = get_non_background_sessions(db).await?;

    if sessions.is_empty() {
        tracing::info!("No non-background chat sessions found to index");
        return Ok(());
    }

    tracing::info!("Indexing {} non-background chat sessions", sessions.len());

    let mut documents = Vec::new();
    for session in &sessions {
        let messages = find_chat_session_by_id(db, &session.id).await?;

        for (message_id, msg) in messages.iter() {
            // Skip system and tool role messages
            let role = msg.role();
            if *role == Role::System || *role == Role::Tool {
                continue;
            }

            documents.push(chat_document(message_id, session.title.as_deref(), msg));
        }
    }

    let index = Index::open(db.clone(), index_dir_path, fastembed_cache_dir());
    index.upsert(&documents, true, false).await?;

    tracing::info!("Finished indexing chat sessions");
    Ok(())
}

/// Delete all chat messages for a session from the full-text search index.
///
/// The index has no `session_id` field — chat messages are indexed by their
/// message UUID (`id`, a STRING field). To delete a session's messages, first
/// look up all `chat_message.id` values for the session, then delete each one.
/// Must be called BEFORE `delete_chat_session`, while the message rows still
/// exist in the DB.
pub async fn delete_chat_session_index(
    db: &Connection,
    index_dir_path: &str,
    session_id: &str,
) -> Result<()> {
    let s_id = session_id.to_string();
    let message_ids: Vec<String> = db
        .call(move |conn| {
            let mut stmt = conn.prepare("SELECT id FROM chat_message WHERE session_id = ?")?;
            let ids: Vec<String> = stmt
                .query_map([&s_id], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            Ok::<_, rusqlite::Error>(ids)
        })
        .await?;

    if message_ids.is_empty() {
        return Ok(());
    }

    let index = Index::open(db.clone(), index_dir_path, fastembed_cache_dir());
    index.delete(&message_ids).await?;

    tracing::info!(
        "Deleted {} chat message(s) from tantivy index for session {}",
        message_ids.len(),
        session_id
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat::db::{get_or_create_session, insert_chat_message};
    use crate::ai::chat::models::SessionMode;
    use crate::core::db::{async_db, initialize_db};
    use hq_search::parse_query;
    use tempfile::TempDir;

    async fn test_db() -> (Connection, TempDir) {
        let dir = TempDir::new().unwrap();
        let vec_db_path = dir.path().to_str().unwrap().to_string();
        let db = async_db(&vec_db_path).await.unwrap();
        db.call(|conn| {
            initialize_db(conn).unwrap();
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .unwrap();
        (db, dir)
    }

    async fn setup_session_with_messages(db: &Connection, session_id: &str) -> Vec<Message> {
        get_or_create_session(db, session_id, &[], SessionMode::Chat)
            .await
            .unwrap();
        let msg1 = Message::new(Role::User, "Hello world");
        let msg2 = Message::new(Role::Assistant, "Hi there!");
        insert_chat_message(db, session_id, &msg1).await.unwrap();
        insert_chat_message(db, session_id, &msg2).await.unwrap();

        // Return the messages in insertion order for `index_chat_messages`
        vec![msg1, msg2]
    }

    #[tokio::test]
    async fn test_delete_chat_session_index_removes_messages() {
        let (db, _db_dir) = test_db().await;
        let idx_dir = TempDir::new().unwrap();
        let index_dir_path = idx_dir.path().to_str().unwrap().to_string();

        let session_id = "test-session-index-1";
        let messages = setup_session_with_messages(&db, session_id).await;

        index_chat_messages(&db, &index_dir_path, session_id, messages)
            .await
            .unwrap();

        let index = Index::open(db.clone(), index_dir_path.as_str(), fastembed_cache_dir());
        let query = parse_query("Hello").unwrap();
        let results = index.search(&query, false, false, 10).await.unwrap();
        assert_eq!(
            results.chat_message_ids.len(),
            1,
            "expected the user message to be indexed"
        );

        delete_chat_session_index(&db, &index_dir_path, session_id)
            .await
            .unwrap();

        let results = index.search(&query, false, false, 10).await.unwrap();
        assert!(
            results.chat_message_ids.is_empty(),
            "expected docs to be deleted from the index"
        );
    }

    #[tokio::test]
    async fn test_delete_chat_session_index_no_messages() {
        let (db, _db_dir) = test_db().await;
        let idx_dir = TempDir::new().unwrap();
        let index_dir_path = idx_dir.path().to_str().unwrap().to_string();

        // Session exists but has no messages — should be a no-op
        let session_id = "test-session-no-messages";
        get_or_create_session(&db, session_id, &[], SessionMode::Chat)
            .await
            .unwrap();

        let result = delete_chat_session_index(&db, &index_dir_path, session_id).await;
        assert!(
            result.is_ok(),
            "expected Ok for empty session, got {result:?}"
        );
    }

    #[tokio::test]
    async fn test_delete_chat_session_index_nonexistent_session() {
        let (db, _db_dir) = test_db().await;
        let idx_dir = TempDir::new().unwrap();
        let index_dir_path = idx_dir.path().to_str().unwrap().to_string();

        // Session doesn't exist — no messages, so this is a no-op
        let result = delete_chat_session_index(&db, &index_dir_path, "nonexistent-session").await;
        assert!(
            result.is_ok(),
            "expected Ok for nonexistent session, got {result:?}"
        );
    }
}
