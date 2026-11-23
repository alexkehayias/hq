//! Sandboxed Python execution via the embedded pybox WASM component.
//!
//! Adapted from the `pybox` project (https://github.com/alexkehayias/pybox).
//! The component is built with `componentize-py` from a guest module and runs
//! with no ambient capabilities: no environment, network, subprocesses, or
//! ctypes. The only host access granted is a single preopened directory — the
//! session workspace — mounted at `/`, matching the bash tool. Memory and
//! captured output are bounded, and a wall-clock timeout is enforced via engine
//! epoch interruption.

use anyhow::{Context, Result, anyhow};
use once_cell::sync::OnceCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use wasmtime::component::{Accessor, Component, HasSelf, Linker};
use wasmtime::{Cache, Config, Engine, Store, StoreLimits, StoreLimitsBuilder, Strategy};

use crate::ai::chat::middleware::{ToolCallRule, default_tool_security_rules};
use crate::openai::BoxedToolCall;
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::{FsPerms, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView};

// Default timeout in seconds.
const DEFAULT_TIMEOUT_SECONDS: u64 = 40;

// The engine epoch is advanced by a single background thread. A store's
// deadline is expressed in these ticks, so granularity is TICK_MS.
const TICK_MS: u64 = 100;
const TICKS_PER_SECOND: u64 = 1000 / TICK_MS;

// Caps on untrusted code. Memory is limited per linear memory; output is
// captured into bounded pipes so a guest cannot exhaust host memory or disk.
const MAX_MEMORY_BYTES: usize = 512 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

// Caps on host tool access. A guest can loop over tool calls, so bound how many
// it may make per execution and how long a single call may take. Results are
// copied into the guest's linear memory, so reject oversized ones rather than
// letting a single call exhaust the memory limit.
const MAX_TOOL_CALLS: usize = 64;
const TOOL_CALL_TIMEOUT_SECONDS: u64 = 120;
const MAX_TOOL_HOST_TIME_SECONDS: u64 = 300;
const MAX_TOOL_RESULT_BYTES: usize = 4 * 1024 * 1024;

// The sandbox component is embedded so the binary is self-contained and never
// loads executable code from the (potentially untrusted) working directory.
// `build.rs` compiles it from `guest.py` into OUT_DIR; the artifact is not
// committed.
const SANDBOX_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sandbox.wasm"));

/// Host tools exposed to guest code.
///
/// One bridge is built per execution, so the call budget is scoped to a single
/// `exec`. It is held in an `Arc` because the async host function clones it out
/// of the store between await points.
pub struct ToolBridge {
    tools: Arc<HashMap<String, BoxedToolCall>>,
    definitions: String,
    max_calls: usize,
    calls: AtomicUsize,
    call_timeout: Duration,
    max_host_time: Duration,
    /// Total time spent inside host tools, in milliseconds. Atomics so the
    /// async host function can update it through a shared `&ToolBridge`.
    host_time_ms: AtomicU64,
    /// Prompt-injection rules, shared with the chat middleware.
    security_rules: Vec<ToolCallRule>,
    /// Tools already used in this session, including nested calls from earlier
    /// executions. Shared across `code` calls in the same turn.
    used_tools: Arc<Mutex<HashSet<String>>>,
}

impl ToolBridge {
    pub fn new(
        tools: Arc<HashMap<String, BoxedToolCall>>,
        definitions: String,
        used_tools: Arc<Mutex<HashSet<String>>>,
    ) -> Self {
        Self {
            tools,
            definitions,
            max_calls: MAX_TOOL_CALLS,
            calls: AtomicUsize::new(0),
            call_timeout: Duration::from_secs(TOOL_CALL_TIMEOUT_SECONDS),
            max_host_time: Duration::from_secs(MAX_TOOL_HOST_TIME_SECONDS),
            host_time_ms: AtomicU64::new(0),
            security_rules: default_tool_security_rules(),
            used_tools,
        }
    }

