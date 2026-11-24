//! Truncation of oversized tool output before it is added to the transcript.
//!
//! Tools return their results as plain strings, and a single result can be far
//! larger than the model's context window — a whole web page, a large file
//! dump, a subagent transcript. The inner chat loop truncates every tool result
//! here rather than each tool enforcing its own limit, so the policy stays in
//! one place. A truncated result keeps a prefix of the content and points at a
//! file in the session workspace that holds the full output.

use std::path::Path;

use anyhow::Result;
use tokio::fs;

use crate::ai::tools::bash::SANDBOX_ROOT;

/// Maximum estimated token count before we truncate the output.
/// Using a rough estimate of ~4 characters per token.
const MAX_OUTPUT_TOKENS: usize = 10_000;
const CHARS_PER_TOKEN: usize = 4;
const MAX_OUTPUT_CHARS: usize = MAX_OUTPUT_TOKENS * CHARS_PER_TOKEN;

/// How many characters to keep in the truncated response returned to the AI.
const TRUNCATED_PREFIX_CHARS: usize = 7_500 * CHARS_PER_TOKEN;

/// Directory under the session workspace holding full tool output. Kept out of
/// the workspace root, which holds stable artifacts like MEMORY.md and skill
/// directories.
const RESULTS_DIR: &str = "tool-results";

