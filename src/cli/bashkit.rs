use crate::cli;
use bashkit::{async_trait, Builtin, BuiltinContext, ExecResult, Result};
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::time::Duration;

/// A bashkit custom builtin that wraps the `hq` CLI.
///
/// Registered as the `hq` command inside bashkit sandboxes. Only subcommands
/// in [`ALLOWED_SUBCOMMANDS`] are permitted; all others return an error.
/// Help output is filtered to show only allowed subcommands.
/// Arguments are forwarded to [`cli::run_with_args`], with stdout and stderr
/// captured via Unix pipes and returned as the builtin's output.
pub struct HqBuiltin;

/// Subcommands that the `hq` builtin is allowed to run inside the sandbox.
const ALLOWED_SUBCOMMANDS: &[&str] = &["task", "eval", "projects"];

/// Help text shown for the `hq` builtin, listing only allowed subcommands.
const FILTERED_HELP: &str = "\
hq - personal AI assistant

Usage: hq <COMMAND>

Commands:
  task       Create, update, or delete tasks
  eval       Run an eval
  projects   List projects

Options:
  -h, --help       Print help
  -V, --version    Print version
";

struct CapturedOutput {
    stdout: String,
    stderr: String,
}

#[async_trait]
impl Builtin for HqBuiltin {
    async fn execute(&self, ctx: BuiltinContext<'_>) -> Result<ExecResult> {
        let wants_help = ctx.args.is_empty()
            || ctx.args.iter().any(|a| a == "--help" || a == "-h");

        if wants_help {
            return Ok(ExecResult::ok(FILTERED_HELP));
        }

        // Check the subcommand against the allowlist before running anything.
        if let Some(cmd) = ctx.args.iter().find(|a| !a.starts_with('-')) {
            if !ALLOWED_SUBCOMMANDS.contains(&cmd.as_str()) {
                return Ok(ExecResult::err(
                    format!(
                        "subcommand '{cmd}' is not allowed. Allowed subcommands: {}",
                        ALLOWED_SUBCOMMANDS.join(", "),
                    ),
                    1,
                ));
            }
        }

        // Build args: program name + user-supplied subcommand and flags
        let mut args = vec!["hq".to_string()];
        args.extend(ctx.args.iter().cloned());

        match capture_async(move || async { cli::run_with_args(args).await }).await {
            Ok(output) => {
                if output.stderr.is_empty() {
                    Ok(ExecResult::ok(output.stdout))
                } else {
                    Ok(ExecResult {
                        stdout: output.stdout,
                        stderr: output.stderr,
                        exit_code: 0,
                        ..Default::default()
                    })
                }
            }
            Err((output, err)) => {
                let msg = format!("{err}");
                let combined_stderr = if output.stderr.is_empty() {
                    msg
                } else {
                    format!("{stderr}\n{msg}", stderr = output.stderr)
                };
                Ok(ExecResult {
                    stdout: output.stdout,
                    stderr: combined_stderr,
                    exit_code: 1,
                    ..Default::default()
                })
            }
        }
    }
}

