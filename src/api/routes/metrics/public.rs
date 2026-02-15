//! Public types for the metrics API
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub enum MetricName {
    #[serde(rename = "token-count")]
    TokenCount,
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
