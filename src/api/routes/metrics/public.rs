//! Public types for the metrics API
use serde::{Deserialize, Serialize};

/// Request to record a metric event.
///
/// Token buckets are normalized by the upstream provider so downstream
/// cost estimators can apply provider-specific rates (cache reads are
/// cheaper than fresh input on Anthropic; cache writes are billed at a
/// premium, etc.).
#[derive(Deserialize)]
pub struct MetricRequest {
    pub input: u32,
    pub output: u32,
    pub cache_read: u32,
    pub cache_write: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u32>,
}

/// Query parameters for getting metric events
#[derive(Deserialize)]
pub struct MetricsQuery {
    pub limit_days: Option<i64>,
}

/// A single metric event (daily aggregate).
///
/// `timestamp` is a calendar day (e.g. "2026-07-09") produced by
/// `DATE(timestamp)` grouping in the GET query — not an ISO timestamp.
#[derive(Serialize)]
pub struct MetricEvent {
    pub timestamp: String,
    pub input: u32,
    pub output: u32,
    pub cache_read: u32,
    pub cache_write: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u32>,
}

/// Response containing metric events
#[derive(Serialize)]
pub struct MetricsResponse {
    pub events: Vec<MetricEvent>,
}