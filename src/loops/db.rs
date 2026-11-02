use std::collections::HashMap;

use anyhow::Result;
use tokio_rusqlite::Connection;

use super::models::{Loop, LoopRun, RunStatus};

pub async fn find_all_loops(db: &Connection) -> Result<Vec<Loop>> {
    let loops = db.call(|conn| {
        let mut stmt = conn.prepare("SELECT id, system_prompt, tools, created_at FROM loop")?;
        let mut loops = stmt
            .query_map([], |i| {
                let tools: String = i.get(2)?;
                Ok(Loop {
                    id: i.get(0)?,
                    system_prompt: i.get(1)?,
                    tools: serde_json::from_str(&tools).unwrap_or_default(),
                    created_at: i.get(3)?,
                    subscribe_channels: Vec::new(),
                    emit_channels: Vec::new(),
                })
            })?
            .filter_map(Result::ok)
            .collect::<Vec<Loop>>();

        // Channel ids per loop, ordered by subscription position.
        let mut ch_stmt = conn
            .prepare("SELECT loop_id, channel_id FROM loop_channel ORDER BY loop_id, position")?;
        let mut subscribe_channels: HashMap<String, Vec<String>> = HashMap::new();
        let rows =
            ch_stmt.query_map([], |i| Ok((i.get::<_, String>(0)?, i.get::<_, String>(1)?)))?;
        for row in rows.filter_map(Result::ok) {
            subscribe_channels.entry(row.0).or_default().push(row.1);
        }

        // Emit-permission channel ids per loop, ordered by channel id.
        let mut emit_stmt = conn.prepare(
            "SELECT loop_id, channel_id FROM loop_emit_channel ORDER BY loop_id, channel_id",
        )?;
        let mut emit_channels: HashMap<String, Vec<String>> = HashMap::new();
        let rows =
            emit_stmt.query_map([], |i| Ok((i.get::<_, String>(0)?, i.get::<_, String>(1)?)))?;
        for row in rows.filter_map(Result::ok) {
            emit_channels.entry(row.0).or_default().push(row.1);
        }

        for l in loops.iter_mut() {
            if let Some(ch) = subscribe_channels.remove(&l.id) {
                l.subscribe_channels = ch;
            }
            if let Some(ch) = emit_channels.remove(&l.id) {
                l.emit_channels = ch;
            }
        }
        Ok(loops)
    });
    Ok(loops.await?)
}

