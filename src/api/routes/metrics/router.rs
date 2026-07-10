//! Router for the metrics API

use std::sync::{Arc, RwLock};

use axum::{Router, extract::State, http::StatusCode, response::Json};
use axum_extra::extract::Query;

use super::public;
use crate::api::state::AppState;

type SharedState = Arc<RwLock<AppState>>;

/// Record a metric event
async fn record_metric(
    State(state): State<SharedState>,
    Json(payload): Json<public::MetricRequest>,
) -> Result<StatusCode, crate::api::public::ApiError> {
    let db = state.read().unwrap().db.clone();

    // Insert the metric event into the database. name/value stay NULL
    // for new rows — they only exist on legacy data migrated from the
    // pre-bucket schema.
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO metric_event (input, output, cache_read, cache_write, reasoning) \
             VALUES (?, ?, ?, ?, ?)",
            tokio_rusqlite::params![
                payload.input,
                payload.output,
                payload.cache_read,
                payload.cache_write,
                payload.reasoning,
            ],
        )?;
        Ok(())
    })
    .await?;

    Ok(StatusCode::OK)
}

/// Get metric events for visualization
async fn get_metrics(
    State(state): State<SharedState>,
    Query(params): Query<public::MetricsQuery>,
) -> Result<Json<public::MetricsResponse>, crate::api::public::ApiError> {
    let db = state.read().unwrap().db.clone();

    // Default to last 30 days if not specified
    let limit_days = params.limit_days.unwrap_or(30);

    // Aggregate token buckets by calendar day. Legacy rows (migrated
    // from the pre-bucket schema) have 0 in all bucket columns so they
    // don't contribute to the new aggregations; their name/value stay
    // readable via direct queries for historical purposes.
    let results = db
        .call(move |conn| {
            let mut stmt = conn.prepare(
                r#"
            SELECT DATE(timestamp) AS day,
                   SUM(input),
                   SUM(output),
                   SUM(cache_read),
                   SUM(cache_write),
                   SUM(reasoning)
            FROM metric_event
            WHERE timestamp >= datetime('now', '-' || ? || ' days')
            GROUP BY day
            ORDER BY day DESC
            "#,
            )?;

            let events = stmt
                .query_map([limit_days], |row| {
                    Ok(public::MetricEvent {
                        timestamp: row.get(0)?,
                        input: row.get(1)?,
                        output: row.get(2)?,
                        cache_read: row.get(3)?,
                        cache_write: row.get(4)?,
                        reasoning: row.get(5)?,
                    })
                })?
                .filter_map(Result::ok)
                .collect::<Vec<public::MetricEvent>>();

            Ok(events)
        })
        .await?;

    Ok(Json(public::MetricsResponse { events: results }))
}

/// Create the metrics router
pub fn router() -> Router<SharedState> {
    Router::new().route("/", axum::routing::post(record_metric).get(get_metrics))
}