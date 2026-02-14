//! Database queries for the notes API
use tokio_rusqlite::Connection;
use super::public::ViewNoteResponse;

/// Get a note by ID from the database
pub async fn get_note_by_id(
    db: &Connection,
    id: String,
) -> Result<ViewNoteResponse, anyhow::Error> {
    db.call(move |conn| {
        let result = conn
            .prepare(
                r"
          SELECT
            id,
            title,
            body,
            tags
          FROM note_meta
          WHERE id = ?
          LIMIT 1
        ",
            )
            .expect("Failed to prepare sql statement")
            .query_map([id], |i| {
                Ok(ViewNoteResponse {
                    id: i.get(0)?,
                    title: i.get(1)?,
                    body: i.get(2)?,
                    tags: i.get(3)?,
                })
            })
            .unwrap()
            .last()
            .unwrap()
            .unwrap();
        Ok(result)
    })
    .await
    .map_err(|e| e.into())
}
