use anyhow::{Error, Result};
use serde::Serialize;
use serde_json::json;
use tokio_rusqlite::Connection;

use crate::ai::chat::models::{Session, SessionMode};
use crate::openai::Message;

#[derive(Serialize)]
pub struct SessionWithTitle {
    pub id: String,
    pub title: Option<String>,
}

pub async fn insert_chat_message(
    db: &Connection,
    session_id: &str,
    msg: &Message,
) -> Result<String, Error> {
    let s_id = session_id.to_owned();
    let data = json!(msg).to_string();
    let id = db
        .call(move |conn| {
            let mut stmt =
                conn.prepare("INSERT INTO chat_message (session_id, data) VALUES (?, ?)")?;
            stmt.execute([s_id.clone(), data])?;

            // Return the id using a separate query since we can't get it from last_insert_rowid
            let id: String = conn.query_row(
                "SELECT id FROM chat_message WHERE rowid = last_insert_rowid()",
                [],
                |row| row.get(0),
            )?;
            Ok(id)
        })
        .await?;

    Ok(id)
}

/// Upserts a session. If the session already exists, the `mode` will
/// not be updated and the caller should use `set_session_mode` to
/// change it. This allows the caller to handle switching modes if
/// requested mode and the saved mode are not the same.
pub async fn get_or_create_session(
    db: &Connection,
    session_id: &str,
    tags: &[&str],
    mode: SessionMode,
) -> Result<Session, Error> {
    let session_id_owned = session_id.to_owned(); // String
    let tag_names: Vec<String> = tags
        .iter()
        .map(|s| s.to_lowercase().trim().to_string())
        .collect();
    let mode_str = mode.to_string();

    let (id, mode_str) = db
        .call(move |conn| {
            // All tag-related database calls either all succeed or it
            // fails and rollsback to avoid inconsistent data
            let tx = conn.transaction()?;

            // Insert a new session record if it doesn't already exist. If
            // it does exist, use the existing session mode, do not
            // overwrite it.
            let (id, mode): (String, String) = tx.query_row(
                r"INSERT INTO session (id, mode)
VALUES (?, ?)
ON CONFLICT(id) DO UPDATE
SET mode = session.mode
RETURNING id, mode",
                rusqlite::params![session_id_owned, mode_str],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            // Handle inserting tags if they don't exist and associating
            // tags with the session
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
            Ok((id, mode))
        })
        .await?;

    // If this is an existing session, the mode may not match with the
    // `mode` argument. If it's a new session, the mode will always
    // match the `mode` argument.
    let mode = mode_str.parse()?;
    Ok(Session { id, mode })
}

pub async fn find_chat_session_by_id(
    db: &Connection,
    session_id: &str,
) -> Result<Vec<(String, Message)>, Error> {
    let s_id = session_id.to_owned();
    let history = db.call(move |conn| {
        let mut stmt =
            conn.prepare("SELECT id, data FROM chat_message WHERE session_id=? ORDER BY rowid")?;
        let rows = stmt
            .query_map([s_id], |i| {
                let id: String = i.get(0)?;
                let val: String = i.get(1)?;
                let msg: Message = serde_json::from_str(&val).unwrap();
                Ok((id, msg))
            })?
            .filter_map(Result::ok)
            .collect::<Vec<(String, Message)>>();
        Ok(rows)
    });
    Ok(history.await?)
}

/// Retrieve a single chat message by session ID and index, along with the session title
pub async fn get_chat_message_by_index(
    db: &Connection,
    session_id: &str,
    message_index: usize,
) -> Result<Option<(Message, Option<String>)>, Error> {
    let s_id = session_id.to_string();
    let result = db.call(move |conn| {
        let mut stmt = conn.prepare("SELECT data FROM chat_message WHERE session_id=?")?;
        let rows: Vec<Message> = stmt
            .query_map([&s_id], |i| {
                let val: String = i.get(0)?;
                let msg: Message = serde_json::from_str(&val).unwrap();
                Ok(msg)
            })?
            .filter_map(Result::ok)
            .collect();

        // Get the message at the specific index
        let msg = rows.into_iter().nth(message_index);

        // Also get session title
        let mut stmt2 = conn.prepare("SELECT title FROM session WHERE id=?")?;
        let title: Option<String> = stmt2
            .query_map([&s_id], |i| i.get(0))?
            .filter_map(Result::ok)
            .next();

        Ok(msg.map(|m| (m, title)))
    });
    Ok(result.await?)
}

/// Retrieve a single chat message by its ID, along with the session title
pub async fn get_chat_message_by_id(
    db: &Connection,
    message_id: &str,
) -> Result<Option<(Message, Option<String>)>, Error> {
    let m_id = message_id.to_string();
    let result = db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT cm.data, s.title FROM chat_message cm
             JOIN session s ON cm.session_id = s.id
             WHERE cm.id=?",
        )?;
        let row = stmt.query_row([&m_id], |i| {
            let val: String = i.get(0)?;
            let msg: Message = serde_json::from_str(&val).unwrap();
            let title: Option<String> = i.get(1)?;
            Ok((msg, title))
        });
        match row {
            Ok((msg, title)) => Ok(Some((msg, title))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(tokio_rusqlite::Error::Rusqlite(e)),
        }
    });
    Ok(result.await?)
}

