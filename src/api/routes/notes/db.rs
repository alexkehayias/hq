//! Database queries for the notes API
use super::public::ViewNoteResponse;
use tokio_rusqlite::Connection;

/// Get a note by ID from the database
pub async fn get_note_by_id(
    db: &Connection,
    id: String,
) -> Result<Option<ViewNoteResponse>, anyhow::Error> {
    let result = db
        .call(move |conn: &mut rusqlite::Connection| -> Result<Option<ViewNoteResponse>, tokio_rusqlite::Error> {
            let query_result = conn.prepare(
                r"
          SELECT
            id,
            title,
            body,
            tags
          FROM note_meta
          WHERE id = ?1
        ",
            )
            .and_then(|mut stmt| {
                stmt.query_row([id], |row| {
                    Ok(ViewNoteResponse {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        body: row.get(2)?,
                        tags: row.get(3)?,
                    })
                })
            });
            match query_result {
                Ok(note) => Ok(Some(note)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(tokio_rusqlite::Error::from(e)),
            }
        })
        .await;
    result.map_err(anyhow::Error::from)
}