    fn host_time(&self) -> Duration {
        Duration::from_millis(self.host_time_ms.load(Ordering::SeqCst))
    }

    fn add_host_time(&self, elapsed: Duration) {
        self.host_time_ms
            .fetch_add(elapsed.as_millis() as u64, Ordering::SeqCst);
    }
}

struct SandboxState {
    wasi_ctx: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    timed_out: bool,
    tools: Option<Arc<ToolBridge>>,
    /// Wall-clock deadline for guest code only. Time spent awaiting a host tool
    /// call is added back so a slow tool does not consume the budget.
    guest_deadline: Instant,
}

impl wasmtime_wasi::WasiView for SandboxState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi_ctx,
            table: &mut self.table,
        }
    }
}

// NOTE: This generates a `Sandbox` type automatically and your IDE
// might not pick it up
wasmtime::component::bindgen!({
    path: "src/ai/tools/python/sandbox.wit",
    world: "sandbox",
});

impl local::sandbox::tools::Host for SandboxState {
    fn definitions(&mut self) -> String {
        self.tools
            .as_ref()
            .map(|bridge| bridge.definitions.clone())
            .unwrap_or_else(|| "[]".to_string())
    }
}

impl local::sandbox::tools::HostWithStore<SandboxState> for HasSelf<SandboxState> {
    async fn call(
        accessor: &Accessor<SandboxState, Self>,
        name: String,
        args: String,
    ) -> std::result::Result<String, String> {
        // `Accessor::with` is synchronous and cannot be held across an await,
        // so clone the shared bridge out first.
        let Some(bridge) = accessor.with(|mut access| access.get().tools.clone()) else {
            return Err("no tools are available in this sandbox".to_string());
        };

        if bridge.calls.fetch_add(1, Ordering::SeqCst) >= bridge.max_calls {
            return Err(format!(
                "tool call budget exceeded (max {})",
                bridge.max_calls
            ));
        }
        if bridge.host_time() >= bridge.max_host_time {
            return Err(format!(
                "tool time budget exceeded (max {}s)",
                bridge.max_host_time.as_secs()
            ));
        }

        let Some(tool) = bridge.tools.get(&name) else {
            return Err(format!("unknown tool '{name}'"));
        };

        // Nested calls bypass the chat's tool-call middleware, so enforce
        // the same prompt-injection rules here. The set is shared across
        // executions in a turn, and seeded from the transcript.
        {
            let mut used = bridge
                .used_tools
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for rule in &bridge.security_rules {
                if let Some(reason) = rule.rejection(&used, &name) {
                    return Err(reason);
                }
            }
            used.insert(name.clone());
        }

        let started = Instant::now();
        let result = match tokio::time::timeout(bridge.call_timeout, tool.call(&args)).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(error.to_string()),
            Err(_) => Err(format!("tool '{name}' timed out")),
        };
        let elapsed = started.elapsed();
        bridge.add_host_time(elapsed);

        // Host time must not consume the guest's execution budget: the
        // epoch deadline is wall-clock based, and the guest is suspended
        // while the host future runs.
        accessor.with(|mut access| {
            access.get().guest_deadline += elapsed;
        });

        match result {
            Ok(value) if value.len() > MAX_TOOL_RESULT_BYTES => Err(format!(
                "tool '{name}' returned {} bytes (max {MAX_TOOL_RESULT_BYTES})",
                value.len()
            )),
            other => other,
        }
    }
}

/// The captured result of running guest code.
#[derive(Debug, Clone, Default)]
pub struct ExecResult {
    /// JSON-serialized value of the last expression, or `null`.
    pub value: String,
    /// Bytes the guest wrote to stdout, decoded lossily.
    pub stdout: String,
    /// Bytes the guest wrote to stderr, decoded lossily.
    pub stderr: String,
}

