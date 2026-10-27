//! System-message section describing channels, loops, and the default timer
//! channels. Appended to the main chat's system prompt so the model knows how
//! to schedule work and which channels are available.

use std::time::Duration;

use super::default::DEFAULT_TIMER_CHANNELS;
use super::errors::SERVER_ERROR_CHANNEL;
use super::notes::NEW_NOTE_CHANNEL;

/// Human-readable label for a timer interval (e.g. `every 30 minutes`).
fn interval_label(interval: Duration) -> String {
    let secs = interval.as_secs();
    if secs.is_multiple_of(3600) {
        let hours = secs / 3600;
        format!("every {hours} hour{}", if hours == 1 { "" } else { "s" })
    } else if secs.is_multiple_of(60) {
        format!("every {} minutes", secs / 60)
    } else {
        format!("every {secs} seconds")
    }
}

/// System-message section describing the channel tools. Shared by the chat
/// prompt and any loop configured with `create_channel`/`emit_channel`.
pub fn channel_tools_section() -> &'static str {
    "\n\n## Channels\n\
     `create_channel` creates a channel you own and returns its id; `emit_channel` \
     publishes a message to a channel you created. Only the agent that created a channel \
     can emit to it, and other loops can subscribe to it by id."
}

/// System-message section describing channels and loops, including the default
/// timer channels.
pub fn system_prompt_section() -> String {
    let channels = DEFAULT_TIMER_CHANNELS
        .iter()
        .map(|(id, interval)| format!("- {id}: {}", interval_label(*interval)))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "\n\n## Channels and Loops\n\
         Channels are streams of events. A loop is a persistent subscriber: each event \
         published to a channel runs an LLM turn with the loop's system prompt and tools. \
         Use the `create_loop` tool to create a loop subscribed to one or more channels, \
         `list_loops` to inspect them, and `delete_loop` to remove one. Loops keep running \
         across server restarts.\n\n\
         The following timer channels are always available; each publishes a `tick` event \
         with the current UTC time on a fixed interval:\n\
         {channels}\n\n\
         The `{errors}` channel publishes every ERROR-level server log event, so a loop \
         subscribed to it can react to failures.\n\n\
         The `{notes}` channel publishes an event whenever a new org note is indexed, \
         carrying the note's title, path, and a short body excerpt.\n\n\
         To run something on a schedule, create a loop subscribed to the matching timer channel.{channel_tools}",
        errors = SERVER_ERROR_CHANNEL,
        notes = NEW_NOTE_CHANNEL,
        channel_tools = channel_tools_section()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_section_lists_default_channels() {
        let section = system_prompt_section();
        assert!(section.contains("## Channels and Loops"));
        assert!(section.contains("create_loop"));
        for (id, _) in DEFAULT_TIMER_CHANNELS {
            assert!(section.contains(id), "missing channel {id}");
        }
        assert!(section.contains("every 30 minutes"), "{section}");
        assert!(section.contains("every 1 hour"), "{section}");
        assert!(section.contains("every 4 hours"), "{section}");
        assert!(section.contains("every 12 hours"), "{section}");
        assert!(section.contains(SERVER_ERROR_CHANNEL), "{section}");
        assert!(
            section.contains("ERROR-level server log event"),
            "{section}"
        );
        assert!(section.contains(NEW_NOTE_CHANNEL), "{section}");
        assert!(section.contains("new org note is indexed"), "{section}");
    }

    #[test]
    fn system_prompt_section_describes_channel_tools() {
        let section = system_prompt_section();
        assert!(section.contains("create_channel"));
        assert!(section.contains("emit_channel"));
    }
}
