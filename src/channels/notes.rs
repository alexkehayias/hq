//! The default `new-notes` channel, fed when a new org note is indexed.
//!
//! [`publish_new_note`] is called by the indexer (`crate::search::indexing`)
//! after a note's metadata is first written, so loops subscribed to
//! [`NEW_NOTE_CHANNEL`] can react to notes arriving in the notes repo. It
//! publishes to the in-process [`registry`]; with no registered publisher (for
//! example when indexing from the CLI) the event is simply dropped.

use super::registry;

/// The default channel that receives an event when a new org note is indexed.
pub const NEW_NOTE_CHANNEL: &str = "new-notes";

/// Maximum number of characters of the note body included in an event.
const EXCERPT_MAX_CHARS: usize = 200;

/// Register the new-notes channel's in-process publisher.
///
/// Idempotent; existing subscribers keep their receivers. Call this at boot so
/// loops can subscribe before the first note arrives.
pub fn register() {
    registry::ensure_registered(NEW_NOTE_CHANNEL);
}

/// Publish an event describing a newly indexed note.
///
/// Returns `false` when no publisher is registered (e.g. indexing from the
/// CLI), in which case nothing is emitted.
pub fn publish_new_note(file_name: &str, title: &str, body: &str) -> bool {
    registry::publish(NEW_NOTE_CHANNEL, format_event(file_name, title, body))
}

/// Render a new-note event as a single channel message.
fn format_event(file_name: &str, title: &str, body: &str) -> String {
    let mut message = format!("new note: {title} ({file_name})");
    let excerpt = excerpt(body);
    if !excerpt.is_empty() {
        message.push_str(" — ");
        message.push_str(&excerpt);
    }
    message
}

/// Collapse whitespace in `body` to single spaces and truncate to
/// [`EXCERPT_MAX_CHARS`] characters, appending `…` when truncated.
fn excerpt(body: &str) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= EXCERPT_MAX_CHARS {
        return collapsed;
    }
    let truncated: String = collapsed.chars().take(EXCERPT_MAX_CHARS).collect();
    format!("{truncated}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpt_collapses_whitespace_and_truncates() {
        assert_eq!(excerpt("  hello   world \n next "), "hello world next");

        let long = "a".repeat(EXCERPT_MAX_CHARS + 10);
        let got = excerpt(&long);
        assert_eq!(got.chars().count(), EXCERPT_MAX_CHARS + 1);
        assert!(got.ends_with('…'), "{got}");
    }

    #[test]
    fn format_event_includes_title_path_and_excerpt() {
        let msg = format_event("notes/foo.org", "Foo", "Some body text");
        assert_eq!(msg, "new note: Foo (notes/foo.org) — Some body text");
    }

    #[test]
    fn format_event_omits_empty_excerpt() {
        let msg = format_event("notes/foo.org", "Foo", "   \n  ");
        assert_eq!(msg, "new note: Foo (notes/foo.org)");
    }

    #[test]
    fn publish_new_note_reaches_subscribers() {
        register();
        let mut rx = registry::subscribe(NEW_NOTE_CHANNEL).expect("channel is registered");

        assert!(publish_new_note("notes/foo.org", "Foo", "body"));

        let event = rx.try_recv().expect("an event was published");
        assert_eq!(event, "new note: Foo (notes/foo.org) — body");
    }
}
