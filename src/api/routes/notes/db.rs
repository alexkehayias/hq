//! Database queries for the notes API
use super::public::ViewNoteResponse;
use tokio_rusqlite::Connection;

/// Get a note by ID from the database
pub async fn get_note_by_id(
    db: &Connection,
    id: String,
) -> Result<ViewNoteResponse, anyhow::Error> {
    let result = db
        .call(move |conn: &mut rusqlite::Connection| -> Result<ViewNoteResponse, tokio_rusqlite::Error> {
            let mut stmt = conn.prepare(
                r"
          SELECT
            id,
            title,
            body,
            tags
          FROM note_meta
          WHERE id = ?1
          LIMIT 1
        ",
            )?;
            let mut rows = stmt.query_map([id.clone()], |i| {
                Ok(ViewNoteResponse {
                    id: i.get(0)?,
                    title: i.get(1)?,
                    body: i.get(2)?,
                    tags: i.get(3)?,
                })
            })?;
            match rows.next() {
                Some(Ok(note)) => Ok(note),
                Some(Err(e)) => Err(tokio_rusqlite::Error::from(e)),
                None => {
                    let msg = "Note not found";
                    Err(tokio_rusqlite::Error::from(rusqlite::Error::InvalidParameterName(msg.into())))
                }
            }
        })
        .await;
    result.map_err(anyhow::Error::from)
}
