use crate::ai::chat::BACKGROUND_TAG;
use crate::ai::chat::db::find_chat_session_by_id;
use crate::ai::chat::summarize::generate_and_update_session_info;
use crate::core::AppConfig;
use crate::openai::Message;
use async_trait::async_trait;
use std::time::Duration;
use tokio_rusqlite::{Connection, rusqlite};

#[derive(Debug)]
pub struct GenerateSessionTitles;

#[async_trait]
impl crate::jobs::PeriodicJob for GenerateSessionTitles {
    fn interval(&self) -> Duration {
        // Run every 10 minutes
        Duration::from_secs(60 * 60 * 2)
    }

    async fn run_job(&self, config: &AppConfig, db_conn: &Connection) {
        tracing::info!("Starting session title/summary generation job");

        // Find sessions that don't have a title or summary
        let sessions = db_conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT s.id FROM session s
                 LEFT JOIN chat_message cm ON s.id = cm.session_id
                 WHERE (s.title IS NULL OR s.title = '')
                 AND (s.summary IS NULL OR s.summary = '')
                 AND cm.session_id IS NOT NULL
                 AND NOT EXISTS (
                     SELECT 1 FROM session_tag st
                     JOIN tag t ON st.tag_id = t.id
                     WHERE st.session_id = s.id AND t.name = ?1
                 )",
                )?;

                let rows = stmt
                    .query_map([BACKGROUND_TAG], |row| {
                        let session_id: String = row.get(0)?;
                        Ok(session_id)
                    })?
                    .filter_map(Result::ok)
                    .collect::<Vec<String>>();

                Ok::<_, rusqlite::Error>(rows)
            })
            .await
            .expect("Session query failed");

        tracing::info!("Found {} sessions to update", sessions.len());

        for session_id in sessions {
            // Get the chat transcript for this session
            let transcript: Vec<Message> = find_chat_session_by_id(db_conn, &session_id)
                .await
                .expect("Loading chat session transcript failed")
                .into_iter()
                .map(|(_, msg)| msg)
                .collect();

            if !transcript.is_empty() {
                // Generate title and summary from the transcript
                let result = generate_and_update_session_info(
                    db_conn,
                    &config.openai_api_hostname,
                    &config.openai_api_key,
                    &config.openai_model,
                    &session_id,
                    &transcript,
                )
                .await;

                if let Err(e) = result {
                    tracing::error!(
                        "Failed to generate title/summary for session {}: {}",
                        session_id,
                        e
                    );
                }
            }
        }

        tracing::info!("Completed session title/summary generation job");
    }
}
