//! Public types for the calendar API
use serde::Deserialize;

// Event shapes are produced by the library, so they live there and are
// re-exported here for API consumers.
pub use hq::types::calendar::{CalendarAttendee, CalendarResponse};

#[derive(Deserialize)]
pub struct CalendarQuery {
    pub email: String,
    pub days_ahead: Option<i64>,
    pub calendar_id: Option<String>,
}
