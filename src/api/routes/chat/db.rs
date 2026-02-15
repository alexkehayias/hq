use tokio_rusqlite::{Connection, params};
use serde_json::json;
use anyhow::{Error, Result};

use super::public;

pub async fn chat_session_count(
    db: &Connection,
    include_tags: &[String],
    exclude_tags: &[String],
) -> Result<i64, Error> {
    // If no filters, simple count
    if include_tags.is_empty() && exclude_tags.is_empty() {
        return db
            .call(|conn| {
                let mut stmt = conn.prepare("SELECT COUNT(*) FROM session")?;
                let count: i64 = stmt.query_row([], |row| row.get(0))?;
                Ok(count)
            })
            .await
            .map_err(anyhow::Error::from);
    }

    let include_json = json!(include_tags).to_string();
    let exclude_json = json!(exclude_tags).to_string();
    let inc_len = include_tags.len() as i64;
    let exc_len = exclude_tags.len() as i64;
    let count = db
        .call(move |conn| {
            let mut stmt = conn.prepare(
                r#"
                    SELECT COUNT(*) FROM session s
                    WHERE ( ?1 = 0 OR EXISTS (
                        SELECT 1 FROM session_tag st JOIN tag t ON st.tag_id = t.id
                        WHERE st.session_id = s.id AND t.name IN (SELECT value FROM json_each(?2))
                    ))
                    AND ( ?3 = 0 OR NOT EXISTS (
                        SELECT 1 FROM session_tag st2 JOIN tag t2 ON st2.tag_id = t2.id
                        WHERE st2.session_id = s.id AND t2.name IN (SELECT value FROM json_each(?4))
                    ))
                "#,
            )?;
            let count: i64 = stmt.query_row(
                params![
                    inc_len,
                    include_json.as_bytes(),
                    exc_len,
                    exclude_json.as_bytes()
                ],
                |row| row.get(0),
            )?;
            Ok(count)
        })
        .await?;
    Ok(count)
}

pub async fn chat_session_list(
    db: &Connection,
    include_tags: &[String],
    exclude_tags: &[String],
    limit: usize,
    offset: usize,
) -> Result<Vec<public::ChatSession>, Error> {
    // If no filters, simple query without tag joins for performance
    if include_tags.is_empty() && exclude_tags.is_empty() {
        return Ok(db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    r#"
                SELECT s.id, s.title, s.summary,
                       '' as tags
                FROM session s
                ORDER BY s.created_at DESC
                LIMIT ?1 OFFSET ?2
                "#,
                )?;
                let session_list = stmt
                    .query_map(params![limit, offset], |row| {
                        Ok(public::ChatSession {
                            id: row.get(0)?,
                            title: row.get(1)?,
                            summary: row.get(2)?,
                            tags: vec![],
                        })
                    })?
                    .filter_map(Result::ok)
                    .collect::<Vec<_>>();
                Ok(session_list)
            })
            .await?);
    }

    let include_json = json!(include_tags).to_string();
    let exclude_json = json!(exclude_tags).to_string();
    let inc_len = include_tags.len() as i64;
    let exc_len = exclude_tags.len() as i64;

    let results = db
        .call(move |conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT
                    s.id,
                    s.title,
                    s.summary,
                    GROUP_CONCAT(DISTINCT t.name) as tags
                FROM session s
                LEFT JOIN session_tag st ON s.id = st.session_id
                LEFT JOIN tag t ON st.tag_id = t.id
                WHERE ( ?1 = 0 OR EXISTS (
                        SELECT 1 FROM session_tag st2 JOIN tag t2 ON st2.tag_id = t2.id
                        WHERE st2.session_id = s.id AND t2.name IN (SELECT value FROM json_each(?2))
                    ))
                  AND ( ?3 = 0 OR NOT EXISTS (
                        SELECT 1 FROM session_tag st3 JOIN tag t3 ON st3.tag_id = t3.id
                        WHERE st3.session_id = s.id AND t3.name IN (SELECT value FROM json_each(?4))
                    ))
                GROUP BY s.id, s.title, s.summary, s.created_at
                ORDER BY s.created_at DESC
                LIMIT ?5 OFFSET ?6
                "#,
            )?;
            let session_list = stmt
                .query_map(
                    params![
                        inc_len,
                        include_json.as_str(),
                        exc_len,
                        exclude_json.as_str(),
                        limit,
                        offset
                    ],
                    |row| {
                        let session_id: String = row.get(0)?;
                        let title: Option<String> = row.get(1)?;
                        let summary: Option<String> = row.get(2)?;
                        let tags_str: Option<String> = row.get(3)?;
                        let tags = match tags_str {
                            Some(tag_str) => tag_str.split(',').map(|s| s.to_string()).collect(),
                            None => vec![],
                        };
                        Ok(public::ChatSession {
                            id: session_id,
                            title,
                            summary,
                            tags,
                        })
                    },
                )?
                .filter_map(Result::ok)
                .collect::<Vec<_>>();
            Ok(session_list)
        })
        .await
        .map_err(anyhow::Error::from)?;
    Ok(results)
}
