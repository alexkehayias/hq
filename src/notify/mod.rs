pub mod db;
pub mod models;
pub use db::*;
pub use models::*;

use anyhow::{Error, Result};
use web_push::{
    ContentEncoding, HyperWebPushClient, SubscriptionInfo, VapidSignatureBuilder, WebPushClient,
    WebPushMessageBuilder,
};

pub async fn send_push_notification(
    vapid_private_pem_path: String,
    endpoint: String,
    p256dh: String,
    auth: String,
    payload: PushNotificationPayload,
) -> Result<(), Error> {
    // Create subscription info
    let subscription_info = SubscriptionInfo::new(endpoint, p256dh, auth);

    // Read the VAPID signing material from the PEM file
    let file = std::fs::File::open(vapid_private_pem_path)?;
    let sig_builder = VapidSignatureBuilder::from_pem(file, &subscription_info)?.build()?;

    // Create the message with payload
    let mut builder = WebPushMessageBuilder::new(&subscription_info);
    let content = serde_json::to_string(&payload)?;
    builder.set_payload(ContentEncoding::Aes128Gcm, content.as_bytes());
    builder.set_vapid_signature(sig_builder);
    let message = builder.build()?;

    // Send the notification
    let client = HyperWebPushClient::new();
    let result = client.send(message).await;

    if let Err(error) = result {
        println!("An error occured: {:?}", error);
    }

    Ok(())
}

pub async fn broadcast_push_notification(
    subscriptions: Vec<PushSubscription>,
    vapid_key_path: String,
    payload: PushNotificationPayload,
) {
    let mut tasks = tokio::task::JoinSet::new();
    for sub in subscriptions {
        let vapid = vapid_key_path.clone();
        tasks.spawn(send_push_notification(
            vapid,
            sub.endpoint,
            sub.p256dh,
            sub.auth,
            payload.clone(),
        ));
    }
    while let Some(_res) = tasks.join_next().await {}
}
