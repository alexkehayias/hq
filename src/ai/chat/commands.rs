//! Slash commands for chat sessions using Clap for parsing.

use clap::{CommandFactory, Parser, Subcommand};

/// Result of parsing a slash command
#[derive(Debug)]
pub enum ParsedCommand {
    /// Execute the /code command to enter agent mode
    Code { prompt: String },
    /// Exit current mode and return to chat
    Exit,
    /// Show available commands (help)
    Help,
    /// Not a slash command - regular chat message
    None(String),
}

/// Slash commands parser for chat sessions.
///
/// This uses Clap to parse user messages that start with `/`, providing
/// nice error messages and help text when commands are invalid.
#[derive(Parser)]
#[command(name = "slash")]
#[command(about = "Slash commands for chat sessions", long_about = None)]
pub struct SlashCommand {
    #[command(subcommand)]
    pub command: Command,
}

/// Available slash commands
#[derive(Subcommand)]
pub enum Command {
    /// Enter Claude Code agent mode with an optional prompt
    ///
    /// Usage: /code [prompt]
    ///
    /// Examples:
    ///   /code fix this bug
    ///   /code
    Code {
        /// Prompt to send to the agent (optional)
        #[arg(num_args = 0..)]
        prompt: Vec<String>,
    },
    /// Exit current mode and return to chat
    ///
    /// Usage: /exit
    Exit,
}

/// Parse a user message and extract any slash command.
///
/// Returns `ParsedCommand::None(message)` if the message doesn't start with `/`.
pub fn parse_slash_command(message: &str) -> ParsedCommand {
    let trimmed = message.trim();

    // Only parse if message starts with /
    if !trimmed.starts_with('/') {
        return ParsedCommand::None(message.to_string());
    }

    // Strip the leading '/' and split into words for Clap
    let input = &trimmed[1..];
    let args: Vec<&str> = input.split_whitespace().collect();

    // Build the argument list for Clap: ["slash", "code", "fix", "this", "bug"]
    let mut clap_args = vec!["slash"];
    clap_args.extend(args.iter().map(|s| *s));

    // Try to parse with Clap
    match SlashCommand::try_parse_from(clap_args) {
        Ok(slash_cmd) => match slash_cmd.command {
            Command::Code { prompt } => {
                let joined_prompt = prompt.join(" ");
                ParsedCommand::Code {
                    prompt: joined_prompt,
                }
            }
            Command::Exit => ParsedCommand::Exit,
        },
        Err(e) => {
            // For parse errors, we'll handle them in the router
            // by returning a helpful message to the user
            ParsedCommand::None(format!("/{}\n\n{}", input, e))
        }
    }
}

/// Get a help message for available commands.
pub fn get_help_text() -> String {
    let mut cmd = SlashCommand::command();
    // Generate short help without the binary name prefix
    format!(
        "Available slash commands:\n\n{}",
        cmd.render_help().to_string()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_code_with_prompt() {
        let result = parse_slash_command("/code fix this bug");
        eprintln!("Result: {:?}", result);
        match result {
            ParsedCommand::Code { prompt } => assert_eq!(prompt, "fix this bug"),
            other => panic!("Expected Code command, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_code_empty_prompt() {
        let result = parse_slash_command("/code");
        match result {
            ParsedCommand::Code { prompt } => assert_eq!(prompt, ""),
            _ => panic!("Expected Code command"),
        }
    }

    #[test]
    fn test_parse_exit() {
        let result = parse_slash_command("/exit");
        match result {
            ParsedCommand::Exit => (),
            _ => panic!("Expected Exit command"),
        }
    }

    #[test]
    fn test_parse_regular_message() {
        let result = parse_slash_command("Hello, how are you?");
        match result {
            ParsedCommand::None(msg) => assert_eq!(msg, "Hello, how are you?"),
            _ => panic!("Expected None"),
        }
    }

    #[test]
    fn test_parse_invalid_command() {
        let result = parse_slash_command("/unknown");
        // Should return a message with the error
        match result {
            ParsedCommand::None(msg) => {
                assert!(msg.contains("unexpected") || msg.contains("/unknown"))
            }
            _ => panic!("Expected None with error message"),
        }
    }

    #[test]
    fn test_whitespace_handling() {
        let result = parse_slash_command("  /code   hello world  ");
        match result {
            ParsedCommand::Code { prompt } => assert_eq!(prompt, "hello world"),
            _ => panic!("Expected Code command"),
        }
    }
}
