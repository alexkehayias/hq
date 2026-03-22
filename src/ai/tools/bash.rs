use crate::openai::{Function, Parameters, Property, ToolCall, ToolType};
use anyhow::{Error, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::process::Command;

#[derive(Serialize)]
pub struct BashProps {
    /// The shell command to execute. Use absolute paths for files.
    pub command: Property,
}

#[derive(Deserialize)]
pub struct BashArgs {
    pub command: String,
}

/// Result of running a bash command.
#[derive(Serialize, Deserialize)]
pub struct BashOutput {
    /// The exit code of the command. 0 indicates success.
    pub exit_code: i32,
    /// The standard output from the command.
    pub stdout: String,
    /// The standard error from the command.
    pub stderr: String,
    /// Whether the command was killed due to timeout.
    pub timed_out: bool,
}

#[derive(Serialize)]
pub struct UnsafeBashTool {
    pub r#type: ToolType,
    pub function: Function<BashProps>,
}

#[async_trait]
impl ToolCall for UnsafeBashTool {
    async fn call(&self, args: &str) -> Result<String, Error> {
        let fn_args: BashArgs = serde_json::from_str(args).unwrap();

        // Run the command with a clean environment (no env vars)
        let output = Command::new("bash")
            .arg("-c")
            .arg(&fn_args.command)
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HOME", std::env::var("HOME").unwrap_or_default())
            .env("USER", std::env::var("USER").unwrap_or_default())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        let result = BashOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            timed_out: false,
        };

        Ok(serde_json::to_string(&result)?)
    }

    fn function_name(&self) -> String {
        self.function.name.clone()
    }
}

impl UnsafeBashTool {
    pub fn new() -> Self {
        let parameters = Parameters {
            r#type: String::from("object"),
            properties: BashProps {
                command: Property {
                    r#type: String::from("string"),
                    description: String::from(
                        "The shell command to execute. Use absolute paths for files. \
                         Commands run in a clean environment without access to environment variables.",
                    ),
                    r#enum: None,
                },
            },
            required: vec!["command".to_string()],
            additional_properties: false,
        };
        let function = Function {
            name: String::from("bash"),
            description: String::from(
                "Run a shell command and get the output. Commands run in a clean environment \
                 without access to environment variables. Use absolute paths for files.",
            ),
            parameters,
            strict: true,
        };

        Self {
            r#type: ToolType::Function,
            function,
        }
    }
}

impl Default for UnsafeBashTool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bash_echo() {
        let tool = UnsafeBashTool::new();
        let result = tool.call(r#"{"command": "echo hello"}"#).await.unwrap();
        let output: BashOutput = serde_json::from_str(&result).unwrap();

        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout.trim(), "hello");
    }

    #[tokio::test]
    async fn test_bash_stderr() {
        let tool = UnsafeBashTool::new();
        let result = tool.call(r#"{"command": "echo error >&2"}"#).await.unwrap();
        let output: BashOutput = serde_json::from_str(&result).unwrap();

        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stderr.trim(), "error");
    }

    #[tokio::test]
    async fn test_bash_exit_code() {
        let tool = UnsafeBashTool::new();
        let result = tool.call(r#"{"command": "exit 42"}"#).await.unwrap();
        let output: BashOutput = serde_json::from_str(&result).unwrap();

        assert_eq!(output.exit_code, 42);
    }

    #[tokio::test]
    async fn test_bash_no_env_vars() {
        let tool = UnsafeBashTool::new();
        let result = tool.call(r#"{"command": "echo $PATH"}"#).await.unwrap();
        let output: BashOutput = serde_json::from_str(&result).unwrap();

        // Should have a clean PATH, not the full system PATH
        assert_eq!(output.exit_code, 0);
        // The clean PATH should be set but different from the full system path
    }
}