/// A sandboxed Python execution environment using WebAssembly.
pub struct PySandbox {
    engine: Engine,
    component: Component,
    pub timeout_seconds: u64,
    stop_ticker: Arc<AtomicBool>,
    ticker: Option<JoinHandle<()>>,
}

/// The process-wide sandbox. Compiling the embedded component is expensive, so
/// it happens once and is shared across all tool calls. `exec` takes `&self`
/// and builds a fresh store per call, so concurrent calls are safe.
pub fn shared_sandbox() -> Result<&'static PySandbox> {
    static SANDBOX: OnceCell<PySandbox> = OnceCell::new();
    SANDBOX.get_or_try_init(|| {
        if cfg!(test) {
            // Winch compiles the ~20 MB component far faster, which keeps the
            // test suite quick. Production uses the Cranelift default.
            PySandbox::new_for_test(None)
        } else {
            PySandbox::new(None)
        }
    })
}

impl PySandbox {
    /// Create a new webassembly sandbox for executing untrusted python code.
    ///
    /// # Arguments
    /// * `timeout_secs` - Optional timeout in seconds. Defaults to `DEFAULT_TIMEOUT_SECONDS`.
    pub fn new(timeout_secs: Option<u64>) -> Result<Self> {
        let mut cfg = Config::new();
        // Enable timeouts
        cfg.epoch_interruption(true);
        // Enable the compilation cache, using the default cache configuration
        // settings.
        cfg.cache(Some(Cache::from_file(None)?));
        let engine = Engine::new(&cfg)
            .map_err(anyhow::Error::from)
            .context("Failed to create wasm engine")?;

        Self::from_engine(engine, timeout_secs)
    }

    /// Create a sandbox with fast compilation settings for tests.
    ///
    /// Uses Winch baseline compiler, parallel compilation, and caching
    /// to significantly speed up test execution.
    ///
    /// # Arguments
    /// * `timeout_secs` - Optional timeout in seconds. Defaults to `DEFAULT_TIMEOUT_SECONDS`.
    pub fn new_for_test(timeout_secs: Option<u64>) -> Result<Self> {
        let mut config = Config::new();

        // Enable the compilation cache, using the default cache configuration
        // settings.
        config.cache(Some(Cache::from_file(None)?));

        // Enable Winch, Wasmtime's baseline compiler.
        config.strategy(Strategy::Winch);

        // Enable parallel compilation.
        config.parallel_compilation(true);

        // Enable epoch interruption for timeout support
        config.epoch_interruption(true);

        let engine = Engine::new(&config)
            .map_err(anyhow::Error::from)
            .context("Failed to create wasm engine")?;

        Self::from_engine(engine, timeout_secs)
    }

    fn from_engine(engine: Engine, timeout_secs: Option<u64>) -> Result<Self> {
        let timeout_seconds = timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECONDS);

        let component = Component::new(&engine, SANDBOX_WASM)
            .map_err(anyhow::Error::from)
            .context("Failed to load embedded sandbox.wasm")?;

