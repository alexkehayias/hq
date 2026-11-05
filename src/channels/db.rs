use anyhow::Result;
use tokio_rusqlite::Connection;

use super::models::{Channel, ChannelSource};

/// List every channel, ordered by id.
pub async fn find_all_channels(db: &Connection) -> Result<Vec<Channel>> {
    let channels = db.call(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, source, debounce_ms, created_by, created_at FROM channel ORDER BY id",
        )?;
        let channels = stmt
            .query_map([], |i| {
                Ok(Channel {
                    id: i.get(0)?,
                    source: i.get::<_, String>(1)?.parse::<ChannelSource>().unwrap(),
                    debounce_ms: i.get(2)?,
                    created_by: i.get(3)?,
                    created_at: i.get(4)?,
                })
            })?
            .filter_map(Result::ok)
            .collect::<Vec<Channel>>();
        Ok(channels)
    });
    Ok(channels.await?)
}

/// Find a single channel by id.
pub async fn find_channel(db: &Connection, id: &str) -> Result<Option<Channel>> {
    let id = id.to_string();
    let channel = db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, source, debounce_ms, created_by, created_at FROM channel WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map([&id], |i| {
            Ok(Channel {
                id: i.get(0)?,
                source: i.get::<_, String>(1)?.parse::<ChannelSource>().unwrap(),
                debounce_ms: i.get(2)?,
                created_by: i.get(3)?,
                created_at: i.get(4)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    });
    Ok(channel.await?)
}

/// Create a channel row. Fails if the id already exists.
///
/// `created_by` is the agent session that owns the channel, if any; see
/// [`Channel::created_by`].
pub async fn insert_channel(
    db: &Connection,
    source: &ChannelSource,
    id: &str,
    created_by: Option<&str>,
) -> Result<()> {
    let id = id.to_string();
    let source = source.as_str().to_string();
    let created_by = created_by.map(str::to_string);
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO channel (id, source, created_by) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, source, created_by],
        )?;
        Ok(())
    })
    .await
    .map_err(anyhow::Error::new)
}

