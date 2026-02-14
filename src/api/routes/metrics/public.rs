//! Public types for the metrics API
use rusqlite::{ToSql, types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef}};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub enum MetricName {
    #[serde(rename = "token-count")]
    TokenCount,
}

impl ToSql for MetricName {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        // Use serde serialization to convert the enum back into a
        // string to save to the database while still enforcing metric
        // names can only be a `MetricName` variant.
        let name = serde_json::to_string(self).expect("Failed to parse enum into string");
        let value: String = serde_json::from_str(&name).expect("Failed to parse string from enum");
        Ok(value.into())
    }
}

impl FromSql for MetricName {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        // Serde deserialization can only parse an enum from string if
        // it's double quoted.
        serde_json::from_str(&format!("\"{}\"", value.as_str()?))
            .map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

/// Request to record a metric event
#[derive(Deserialize)]
pub struct MetricRequest {
    pub name: MetricName,
    pub value: i64,
}

/// Query parameters for getting metric events
#[derive(Deserialize)]
pub struct MetricsQuery {
    pub limit_days: Option<i64>,
}

/// A single metric event
#[derive(Serialize)]
pub struct MetricEvent {
    pub name: MetricName,
    pub timestamp: String,
    pub value: i64,
}

/// Response containing metric events
#[derive(Serialize)]
pub struct MetricsResponse {
    pub events: Vec<MetricEvent>,
}