pub async fn find_loop(db: &Connection, id: &str) -> Result<Option<Loop>> {
    let id = id.to_string();
    let record = db.call(move |conn| {
        let mut record = {
            let mut stmt = conn
                .prepare("SELECT id, system_prompt, tools, created_at FROM loop WHERE id = ?1")?;
            let mut rows = stmt.query_map([&id], |i| {
                let tools: String = i.get(2)?;
                Ok(Loop {
                    id: i.get(0)?,
                    system_prompt: i.get(1)?,
                    tools: serde_json::from_str(&tools).unwrap_or_default(),
                    created_at: i.get(3)?,
                    subscribe_channels: Vec::new(),
                    emit_channels: Vec::new(),
                })
            })?;
            match rows.next().transpose()? {
                Some(record) => record,
                None => return Ok(None),
            }
        };

        let mut ch_stmt = conn
            .prepare("SELECT channel_id FROM loop_channel WHERE loop_id = ?1 ORDER BY position")?;
        record.subscribe_channels = ch_stmt
            .query_map([&id], |i| i.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect();

        let mut emit_stmt = conn.prepare(
            "SELECT channel_id FROM loop_emit_channel WHERE loop_id = ?1 ORDER BY channel_id",
        )?;
        record.emit_channels = emit_stmt
            .query_map([&id], |i| i.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect();

        Ok(Some(record))
    });
    Ok(record.await?)
}

/// Replace a loop's system prompt. Returns the number of rows updated (0 if no
/// loop with `id` exists).
pub async fn update_loop_system_prompt(
    db: &Connection,
    id: &str,
    system_prompt: &str,
) -> Result<usize> {
    let id = id.to_string();
    let system_prompt = system_prompt.to_string();
    let updated = db.call(move |conn| {
        Ok(conn.execute(
            "UPDATE loop SET system_prompt = ?1 WHERE id = ?2",
            rusqlite::params![system_prompt, id],
        )?)
    });
    Ok(updated.await?)
}

/// Whether loop `loop_id` is permitted to emit to `channel_id`, independent of
/// channel ownership. `false` if the loop has no such grant (or does not exist).
pub async fn loop_may_emit(db: &Connection, loop_id: &str, channel_id: &str) -> Result<bool> {
    let loop_id = loop_id.to_string();
    let channel_id = channel_id.to_string();
    let permitted = db.call(move |conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM loop_emit_channel WHERE loop_id = ?1 AND channel_id = ?2",
            rusqlite::params![loop_id, channel_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    });
    Ok(permitted.await?)
}

pub async fn insert_loop(db: &Connection, record: &Loop) -> Result<()> {
    let id = record.id.clone();
    let system_prompt = record.system_prompt.clone();
    let tools = serde_json::to_string(&record.tools)?;
    let subscribe_channels = record.subscribe_channels.clone();
    let emit_channels = record.emit_channels.clone();
    db.call(move |conn| {
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO loop (id, system_prompt, tools) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, system_prompt, tools],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO loop_channel (loop_id, channel_id, position) VALUES (?1, ?2, ?3)",
            )?;
            for (position, channel_id) in subscribe_channels.iter().enumerate() {
                stmt.execute(rusqlite::params![id, channel_id, position as i64])?;
            }
        }
        {
            let mut stmt =
                tx.prepare("INSERT INTO loop_emit_channel (loop_id, channel_id) VALUES (?1, ?2)")?;
            for channel_id in emit_channels.iter() {
                stmt.execute(rusqlite::params![id, channel_id])?;
            }
        }
        tx.commit()?;
        Ok(())
    })
    .await
    .map_err(anyhow::Error::new)
}

pub async fn delete_loop(db: &Connection, id: &str) -> Result<usize> {
    let id = id.to_string();
    let deleted = db.call(move |conn| {
        let tx = conn.transaction()?;
        let session_ids = select_run_session_ids(&tx, &id)?;
        delete_transcript_sessions(&tx, &session_ids)?;
        tx.execute("DELETE FROM loop_run WHERE loop_id = ?1", [&id])?;
        tx.execute("DELETE FROM loop_channel WHERE loop_id = ?1", [&id])?;
        tx.execute("DELETE FROM loop_emit_channel WHERE loop_id = ?1", [&id])?;
        let count = tx.execute("DELETE FROM loop WHERE id = ?1", [&id])?;
        tx.commit()?;
        Ok(count)
    });
    Ok(deleted.await?)
}

/// Record a new run. The run id doubles as the transcript session id.
pub async fn insert_loop_run(
    db: &Connection,
    id: &str,
    loop_id: &str,
    channel_id: &str,
    event: &str,
) -> Result<()> {
    let id = id.to_string();
    let loop_id = loop_id.to_string();
    let channel_id = channel_id.to_string();
    let event = event.to_string();
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO loop_run (id, loop_id, channel_id, event, status, session_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?1)",
            rusqlite::params![id, loop_id, channel_id, event, RunStatus::Running.as_str()],
        )?;
        Ok(())
    })
    .await
    .map_err(anyhow::Error::new)
}

pub async fn finish_loop_run(
    db: &Connection,
    id: &str,
    status: RunStatus,
    response: Option<&str>,
    error: Option<&str>,
    duration_ms: i64,
) -> Result<()> {
    let id = id.to_string();
    let status = status.to_string();
    let response = response.map(str::to_string);
    let error = error.map(str::to_string);
    db.call(move |conn| {
        conn.execute(
            "UPDATE loop_run
             SET status = ?2,
                 finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 duration_ms = ?3,
                 response = ?4,
                 error = ?5
             WHERE id = ?1",
            rusqlite::params![id, status, duration_ms, response, error],
        )?;
        Ok(())
    })
    .await
    .map_err(anyhow::Error::new)
}