/// Run an async closure with stdout and stderr redirected to pipes, capturing
/// all output into the returned strings.
async fn capture_async<F, Fut>(
    f: F,
) -> std::result::Result<CapturedOutput, (CapturedOutput, anyhow::Error)>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    // Flush before redirecting
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();

    // Safety: We temporarily replace fd 1 and 2 with pipe write-ends so the
    // closure's stdout/stderr output is captured instead of reaching the
    // process's original output. The original fds are restored before the
    // function returns. This is safe because:
    //   - No other code in this process concurrently reads from or duplicates
    //     fd 1/2 during the capture window (the bashkit sandbox has its own
    //     fd setup and this builtin is called synchronously within it).
    //   - The pipe read-ends are immediately moved into background reader
    //     threads (via from_raw_fd, taking ownership) so there is no aliasing.
    //   - Saved fds from dup() are closed after restore, avoiding leaks.
    unsafe {
        let mut stdout_fds: [libc::c_int; 2] = [0, 0];
        let mut stderr_fds: [libc::c_int; 2] = [0, 0];

        if libc::pipe(stdout_fds.as_mut_ptr()) != 0 {
            return Err((
                CapturedOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                },
                anyhow::anyhow!("pipe failed"),
            ));
        }
        if libc::pipe(stderr_fds.as_mut_ptr()) != 0 {
            libc::close(stdout_fds[0]);
            libc::close(stdout_fds[1]);
            return Err((
                CapturedOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                },
                anyhow::anyhow!("pipe failed"),
            ));
        }

        let saved_stdout = libc::dup(1);
        let saved_stderr = libc::dup(2);

        libc::dup2(stdout_fds[1], 1);
        libc::dup2(stderr_fds[1], 2);
        libc::close(stdout_fds[1]);
        libc::close(stderr_fds[1]);

        // Read from pipes on background threads to avoid deadlock when
        // output exceeds the OS pipe buffer size (~64 KB on macOS/Linux).
        let (stdout_tx, stdout_rx) = std::sync::mpsc::channel();
        let (stderr_tx, stderr_rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let mut buf = String::new();
            std::fs::File::from_raw_fd(stdout_fds[0])
                .read_to_string(&mut buf)
                .ok();
            let _ = stdout_tx.send(buf);
        });
        std::thread::spawn(move || {
            let mut buf = String::new();
            std::fs::File::from_raw_fd(stderr_fds[0])
                .read_to_string(&mut buf)
                .ok();
            let _ = stderr_tx.send(buf);
        });

        // Run the async function
        let result = f().await;

        // Restore original fds
        libc::dup2(saved_stdout, 1);
        libc::dup2(saved_stderr, 2);
        libc::close(saved_stdout);
        libc::close(saved_stderr);

        // Flush any remaining data into the pipes before reading from readers
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();

        // Wait for pipe readers with a timeout to prevent hangs if a pipe
        // write end reference is not properly closed (e.g. inherited by a
        // background thread). Fall back to empty string on timeout.
        let capture_timeout = Duration::from_secs(5);
        let captured_stdout = stdout_rx
            .recv_timeout(capture_timeout)
            .unwrap_or_default();
        let captured_stderr = stderr_rx
            .recv_timeout(capture_timeout)
            .unwrap_or_default();

        let output = CapturedOutput {
            stdout: captured_stdout,
            stderr: captured_stderr,
        };

        match result {
            Ok(_) => Ok(output),
            Err(e) => Err((output, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bashkit::Bash;

    /// Create a bash instance with the hq builtin registered.
    fn bash_with_hq() -> Bash {
        Bash::builder()
            .builtin("hq", Box::new(HqBuiltin))
            .build()
    }

    #[tokio::test]
    async fn test_hq_help() {
        let mut bash = bash_with_hq();
        let result = bash.exec("hq --help").await.unwrap();

        let output = format!("{}{}", result.stdout, result.stderr);
        assert!(
            output.contains("hq"),
            "help output should contain the program name: {output}"
        );
        assert!(
            output.contains("Usage") || output.contains("Commands"),
            "help output should list usage or commands: {output}"
        );
        assert!(
            output.contains("task"),
            "help output should list allowed subcommand 'task': {output}"
        );
        assert!(
            output.contains("eval"),
            "help output should list allowed subcommand 'eval': {output}"
        );
        assert!(
            output.contains("projects"),
            "help output should list allowed subcommand 'projects': {output}"
        );
        assert!(
            !output.contains("serve"),
            "help output should not list disallowed subcommand 'serve': {output}"
        );
        assert!(
            !output.contains("query"),
            "help output should not list disallowed subcommand 'query': {output}"
        );
    }

    #[tokio::test]
    async fn test_hq_no_args_shows_help() {
        let mut bash = bash_with_hq();
        let result = bash.exec("hq").await.unwrap();

        // With arg_required_else_help = true, no subcommand should
        // display help text.
        let output = format!("{}{}", result.stdout, result.stderr);
        assert!(
            !output.is_empty(),
            "running hq with no args should produce output"
        );
        assert!(
            output.contains("Usage") || output.contains("Commands"),
            "no-args output should show usage or commands: {output}"
        );
    }

    #[tokio::test]
    async fn test_hq_disallowed_subcommand() {
        let mut bash = bash_with_hq();
        let result = bash.exec("hq serve").await.unwrap();

        assert_ne!(result.exit_code, 0);
        let output = format!("{}{}", result.stdout, result.stderr);
        assert!(
            output.contains("not allowed"),
            "output should indicate the subcommand is not allowed: {output}"
        );
        assert!(
            output.contains("task") && output.contains("eval") && output.contains("projects"),
            "output should list allowed subcommands: {output}"
        );
    }

    #[tokio::test]
    async fn test_hq_version() {
        let mut bash = bash_with_hq();
        let result = bash.exec("hq --version").await.unwrap();

        let output = format!("{}{}", result.stdout, result.stderr);
        assert!(
            output.contains("hq"),
            "version output should mention hq: {output}"
        );
        assert!(
            output.contains(env!("CARGO_PKG_VERSION")),
            "version output should contain the package version: {output}"
        );
    }

    #[tokio::test]
    async fn test_hq_echo_in_bash() {
        // Verify the hq builtin doesn't interfere with normal bash commands.
        let mut bash = bash_with_hq();
        let result = bash.exec("echo hello from bash").await.unwrap();

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "hello from bash");
    }
}
