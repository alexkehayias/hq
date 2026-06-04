use crate::cli;
use bashkit::{async_trait, Builtin, BuiltinContext, ExecResult, Result};
use std::io::{Read, Write};
use std::os::fd::FromRawFd;

/// A bashkit custom builtin that wraps the `hq` CLI.
///
/// Registered as the `hq` command inside bashkit sandboxes. Arguments are
/// forwarded directly to [`cli::run_with_args`], with stdout and stderr
/// captured via Unix pipes and returned as the builtin's output.
pub struct HqBuiltin;

struct CapturedOutput {
    stdout: String,
    stderr: String,
}

#[async_trait]
impl Builtin for HqBuiltin {
    async fn execute(&self, ctx: BuiltinContext<'_>) -> Result<ExecResult> {
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
        let stdout_handle = std::thread::spawn(move || {
            let mut buf = String::new();
            std::fs::File::from_raw_fd(stdout_fds[0])
                .read_to_string(&mut buf)
                .ok();
            buf
        });
        let stderr_handle = std::thread::spawn(move || {
            let mut buf = String::new();
            std::fs::File::from_raw_fd(stderr_fds[0])
                .read_to_string(&mut buf)
                .ok();
            buf
        });

        // Run the async function
        let result = f().await;

        // Restore original fds
        libc::dup2(saved_stdout, 1);
        libc::dup2(saved_stderr, 2);
        libc::close(saved_stdout);
        libc::close(saved_stderr);

        // Flush any remaining data into the pipes before joining readers
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();

        let captured_stdout = stdout_handle.join().unwrap_or_default();
        let captured_stderr = stderr_handle.join().unwrap_or_default();

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

        // --help prints usage info; clap returns it as an error via
        // try_parse_from, so exit_code should be non-zero and the help
        // text ends up in stderr (or stdout if clap wrote it there).
        let output = format!("{}{}", result.stdout, result.stderr);
        assert!(
            output.contains("hq"),
            "help output should contain the program name: {output}"
        );
        assert!(
            output.contains("Usage") || output.contains("Commands"),
            "help output should list usage or commands: {output}"
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
    async fn test_hq_invalid_subcommand() {
        let mut bash = bash_with_hq();
        let result = bash.exec("hq nonexistent-subcommand").await.unwrap();

        assert_ne!(result.exit_code, 0);
        let output = format!("{}{}", result.stdout, result.stderr);
        assert!(
            output.contains("error")
                || output.contains("unrecognized")
                || output.contains("not found")
                || output.contains("valid"),
            "output should indicate an invalid subcommand: {output}"
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
