//! The default `new-notes` channel, fed when new entries are indexed.
//!
//! [`publish_new_note`] is called by the indexer (`crate::search::indexing`)
//! after a note's metadata is written, so loops subscribed to
//! [`NEW_NOTE_CHANNEL`] can react to entries arriving in the notes repo. It
//! publishes to the in-process [`registry`]; with no registered publisher (for
//! example when indexing from the CLI) the event is simply dropped.

use super::registry;

/// The default channel that receives an event when new entries are indexed.
pub const NEW_NOTE_CHANNEL: &str = "new-notes";

/// A single entry that was newly written to the index for a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEntry {
    /// The entry's org ID.
    pub id: String,
    /// The entry's title.
    pub title: String,
    /// The entry's kind: `note`, `task`, `meeting`, or `heading`.
    pub kind: &'static str,
}

/// Register the new-notes channel's in-process publisher.
///
/// Idempotent; existing subscribers keep their receivers. Call this at boot so
/// loops can subscribe before the first note arrives.
pub fn register() {
    registry::ensure_registered(NEW_NOTE_CHANNEL);
}

/// Publish an event describing the entries newly indexed for a note.
///
/// Returns `false` when no publisher is registered (e.g. indexing from the
/// CLI), in which case nothing is emitted.
pub fn publish_new_note(file_name: &str, note_title: &str, entries: &[NewEntry]) -> bool {
    registry::publish(
        NEW_NOTE_CHANNEL,
        format_event(file_name, note_title, entries),
    )
}

/// Render a new-note event as a single channel message: the note's title and
/// path, followed by one line per newly indexed entry.
fn format_event(file_name: &str, note_title: &str, entries: &[NewEntry]) -> String {
    let mut message = format!("new note: {note_title} ({file_name})");
    for entry in entries {
        message.push_str(&format!(
            "\n- {}: {} ({})",
            entry.kind, entry.title, entry.id
        ));
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_event_includes_title_path_and_entries() {
        let entries = vec![
            NewEntry {
                id: "id-1".to_string(),
                title: "Foo".to_string(),
                kind: "note",
            },
            NewEntry {
                id: "id-2".to_string(),
                title: "Buy milk".to_string(),
                kind: "task",
            },
        ];
        let msg = format_event("notes/foo.org", "Foo", &entries);
        assert_eq!(
            msg,
            "new note: Foo (notes/foo.org)\n- note: Foo (id-1)\n- task: Buy milk (id-2)"
        );
    }

    #[test]
    fn format_event_without_entries_is_just_the_note() {
        let msg = format_event("notes/foo.org", "Foo", &[]);
        assert_eq!(msg, "new note: Foo (notes/foo.org)");
    }

    #[test]
    fn publish_new_note_reaches_subscribers() {
        register();
        let mut rx = registry::subscribe(NEW_NOTE_CHANNEL).expect("channel is registered");

        let entries = vec![NewEntry {
            id: "id-1".to_string(),
            title: "Buy milk".to_string(),
            kind: "task",
        }];
        assert!(publish_new_note("notes/foo.org", "Foo", &entries));

        let event = rx.try_recv().expect("an event was published");
        assert_eq!(
            event,
            "new note: Foo (notes/foo.org)\n- task: Buy milk (id-1)"
        );
    }
}
