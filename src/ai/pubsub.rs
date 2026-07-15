//! In-memory pub/sub broker for chat sessions.
//!
//! Any code with access to `AppState` can publish a [`Message`] to a
//! named channel. Chat sessions subscribe to channels via their
//! [`crate::ai::chat::session::ChatSessionManager`]; messages are
//! delivered to subscriber sessions and processed through their normal
//! `Chat::next_msg` loop.
//!
//! Channels are open-ended strings — there is no central registry. A
//! channel exists as long as it has at least one subscriber; channels
//! with no subscribers are cleaned up lazily on `publish`.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::openai::Message;

/// Opaque identifier returned by [`PubSubBroker::subscribe`] so callers
/// can later unsubscribe without exposing the underlying sender.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubscriptionId(Uuid);

/// Internal handle representing one subscriber's registration on a
/// channel. The sender delivers [`Message`]s to the subscriber's task.
struct SubscriberHandle {
    id: SubscriptionId,
    sender: UnboundedSender<Message>,
}

/// In-memory pub/sub broker. Fan-out is synchronous and unbuffered —
/// `publish` clones the message once per active subscriber and sends it
/// on each channel. Closed senders (subscriber task dropped) are
/// filtered out during publish and cleaned up.
///
/// Shared across the process via `AppState`. Clone the `Arc<PubSubBroker>`
/// to pass into handlers, jobs, and ChatSessionManager.
#[derive(Clone)]
pub struct PubSubBroker {
    channels: Arc<RwLock<HashMap<String, Vec<SubscriberHandle>>>>,
}

impl PubSubBroker {
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Subscribe to a channel. Returns a [`SubscriptionId`] that can be
    /// passed to [`unsubscribe`](Self::unsubscribe) to stop delivery.
    /// The provided sender will receive a clone of every [`Message`]
    /// published to the channel while this subscription is active.
    ///
    /// The channel is created lazily if it does not already exist.
    pub fn subscribe(&self, channel: &str, sender: UnboundedSender<Message>) -> SubscriptionId {
        let id = SubscriptionId(Uuid::new_v4());
        let handle = SubscriberHandle { id: id.clone(), sender };
        let mut channels = self
            .channels
            .write()
            .expect("PubSubBroker channels lock poisoned");
        channels
            .entry(channel.to_string())
            .or_default()
            .push(handle);
        id
    }

    /// Remove a subscription from a channel. No-op if the channel or
    /// subscription ID doesn't exist. If the channel becomes empty as a
    /// result, it is removed from the map.
    pub fn unsubscribe(&self, channel: &str, id: SubscriptionId) {
        let mut channels = self
            .channels
            .write()
            .expect("PubSubBroker channels lock poisoned");
        if let Some(subs) = channels.get_mut(channel) {
            subs.retain(|h| h.id != id);
            if subs.is_empty() {
                channels.remove(channel);
            }
        }
    }

    /// Publish a message to all subscribers of a channel. Each
    /// subscriber receives a clone of the message. Subscribers whose
    /// sender has closed (their task dropped) are filtered out and the
    /// channel's subscriber list is pruned in place.
    ///
    /// No-op if the channel has no subscribers (the message is simply
    /// dropped — pub/sub does not queue messages for absent
    /// subscribers).
    pub fn publish(&self, channel: &str, msg: Message) {
        let mut channels = self
            .channels
            .write()
            .expect("PubSubBroker channels lock poisoned");
        if let Some(subs) = channels.get_mut(channel) {
            // Drop closed senders while delivering. Retain only those
            // that successfully accepted the message (or are still open).
            subs.retain(|h| {
                if h.sender.is_closed() {
                    return false;
                }
                // UnboundedSender::send only errors if the receiver was
                // dropped, which we just checked — but check anyway in
                // case of a race.
                h.sender.send(msg.clone()).is_ok()
            });
            if subs.is_empty() {
                channels.remove(channel);
            }
        }
    }

