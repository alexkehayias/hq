//! Slash commands for chat sessions using Clap for parsing.
use clap::{CommandFactory, Parser, Subcommand};
use std::str::FromStr;

/// Result of parsing a slash command
#[derive(Debug, PartialEq)]
pub enum SlashCommand {
    /// Execute the /code command to enter agent mode
    Code { prompt: String },
    /// Exit current mode and return to chat
    Exit,
    /// Show available commands (help)
    Help,
    /// List all skills or show a specific skill's content
    Skill { name: Option<String> },
    /// Parse error for a specific slash command
    Error(String),
    /// Not a slash command - regular chat message
    None(String),
}

impl FromStr for SlashCommand {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();

        // Only parse if message starts with /
        if !trimmed.starts_with('/') {
            return Ok(SlashCommand::None(trimmed.to_string()));
        }

        // Strip the leading '/' and split into words for Clap
        let input = &trimmed[1..];
        let args: Vec<&str> = input.split_whitespace().collect();

        // Build the argument list for Clap: ["slash", "code", "fix", "this", "bug"]
        let mut clap_args = vec!["slash"];
        clap_args.extend(args.iter().copied());

        // Try to parse with Clap
        match SlashCommandParser::try_parse_from(clap_args) {
            Ok(slash_cmd) => match slash_cmd.command {
                Command::Code { prompt } => {
                    let joined_prompt = prompt.join(" ");
                    Ok(SlashCommand::Code {
                        prompt: joined_prompt,
                    })
                }
                Command::Exit => Ok(SlashCommand::Exit),
                Command::Skills { name } => {
                    let joined_name = if name.is_empty() {
                        None
                    } else {
                        Some(name.join(" "))
                    };
                    Ok(SlashCommand::Skill { name: joined_name })
                }
            },
            Err(e) => {
                // For parse errors, we'll handle them in the router
                // by returning a helpful message to the user
                Ok(SlashCommand::Error(format!("```\n{}\n```", e)))
            }
        }
    }
}

/// Slash commands parser for chat sessions.
///
/// This uses Clap to parse user messages that start with `/`, providing
/// nice error messages and help text when commands are invalid.
#[derive(Parser)]
#[command(name = "slash")]
#[command(override_usage = "/{command} [OPTIONS]")]
#[command(about = "Slash commands enable specific commands to be run in the chat session.", long_about = None)]
pub struct SlashCommandParser {
    #[command(subcommand)]
    pub command: Command,
}

/// Available slash commands
#[derive(Subcommand)]
pub enum Command {
    /// Enter Claude Code agent mode with a prompt
    #[command(override_usage = "/code <PROMPT>")]
    #[command(after_help = "Example: /code fix this bug")]
    Code {
        /// Prompt to send to the agent (optional)
        #[arg(required = true, num_args = 1..)]
        prompt: Vec<String>,
    },
    /// Exit current mode and return to chat
    #[command(override_usage = "/exit")]
    Exit,
    /// List all available skills or show a specific skill's content
    #[command(override_usage = "/skills <SKILL_NAME>")]
    #[command(after_help = "List all skills: /skills\nShow a specific skill: /skills test-repo")]
    Skills {
        /// Optional skill name to show (if not provided, lists all skills)
        #[arg(required = false, num_args = 1..)]
        name: Vec<String>,
    },
}

/// Get a help message for available commands.
pub fn get_help_text() -> String {
    let mut cmd = SlashCommandParser::command();
    // Generate short help without the binary name prefix
    format!("Available slash commands:\n\n{}", cmd.render_help())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_code_with_prompt() {
        let result = SlashCommand::from_str("/code fix this bug");
        assert!(result.is_ok());
        let _ = result.map(|slash_cmd| match slash_cmd {
            SlashCommand::Code { prompt } => assert_eq!(prompt, "fix this bug"),
            other => panic!("Expected Code command, got {:?}", other),
        });
    }

    #[test]
    fn test_parse_exit() {
        let result = SlashCommand::from_str("/exit");
        assert!(result.is_ok());
        let _ = result.map(|slash_cmd| assert_eq!(SlashCommand::Exit, slash_cmd));
    }

    #[test]
    fn test_parse_regular_message() {
        let result = SlashCommand::from_str("Hello, how are you?");
        assert!(result.is_ok());
        let _ = result.map(|slash_cmd| match slash_cmd {
            SlashCommand::None(msg) => assert_eq!(msg, "Hello, how are you?"),
            _ => panic!("Expected None"),
        });
    }

    #[test]
    fn test_parse_invalid_command() {
        let result = SlashCommand::from_str("/unknown");
        assert!(result.is_ok());
        let _ = result.map(|slash_cmd| match slash_cmd {
            SlashCommand::Error(msg) => assert!(msg.contains("error")),
            _ => panic!("Expected None"),
        });
    }

    #[test]
    fn test_whitespace_handling() {
        let result = SlashCommand::from_str("  /code   hello world  ");
        assert!(result.is_ok());
        let _ = result.map(|slash_cmd| match slash_cmd {
            SlashCommand::Code { prompt } => assert_eq!(prompt, "hello world"),
            _ => panic!("Expected Code command"),
        });
    }

    #[test]
    fn test_parse_skill_list_all() {
        let result = SlashCommand::from_str("/skills");
        assert!(result.is_ok());
        let _ = result.map(|slash_cmd| match slash_cmd {
            SlashCommand::Skill { name } => assert!(name.is_none()),
            other => panic!("Expected Skill command, got {:?}", other),
        });
    }

    #[test]
    fn test_parse_skill_with_name() {
        let result = SlashCommand::from_str("/skills test-repo");
        assert!(result.is_ok());
        let _ = result.map(|slash_cmd| match slash_cmd {
            SlashCommand::Skill { name } => assert_eq!(name, Some("test-repo".to_string())),
            other => panic!("Expected Skill command, got {:?}", other),
        });
    }

    #[test]
    fn test_parse_skill_with_name_and_args() {
        let result = SlashCommand::from_str("/skills test-repo with args");
        assert!(result.is_ok());
        let _ = result.map(|slash_cmd| match slash_cmd {
            SlashCommand::Skill { name } => assert_eq!(name, Some("test-repo with args".to_string())),
            other => panic!("Expected Skill command, got {:?}", other),
        });
    }
}