        // A single background thread advances the engine epoch for all stores.
        // Unlike a per-call timer, it cannot fire during a later call and cause
        // a spurious timeout.
        let stop_ticker = Arc::new(AtomicBool::new(false));
        let ticker = {
            let engine = engine.clone();
            let stop = stop_ticker.clone();
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(TICK_MS));
                    engine.increment_epoch();
                }
            })
        };

        Ok(Self {
            engine,
            component,
            timeout_seconds,
            stop_ticker,
            ticker: Some(ticker),
        })
    }

    /// Execute Python code in the sandbox without host tools.
    ///
    /// See [`PySandbox::exec_with_tools`] for the full contract.
    pub async fn exec(
        &self,
        code: &str,
        workspace: &Path,
        stdin: &[u8],
        argv: &[String],
    ) -> Result<ExecResult> {
        self.exec_with_tools(code, workspace, stdin, argv, None)
            .await
    }

    /// Execute Python code in the sandbox.
    ///
    /// Returns the last expression's value plus any captured stdout/stderr.
    /// `workspace` is exposed to guest code as the root directory (`/`) — the
    /// same directory the bash tool mounts — so files are shared between the two
    /// tools. `stdin` is the guest's standard input; pass an empty slice to
    /// make reads see EOF immediately. `argv` becomes `sys.argv` in the guest;
    /// include `argv[0]` (the script name, `-c`, or `-`) to match CPython. Guest
    /// code cannot reach anything outside the workspace, nor the host's
    /// stdout/stderr, environment, or network. `tools` is the optional bridge to
    /// host tools; `None` leaves the guest with no host access beyond the
    /// workspace. The caller must ensure `workspace` already exists on the host;
    /// the preopen fails otherwise.
    pub async fn exec_with_tools(
        &self,
        code: &str,
        workspace: &Path,
        stdin: &[u8],
        argv: &[String],
        tools: Option<Arc<ToolBridge>>,
    ) -> Result<ExecResult> {
        let stdout_pipe = MemoryOutputPipe::new(MAX_OUTPUT_BYTES);
        let stderr_pipe = MemoryOutputPipe::new(MAX_OUTPUT_BYTES);

        // Grant no ambient capabilities beyond the workspace: caller-provided
        // stdin and bounded in-memory stdout/stderr. The single preopened
        // directory is confined to `workspace` by WASI/cap-std, so `..` and
        // symlinks cannot escape it.
        let mut builder = WasiCtxBuilder::new();
        builder.stdin(MemoryInputPipe::new(stdin.to_vec()));
        builder.stdout(stdout_pipe.clone());
        builder.stderr(stderr_pipe.clone());
        builder.preopened_dir(workspace, "/", FsPerms::ReadWrite)?;

        let timeout = Duration::from_secs(self.timeout_seconds);
        let state = SandboxState {
            wasi_ctx: builder.build(),
            table: ResourceTable::new(),
            limits: StoreLimitsBuilder::new()
                .memory_size(MAX_MEMORY_BYTES)
                .build(),
            timed_out: false,
            tools,
            guest_deadline: Instant::now() + timeout,
        };

        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);

        // Deadline is relative to the current epoch, in ticker ticks.
        let deadline = self.timeout_seconds.saturating_mul(TICKS_PER_SECOND).max(1);
        store.set_epoch_deadline(deadline);
        store.epoch_deadline_callback(|mut ctx| {
            let now = Instant::now();
            let remaining_ticks = {
                let state = ctx.data_mut();
                if now < state.guest_deadline {
                    // Host tool calls extend the deadline, so re-arm instead of
                    // trapping when the guest still has budget.
                    Some(((state.guest_deadline - now).as_millis() as u64 / TICK_MS).max(1))
                } else {
                    state.timed_out = true;
                    None
                }
            };
            Ok(match remaining_ticks {
                // `Yield` lets the async executor run other tasks before the
                // guest resumes; `Interrupt` traps when the budget is gone.
                Some(ticks) => wasmtime::UpdateDeadline::Yield(ticks),
                None => wasmtime::UpdateDeadline::Interrupt,
            })
        });

        // Set up linker with WASI and the host tool interface. The async
        // linker is required because the guest exports are async.
        let mut linker = Linker::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
        Sandbox::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)?;

        // Instantiate the component
        let wasm_sandbox = Sandbox::instantiate_async(&mut store, &self.component, &linker).await?;

        // Give the guest a fresh budget: instantiation above may have consumed
        // part of the deadline, which should only bound user code.
        store.data_mut().guest_deadline = Instant::now() + timeout;
        store.set_epoch_deadline(deadline);

        // Execute the code through the concurrent (async) entry point. The
        // guest's async exports need an `Accessor`, which `run_concurrent`
        // provides.
        let code_owned = code.to_string();
        let argv_owned = argv.to_vec();
        let result = store
            .run_concurrent(async move |accessor| {
                wasm_sandbox
                    .call_exec(accessor, code_owned, argv_owned)
                    .await
            })
            .await;
        let timed_out = store.data().timed_out;

        match result {
            Ok(Ok(Ok(value))) => Ok(ExecResult {
                value,
                stdout: decode_pipe(&stdout_pipe),
                stderr: decode_pipe(&stderr_pipe),
            }),
            // Guest-level error: a Python traceback from the wrapper, safe to
            // surface to the caller.
            Ok(Ok(Err(e))) => Err(anyhow!("{}", e)),
            // Host-level trap or concurrent-execution failure. Surface a clean
            // message rather than the raw wasmtime trap, which leaks internal
            // symbols and addresses.
            Ok(Err(_)) | Err(_) if timed_out => Err(anyhow!("Execution timed out")),
            Ok(Err(_)) | Err(_) => {
                Err(anyhow!("sandbox execution failed (resource limit or trap)"))
            }
        }
    }
}

