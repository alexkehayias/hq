//! Router for the push API

use std::sync::{Arc, RwLock};

use axum::{Json, Router, extract::State};
use serde_json::Value;

use super::public;
use crate::api::state::AppState;
use crate::notify::{
    mark_push_subscription_invalid, broadcast_push_notification,
    find_all_notification_subscriptions, PushNotificationPayload,
};

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
            // Use INSERT OR REPLACE to handle re-subscriptions
            let mut subscription_stmt = conn.prepare(
                "INSERT OR REPLACE INTO push_subscription(endpoint, p256dh, auth, is_valid) VALUES (?, ?, ?, 1)",
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

    let db = state.read().unwrap().db.clone();
    let subscriptions = find_all_notification_subscriptions(&db).await.unwrap_or_default();

    let notification_payload = PushNotificationPayload::new(
        "Notification",
        &payload.message,
        None,
        None,
        Some("index_updated"),
    );

    let failed_subscriptions =
        broadcast_push_notification(subscriptions, vapid_key_path, notification_payload).await;

    // Mark failed subscriptions as invalid
    for sub in failed_subscriptions {
        let _ = mark_push_subscription_invalid(&db, &sub.endpoint).await;
    }

    Ok(Json(serde_json::json!({ "success": true })))
}

/// Create the push router
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/subscribe", axum::routing::post(push_subscription))
        .route("/notification", axum::routing::post(send_notification))
}
