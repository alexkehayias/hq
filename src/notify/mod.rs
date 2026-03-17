pub mod db;
pub mod models;
pub use db::*;
pub use models::*;

use anyhow::Error;
use web_push::{
    ContentEncoding, HyperWebPushClient, SubscriptionInfo, VapidSignatureBuilder, WebPushClient,
    WebPushMessageBuilder,
};

/// Result of sending a push notification
#[derive(Debug)]
pub enum PushSendResult {
    Success,
    InvalidSubscription,   // Subscription is no longer valid (410 Gone)
    RetryableError(Error), // Temporary error that could be retried
}

/// Send a single push notification and return whether it succeeded or failed
pub async fn send_push_notification(
    vapid_private_pem_path: String,
    endpoint: String,
    p256dh: String,
    auth: String,
    payload: PushNotificationPayload,
) -> PushSendResult {
    // Create subscription info
    let subscription_info = SubscriptionInfo::new(endpoint.clone(), p256dh, auth);

    // Read the VAPID signing material from the PEM file
    let file = match std::fs::File::open(vapid_private_pem_path) {
        Ok(f) => f,
        Err(e) => return PushSendResult::RetryableError(Error::new(e)),
    };

    let sig_builder = match VapidSignatureBuilder::from_pem(file, &subscription_info) {
        Ok(builder) => builder,
        Err(e) => return PushSendResult::RetryableError(Error::new(e)),
    };

    let vapid_signature = match sig_builder.build() {
        Ok(sig) => sig,
        Err(e) => return PushSendResult::RetryableError(Error::new(e)),
    };

    // Create the message with payload
    let mut builder = WebPushMessageBuilder::new(&subscription_info);
    let content = match serde_json::to_string(&payload) {
        Ok(c) => c,
        Err(e) => return PushSendResult::RetryableError(Error::new(e)),
    };
    builder.set_payload(ContentEncoding::Aes128Gcm, content.as_bytes());
    builder.set_vapid_signature(vapid_signature);
    let message = match builder.build() {
        Ok(m) => m,
        Err(e) => return PushSendResult::RetryableError(Error::new(e)),
    };

    // Send the notification
    let client = HyperWebPushClient::new();
    match client.send(message).await {
        Ok(()) => PushSendResult::Success,
        Err(e) => {
            let error_str = format!("{:?}", e);
            // Check for 410 Gone - subscription is no longer valid
            if error_str.contains("410") || error_str.contains("Gone") {
                tracing::info!("Subscription no longer valid for {}: {}", endpoint, e);
                PushSendResult::InvalidSubscription
            } else {
                tracing::warn!("Failed to send push notification to {}: {}", endpoint, e);
                PushSendResult::RetryableError(Error::new(e))
            }
        }
    }
}

/// Broadcast a push notification to all subscriptions.
/// Returns a list of subscriptions that failed with InvalidSubscription
/// so they can be marked as invalid in the database.
pub async fn broadcast_push_notification(
    subscriptions: Vec<PushSubscription>,
    vapid_key_path: String,
    payload: PushNotificationPayload,
) -> Vec<PushSubscription> {
    let mut tasks = tokio::task::JoinSet::new();
    let mut failed_subscriptions = Vec::new();

    for sub in subscriptions {
        let vapid = vapid_key_path.clone();
        // Clone the necessary fields to move into the async block
        let endpoint = sub.endpoint.clone();
        let p256dh = sub.p256dh.clone();
        let auth = sub.auth.clone();
        let payload_clone = payload.clone();
        tasks.spawn(async move {
            (
                sub,
                send_push_notification(vapid, endpoint, p256dh, auth, payload_clone).await,
            )
        });
    }

    while let Some(result) = tasks.join_next().await {
        if let Ok((sub, send_result)) = result
            && matches!(send_result, PushSendResult::InvalidSubscription) {
                failed_subscriptions.push(sub);
            }
    }

    failed_subscriptions
}
