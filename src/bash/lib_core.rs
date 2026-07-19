//! Core Bash shell — struct, builder, exec, mount.
//!
//! Trimmed from bashkit's `lib.rs` (261 KB) down to the surface `hq` uses:
//! `Bash::new()`, `Bash::builder().builtin(name, boxed).build()`,
//! `bash.exec(script).await`, and `bash.mount(root, fs)`.
//!
//! All network/git/ssh/sqlite/python/logging plumbing has been removed. The
//! parser/interpreter/fs layers are delegated to their own modules; this file
//! is just the orchestration layer.

use crate::bash::builtins::{ExecutionDeadline, ExecutionExtensions, Builtin};
use crate::bash::error::{Error, Result};
use crate::bash::fs::{FileSystem, InMemoryFs, MountableFs};
use crate::bash::interpreter::{ExecResult, Interpreter};
use crate::bash::limits::{ExecutionLimits, LimitExceeded};
use crate::bash::parser::Parser;
use std::path::PathBuf;
use std::sync::Arc;

/// Default maximum input script size (1 MiB).
const DEFAULT_MAX_INPUT_BYTES: usize = 1024 * 1024;

/// Default parser timeout (5 seconds).
const DEFAULT_PARSER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Default maximum AST nesting depth.
const DEFAULT_MAX_AST_DEPTH: usize = 100;

/// Default maximum parser operations (fuel).
const DEFAULT_MAX_PARSER_OPERATIONS: usize = 1_000_000;

/// Threshold below which we parse inline (avoid threadpool hop).
const SPAWN_BLOCKING_THRESHOLD: usize = 16 * 1024;

/// A virtual bash shell with a POSIX filesystem.
///
/// Create with [`Bash::new`] (defaults) or [`Bash::builder`] (custom builtins,
/// env, cwd). Execute scripts with [`Bash::exec`]. Mount host directories
/// with [`Bash::mount`].
pub struct Bash {
    fs: Arc<dyn FileSystem>,
    mountable: Arc<MountableFs>,
    interpreter: Interpreter,
    parser_timeout: std::time::Duration,
    max_input_bytes: usize,
    max_ast_depth: usize,
    max_parser_operations: usize,
}

impl Default for Bash {
    fn default() -> Self {
        Self::new()
    }
}

impl Bash {
    /// Create a new Bash instance with default settings (in-memory filesystem,
    /// no custom builtins, default limits).
    pub fn new() -> Self {
        let base_fs: Arc<dyn FileSystem> = Arc::new(InMemoryFs::new());
        let mountable = Arc::new(MountableFs::new(base_fs));
        let fs: Arc<dyn FileSystem> = Arc::clone(&mountable) as Arc<dyn FileSystem>;
        let interpreter = Interpreter::new(Arc::clone(&fs));
        Self {
            fs,
            mountable,
            interpreter,
            parser_timeout: DEFAULT_PARSER_TIMEOUT,
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
            max_ast_depth: DEFAULT_MAX_AST_DEPTH,
            max_parser_operations: DEFAULT_MAX_PARSER_OPERATIONS,
        }
    }

    /// Create a new [`BashBuilder`] for customized configuration.
    pub fn builder() -> BashBuilder {
        BashBuilder::default()
    }

    /// Execute a bash script and return the result.
    ///
    /// Validates input size, parses with timeout/AST-depth/fuel limits, then
    /// executes the AST. See [`ExecResult`] for fields.
    pub async fn exec(&mut self, script: &str) -> Result<ExecResult> {
        let options = ExecOptions::new();
        self.exec_with_options(script, options).await
    }

    /// Execute a bash script with per-execution builtin extensions.
    pub async fn exec_with_extensions(
        &mut self,
        script: &str,
        extensions: ExecutionExtensions,
    ) -> Result<ExecResult> {
        self.exec_with_options(script, ExecOptions::new().extensions(extensions))
            .await
    }

    /// Canonical entry point: parse + execute with the given options.
    pub async fn exec_with_options(
        &mut self,
        script: &str,
        options: ExecOptions,
    ) -> Result<ExecResult> {
        let ExecOptions { mut extensions, output_callback: _ } = options;
        let active_limits = self.interpreter.limits().clone();
        let _ = extensions.insert(active_limits.clone());
        let _ = extensions.insert(ExecutionDeadline::new(active_limits.timeout));
        let _extensions_guard = self.interpreter.scoped_execution_extensions(extensions);
        self.exec_impl(script).await
    }

