//! The `python` command for bashkit sandboxes.
//!
//! Bridges the shell to the pybox WASM sandbox in [`super::sandbox`], so shell
//! scripts can run Python with the same isolation guarantees as the `python`
//! tool: no network, environment, subprocesses, or host filesystem beyond the
//! session workspace. The workspace is preopened at `/`, matching the tool.

use std::path::PathBuf;

use hq_bash::builtins::resolve_path;
use hq_bash::{Builtin, BuiltinContext, ExecResult, async_trait};

use super::sandbox::shared_sandbox;

/// Runs Python as a shell command. Registered as `python` and `python3`.
pub struct PythonBuiltin {
    /// Host path of the session workspace, preopened at `/` in the sandbox.
    workspace: PathBuf,
}

impl PythonBuiltin {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }
}

#[async_trait]
impl Builtin for PythonBuiltin {
    async fn execute(&self, ctx: BuiltinContext<'_>) -> hq_bash::Result<ExecResult> {
        let program = match Program::parse(&ctx).await {
            Ok(program) => program,
            Err(result) => return Ok(result),
        };

        // The sandbox is CPU-bound and can run up to its timeout, so keep it
        // off the async runtime's worker threads.
        let workspace = self.workspace.clone();
        let result = tokio::task::spawn_blocking(move || {
            let sandbox = shared_sandbox()?;
            sandbox.exec(&program.code, &workspace, &program.stdin)
        })
        .await;

        let exec = match result {
            Ok(Ok(exec)) => exec,
            Ok(Err(e)) => return Ok(guest_error(e)),
            Err(e) => {
                return Ok(ExecResult::err(
                    format!("python: sandbox task failed: {e}\n"),
                    1,
                ));
            }
        };

        Ok(ExecResult {
            stdout: exec.stdout,
            stderr: exec.stderr,
            exit_code: 0,
            ..Default::default()
        })
    }
}

/// The program to execute and the bytes the guest should see on stdin.
struct Program {
    code: String,
    stdin: Vec<u8>,
}

impl Program {
    /// Parse the `python` command line: `-c CODE`, `FILE`, `-`, or no argument
    /// (read the program from stdin).
    ///
    /// Errors are already-formed command results so the caller can return them
    /// without rebuilding the message.
    #[allow(clippy::result_large_err)]
    async fn parse(ctx: &BuiltinContext<'_>) -> Result<Self, ExecResult> {
        // Stdin is forwarded to the guest for every form except `-`/no-args,
        // where it *is* the program.
        let stdin = ctx.stdin.unwrap_or_default().as_bytes().to_vec();

        let Some(first) = ctx.args.first() else {
            return Ok(Self {
                code: ctx.stdin.unwrap_or_default().to_string(),
                stdin: Vec::new(),
            });
        };

        match first.as_str() {
            "-c" => {
                let Some(code) = ctx.args.get(1) else {
                    return Err(usage_error("argument expected for the -c option"));
                };
                if ctx.args.len() > 2 {
                    return Err(usage_error("command-line arguments are not supported"));
                }
                Ok(Self {
                    code: code.clone(),
                    stdin,
                })
            }
            "-" => Ok(Self {
                code: ctx.stdin.unwrap_or_default().to_string(),
                stdin: Vec::new(),
            }),
            "-h" | "--help" => Err(ExecResult::ok(HELP)),
            "-V" | "--version" => Err(ExecResult::ok("Python 3 (pybox WASM)\n")),
            arg if arg.starts_with('-') => Err(usage_error(&format!("unsupported option '{arg}'"))),
            file => {
                if ctx.args.len() > 1 {
                    return Err(usage_error("command-line arguments are not supported"));
                }
                let path = resolve_path(ctx.cwd.as_path(), file);
                let bytes = ctx.fs.read_file(&path).await.map_err(|e| {
                    ExecResult::err(format!("python: can't open file '{file}': {e}\n"), 2)
                })?;
                Ok(Self {
                    code: String::from_utf8_lossy(&bytes).into_owned(),
                    stdin,
                })
            }
        }
    }
}

const HELP: &str = "\
Usage: python [-c cmd | - | file]
Run Python in an isolated WebAssembly sandbox.

Options:
  -c cmd       program passed in as a string
  -            read program from stdin
  -h, --help   print this help
  -V, --version
               print the Python version

Files are read and written relative to `/`, the session workspace shared with
the bash tool. There is no network or environment access and only a subset of
the standard library is available.
";

fn usage_error(message: &str) -> ExecResult {
    ExecResult::err(format!("python: {message}\n"), 2)
}

/// Map a sandbox error to the command result. Guest exceptions arrive as a
/// formatted traceback and should be printed verbatim; host-level failures get
/// the `python:` prefix.
fn guest_error(err: anyhow::Error) -> ExecResult {
    let message = err.to_string();
    if message.contains("timed out") {
        ExecResult::err("python: execution timed out\n", 124)
    } else {
        ExecResult::err(format!("{message}\n"), 1)
    }
}

#[cfg(test)]
mod tests {
    use crate::ai::tools::BashOutput;
    use crate::ai::tools::bash::run_in_sandbox;
    use uuid::Uuid;

    async fn run(command: &str) -> BashOutput {
        let temp_dir = std::env::temp_dir();
        let storage = temp_dir.to_string_lossy().into_owned();
        let workspace = temp_dir.join("workspace").join(Uuid::new_v4().to_string());
        run_in_sandbox(&storage, &workspace, command).await.unwrap()
    }

    #[tokio::test]
    async fn runs_inline_program() {
        let output = run(r#"python -c 'print(21 * 2)'"#).await;
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout.trim(), "42");
    }

    #[tokio::test]
    async fn runs_script_file() {
        let output = run(r#"echo 'print(6 * 7)' > answer.py && python answer.py"#).await;
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout.trim(), "42");
    }

    #[tokio::test]
    async fn reads_program_from_stdin() {
        let output = run(r#"echo 'print(21 * 2)' | python"#).await;
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout.trim(), "42");
    }

    #[tokio::test]
    async fn forwards_stdin_to_guest() {
        let output =
            run(r#"echo hello | python -c 'import sys; print(sys.stdin.read().strip())'"#).await;
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout.trim(), "hello");
    }

    #[tokio::test]
    async fn exception_sets_exit_code_and_traceback() {
        let output = run(r#"python -c '1 / 0'"#).await;
        assert_eq!(output.exit_code, 1);
        assert!(output.stderr.contains("ZeroDivisionError"));
    }

    #[tokio::test]
    async fn missing_file_is_an_error() {
        let output = run("python nope.py").await;
        assert_eq!(output.exit_code, 2);
        assert!(output.stderr.contains("can't open file"));
    }

    #[tokio::test]
    async fn shares_files_with_the_shell() {
        let output =
            run(r#"python -c "open('/shared.txt', 'w').write('hi')" && cat /shared.txt"#).await;
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout.trim(), "hi");
    }
}
