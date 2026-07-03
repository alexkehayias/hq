use tokio_rusqlite::Connection;
use crate::eval::models::{EvalRun, EvalResult};

pub async fn insert_run(db: &Connection, id: &str, name: &str, model: &str) -> anyhow::Result<()> {
    let id = id.to_string();
    let name = name.to_string();
    let model = model.to_string();

    db.call(move |conn| {
        Ok(conn.execute(
            "INSERT INTO eval_run (id, name, model, status) VALUES (?1, ?2, ?3, 'pending')",
            rusqlite::params![id, name, model],
        )?)
    })
    .await?;

    Ok(())
}

pub async fn update_run_status(
    db: &Connection,
    run_id: &str,
    status: &str,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let run_id = run_id.to_string();
    let status = status.to_string();

    db.call(move |conn| {
        Ok(conn.execute(
            "UPDATE eval_run SET status = ?1, started_at = CASE WHEN ?1 = 'running' THEN ?2 ELSE started_at END, completed_at = CASE WHEN ?1 IN ('completed', 'failed') THEN ?2 ELSE completed_at END WHERE id = ?3",
            rusqlite::params![status, now, run_id],
        )?)
    })
    .await?;

    Ok(())
}

pub async fn insert_result(
    db: &Connection,
    id: &str,
    run_id: &str,
    case_id: &str,
    input: &str,
    output: Option<&str>,
    passed: bool,
    error: Option<&str>,
) -> anyhow::Result<()> {
    let id = id.to_string();
    let run_id = run_id.to_string();
    let case_id = case_id.to_string();
    let input = input.to_string();
    let output = output.map(|s| s.to_string());
    let error = error.map(|s| s.to_string());

    db.call(move |conn| {
        Ok(conn.execute(
            "INSERT INTO eval_result (id, run_id, case_id, input, output, passed, error) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, run_id, case_id, input, output, passed as i32, error],
        )?)
    })
    .await?;

    Ok(())
}

pub async fn get_run_results(db: &Connection, run_id: &str) -> anyhow::Result<Vec<EvalResult>> {
    let r_id = run_id.to_string();

    Ok(db.call(move |conn| {
        let mut stmt = conn.prepare("SELECT id, run_id, case_id, input, output, passed, error FROM eval_result WHERE run_id = ?1")?;

        let rows = stmt.query_map([&r_id], |row| {
            Ok(EvalResult {
                id: row.get(0)?,
                run_id: row.get(1)?,
                case_id: row.get(2)?,
                input: row.get(3)?,
                output: row.get(4)?,
                passed: row.get::<_, i32>(5)? != 0,
                error: row.get(6)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    })
    .await?)
}

pub async fn get_run(db: &Connection, run_id: &str) -> anyhow::Result<Option<EvalRun>> {
    let r_id = run_id.to_string();

    Ok(db.call(move |conn| {
        match conn.query_row(
            "SELECT id, name, model, status, started_at, completed_at FROM eval_run WHERE id = ?1",
            [&r_id],
            |row| {
                Ok(EvalRun {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    model: row.get(2)?,
                    status: row.get(3)?,
                    started_at: row.get(4)?,
                    completed_at: row.get(5)?,
                })
            },
        ) {
            Ok(run) => Ok(Some(run)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(tokio_rusqlite::Error::Rusqlite(e)),
        }
    })
    .await?)
}