/// Most recent runs for a loop, newest first.
pub async fn find_loop_runs(db: &Connection, loop_id: &str, limit: usize) -> Result<Vec<LoopRun>> {
    let loop_id = loop_id.to_string();
    let runs = db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT r.id, r.loop_id, r.channel_id, r.event, r.status, r.session_id,
                    r.started_at, r.finished_at, r.duration_ms, r.response, r.error,
                    EXISTS(SELECT 1 FROM session s WHERE s.id = r.session_id)
             FROM loop_run r
             WHERE r.loop_id = ?1
             ORDER BY r.started_at DESC, r.rowid DESC
             LIMIT ?2",
        )?;
        let runs = stmt
            .query_map(rusqlite::params![loop_id, limit as i64], |i| {
                let status: String = i.get(4)?;
                Ok(LoopRun {
                    id: i.get(0)?,
                    loop_id: i.get(1)?,
                    channel_id: i.get(2)?,
                    event: i.get(3)?,
                    status: status.parse().unwrap_or(RunStatus::Error),
                    session_id: i.get(5)?,
                    started_at: i.get(6)?,
                    finished_at: i.get(7)?,
                    duration_ms: i.get(8)?,
                    response: i.get(9)?,
                    error: i.get(10)?,
                    has_transcript: i.get(11)?,
                })
            })?
            .filter_map(Result::ok)
            .collect::<Vec<LoopRun>>();
        Ok(runs)
    });
    Ok(runs.await?)
}

/// Mark runs left in `running` by a previous process as interrupted.
pub async fn mark_running_loop_runs_interrupted(db: &Connection) -> Result<usize> {
    let updated = db.call(|conn| {
        let count = conn.execute(
            "UPDATE loop_run
             SET status = ?1,
                 finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE status = ?2",
            rusqlite::params![RunStatus::Interrupted.as_str(), RunStatus::Running.as_str()],
        )?;
        Ok(count)
    });
    Ok(updated.await?)
}

/// Delete all but the newest `keep` runs for a loop, along with their
/// transcript sessions. Returns the number of runs removed.
pub async fn prune_loop_runs(db: &Connection, loop_id: &str, keep: usize) -> Result<usize> {
    let loop_id = loop_id.to_string();
    let removed = db.call(move |conn| {
        let tx = conn.transaction()?;
        let stale_ids: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM loop_run
                 WHERE loop_id = ?1
                   AND id NOT IN (
                       SELECT id FROM loop_run
                       WHERE loop_id = ?1
                       ORDER BY started_at DESC, rowid DESC
                       LIMIT ?2
                   )",
            )?;
            stmt.query_map(rusqlite::params![loop_id, keep as i64], |i| i.get(0))?
                .filter_map(Result::ok)
                .collect()
        };

        if stale_ids.is_empty() {
            return Ok(0);
        }

        let session_ids = select_session_ids_for_runs(&tx, &stale_ids)?;
        delete_transcript_sessions(&tx, &session_ids)?;
        let ids_json = serde_json::to_string(&stale_ids).unwrap();
        let count = tx.execute(
            "DELETE FROM loop_run WHERE id IN (SELECT value FROM json_each(?1))",
            [&ids_json],
        )?;
        tx.commit()?;
        Ok(count)
    });
    Ok(removed.await?)
}

fn select_run_session_ids(
    tx: &rusqlite::Transaction,
    loop_id: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = tx.prepare("SELECT session_id FROM loop_run WHERE loop_id = ?1")?;
    let ids = stmt
        .query_map([loop_id], |i| i.get(0))?
        .filter_map(Result::ok)
        .collect();
    Ok(ids)
}

