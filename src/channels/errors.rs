//! The default `server-errors` channel and the tracing layer that feeds it.
//!
//! [`ErrorChannelLayer`] is installed in the server's tracing subscriber. Every
//! `ERROR`-level event is rendered as one line and published to the in-process
//! [`registry`], so loops can subscribe to [`SERVER_ERROR_CHANNEL`] and react to
//! failures. Lower levels are ignored, which also keeps the layer from feeding
//! its own diagnostics back into the channel (the channel's subscriber logs
//! warnings on lag, not errors).

use std::fmt;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

use super::registry;

/// The default channel that receives the server's `ERROR`-level log events.
pub const SERVER_ERROR_CHANNEL: &str = "server-errors";

/// Register the error channel's in-process publisher.
///
/// Idempotent; existing subscribers keep their receivers. Call this before
/// installing [`ErrorChannelLayer`] so boot-time errors are not dropped.
pub fn register() {
    registry::ensure_registered(SERVER_ERROR_CHANNEL);
}

/// A tracing layer that forwards `ERROR` events to a channel.
#[derive(Debug, Clone)]
pub struct ErrorChannelLayer {
    channel: String,
}

impl ErrorChannelLayer {
    /// Forward `ERROR` events to `channel`.
    pub fn new(channel: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
        }
    }
}

impl Default for ErrorChannelLayer {
    fn default() -> Self {
        Self::new(SERVER_ERROR_CHANNEL)
    }
}

impl<S: Subscriber> Layer<S> for ErrorChannelLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if let Some(message) = format_event(event) {
            // Non-blocking send: with no subscriber the event is dropped, and a
            // lagging subscriber misses events rather than stalling logging.
            let _ = registry::publish(&self.channel, message);
        }
    }
}

/// Render an `ERROR` event as a single channel message, or `None` for lower
/// levels.
fn format_event(event: &Event<'_>) -> Option<String> {
    if *event.metadata().level() != Level::ERROR {
        return None;
    }

    let mut visitor = EventVisitor::default();
    event.record(&mut visitor);

    let mut message = format!("{} ERROR {}", timestamp(), event.metadata().target());
    if !visitor.message.is_empty() {
        message.push_str(": ");
        message.push_str(&visitor.message);
    }
    for field in &visitor.fields {
        message.push(' ');
        message.push_str(field);
    }
    Some(message)
}

/// Current UTC time as an RFC 3339 string with second precision.
fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Collects an event's `message` and remaining fields for formatting.
#[derive(Default)]
struct EventVisitor {
    message: String,
    fields: Vec<String>,
}

impl Visit for EventVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.fields.push(format!("{}={}", field.name(), value));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message.push_str(&format!("{:?}", value));
        } else {
            self.fields.push(format!("{}={:?}", field.name(), value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;
    use uuid::Uuid;

    /// Run `f` under a subscriber with an error layer for `channel`, returning
    /// the events it published.
    fn capture_with(channel: &str, f: impl FnOnce()) -> Vec<String> {
        registry::ensure_registered(channel);
        let mut rx = registry::subscribe(channel).unwrap();

        let subscriber = tracing_subscriber::registry().with(ErrorChannelLayer::new(channel));
        tracing::subscriber::with_default(subscriber, f);

        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    #[test]
    fn error_events_are_published_with_level_target_and_fields() {
        let channel = format!("errors-test-{}", Uuid::new_v4());

        let events = capture_with(&channel, || {
            tracing::error!(target: "hq::test", code = 7, "boom");
        });

        assert_eq!(events.len(), 1, "{events:?}");
        let event = &events[0];
        assert!(event.contains("ERROR hq::test"), "{event}");
        assert!(event.contains("boom"), "{event}");
        assert!(event.contains("code=7"), "{event}");
    }

    #[test]
    fn non_error_events_are_not_published() {
        let channel = format!("errors-test-{}", Uuid::new_v4());

        let events = capture_with(&channel, || {
            tracing::warn!(target: "hq::test", "warn");
            tracing::info!(target: "hq::test", "info");
        });

        assert!(events.is_empty(), "{events:?}");
    }

    #[test]
    fn register_makes_the_default_channel_subscribable() {
        register();
        assert!(registry::subscribe(SERVER_ERROR_CHANNEL).is_some());
    }
}