/// Get all sessions that do NOT have the "background" tag
pub async fn get_non_background_sessions(db: &Connection) -> Result<Vec<SessionWithTitle>, Error> {
    let sessions = db.call(move |conn| {
        let mut stmt = conn.prepare(
            r#"
            SELECT s.id, s.title FROM session s
            WHERE NOT EXISTS (
                SELECT 1 FROM session_tag st
                JOIN tag t ON st.tag_id = t.id
                WHERE st.session_id = s.id AND t.name = 'background'
            )
            "#,
        )?;
        let rows = stmt
            .query_map([], |i| {
                let id: String = i.get(0)?;
                let title: Option<String> = i.get(1)?;
                Ok(SessionWithTitle { id, title })
            })?
            .filter_map(Result::ok)
            .collect::<Vec<SessionWithTitle>>();
        Ok(rows)
    });
    Ok(sessions.await?)
}

/// Retrieve multiple chat messages by their IDs, along with session titles
pub async fn get_chat_messages_by_ids(
    db: &Connection,
    message_ids: &[String],
) -> Result<Vec<(String, Message, Option<String>)>, Error> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }

    let ids_json = serde_json::to_string(message_ids).unwrap();
    let result = db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT cm.id, cm.data, s.title FROM chat_message cm
             JOIN session s ON cm.session_id = s.id
             WHERE cm.id IN (SELECT value FROM json_each(?))",
        )?;
        let rows = stmt
            .query_map([ids_json.as_bytes()], |i| {
                let id: String = i.get(0)?;
                let val: String = i.get(1)?;
                let msg: Message = serde_json::from_str(&val).unwrap();
                let title: Option<String> = i.get(2)?;
                Ok((id, msg, title))
            })?
            .filter_map(Result::ok)
            .collect::<Vec<(String, Message, Option<String>)>>();
        Ok(rows)
    });
    Ok(result.await?)
}

/// Check if a session has the "background" tag
pub async fn session_has_background_tag(db: &Connection, session_id: &str) -> Result<bool, Error> {
    let s_id = session_id.to_owned();
    let has_tag = db.call(move |conn| {
        let result: Result<i64, _> = conn.query_row(
            r#"
            SELECT COUNT(*) FROM session_tag st
            JOIN tag t ON st.tag_id = t.id
            WHERE st.session_id = ? AND t.name = 'background'
            "#,
            [s_id],
            |row| row.get(0),
        );
        Ok(result.unwrap_or(0) > 0)
    });
    Ok(has_tag.await?)
}

/// Get the mode for a session
pub async fn get_session_mode(db: &Connection, session_id: &str) -> Result<SessionMode, Error> {
    let s_id = session_id.to_owned();
    let mode = db
        .call(move |conn| {
            let mut stmt = conn
                .prepare("SELECT mode FROM session WHERE id = ?")
                .expect("Invalid sql");
            let val: String = stmt.query_row([s_id], |row| row.get::<_, String>(0))?;
            Ok(val.parse().expect("Parsing session mode failed"))
        })
        .await
        .map_err(anyhow::Error::from)?;

    Ok(mode)
}

/// Set the mode for a session
pub async fn set_session_mode(
    db: &Connection,
    session_id: &str,
    mode: SessionMode,
) -> Result<(), Error> {
    let s_id = session_id.to_owned();
    let mode_str = mode.to_string();
    db.call(move |conn| {
        conn.execute(
            "UPDATE session SET mode = ? WHERE id = ?",
            rusqlite::params![mode_str, s_id],
        )?;
        Ok(())
    })
    .await
    .map_err(anyhow::Error::from)
}