fn select_session_ids_for_runs(
    tx: &rusqlite::Transaction,
    run_ids: &[String],
) -> rusqlite::Result<Vec<String>> {
    let ids_json = serde_json::to_string(run_ids).unwrap();
    let mut stmt = tx
        .prepare("SELECT session_id FROM loop_run WHERE id IN (SELECT value FROM json_each(?1))")?;
    let ids = stmt
        .query_map([&ids_json], |i| i.get(0))?
        .filter_map(Result::ok)
        .collect();
    Ok(ids)
}

/// Delete transcript sessions in dependency order. Foreign keys are ON, so
/// child rows must go before the `session` parent.
fn delete_transcript_sessions(
    tx: &rusqlite::Transaction,
    session_ids: &[String],
) -> rusqlite::Result<()> {
    if session_ids.is_empty() {
        return Ok(());
    }
    let ids_json = serde_json::to_string(session_ids).unwrap();
    tx.execute(
        "DELETE FROM chat_message WHERE session_id IN (SELECT value FROM json_each(?1))",
        [&ids_json],
    )?;
    tx.execute(
        "DELETE FROM session_tag WHERE session_id IN (SELECT value FROM json_each(?1))",
        [&ids_json],
    )?;
    tx.execute(
        "DELETE FROM session WHERE id IN (SELECT value FROM json_each(?1))",
        [&ids_json],
    )?;
    Ok(())
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

    /// Insert a channel row directly, bypassing the helpers.
    async fn insert_raw_channel(db: &Connection, id: &str, source: &str) {
        let id = id.to_string();
        let source = source.to_string();
        db.call(move |conn| {
            conn.execute(
                "INSERT INTO channel (id, source) VALUES (?1, ?2)",
                rusqlite::params![id, source],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    fn sample_loop() -> Loop {
        Loop {
            id: "loop-1".to_string(),
            subscribe_channels: vec!["alpha".to_string(), "beta".to_string()],
            emit_channels: vec!["beta".to_string()],
            system_prompt: "You are a loop.".to_string(),
            tools: vec!["bash".to_string(), "note_search".to_string()],
            created_at: String::new(),
        }
    }

    #[tokio::test]
    async fn initialize_db_creates_loop_tables() {
        let (_dir, db) = setup_db().await;
        let tables = db
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT name FROM sqlite_master WHERE type = 'table'
                    AND name IN ('channel', 'loop', 'loop_channel', 'loop_emit_channel', 'loop_run')",
                )?;
                let names = stmt
                    .query_map([], |row| row.get::<_, String>(0))?
                    .filter_map(Result::ok)
                    .collect::<Vec<String>>();
                Ok(names)
            })
            .await
            .unwrap();
        assert!(tables.contains(&"channel".to_string()));
        assert!(tables.contains(&"loop".to_string()));
        assert!(tables.contains(&"loop_channel".to_string()));
        assert!(tables.contains(&"loop_emit_channel".to_string()));
        assert!(tables.contains(&"loop_run".to_string()));
    }

    #[tokio::test]
    async fn insert_find_and_delete_round_trip() {
        let (_dir, db) = setup_db().await;
        insert_raw_channel(&db, "alpha", "socket").await;
        insert_raw_channel(&db, "beta", "server").await;
        let record = sample_loop();

        insert_loop(&db, &record).await.unwrap();

        let all = find_all_loops(&db).await.unwrap();
        assert_eq!(all.len(), 1);
        let found = &all[0];
        assert_eq!(found.id, record.id);
        assert_eq!(found.subscribe_channels, record.subscribe_channels);
        assert_eq!(found.emit_channels, record.emit_channels);
        assert_eq!(found.tools, record.tools);
        assert_eq!(found.system_prompt, record.system_prompt);
        assert!(!found.created_at.is_empty());

        let deleted = delete_loop(&db, &record.id).await.unwrap();
        assert_eq!(deleted, 1);

        let all = find_all_loops(&db).await.unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn delete_loop_removes_channel_links() {
        let (_dir, db) = setup_db().await;
        insert_raw_channel(&db, "alpha", "socket").await;
        insert_raw_channel(&db, "beta", "server").await;
        insert_loop(&db, &sample_loop()).await.unwrap();

        delete_loop(&db, "loop-1").await.unwrap();

        let (subscriptions, emit_grants) = db
            .call(|conn| {
                let subscriptions: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM loop_channel WHERE loop_id = ?1",
                    ["loop-1"],
                    |row| row.get(0),
                )?;
                let emit_grants: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM loop_emit_channel WHERE loop_id = ?1",
                    ["loop-1"],
                    |row| row.get(0),
                )?;
                Ok((subscriptions, emit_grants))
            })
            .await
            .unwrap();
        assert_eq!(subscriptions, 0);
        assert_eq!(emit_grants, 0);
    }

    #[tokio::test]
    async fn loop_may_emit_reflects_grants() {
        let (_dir, db) = setup_db().await;
        insert_raw_channel(&db, "alpha", "socket").await;
        insert_raw_channel(&db, "beta", "server").await;
        insert_loop(&db, &sample_loop()).await.unwrap();

        // sample_loop is granted emit access to "beta" only.
        assert!(loop_may_emit(&db, "loop-1", "beta").await.unwrap());
        assert!(!loop_may_emit(&db, "loop-1", "alpha").await.unwrap());
        assert!(!loop_may_emit(&db, "loop-1", "missing").await.unwrap());
        assert!(!loop_may_emit(&db, "no-such-loop", "beta").await.unwrap());
    }

    #[tokio::test]
    async fn find_loop_returns_some_for_present_and_none_for_missing() {
        let (_dir, db) = setup_db().await;
        insert_raw_channel(&db, "alpha", "socket").await;
        insert_raw_channel(&db, "beta", "server").await;
        let record = sample_loop();
        insert_loop(&db, &record).await.unwrap();

        let found = find_loop(&db, &record.id).await.unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.id, record.id);
        assert_eq!(found.subscribe_channels, record.subscribe_channels);
        assert_eq!(found.emit_channels, record.emit_channels);

        let missing = find_loop(&db, "does-not-exist").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn delete_loop_missing_id_returns_zero() {
        let (_dir, db) = setup_db().await;
        let deleted = delete_loop(&db, "does-not-exist").await.unwrap();
        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn update_loop_system_prompt_changes_prompt() {
        let (_dir, db) = setup_db().await;
        insert_raw_channel(&db, "alpha", "socket").await;
        insert_raw_channel(&db, "beta", "server").await;
        insert_loop(&db, &sample_loop()).await.unwrap();

        let updated = update_loop_system_prompt(&db, "loop-1", "New instructions.")
            .await
            .unwrap();
        assert_eq!(updated, 1);

        let found = find_loop(&db, "loop-1").await.unwrap().unwrap();
        assert_eq!(found.system_prompt, "New instructions.");
        // Unrelated fields are untouched.
        assert_eq!(found.tools, sample_loop().tools);
        assert_eq!(found.subscribe_channels, sample_loop().subscribe_channels);

        let missing = update_loop_system_prompt(&db, "does-not-exist", "x")
            .await
            .unwrap();
        assert_eq!(missing, 0);
    }

    async fn insert_raw_session(db: &Connection, id: &str) {
        let id = id.to_string();
        db.call(move |conn| {
            conn.execute("INSERT INTO session (id) VALUES (?1)", [&id])?;
            Ok(())
        })
        .await
        .unwrap();
    }

    async fn count_runs(db: &Connection, loop_id: &str) -> i64 {
        let loop_id = loop_id.to_string();
        db.call(move |conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM loop_run WHERE loop_id = ?1",
                [&loop_id],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn loop_run_round_trip() {
        let (_dir, db) = setup_db().await;
        insert_raw_channel(&db, "alpha", "socket").await;
        insert_raw_channel(&db, "beta", "server").await;
        insert_loop(&db, &sample_loop()).await.unwrap();

        insert_loop_run(&db, "run-1", "loop-1", "alpha", "hello")
            .await
            .unwrap();

        let runs = find_loop_runs(&db, "loop-1", 10).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, "run-1");
        assert_eq!(runs[0].session_id, "run-1");
        assert_eq!(runs[0].status, RunStatus::Running);
        assert_eq!(runs[0].event, "hello");
        assert!(!runs[0].has_transcript);

        insert_raw_session(&db, "run-1").await;
        finish_loop_run(&db, "run-1", RunStatus::Success, Some("done"), None, 42)
            .await
            .unwrap();

        let runs = find_loop_runs(&db, "loop-1", 10).await.unwrap();
        assert_eq!(runs[0].status, RunStatus::Success);
        assert_eq!(runs[0].duration_ms, Some(42));
        assert_eq!(runs[0].response.as_deref(), Some("done"));
        assert!(runs[0].finished_at.is_some());
        assert!(runs[0].has_transcript);
    }

    #[tokio::test]
    async fn mark_running_runs_interrupted_only_touches_running() {
        let (_dir, db) = setup_db().await;
        insert_raw_channel(&db, "alpha", "socket").await;
        insert_raw_channel(&db, "beta", "server").await;
        insert_loop(&db, &sample_loop()).await.unwrap();
        insert_loop_run(&db, "run-1", "loop-1", "alpha", "one")
            .await
            .unwrap();
        insert_loop_run(&db, "run-2", "loop-1", "alpha", "two")
            .await
            .unwrap();
        finish_loop_run(&db, "run-2", RunStatus::Success, Some("ok"), None, 10)
            .await
            .unwrap();

        let updated = mark_running_loop_runs_interrupted(&db).await.unwrap();
        assert_eq!(updated, 1);

        let runs = find_loop_runs(&db, "loop-1", 10).await.unwrap();
        let interrupted = runs.iter().find(|r| r.id == "run-1").unwrap();
        let success = runs.iter().find(|r| r.id == "run-2").unwrap();
        assert_eq!(interrupted.status, RunStatus::Interrupted);
        assert_eq!(success.status, RunStatus::Success);
    }

    #[tokio::test]
    async fn prune_loop_runs_keeps_newest_and_deletes_transcripts() {
        let (_dir, db) = setup_db().await;
        insert_raw_channel(&db, "alpha", "socket").await;
        insert_raw_channel(&db, "beta", "server").await;
        insert_loop(&db, &sample_loop()).await.unwrap();
        for i in 0..5 {
            let id = format!("run-{i}");
            insert_raw_session(&db, &id).await;
            insert_loop_run(&db, &id, "loop-1", "alpha", "event")
                .await
                .unwrap();
        }

        let removed = prune_loop_runs(&db, "loop-1", 2).await.unwrap();
        assert_eq!(removed, 3);
        assert_eq!(count_runs(&db, "loop-1").await, 2);

        let runs = find_loop_runs(&db, "loop-1", 10).await.unwrap();
        let ids: Vec<&str> = runs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["run-4", "run-3"]);

        let remaining_sessions: i64 = db
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM session WHERE id LIKE 'run-%'",
                    [],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(remaining_sessions, 2);
    }

    #[tokio::test]
    async fn delete_loop_removes_runs_and_transcripts() {
        let (_dir, db) = setup_db().await;
        insert_raw_channel(&db, "alpha", "socket").await;
        insert_raw_channel(&db, "beta", "server").await;
        insert_loop(&db, &sample_loop()).await.unwrap();
        insert_raw_session(&db, "run-1").await;
        insert_loop_run(&db, "run-1", "loop-1", "alpha", "event")
            .await
            .unwrap();

        delete_loop(&db, "loop-1").await.unwrap();

        assert_eq!(count_runs(&db, "loop-1").await, 0);
        let sessions: i64 = db
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM session WHERE id = 'run-1'",
                    [],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(sessions, 0);
    }
}
