//! Router for the push API

use std::sync::{Arc, RwLock};

use axum::{Json, Router, extract::State};
use serde_json::Value;

use super::public;
use crate::api::state::AppState;
use crate::notify::{PushNotificationPayload, PushSubscription, broadcast_push_notification};

type SharedState = Arc<RwLock<AppState>>;

// Register a client for push notifications
async fn push_subscription(
    State(state): State<SharedState>,
    Json(subscription): Json<public::PushSubscriptionRequest>,
) -> Result<Json<Value>, crate::api::public::ApiError> {
    let p256dh = subscription
        .keys
        .get("p256dh")
        .expect("Missing p256dh key")
        .clone();
    let auth = subscription
        .keys
        .get("auth")
        .expect("Missing auth key")
        .clone();

    {
        let db = state.read().unwrap().db.clone();
        db.call(move |conn| {
            let mut subscription_stmt = conn.prepare(
                "REPLACE INTO push_subscription(endpoint, p256dh, auth) VALUES (?, ?, ?)",
            )?;
            subscription_stmt.execute(tokio_rusqlite::params![
                subscription.endpoint,
                p256dh,
                auth,
            ])?;
            conn.execute("DELETE FROM vec_items", [])?;
            Ok(())
        })
        .await?;
    }

    Ok(Json(serde_json::json!({"success": true})))
}

// Endpoint to send push notification to all subscriptions
async fn send_notification(
    State(state): State<SharedState>,
    Json(payload): Json<public::NotificationRequest>,
) -> Result<Json<Value>, crate::api::public::ApiError> {
    let vapid_key_path = state
        .read()
        .expect("Unable to read share state")
        .config
        .vapid_key_path
        .clone();

    let subscriptions = {
        let db = state.read().unwrap().db.clone();
        db.call(move |conn| {
            let mut stmt = conn.prepare("SELECT endpoint, p256dh, auth FROM push_subscription")?;
            let result = stmt
                .query_map([], |i| {
                    Ok(PushSubscription {
                        endpoint: i.get(0)?,
                        p256dh: i.get(1)?,
                        auth: i.get(2)?,
                    })
                })?
                .filter_map(Result::ok)
                .collect::<Vec<_>>();
            Ok(result)
        })
        .await?
    };

    let notification_payload = PushNotificationPayload::new(
        "Notification",
        &payload.message,
        None,
        None,
        Some("index_updated"),
    );
    broadcast_push_notification(subscriptions, vapid_key_path, notification_payload).await;

    Ok(Json(serde_json::json!({ "success": true })))
}

/// Create the push router
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/subscribe", axum::routing::post(push_subscription))
        .route("/notification", axum::routing::post(send_notification))
}