/// Ids of loops subscribed to `channel_id`, ordered by loop id.
///
/// A channel with subscribers cannot be deleted outright — that would silently
/// break the loops. See [`delete_channel`] and
/// [`crate::cli::channel::run_delete`], which refuse unless the loops are
/// deleted too.
pub async fn find_channel_subscribers(db: &Connection, channel_id: &str) -> Result<Vec<String>> {
    let channel_id = channel_id.to_string();
    let ids = db.call(move |conn| {
        let mut stmt = conn
            .prepare("SELECT loop_id FROM loop_channel WHERE channel_id = ?1 ORDER BY loop_id")?;
        let ids = stmt
            .query_map([&channel_id], |i| i.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect::<Vec<String>>();
        Ok(ids)
    });
    Ok(ids.await?)
}

/// Delete a channel row and any emit grants that reference it.
///
/// Does not touch `loop_channel` subscriptions: callers must first delete (or
/// refuse to delete) loops subscribed to the channel — see
/// [`find_channel_subscribers`]. Emit grants are permissions for a channel that
/// is about to disappear, so they are cleared here. Returns the number of
/// channel rows deleted (0 if no channel had that id).
pub async fn delete_channel(db: &Connection, id: &str) -> Result<usize> {
    let id = id.to_string();
    let deleted = db.call(move |conn| {
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM loop_emit_channel WHERE channel_id = ?1", [&id])?;
        let count = tx.execute("DELETE FROM channel WHERE id = ?1", [&id])?;
        tx.commit()?;
        Ok(count)
    });
    Ok(deleted.await?)
}

/// Ids of agent-created channels (those with a `created_by` owner). Their
/// in-process publishers must be re-registered on boot; see
/// [`crate::channels::default::register_agent_channels`].
pub async fn find_agent_channels(db: &Connection) -> Result<Vec<String>> {
    let ids = db.call(|conn| {
        let mut stmt =
            conn.prepare("SELECT id FROM channel WHERE created_by IS NOT NULL ORDER BY id")?;
        let ids = stmt
            .query_map([], |i| i.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect::<Vec<String>>();
        Ok(ids)
    });
    Ok(ids.await?)
}

/// Ensure a `channel` row exists for each of `ids` so a loop's `loop_channel`
/// links satisfy the foreign key. Rows that already exist are left untouched,
/// preserving a channel's stored source and debounce.
///
/// `default_source` is the source assigned only to rows that don't exist yet; it
/// never reassigns an existing channel's source. Uses `ON CONFLICT(id) DO
/// NOTHING` rather than `INSERT OR IGNORE` so only the duplicate-id conflict is
/// suppressed; any other constraint failure still surfaces as an error.
pub async fn ensure_channels(
    db: &Connection,
    default_source: &ChannelSource,
    ids: &[String],
) -> Result<()> {
    let default_source = default_source.as_str().to_string();
    let ids = ids.to_vec();
    let inserted = db.call(move |conn| {
        let mut stmt = conn.prepare(
            "INSERT INTO channel (id, source) VALUES (?1, ?2)
             ON CONFLICT(id) DO NOTHING",
        )?;
        for id in &ids {
            stmt.execute(rusqlite::params![id, default_source])?;
        }
        Ok(())
    });
    Ok(inserted.await?)
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

    #[tokio::test]
    async fn find_all_channels_returns_rows_ordered_by_id() {
        let (_dir, db) = setup_db().await;
        insert_raw_channel(&db, "beta", "server").await;
        insert_raw_channel(&db, "alpha", "socket").await;

        let channels = find_all_channels(&db).await.unwrap();

        let ids: Vec<&str> = channels.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["alpha", "beta"]);
        assert_eq!(channels[0].source, ChannelSource::Socket);
        assert_eq!(channels[0].created_by, None);
        assert_eq!(channels[0].debounce_ms, 250);
        assert!(!channels[0].created_at.is_empty());
        assert_eq!(channels[1].source, ChannelSource::Server);
    }

    #[tokio::test]
    async fn find_all_channels_empty_table_returns_empty() {
        let (_dir, db) = setup_db().await;
        assert!(find_all_channels(&db).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn insert_and_find_channel_round_trip() {
        let (_dir, db) = setup_db().await;

        insert_channel(&db, &ChannelSource::Server, "owned", Some("agent-a"))
            .await
            .unwrap();

        let found = find_channel(&db, "owned").await.unwrap().unwrap();
        assert_eq!(found.source, ChannelSource::Server);
        assert_eq!(found.created_by.as_deref(), Some("agent-a"));
        assert!(find_channel(&db, "missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn insert_channel_rejects_duplicate_id() {
        let (_dir, db) = setup_db().await;
        insert_channel(&db, &ChannelSource::Socket, "dup", None)
            .await
            .unwrap();
        assert!(
            insert_channel(&db, &ChannelSource::Socket, "dup", None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn find_agent_channels_returns_only_owned() {
        let (_dir, db) = setup_db().await;
        insert_raw_channel(&db, "timer", "server").await; // server, no owner
        insert_raw_channel(&db, "sock", "socket").await; // socket
        insert_channel(&db, &ChannelSource::Server, "owned", Some("agent-a"))
            .await
            .unwrap();

        let ids = find_agent_channels(&db).await.unwrap();
        assert_eq!(ids, vec!["owned"]);
    }

    #[tokio::test]
    async fn ensure_channels_creates_rows_and_preserves_existing() {
        let (_dir, db) = setup_db().await;

        // A channel that already exists with a different source.
        insert_raw_channel(&db, "gamma", "server").await;

        ensure_channels(
            &db,
            &ChannelSource::Socket,
            &["alpha".to_string(), "beta".to_string(), "gamma".to_string()],
        )
        .await
        .unwrap();

        let rows = db
            .call(|conn| {
                let mut stmt = conn.prepare("SELECT id, source FROM channel ORDER BY id")?;
                let rows = stmt
                    .query_map([], |i| Ok((i.get::<_, String>(0)?, i.get::<_, String>(1)?)))?
                    .filter_map(Result::ok)
                    .collect::<Vec<(String, String)>>();
                Ok(rows)
            })
            .await
            .unwrap();

        assert_eq!(
            rows,
            vec![
                ("alpha".to_string(), "socket".to_string()),
                ("beta".to_string(), "socket".to_string()),
                ("gamma".to_string(), "server".to_string()),
            ]
        );
    }

    /// Insert a loop, optionally subscribed to `subscribe` and permitted to
    /// emit to `emit`.
    async fn insert_loop(db: &Connection, id: &str, subscribe: Option<&str>, emit: Option<&str>) {
        let id = id.to_string();
        let subscribe = subscribe.map(str::to_string);
        let emit = emit.map(str::to_string);
        db.call(move |conn| {
            conn.execute(
                "INSERT INTO loop (id, system_prompt, tools) VALUES (?1, 'prompt', '[]')",
                [&id],
            )?;
            if let Some(channel_id) = &subscribe {
                conn.execute(
                    "INSERT INTO loop_channel (loop_id, channel_id, position) VALUES (?1, ?2, 0)",
                    rusqlite::params![id, channel_id],
                )?;
            }
            if let Some(channel_id) = &emit {
                conn.execute(
                    "INSERT INTO loop_emit_channel (loop_id, channel_id) VALUES (?1, ?2)",
                    rusqlite::params![id, channel_id],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn find_channel_subscribers_returns_subscribed_loops_only() {
        let (_dir, db) = setup_db().await;
        insert_raw_channel(&db, "shared", "socket").await;
        insert_raw_channel(&db, "other", "socket").await;
        insert_loop(&db, "sub-b", Some("shared"), None).await;
        insert_loop(&db, "sub-a", Some("shared"), Some("shared")).await;
        insert_loop(&db, "emit-only", None, Some("shared")).await;
        insert_loop(&db, "other-sub", Some("other"), None).await;

        let ids = find_channel_subscribers(&db, "shared").await.unwrap();
        assert_eq!(ids, vec!["sub-a", "sub-b"]);
    }

    #[tokio::test]
    async fn delete_channel_removes_row_and_emit_grants() {
        let (_dir, db) = setup_db().await;
        insert_raw_channel(&db, "doomed", "socket").await;
        insert_loop(&db, "emitter", None, Some("doomed")).await;

        let deleted = delete_channel(&db, "doomed").await.unwrap();
        assert_eq!(deleted, 1);
        assert!(find_channel(&db, "doomed").await.unwrap().is_none());

        let (emit_grants, loops) = db
            .call(|conn| {
                let emit: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM loop_emit_channel WHERE channel_id = 'doomed'",
                    [],
                    |i| i.get(0),
                )?;
                let loops: i64 = conn.query_row("SELECT COUNT(*) FROM loop", [], |i| i.get(0))?;
                Ok((emit, loops))
            })
            .await
            .unwrap();
        // The emit grant is cleared; the emit-only loop itself survives.
        assert_eq!((emit_grants, loops), (0, 1));
    }

    #[tokio::test]
    async fn delete_channel_missing_returns_zero() {
        let (_dir, db) = setup_db().await;
        assert_eq!(delete_channel(&db, "missing").await.unwrap(), 0);
    }
}
