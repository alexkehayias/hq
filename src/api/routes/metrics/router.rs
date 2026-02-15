//! Router for the metrics API

use std::sync::{Arc, RwLock};

use axum::{Router, extract::State, http::StatusCode, response::Json};
use axum_extra::extract::Query;
use rusqlite::{ToSql, types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef}};

use crate::api::state::AppState;
use super::public;

type SharedState = Arc<RwLock<AppState>>;

impl ToSql for public::MetricName {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        // Use serde serialization to convert the enum back into a
        // string to save to the database while still enforcing metric
        // names can only be a `MetricName` variant.
        let name = serde_json::to_string(self).expect("Failed to parse enum into string");
        let value: String = serde_json::from_str(&name).expect("Failed to parse string from enum");
        Ok(value.into())
    }
}

impl FromSql for public::MetricName {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        // Serde deserialization can only parse an enum from string if
        // it's double quoted.
        serde_json::from_str(&format!("\"{}\"", value.as_str()?))
            .map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

/// Record a metric event
async fn record_metric(
    State(state): State<SharedState>,
    Json(payload): Json<public::MetricRequest>,
) -> Result<StatusCode, crate::api::public::ApiError> {
    let db = state.read().unwrap().db.clone();

    let name = payload.name;
    let value = payload.value;

    // Insert the metric event into the database
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO metric_event (name, value) VALUES (?, ?)",
            tokio_rusqlite::params![&name, &value],
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

    // Build SQL query to fetch metrics with grouping by name and timestamp
    let results = db
        .call(move |conn| {
            let mut stmt = conn.prepare(
                r#"
            SELECT name,
            DATE(timestamp) AS day,
            SUM(value) AS daily_total
            FROM metric_event
            WHERE timestamp >= datetime('now', '-' || ? || ' days')
            GROUP BY name, day
            ORDER BY name, day DESC
            "#,
            )?;

            let events = stmt
                .query_map([limit_days], |row| {
                    Ok(public::MetricEvent {
                        name: row.get(0)?,
                        timestamp: row.get(1)?,
                        value: row.get(2)?,
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
