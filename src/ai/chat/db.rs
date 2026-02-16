use tokio_rusqlite::Connection;
use serde_json::json;
use anyhow::{Error, Result};

use crate::openai::Message;

pub async fn insert_chat_message(
    db: &Connection,
    session_id: &str,
    msg: &Message,
) -> Result<usize, Error> {
    let s_id = session_id.to_owned();
    let data = json!(msg).to_string();
    let result = db
        .call(move |conn| {
            let mut stmt =
                conn.prepare("INSERT INTO chat_message (session_id, data) VALUES (?, ?)")?;
            let result = stmt.execute([s_id, data])?;
            Ok(result)
        })
        .await?;

    Ok(result)
}

pub async fn get_or_create_session(
    db: &Connection,
    session_id: &str,
    tags: &[&str],
) -> Result<(), Error> {
    let session_id_owned = session_id.to_owned(); // String
    let tag_names: Vec<String> = tags
        .iter()
        .map(|s| s.to_lowercase().trim().to_string())
        .collect();

    db.call(move |conn| {
        // All tag-related database calls either all succeed or it
        // fails and rollsback to avoid inconsistent data
        let tx = conn.transaction()?;

        // Insert a new session record if it doesn't already exist
        let result = tx.execute(
            "INSERT OR IGNORE INTO session (id) VALUES (?)",
            [&session_id_owned],
        )?;
        if !tag_names.is_empty() {
            // Insert all tags first (ignore duplicates)
            for tag in &tag_names {
                tx.execute("INSERT OR IGNORE INTO tag (name) VALUES (?)", [tag.clone()])?;
            }

            // Insert all session_tag relationships using a single query approach
            for tag in &tag_names {
                // Get the tag_id for this tag
                let tag_id: i64 =
                    tx.query_row("SELECT id FROM tag WHERE name = ?", [tag.clone()], |row| {
                        row.get(0)
                    })?;

                // Insert the session_tag relationship if it doesn't already exist
                tx.execute(
                    "INSERT OR IGNORE INTO session_tag (session_id, tag_id) VALUES (?, ?)",
                    [&session_id_owned, &tag_id.to_string()],
                )?;
            }
        }

        tx.commit()?;
        Ok(result)
    })
    .await?;

    Ok(())
}

pub async fn find_chat_session_by_id(
    db: &Connection,
    session_id: &str,
) -> Result<Vec<Message>, Error> {
    let s_id = session_id.to_owned();
    let history = db.call(move |conn| {
        let mut stmt = conn.prepare("SELECT data FROM chat_message WHERE session_id=?")?;
        let rows = stmt
            .query_map([s_id], |i| {
                let val: String = i.get(0)?;
                let msg: Message = serde_json::from_str(&val).unwrap();
                Ok(msg)
            })?
            .filter_map(Result::ok)
            .collect::<Vec<Message>>();
        Ok(rows)
    });
    Ok(history.await?)
}
