//! Serialized shapes shared between the library and the HTTP API.
//!
//! The library produces these values (search results, calendar events, email
//! threads) and the API serves them. They live here, rather than in the API
//! crate, so library code does not depend on the HTTP layer.

pub mod notes {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    pub struct SearchResult {
        pub id: String,
        pub r#type: String,
        pub title: String,
        pub category: String,
        pub file_name: String,
        pub tags: Option<String>,
        pub is_task: bool,
        pub task_status: Option<String>,
        pub task_scheduled: Option<String>,
        pub task_deadline: Option<String>,
        pub task_closed: Option<String>,
        pub meeting_date: Option<String>,
        pub body: String,
        #[serde(default)]
        pub chat_role: Option<String>,
        /// For chat results, the session the message belongs to.
        #[serde(default)]
        pub session_id: Option<String>,
    }

    #[derive(Serialize, Deserialize)]
    pub struct SearchResponse {
        pub raw_query: String,
        pub parsed_query: String,
        pub results: Vec<SearchResult>,
    }
}

pub mod calendar {
    use serde::{Deserialize, Serialize};

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
}

pub mod email {
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Serialize, Deserialize)]
    pub struct EmailMessage {
        pub id: String,
        pub thread_id: String,
        pub from: String,
        pub to: String,
        pub received: String,
        pub subject: String,
        pub body: String,
    }

    #[derive(Clone, Serialize, Deserialize)]
    pub struct EmailThread {
        pub id: String,
        pub received: String,
        pub from: String,
        pub to: String,
        pub subject: String,
        pub messages: Vec<EmailMessage>,
    }
}