/// Truncate `content` if it exceeds the output budget.
///
/// When `workspace_path` is set, the full content is written to a file under
/// the session workspace and the returned text explains how to read it with the
/// bash tool. Without a workspace the content is still truncated, but the full
/// output is discarded.
pub async fn truncate_tool_output(
    workspace_path: Option<&Path>,
    tool_name: &str,
    tool_call_id: &str,
    content: String,
) -> Result<String> {
    let total_chars = content.chars().count();
    if total_chars <= MAX_OUTPUT_CHARS {
        return Ok(content);
    }

    let (prefix, next_line) = prefix_at_line_boundary(&content, TRUNCATED_PREFIX_CHARS);

    let Some(workspace_path) = workspace_path else {
        return Ok(format!(
            "[{tool_name} output truncated — showing the first {} of {} characters. \
             Full output was discarded because this session has no workspace.]\n\n{}",
            prefix.chars().count(),
            total_chars,
            prefix,
        ));
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let filename = result_filename(tool_call_id, timestamp);

    let results_dir = workspace_path.join(RESULTS_DIR);
    fs::create_dir_all(&results_dir).await?;
    fs::write(results_dir.join(&filename), &content).await?;

    // The bash tool mounts the session workspace at the sandbox root, so report
    // the path the agent can actually open.
    let sandbox_path = format!("{}{}/{}", SANDBOX_ROOT, RESULTS_DIR, filename);

    let total_lines = content.lines().count();
    let shown_lines = prefix.lines().count();
    let line_count = if total_lines == 1 {
        String::from("1 line")
    } else {
        format!("{} lines", total_lines)
    };

    let (banner, continuation) = if next_line == 0 {
        let shown_chars = prefix.chars().count();
        (
            format!(
                "[{tool_name} output truncated — showing the first {} of {} characters. \
                 Full output: `{}`.]",
                shown_chars, total_chars, sandbox_path,
            ),
            format!(
                "bash {{\"command\": \"cut -c {}-{} {}\"}}",
                shown_chars + 1,
                shown_chars + TRUNCATED_PREFIX_CHARS,
                sandbox_path,
            ),
        )
    } else {
        (
            format!(
                "[{tool_name} output truncated — showing lines 1-{} of {}. \
                 Full output: `{}`.]",
                shown_lines, total_lines, sandbox_path,
            ),
            format!(
                "bash {{\"command\": \"sed -n '{},{}p' {}\"}}",
                next_line,
                next_line + shown_lines,
                sandbox_path,
            ),
        )
    };

    Ok(format!(
        "{}\n\n{}\n\n---\n**Truncated:** full output is {} characters across {}, \
         saved to `{}`.\n\
         Continue with: {}\n\
         Search it with: bash {{\"command\": \"grep -n -i 'your search term' {}\"}}",
        banner, prefix, total_chars, line_count, sandbox_path, continuation, sandbox_path,
    ))
}

/// Build the file name for a saved result, using the provider's tool call ID
/// when it is usable and a timestamp otherwise.
fn result_filename(tool_call_id: &str, timestamp: u64) -> String {
    let id: String = tool_call_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        .take(64)
        .collect();

    if id.is_empty() {
        format!("call_{}.txt", timestamp)
    } else {
        format!("{}.txt", id)
    }
}

/// Cut `content` to at most `max_chars` characters, preferring a line boundary
/// so the agent can resume from a known line number.
///
/// Returns the prefix and the 1-indexed line the remainder starts on. A line
/// number of `0` means no line boundary fits within the limit (a single line
/// longer than `max_chars`), so the remainder has to be addressed by character
/// offset instead.
fn prefix_at_line_boundary(content: &str, max_chars: usize) -> (String, usize) {
    let mut prefix = String::new();
    let mut consumed = 0;

    for line in content.split_inclusive('\n') {
        let line_chars = line.chars().count();
        if consumed + line_chars > max_chars {
            break;
        }
        prefix.push_str(line);
        consumed += line_chars;
    }

    if prefix.is_empty() {
        return (content.chars().take(max_chars).collect(), 0);
    }

    let next_line = prefix.lines().count() + 1;
    (prefix, next_line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    async fn temp_workspace() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("hq-truncate-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    #[tokio::test]
    async fn small_output_is_returned_unchanged() {
        let out = truncate_tool_output(None, "bash", "call_1", String::from("done"))
            .await
            .unwrap();
        assert_eq!(out, "done");
    }

    #[tokio::test]
    async fn large_output_is_truncated_and_saved() {
        let workspace = temp_workspace().await;
        let content: String = (0..5_000)
            .map(|i| format!("line {} of filler text for the truncation test\n", i))
            .collect();

        let out = truncate_tool_output(Some(&workspace), "bash", "call_test123", content.clone())
            .await
            .unwrap();

        assert!(out.contains("bash output truncated"));
        assert!(out.contains("sed -n '"));
        // The path must be the one the bash sandbox exposes, not a host path.
        assert!(out.contains("/tool-results/call_test123.txt"));
        assert!(!out.contains(workspace.to_str().unwrap()));
        // The end of the output is not part of the truncated prefix.
        assert!(!out.contains("line 4999 of filler"));

        // The full content is on disk.
        let saved = fs::read_to_string(workspace.join("tool-results/call_test123.txt"))
            .await
            .unwrap();
        assert_eq!(saved, content);
    }

    #[tokio::test]
    async fn large_output_without_workspace_is_truncated_without_a_file() {
        let content = "x".repeat(MAX_OUTPUT_CHARS + 100);
        let out = truncate_tool_output(None, "subagent", "call_1", content)
            .await
            .unwrap();

        assert!(out.contains("subagent output truncated"));
        assert!(out.contains("no workspace"));
        assert!(!out.contains("tool-results"));
    }

    #[test]
    fn result_filename_uses_the_tool_call_id() {
        assert_eq!(result_filename("call_abc123", 1), "call_abc123.txt");
    }

    #[test]
    fn result_filename_sanitizes_unusable_ids() {
        assert_eq!(result_filename("../../etc/passwd", 7), "etcpasswd.txt");
        assert_eq!(result_filename("", 7), "call_7.txt");
    }

    #[test]
    fn prefix_at_line_boundary_stops_at_a_line_boundary() {
        let (prefix, next_line) = prefix_at_line_boundary("aaaa\nbbbb\ncccc\n", 11);

        assert_eq!(prefix, "aaaa\nbbbb\n");
        assert_eq!(next_line, 3);
    }

    #[test]
    fn prefix_at_line_boundary_falls_back_to_characters_for_one_long_line() {
        let (prefix, next_line) = prefix_at_line_boundary("abcdefghij", 4);

        assert_eq!(prefix, "abcd");
        assert_eq!(next_line, 0);
    }
}