    async fn exec_impl(&mut self, script: &str) -> Result<ExecResult> {
        self.interpreter.reset_transient_state();
        self.interpreter.begin_exec_invocation()?;

        let input_len = script.len();
        if input_len > self.max_input_bytes {
            return Err(Error::ResourceLimit(LimitExceeded::InputTooLarge(
                input_len,
                self.max_input_bytes,
            )));
        }

        let script = if !self.interpreter.hooks().before_exec.is_empty() {
            let input = crate::bash::hooks::ExecInput { script: script.to_string() };
            match self.interpreter.hooks().fire_before_exec(input) {
                Some(modified) => std::borrow::Cow::Owned(modified.script),
                None => return Ok(ExecResult::err("cancelled by before_exec hook", 1)),
            }
        } else {
            std::borrow::Cow::Borrowed(script)
        };
        let script = script.as_ref();

        let input_len = script.len();
        if input_len > self.max_input_bytes {
            return Err(Error::ResourceLimit(LimitExceeded::InputTooLarge(
                input_len,
                self.max_input_bytes,
            )));
        }

        let parser_timeout = self.parser_timeout;
        let max_ast_depth = self.max_ast_depth;
        let max_parser_operations = self.max_parser_operations;

        // Parse: inline for small scripts, spawn_blocking + timeout for large.
        #[cfg(not(target_family = "wasm"))]
        let ast = if input_len <= SPAWN_BLOCKING_THRESHOLD {
            let parser = Parser::with_limits(script, max_ast_depth, max_parser_operations);
            parser.parse()?
        } else {
            let script_owned = script.to_owned();
            let parse_result = tokio::time::timeout(parser_timeout, async {
                tokio::task::spawn_blocking(move || {
                    let parser =
                        Parser::with_limits(&script_owned, max_ast_depth, max_parser_operations);
                    parser.parse()
                })
                .await
            })
            .await;

            match parse_result {
                Ok(Ok(result)) => result?,
                Ok(Err(join_error)) => {
                    return Err(Error::parse(format!("parser task failed: {}", join_error)));
                }
                Err(_elapsed) => {
                    return Err(Error::ResourceLimit(LimitExceeded::ParserTimeout(parser_timeout)));
                }
            }
        };

        #[cfg(target_family = "wasm")]
        let ast = {
            let parser =
                Parser::with_limits_and_timeout(script, max_ast_depth, max_parser_operations, Some(parser_timeout));
            parser.parse()?
        };

        let result = self.interpreter.execute(&ast).await?;
        self.interpreter.cleanup_proc_sub_files().await;
        Ok(result)
    }

    /// Mount a filesystem at a virtual path.
    ///
    /// After this, paths under `vfs_path` in the sandbox resolve to `fs`.
    /// Used by `hq` to mount a [`RealFs`] workspace directory at `/`.
    pub fn mount(&mut self, vfs_path: impl AsRef<std::path::Path>, fs: Arc<dyn FileSystem>) -> Result<()> {
        self.mountable.mount(vfs_path, fs)
    }

    /// Unmount a previously mounted filesystem.
    pub fn unmount(&mut self, vfs_path: impl AsRef<std::path::Path>) -> Result<()> {
        self.mountable.unmount(vfs_path)
    }

    /// Read-only access to the virtual filesystem.
    pub fn fs(&self) -> Arc<dyn FileSystem> {
        Arc::clone(&self.fs)
    }
}

/// Builder for [`Bash`] with custom configuration.
///
/// `hq` uses `.builtin(name, Box<dyn Builtin>)` to register the `hq` builtin,
/// then `.build()`. Most other builder methods (env, cwd, limits) are available
/// but unused by `hq`'s current call sites.
#[derive(Default)]
pub struct BashBuilder {
    custom_builtins: std::collections::HashMap<String, Box<dyn Builtin>>,
    env: std::collections::HashMap<String, String>,
    cwd: Option<PathBuf>,
    limits: ExecutionLimits,
}

impl BashBuilder {
    /// Register a custom builtin. Registered by name; the builtin's `execute`
    /// is called when the shell dispatches a command with that name.
    pub fn builtin(mut self, name: impl Into<String>, builtin: Box<dyn Builtin>) -> Self {
        self.custom_builtins.insert(name.into(), builtin);
        self
    }

    /// Set an environment variable (available to scripts via `$VAR`).
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Set the current working directory for script execution.
    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Set execution limits (parser timeout, fuel, input size).
    pub fn limits(mut self, limits: ExecutionLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Build the [`Bash`] instance.
    pub fn build(self) -> Bash {
        let base_fs: Arc<dyn FileSystem> = Arc::new(InMemoryFs::new());
        let mountable = Arc::new(MountableFs::new(base_fs));
        let fs: Arc<dyn FileSystem> = Arc::clone(&mountable) as Arc<dyn FileSystem>;

        let mut interpreter = Interpreter::with_config(
            Arc::clone(&fs),
            None,
            None,
            None,
            None,
            self.custom_builtins,
            None,
            crate::bash::interpreter::ShellProfile::Full,
        );

        for (key, value) in &self.env {
            interpreter.set_env(key, value);
            interpreter.set_var(key, value);
        }

        if let Some(cwd) = self.cwd {
            interpreter.set_cwd(cwd);
        }

        let limits = self.limits;
        interpreter.set_limits(limits.clone());

        Bash {
            fs,
            mountable,
            interpreter,
            parser_timeout: limits.parser_timeout,
            max_input_bytes: limits.max_input_bytes,
            max_ast_depth: limits.max_ast_depth,
            max_parser_operations: limits.max_parser_operations,
        }
    }
}

/// Per-execution options for [`Bash::exec_with_options`].
///
/// `output_callback` streams stdout/stderr chunks as they're produced.
/// `extensions` carries per-call builtin state (e.g. active limits).
#[derive(Default)]
pub struct ExecOptions {
    extensions: ExecutionExtensions,
    output_callback: Option<OutputCallback>,
}

impl ExecOptions {
    /// Create default options (no streaming, no extra extensions).
    pub fn new() -> Self {
        Self::default()
    }

    /// Add per-execution builtin extensions.
    pub fn extensions(mut self, extensions: ExecutionExtensions) -> Self {
        self.extensions = extensions;
        self
    }

    /// Set a streaming output callback (invoked for each stdout/stderr chunk).
    pub fn output_callback(mut self, callback: OutputCallback) -> Self {
        self.output_callback = Some(callback);
        self
    }
}

/// Callback for streaming output chunks during script execution.
pub type OutputCallback = std::sync::Arc<dyn Fn(&str, bool) + Send + Sync>;