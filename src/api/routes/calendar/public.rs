//! Public types for the calendar API
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct CalendarQuery {
    pub email: String,
    pub days_ahead: Option<i64>,
    pub calendar_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct CalendarAttendee {
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct CalendarResponse {
    pub id: String,
    pub summary: String,
    pub start: String, // Using String for datetime to maintain compatibility
    pub end: String,   // Using String for datetime to maintain compatibility
    pub attendees: Option<Vec<CalendarAttendee>>,
}