    /// Return the number of active subscribers on a channel. Mainly
    /// useful for diagnostics and tests.
    pub fn subscriber_count(&self, channel: &str) -> usize {
        let channels = self
            .channels
            .read()
            .expect("PubSubBroker channels lock poisoned");
        channels.get(channel).map(|v| v.len()).unwrap_or(0)
    }
}

impl Default for PubSubBroker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_subscribe_publish_delivers_to_subscriber() {
        let broker = PubSubBroker::new();
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        let _id = broker.subscribe("test", tx);

        broker.publish(
            "test",
            Message::new(crate::openai::Role::User, "hello"),
        );

        let msg = rx.recv().await.expect("Should receive message");
        assert_eq!(msg.content.as_deref(), Some("hello"));
    }

    #[tokio::test]
    async fn test_multiple_subscribers_all_receive() {
        let broker = PubSubBroker::new();
        let (tx1, mut rx1) = mpsc::unbounded_channel::<Message>();
        let (tx2, mut rx2) = mpsc::unbounded_channel::<Message>();
        let _id1 = broker.subscribe("multi", tx1);
        let _id2 = broker.subscribe("multi", tx2);

        broker.publish(
            "multi",
            Message::new(crate::openai::Role::User, "broadcast"),
        );

        let m1 = rx1.recv().await.expect("Sub 1 should receive");
        let m2 = rx2.recv().await.expect("Sub 2 should receive");
        assert_eq!(m1.content.as_deref(), Some("broadcast"));
        assert_eq!(m2.content.as_deref(), Some("broadcast"));
    }

    #[tokio::test]
    async fn test_unsubscribe_stops_delivery() {
        let broker = PubSubBroker::new();
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        let id = broker.subscribe("chan", tx);

        // First publish should deliver
        broker.publish(
            "chan",
            Message::new(crate::openai::Role::User, "first"),
        );
        let m = rx.recv().await.expect("First publish should deliver");
        assert_eq!(m.content.as_deref(), Some("first"));

        broker.unsubscribe("chan", id);

        // Second publish should not deliver
        broker.publish(
            "chan",
            Message::new(crate::openai::Role::User, "second"),
        );
        assert!(
            rx.try_recv().is_err(),
            "Should not receive after unsubscribe"
        );
    }

    #[tokio::test]
    async fn test_publish_to_no_subscribers_is_noop() {
        let broker = PubSubBroker::new();
        // Should not panic
        broker.publish(
            "nonexistent",
            Message::new(crate::openai::Role::User, "ignored"),
        );
        assert_eq!(broker.subscriber_count("nonexistent"), 0);
    }

    #[tokio::test]
    async fn test_closed_sender_is_filtered() {
        let broker = PubSubBroker::new();
        let (tx, _rx) = mpsc::unbounded_channel::<Message>();
        let _id = broker.subscribe("closed", tx);
        // Drop the receiver — sender becomes... actually, unbounded
	// senders only close when the receiver is dropped. Drop _rx here.
        drop(_rx);

        // Give the runtime a moment to let the channel close
        tokio::task::yield_now().await;

        // publish should filter out the closed sender without panic
        broker.publish(
            "closed",
            Message::new(crate::openai::Role::User, "filtered"),
        );
        // Channel should be cleaned up
        assert_eq!(broker.subscriber_count("closed"), 0);
    }

    #[tokio::test]
    async fn test_subscriber_count_tracks_active_subs() {
        let broker = PubSubBroker::new();
        assert_eq!(broker.subscriber_count("counted"), 0);

        let (tx1, _rx1) = mpsc::unbounded_channel::<Message>();
        let (tx2, _rx2) = mpsc::unbounded_channel::<Message>();
        let id1 = broker.subscribe("counted", tx1);
        let _id2 = broker.subscribe("counted", tx2);
        assert_eq!(broker.subscriber_count("counted"), 2);

        broker.unsubscribe("counted", id1);
        assert_eq!(broker.subscriber_count("counted"), 1);
    }
}