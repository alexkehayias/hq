//! Per-session buffer of SSE events so a reconnecting client can resume an
//! in-flight chat response.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use axum::response::sse::Event;
use tokio::sync::broadcast;
use tokio_stream::Stream;

/// A single buffered SSE payload with a monotonically increasing id.
#[derive(Clone)]
struct StreamEvent {
    id: u64,
    payload: String,
}

struct Inner {
    events: Vec<StreamEvent>,
    next_id: u64,
    done: bool,
}

/// Buffers the SSE payloads for one chat turn and fans them out to any
/// subscribers.
///
/// The `events` buffer is authoritative; `notify` only signals that new events
/// exist. Because subscribers always re-read the buffer, a slow subscriber can
/// never lose data to a lagged broadcast.
pub struct SessionStream {
    inner: Mutex<Inner>,
    notify: broadcast::Sender<()>,
}

impl SessionStream {
    fn new() -> Self {
        let (notify, _) = broadcast::channel(16);
        Self {
            inner: Mutex::new(Inner {
                events: Vec::new(),
                next_id: 1,
                done: false,
            }),
            notify,
        }
    }

    /// Append a payload and wake subscribers. Returns the assigned id.
    pub fn publish(&self, payload: String) -> u64 {
        let id = {
            let mut inner = self.inner.lock().expect("stream lock poisoned");
            let id = inner.next_id;
            inner.next_id += 1;
            inner.events.push(StreamEvent { id, payload });
            id
        };
        let _ = self.notify.send(());
        id
    }

    /// Mark the turn complete and wake subscribers so they can finish.
    pub fn finish(&self) {
        {
            let mut inner = self.inner.lock().expect("stream lock poisoned");
            inner.done = true;
        }
        let _ = self.notify.send(());
    }

    pub fn is_done(&self) -> bool {
        self.inner.lock().expect("stream lock poisoned").done
    }

    /// Number of subscribers currently attached. Used to decide whether a push
    /// notification is warranted when the turn completes.
    pub fn subscriber_count(&self) -> usize {
        self.notify.receiver_count()
    }

    /// Stream buffered events after `after`, then live events, then a terminal
    /// `done` event.
    pub fn subscribe(
        self: &Arc<Self>,
        after: u64,
    ) -> impl Stream<Item = Result<Event, Infallible>> + Send + 'static {
        let stream = Arc::clone(self);
        async_stream::stream! {
            // Subscribe before the first buffer read so a publish that lands
            // between the two can't be missed.
            let mut rx = stream.notify.subscribe();
            let mut cursor = after;
            loop {
                let (pending, done) = {
                    let inner = stream.inner.lock().expect("stream lock poisoned");
                    let pending: Vec<StreamEvent> = inner
                        .events
                        .iter()
                        .filter(|event| event.id > cursor)
                        .cloned()
                        .collect();
                    (pending, inner.done)
                };
                for event in pending {
                    cursor = event.id;
                    yield Ok(Event::default().id(event.id.to_string()).data(event.payload));
                }
                if done {
                    yield Ok(Event::default().event("done").data(""));
                    break;
                }
                match rx.recv().await {
                    // Lagged is fine: we re-read the buffer on the next loop.
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

/// Maps session id -> the stream for its current (or most recent) turn.
pub type ChatStreamRegistry = Arc<Mutex<HashMap<String, Arc<SessionStream>>>>;

pub fn new_registry() -> ChatStreamRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Register a fresh stream for a new turn, replacing any previous one.
pub fn register(registry: &ChatStreamRegistry, session_id: &str) -> Arc<SessionStream> {
    let stream = Arc::new(SessionStream::new());
    registry
        .lock()
        .expect("registry lock poisoned")
        .insert(session_id.to_string(), Arc::clone(&stream));
    stream
}

/// Look up the current stream for a session without creating one.
pub fn get(registry: &ChatStreamRegistry, session_id: &str) -> Option<Arc<SessionStream>> {
    registry
        .lock()
        .expect("registry lock poisoned")
        .get(session_id)
        .cloned()
}

/// Remove a stream only if it is still the one we expect, so a late eviction
/// can't delete a newer turn's stream.
pub fn remove_if_same(
    registry: &ChatStreamRegistry,
    session_id: &str,
    stream: &Arc<SessionStream>,
) {
    let mut map = registry.lock().expect("registry lock poisoned");
    let is_current = map
        .get(session_id)
        .is_some_and(|existing| Arc::ptr_eq(existing, stream));
    if is_current {
        map.remove(session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::StreamExt;

    async fn collect_ids(stream: &Arc<SessionStream>, after: u64) -> Vec<String> {
        let mut events = Vec::new();
        let s = stream.subscribe(after);
        tokio::pin!(s);
        while let Some(item) = s.next().await {
            let event = item.expect("infallible");
            let debug = format!("{event:?}");
            events.push(debug);
        }
        events
    }

    #[tokio::test]
    async fn replays_buffered_events_after_cursor() {
        let stream = Arc::new(SessionStream::new());
        stream.publish("a".to_string());
        stream.publish("b".to_string());
        stream.publish("c".to_string());
        stream.finish();

        let events = collect_ids(&stream, 1).await;
        assert_eq!(events.len(), 3); // b, c, done
        assert!(events[0].contains("b"));
        assert!(events[1].contains("c"));
        assert!(events[2].contains("done"));
    }

    #[tokio::test]
    async fn finish_emits_single_done() {
        let stream = Arc::new(SessionStream::new());
        stream.publish("a".to_string());
        stream.finish();

        let events = collect_ids(&stream, 0).await;
        assert_eq!(events.len(), 2); // a, done
        assert!(events[1].contains("done"));
    }

    #[tokio::test]
    async fn subscriber_count_tracks_attached_streams() {
        let stream = Arc::new(SessionStream::new());
        assert_eq!(stream.subscriber_count(), 0);
        {
            let s = stream.subscribe(0);
            tokio::pin!(s);
            // The generator body — which creates the broadcast receiver — only
            // runs on the first poll, so poll once before counting.
            let _ = tokio::time::timeout(std::time::Duration::from_millis(10), s.next()).await;
            assert_eq!(stream.subscriber_count(), 1);
        }
        assert_eq!(stream.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn registry_replaces_previous_stream() {
        let registry = new_registry();
        let first = register(&registry, "session");
        let second = register(&registry, "session");
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(get(&registry, "session").is_some());
        remove_if_same(&registry, "session", &first); // stale, must not remove
        assert!(get(&registry, "session").is_some());
        remove_if_same(&registry, "session", &second);
        assert!(get(&registry, "session").is_none());
    }
}