impl Drop for PySandbox {
    fn drop(&mut self) {
        self.stop_ticker.store(true, Ordering::SeqCst);
        if let Some(ticker) = self.ticker.take() {
            let _ = ticker.join();
        }
    }
}

fn decode_pipe(pipe: &MemoryOutputPipe) -> String {
    String::from_utf8_lossy(&pipe.contents()).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_default_timeout() {
        let sandbox = PySandbox::new_for_test(None).expect("Failed to create sandbox");
        assert_eq!(sandbox.timeout_seconds, DEFAULT_TIMEOUT_SECONDS);
    }

    #[test]
    fn test_sandbox_custom_timeout() {
        let sandbox = PySandbox::new_for_test(Some(10)).expect("Failed to create sandbox");
        assert_eq!(sandbox.timeout_seconds, 10);
    }

    #[test]
    fn test_sandbox_creates_successfully_with_wasm() {
        let result = PySandbox::new_for_test(None);
        assert!(result.is_ok());
    }

    #[derive(serde::Serialize)]
    struct SlowTool;

    #[async_trait::async_trait]
    impl crate::openai::ToolCall for SlowTool {
        async fn call(&self, _args: &str) -> Result<String, anyhow::Error> {
            // Longer than the sandbox's guest budget below.
            tokio::time::sleep(Duration::from_secs(3)).await;
            Ok("null".to_string())
        }

        fn function_name(&self) -> String {
            "slow".to_string()
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_tool_time_does_not_consume_guest_budget() {
        // Two seconds of guest budget, three seconds of host tool time. If the
        // host call were counted, the guest would trap on the next instruction.
        let sandbox = PySandbox::new_for_test(Some(2)).expect("Failed to create sandbox");
        let workspace = std::env::temp_dir().join(format!("hq-sandbox-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();

        let mut tools = HashMap::new();
        tools.insert("slow".to_string(), Box::new(SlowTool) as BoxedToolCall);
        let definitions = serde_json::json!([{
            "name": "slow",
            "description": "Sleeps.",
            "parameters": {
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }
        }])
        .to_string();
        let bridge = Arc::new(ToolBridge::new(
            Arc::new(tools),
            definitions,
            Arc::new(Mutex::new(HashSet::new())),
        ));

        let result = sandbox
            .exec_with_tools(
                "await hq.slow()\nsum(range(1000))",
                &workspace,
                b"",
                &["-c".to_string()],
                Some(bridge),
            )
            .await
            .unwrap();

        assert_eq!(result.value, "499500");
    }

    #[tokio::test]
    async fn test_sandbox_exposes_argv() {
        let sandbox = PySandbox::new_for_test(None).expect("Failed to create sandbox");
        let workspace = std::env::temp_dir();
        let argv = vec!["-c".to_string(), "first".to_string(), "second".to_string()];

        let result = sandbox
            .exec("import sys\nsys.argv", &workspace, b"", &argv)
            .await
            .unwrap();

        assert_eq!(result.value, r#"["-c", "first", "second"]"#);
    }
}
