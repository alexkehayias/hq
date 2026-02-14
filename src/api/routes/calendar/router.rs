//! Router for the calendar API

use std::sync::{Arc, RwLock};

use axum::{Router, extract::State, response::Json};
use axum_extra::extract::Query;

use crate::api::state::AppState;
use crate::core::AppConfig;
use crate::google::gcal::list_events;
use crate::google::oauth::refresh_access_token;
use super::public;

type SharedState = Arc<RwLock<AppState>>;

async fn calendar_handler(
    State(state): State<SharedState>,
    Query(params): Query<public::CalendarQuery>,
) -> Result<Json<Vec<public::CalendarResponse>>, crate::api::public::ApiError> {
    let refresh_token: String = {
        let db = state.read().unwrap().db.clone();

        db.call(move |conn| {
            let result = conn
                .prepare("SELECT refresh_token FROM auth WHERE id = ?1")
                .and_then(|mut stmt| stmt.query_row([&params.email], |row| row.get(0)))?;
            Ok(result)
        })
        .await?
    };

    let (client_id, client_secret) = {
        let shared_state = state.read().expect("Unable to read share state");
        let AppConfig {
            gmail_api_client_id,
            gmail_api_client_secret,
            ..
        } = &shared_state.config;
        (gmail_api_client_id.clone(), gmail_api_client_secret.clone())
    };
    let oauth = refresh_access_token(&client_id, &client_secret, &refresh_token).await?;
    let access_token = oauth.access_token;

    // Default to 7 days ahead if not specified
    let days_ahead = params.days_ahead.unwrap_or(7);

    // Default to primary calendar if not specified
    let calendar_id = params
        .calendar_id
        .clone()
        .unwrap_or_else(|| "primary".to_string());

    // Get the current time and calculate the end time
    let now = chrono::Utc::now();
    let end_time = now + chrono::Duration::days(days_ahead);

    // Fetch upcoming events
    let events = list_events(&access_token, &calendar_id, now, end_time).await?;

    // Transform events to a simpler format for the API response
    let resp = events
        .into_iter()
        .map(|event| {
            let summary = event.summary.unwrap_or_else(|| "No title".to_string());
            public::CalendarResponse {
                id: event.id,
                summary,
                start: event.start.to_rfc3339(),
                end: event.end.to_rfc3339(),
                attendees: event.attendees.map(|attendees| {
                    attendees
                        .into_iter()
                        .map(|attendee| public::CalendarAttendee {
                            email: attendee.email,
                            display_name: attendee.display_name,
                        })
                        .collect::<Vec<_>>()
                }),
            }
        })
        .collect();

    Ok(Json(resp))
}

/// Create the calendar router
pub fn router() -> Router<SharedState> {
    Router::new().route("/", axum::routing::get(calendar_handler))
}
