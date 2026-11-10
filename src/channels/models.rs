use std::convert::Infallible;
use std::str::FromStr;

use anyhow::bail;

/// How a channel is served — where its events come from.
///
/// Mirrors the `channel.source` column. This is distinct from a channel's
/// *owner*: the nullable `created_by` column naming the agent session that
/// created it, if any (see [`Channel::created_by`]). A channel's source says
/// nothing about who may emit to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelSource {
    /// Served in-process by a server publisher: the default timer channels and
    /// webhooks.
    Server,
    /// An external publisher broadcasting over a Unix domain socket.
    Socket,
}

impl ChannelSource {
    /// The `source` column value for this source.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Socket => "socket",
        }
    }
}

impl FromStr for ChannelSource {
    type Err = Infallible;

    /// Parse a `source` column value. Unknown values (including the reserved
    /// `webhook`) are treated as [`Server`], since they are published
    /// in-process.
    ///
    /// [`Server`]: ChannelSource::Server
    fn from_str(source: &str) -> Result<Self, Self::Err> {
        Ok(match source {
            "socket" => Self::Socket,
            _ => Self::Server,
        })
    }
}

/// An event source that loops can subscribe to.
#[derive(Debug, Clone)]
pub struct Channel {
    pub id: String,
    /// How the channel is served (`server` or `socket`).
    pub source: ChannelSource,
    /// The agent session that created the channel, if an agent created it via a
    /// tool. `None` for server- and socket-defined channels. Only this owner may
    /// emit to the channel; see [`Channel::is_owned_by`].
    pub created_by: Option<String>,
    /// Debounce window (ms) coalescing a channel's line burst into one event.
    pub debounce_ms: i64,
    pub created_at: String,
}

impl Channel {
    /// Whether `session_id` is this channel's owner (the agent that created it).
    pub fn is_owned_by(&self, session_id: &str) -> bool {
        self.created_by.as_deref() == Some(session_id)
    }
}

/// Validate a channel ID. Must be non-empty and alphanumeric with dashes/underscores.
///
/// This prevents path traversal (`../`), embedded null bytes, and slash separators
/// from reaching socket path construction.
pub fn validate_channel_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty() {
        bail!("channel ID cannot be empty");
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("channel ID must be alphanumeric with dashes/underscores only");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_distinguishes_socket_and_server() {
        assert_eq!(
            "socket".parse::<ChannelSource>().unwrap(),
            ChannelSource::Socket
        );
        assert_eq!(
            "server".parse::<ChannelSource>().unwrap(),
            ChannelSource::Server
        );
    }

    #[test]
    fn from_str_treats_webhook_and_unknown_as_server() {
        assert_eq!(
            "webhook".parse::<ChannelSource>().unwrap(),
            ChannelSource::Server
        );
        assert_eq!("".parse::<ChannelSource>().unwrap(), ChannelSource::Server);
    }

    #[test]
    fn source_round_trips_through_column() {
        for source in [ChannelSource::Server, ChannelSource::Socket] {
            assert_eq!(source.as_str().parse::<ChannelSource>().unwrap(), source);
        }
    }

    #[test]
    fn is_owned_by_matches_created_by() {
        let owned = Channel {
            id: "c".to_string(),
            source: ChannelSource::Server,
            created_by: Some("sess-1".to_string()),
            debounce_ms: 0,
            created_at: String::new(),
        };
        assert!(owned.is_owned_by("sess-1"));
        assert!(!owned.is_owned_by("sess-2"));

        let unowned = Channel {
            created_by: None,
            ..owned
        };
        assert!(!unowned.is_owned_by("sess-1"));
    }

    #[test]
    fn test_validate_channel_id_valid() {
        assert!(validate_channel_id("my-channel").is_ok());
        assert!(validate_channel_id("channel_123").is_ok());
        assert!(validate_channel_id("a").is_ok());
        assert!(validate_channel_id("abc-123_def").is_ok());
    }

    #[test]
    fn test_validate_channel_id_empty() {
        let err = validate_channel_id("").unwrap_err().to_string();
        assert!(err.contains("cannot be empty"), "{err}");
    }

    #[test]
    fn test_validate_channel_id_rejects_special_chars() {
        let cases = [
            "../channel",
            "channel/../foo",
            "channel\x00name",
            "channel space",
            "channel.name",
            "channel@host",
            "",
        ];
        for id in cases {
            if id.is_empty() {
                continue; // tested separately
            }
            assert!(
                validate_channel_id(id).is_err(),
                "expected '{id}' to be rejected"
            );
        }
    }

    #[test]
    fn test_validate_channel_id_rejects_path_traversal_attempts() {
        // These look like they might slip through naive validation
        let attacks = ["../etc", "foo/bar", "a/b/c", "/absolute"];
        for id in attacks {
            assert!(
                validate_channel_id(id).is_err(),
                "expected '{id}' to be rejected"
            );
        }
    }
}
