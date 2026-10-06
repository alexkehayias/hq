use anyhow::Result;
use tokio_rusqlite::Connection;

use super::models::Loop;

pub async fn find_all_loops(db: &Connection) -> Result<Vec<Loop>> {
    let loops = db.call(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, channels, system_prompt, tools, debounce_ms, created_at FROM loop",
        )?;
        let rows = stmt
            .query_map([], |i| {
                let channels: String = i.get(1)?;
                let tools: String = i.get(3)?;
                Ok(Loop {
                    id: i.get(0)?,
                    channels: serde_json::from_str(&channels).unwrap_or_default(),
                    system_prompt: i.get(2)?,
                    tools: serde_json::from_str(&tools).unwrap_or_default(),
                    debounce_ms: i.get(4)?,
                    created_at: i.get(5)?,
                })
            })?
            .filter_map(Result::ok)
            .collect::<Vec<Loop>>();
        Ok(rows)
    });
    Ok(loops.await?)
}

pub async fn find_loop(db: &Connection, id: &str) -> Result<Option<Loop>> {
    let id = id.to_string();
    let record = db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, channels, system_prompt, tools, debounce_ms, created_at FROM loop WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map([&id], |i| {
            let channels: String = i.get(1)?;
            let tools: String = i.get(3)?;
            Ok(Loop {
                id: i.get(0)?,
                channels: serde_json::from_str(&channels).unwrap_or_default(),
                system_prompt: i.get(2)?,
                tools: serde_json::from_str(&tools).unwrap_or_default(),
                debounce_ms: i.get(4)?,
                created_at: i.get(5)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    });
    Ok(record.await?)
}

pub async fn insert_loop(db: &Connection, record: &Loop) -> Result<()> {
    let id = record.id.clone();
    let channels = serde_json::to_string(&record.channels)?;
    let system_prompt = record.system_prompt.clone();
    let tools = serde_json::to_string(&record.tools)?;
    let debounce_ms = record.debounce_ms;
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO loop (id, channels, system_prompt, tools, debounce_ms)
            VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, channels, system_prompt, tools, debounce_ms],
        )?;
        Ok(())
    })
    .await
    .map_err(anyhow::Error::new)
}

pub async fn delete_loop(db: &Connection, id: &str) -> Result<usize> {
    let id = id.to_string();
    let deleted = db.call(move |conn| {
        let count = conn.execute("DELETE FROM loop WHERE id = ?1", [&id])?;
        Ok(count)
    });
    Ok(deleted.await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db::{async_db, initialize_db};

    async fn setup_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = async_db(dir.path().to_str().unwrap()).await.unwrap();
        db.call(|conn| {
            initialize_db(conn)?;
            Ok(())
        })
        .await
        .unwrap();
        (dir, db)
    }

    fn sample_loop() -> Loop {
        Loop {
            id: "loop-1".to_string(),
            channels: vec!["alpha".to_string(), "beta".to_string()],
            system_prompt: Some("You are a loop.".to_string()),
            tools: vec!["bash".to_string(), "note_search".to_string()],
            debounce_ms: 250,
            created_at: String::new(),
        }
    }

    #[tokio::test]
    async fn initialize_db_creates_loop_table() {
        let (_dir, db) = setup_db().await;
        let exists = db
            .call(|conn| {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'loop'",
                    [],
                    |row| row.get(0),
                )?;
                Ok(count > 0)
            })
            .await
            .unwrap();
        assert!(exists);
    }

    #[tokio::test]
    async fn insert_find_and_delete_round_trip() {
        let (_dir, db) = setup_db().await;
        let record = sample_loop();

        insert_loop(&db, &record).await.unwrap();

        let all = find_all_loops(&db).await.unwrap();
        assert_eq!(all.len(), 1);
        let found = &all[0];
        assert_eq!(found.id, record.id);
        assert_eq!(found.channels, record.channels);
        assert_eq!(found.tools, record.tools);
        assert_eq!(found.system_prompt, record.system_prompt);
        assert_eq!(found.debounce_ms, record.debounce_ms);
        assert!(!found.created_at.is_empty());

        let deleted = delete_loop(&db, &record.id).await.unwrap();
        assert_eq!(deleted, 1);

        let all = find_all_loops(&db).await.unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn find_loop_returns_some_for_present_and_none_for_missing() {
        let (_dir, db) = setup_db().await;
        let record = sample_loop();
        insert_loop(&db, &record).await.unwrap();

        let found = find_loop(&db, &record.id).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, record.id);

        let missing = find_loop(&db, "does-not-exist").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn delete_loop_missing_id_returns_zero() {
        let (_dir, db) = setup_db().await;
        let deleted = delete_loop(&db, "does-not-exist").await.unwrap();
        assert_eq!(deleted, 0);
    }
}
