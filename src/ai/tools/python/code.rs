//! The `codemode` tool: Python in the WASM sandbox with typed host tools.
//!
//! It is offered alongside the normal tools, so the model can call a tool
//! directly or compose several calls in Python, whichever fits the task.
//!
//! Guest code reaches tools through the `hq` namespace, e.g.
//! `await hq.search_notes(query="...")`. The Python side builds a typed wrapper per
//! tool from the JSON schema the host exposes (see `guest.py`), so the model
//! writes ordinary Python instead of one tool call per step.
//!
//! Only the tools in [`BRIDGED_TOOLS`] are reachable. This is deliberately an
//! allowlist: adding a tool to the registry must not silently grant guest code
//! access to it. `bash` and `python` are excluded — the guest already has the
//! session workspace and the Python standard library, so a nested shell or
//! sandbox adds capability without adding anything the code sandbox cannot do.
//! `subagent` is excluded too: it is exposed as a separate top-level tool so
//! its calls stay visible to the chat loop and can run concurrently.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Error, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::ai::chat::workspace::workspace_root;
use crate::ai::tools::registry::{Tool, ToolConfig, ToolRegistry};
use crate::openai::{
    ArrayProperty, BoxedToolCall, Function, Parameters, Property, RecoverableToolError, ToolCall,
    ToolType, parse_tool_args,
};

use super::sandbox::{ExecResult, ToolBridge, shared_sandbox};
use super::tool::{PythonArgs, PythonOutput, PythonProps};

/// Tools guest code may call. Keep this in sync with the tools the model
/// actually has; an explicit list is safer than "everything in the registry".
const BRIDGED_TOOLS: &[&str] = &[
    "create_channel",
    "create_loop",
    "datetime",
    "delete_loop",
    "emit_channel",
    "get_calendar_events",
    "get_unread_emails",
    "list_loops",
    "list_skills",
    "load_skill",
    "notify",
    "read_skill_file",
    "save_skill",
    "search_emails",
    "search_meetings",
    "search_notes",
    "search_skills",
    "tasks_due_today",
    "tasks_scheduled_today",
    "view_website",
    "web_search",
    "work_on_skill",
];

/// One tool as the guest sees it: enough to generate a typed Python wrapper.
#[derive(Clone, Serialize, Deserialize)]
struct ToolDefinition {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

/// Runs Python code with typed access to the host tools in [`BRIDGED_TOOLS`].
#[derive(Serialize)]
pub struct CodeModeTool {
    pub r#type: ToolType,
    pub function: Function<PythonProps>,
    #[serde(skip_serializing)]
    workspace_path: PathBuf,
    #[serde(skip_serializing)]
    tools: Arc<HashMap<String, BoxedToolCall>>,
    #[serde(skip_serializing)]
    definitions: String,
    /// Tools used so far in the session, seeded from the transcript and shared
    /// across executions so nested calls enforce the security middleware.
    #[serde(skip_serializing)]
    used_tools: Arc<Mutex<HashSet<String>>>,
}

#[async_trait]
impl ToolCall for CodeModeTool {
    async fn call(&self, args: &str) -> Result<String, Error> {
        let fn_args: PythonArgs = parse_tool_args(args)?;

        fs::create_dir_all(&self.workspace_path).await?;

        let code = fn_args.code;
        let argv = std::iter::once("-c".to_string())
            .chain(fn_args.argv)
            .collect::<Vec<_>>();
        let workspace = self.workspace_path.clone();
        let tools = self.tools.clone();
        let definitions = self.definitions.clone();
        let used_tools = self.used_tools.clone();
        // Captured here rather than inside `spawn_blocking` so the blocking
        // thread does not have to look up the runtime.
        let runtime = tokio::runtime::Handle::current();

        let exec =
            tokio::task::spawn_blocking(move || -> Result<ExecResult, RecoverableToolError> {
                let sandbox =
                    shared_sandbox().map_err(|e| RecoverableToolError::new(&e.to_string()))?;
                let bridge = Arc::new(ToolBridge::new(tools, definitions, used_tools));
                runtime
                    .block_on(sandbox.exec_with_tools(&code, &workspace, b"", &argv, Some(bridge)))
                    .map_err(|e| RecoverableToolError::new(&e.to_string()))
            })
            .await??;

        let value = serde_json::from_str(&exec.value)
            .unwrap_or_else(|_| serde_json::Value::String(exec.value.clone()));
        let output = PythonOutput {
            value,
            stdout: exec.stdout,
            stderr: exec.stderr,
        };

        Ok(serde_json::to_string(&output)?)
    }

    fn function_name(&self) -> String {
        Self::NAME.to_string()
    }
}

impl Tool for CodeModeTool {
    const NAME: &'static str = "codemode";

    fn from_config(conf: &ToolConfig) -> Result<Self> {
        // Resolve the bridged tools from a fresh registry rather than holding
        // the whole tool set: the guest can only reach what is in the allowlist.
        let registry = ToolRegistry::builtin(conf.clone());
        let tools = BRIDGED_TOOLS
            .iter()
            .filter_map(|name| registry.from_str(name).ok())
            .collect();
        Self::build(
            workspace_root(&conf.storage_path, &conf.session_id),
            tools,
            HashSet::new(),
        )
    }
}

impl CodeModeTool {
    /// Seed the prompt-injection state with the tools already used in this
    /// session. Nested calls are invisible to the chat middleware, so the
    /// bridge checks the same rules itself.
    pub fn seed_used_tools(&self, used: HashSet<String>) {
        let mut set = self
            .used_tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        set.extend(used);
    }

    /// Build a code tool over `tools`, keeping only those in the allowlist.
    ///
    /// `used_tools` seeds the prompt-injection rules from the session
    /// transcript; nested calls add to it as they run.
    pub fn new(
        storage_path: &str,
        session_id: &str,
        tools: Vec<BoxedToolCall>,
        used_tools: HashSet<String>,
    ) -> Result<Self> {
        let tools = tools
            .into_iter()
            .filter(|tool| BRIDGED_TOOLS.contains(&tool.function_name().as_str()))
            .collect();
        Self::build(workspace_root(storage_path, session_id), tools, used_tools)
    }

    fn build(
        workspace_path: PathBuf,
        tools: Vec<BoxedToolCall>,
        used_tools: HashSet<String>,
    ) -> Result<Self> {
        let mut map = HashMap::new();
        let mut definitions = Vec::new();

        for tool in tools {
            let name = tool.function_name();
            let value = serde_json::to_value(&tool)?;
            let function = &value["function"];
            definitions.push(ToolDefinition {
                name: name.clone(),
                description: function["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                parameters: function["parameters"].clone(),
            });
            map.insert(name, tool);
        }

        definitions.sort_by(|a, b| a.name.cmp(&b.name));
        let definitions_json = serde_json::to_string(&definitions)?;

        let index = definitions
            .iter()
            .map(|definition| {
                format!(
                    "- {}: {}",
                    definition.name,
                    first_line(&definition.description)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let parameters = Parameters {
            r#type: String::from("object"),
            properties: PythonProps {
                code: Property {
                    r#type: String::from("string"),
                    description: String::from(
                        "The Python code to execute. Host tools are async and live on the `hq` \
                         namespace, e.g. `await hq.search_notes(query=\"...\")`; use \
                         `asyncio.gather(...)` to run several concurrently. Call `hq.help()` \
                         for every tool and signature, or `hq.help(\"name\")` for one tool's \
                         arguments. Top-level `await` is supported. The value of the last \
                         expression is returned. Use print() to produce stdout output.",
                    ),
                    r#enum: None,
                },
                argv: ArrayProperty::strings(
                    "Command-line arguments for the program, exposed as `sys.argv[1:]`. \
                     `sys.argv[0]` is `-c`, matching `python -c`. Pass an empty array when the \
                     code takes no arguments.",
                ),
            },
            required: vec![String::from("code"), String::from("argv")],
            additional_properties: false,
        };

        let description = format!(
            "Execute Python code in an isolated sandbox with typed access to host tools. Use \
             this when a task needs several tool calls, loops, filtering, or joins that would \
             otherwise take many round-trips; the same tools are also available as regular \
             tool calls. Call tools as `await hq.<name>(...)`; use `asyncio.gather(...)` to \
             run several concurrently, and `hq.help()` to list them or `hq.help(\"name\")` \
             for argument docs. Top-level `await` is supported. The value of the last \
             expression is \
             returned. Files are read and written relative to `/`, the session workspace \
             shared with the bash tool. There is no network or environment access and only a \
             subset of the standard library is available.\n\nAvailable tools:\n{index}"
        );

        let function = Function {
            name: Self::NAME.to_string(),
            description,
            parameters,
            strict: true,
        };

        Ok(Self {
            r#type: ToolType::Function,
            function,
            workspace_path,
            tools: Arc::new(map),
            definitions: definitions_json,
            used_tools: Arc::new(Mutex::new(used_tools)),
        })
    }
}

/// First line of a tool description, truncated for the tool index.
fn first_line(text: &str) -> String {
    let stripped = text.trim();
    let line = stripped.lines().next().unwrap_or_default();
    if line.chars().count() <= 120 {
        line.to_string()
    } else {
        format!("{}...", line.chars().take(117).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[derive(Serialize)]
    struct EchoProps {
        value: Property,
    }

    #[derive(Serialize)]
    struct EchoTool {
        r#type: ToolType,
        function: Function<EchoProps>,
        #[serde(skip_serializing)]
        name: String,
    }

    #[async_trait]
    impl ToolCall for EchoTool {
        async fn call(&self, args: &str) -> Result<String, Error> {
            let value: serde_json::Value = serde_json::from_str(args)?;
            Ok(serde_json::to_string(&value["value"])?)
        }

        fn function_name(&self) -> String {
            self.name.clone()
        }
    }

    #[derive(Serialize)]
    struct EmptyProps {}

    #[derive(Serialize)]
    struct FailTool {
        r#type: ToolType,
        function: Function<EmptyProps>,
    }

    #[async_trait]
    impl ToolCall for FailTool {
        async fn call(&self, _args: &str) -> Result<String, Error> {
            Err(Error::msg("boom"))
        }

        fn function_name(&self) -> String {
            "fail".to_string()
        }
    }

    fn fail_tool() -> BoxedToolCall {
        Box::new(FailTool {
            r#type: ToolType::Function,
            function: Function {
                name: "fail".to_string(),
                description: "Always fails.".to_string(),
                parameters: Parameters {
                    r#type: String::from("object"),
                    properties: EmptyProps {},
                    required: vec![],
                    additional_properties: false,
                },
                strict: true,
            },
        })
    }

    fn named_echo_tool(name: &str) -> BoxedToolCall {
        let parameters = Parameters {
            r#type: String::from("object"),
            properties: EchoProps {
                value: Property::new("string", "Value to echo back."),
            },
            required: vec![String::from("value")],
            additional_properties: false,
        };
        Box::new(EchoTool {
            r#type: ToolType::Function,
            function: Function {
                name: name.to_string(),
                description: "Echo a value.".to_string(),
                parameters,
                strict: true,
            },
            name: name.to_string(),
        })
    }

    fn echo_tool() -> BoxedToolCall {
        named_echo_tool("echo")
    }

    type Intervals = Arc<Mutex<Vec<(std::time::Instant, std::time::Instant)>>>;

    #[derive(Serialize)]
    struct SleepTool {
        r#type: ToolType,
        function: Function<EchoProps>,
        #[serde(skip_serializing)]
        name: String,
        #[serde(skip_serializing)]
        intervals: Intervals,
    }

    #[async_trait]
    impl ToolCall for SleepTool {
        async fn call(&self, args: &str) -> Result<String, Error> {
            let value: serde_json::Value = serde_json::from_str(args)?;
            let started = std::time::Instant::now();
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            self.intervals
                .lock()
                .unwrap()
                .push((started, std::time::Instant::now()));
            Ok(serde_json::to_string(&value["value"])?)
        }

        fn function_name(&self) -> String {
            self.name.clone()
        }
    }

    fn sleep_tool(intervals: Intervals) -> BoxedToolCall {
        Box::new(SleepTool {
            r#type: ToolType::Function,
            function: Function {
                name: "sleep".to_string(),
                description: "Sleeps for a second, then echoes a value.".to_string(),
                parameters: Parameters {
                    r#type: String::from("object"),
                    properties: EchoProps {
                        value: Property::new("string", "Value to echo back."),
                    },
                    required: vec![String::from("value")],
                    additional_properties: false,
                },
                strict: true,
            },
            name: "sleep".to_string(),
            intervals,
        })
    }

    fn code_tool(tools: Vec<BoxedToolCall>) -> CodeModeTool {
        let temp_dir = std::env::temp_dir();
        let workspace = temp_dir.join("workspace").join(Uuid::new_v4().to_string());
        CodeModeTool::build(workspace, tools, HashSet::new()).unwrap()
    }

    #[tokio::test]
    async fn calls_bridged_tool_from_guest() {
        let tool = code_tool(vec![echo_tool()]);
        let result = tool
            .call(r#"{"code": "await hq.echo(value=\"hi\")"}"#)
            .await
            .unwrap();
        let output: PythonOutput = serde_json::from_str(&result).unwrap();

        assert_eq!(output.value, serde_json::json!("hi"));
    }

    #[tokio::test]
    async fn guest_can_compose_tool_calls() {
        let tool = code_tool(vec![echo_tool()]);
        let code = "values = [await hq.echo(value=str(i)) for i in range(3)]\nvalues";
        let result = tool
            .call(&serde_json::json!({ "code": code }).to_string())
            .await
            .unwrap();
        let output: PythonOutput = serde_json::from_str(&result).unwrap();

        assert_eq!(output.value, serde_json::json!(["0", "1", "2"]));
    }

    #[tokio::test]
    async fn gather_interleaves_guest_coroutines() {
        let tool = code_tool(vec![]);
        let code = r#"
import asyncio
order = []
async def step(name):
    order.append(name + ":start")
    await asyncio.sleep(0)
    order.append(name + ":end")
    return name
results = await asyncio.gather(step("a"), step("b"))
(results, order)
"#;
        let result = tool
            .call(&serde_json::json!({ "code": code }).to_string())
            .await
            .unwrap();
        let output: PythonOutput = serde_json::from_str(&result).unwrap();

        assert_eq!(
            output.value,
            serde_json::json!([["a", "b"], ["a:start", "b:start", "a:end", "b:end"]])
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn gather_runs_tool_calls_concurrently() {
        let intervals: Intervals = Arc::new(Mutex::new(Vec::new()));
        let tool = code_tool(vec![sleep_tool(intervals.clone())]);
        let code = r#"
import asyncio
results = await asyncio.gather(hq.sleep(value="a"), hq.sleep(value="b"))
results
"#;
        let result = tool
            .call(&serde_json::json!({ "code": code }).to_string())
            .await
            .unwrap();
        let output: PythonOutput = serde_json::from_str(&result).unwrap();

        assert_eq!(output.value, serde_json::json!(["a", "b"]));

        let intervals = intervals.lock().unwrap();
        assert_eq!(intervals.len(), 2, "expected two tool calls");
        let (first, second) = (&intervals[0], &intervals[1]);
        assert!(
            first.0 < second.1 && second.0 < first.1,
            "tool calls did not overlap: {intervals:?}"
        );
    }

    #[tokio::test]
    async fn unawaited_tool_call_is_an_error() {
        let tool = code_tool(vec![echo_tool()]);
        let err = tool
            .call(r#"{"code": "hq.echo(value=\"hi\")"}"#)
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("not awaited"),
            "expected a missing-await error, got: {err}"
        );
    }

    #[tokio::test]
    async fn missing_arguments_raise_python_type_error() {
        let tool = code_tool(vec![echo_tool()]);
        let err = tool
            .call(r#"{"code": "await hq.echo(bogus=\"x\")"}"#)
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("TypeError"),
            "expected a TypeError traceback, got: {err}"
        );
        assert!(
            err.to_string()
                .contains("missing a required keyword-only argument"),
            "expected the missing-argument message, got: {err}"
        );
    }

    #[tokio::test]
    async fn unknown_arguments_raise_python_type_error() {
        let tool = code_tool(vec![echo_tool()]);
        let err = tool
            .call(r#"{"code": "await hq.echo(value=\"x\", bogus=\"y\")"}"#)
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("unexpected keyword argument"),
            "expected an unexpected-argument message, got: {err}"
        );
    }

    #[tokio::test]
    async fn tool_failures_raise_tool_error() {
        let tool = code_tool(vec![fail_tool()]);
        let err = tool
            .call(r#"{"code": "await hq.fail()"}"#)
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("boom"),
            "expected the tool error message, got: {err}"
        );
    }

    #[tokio::test]
    async fn guest_can_catch_tool_errors() {
        let tool = code_tool(vec![fail_tool()]);
        let code =
            "try:\n    await hq.fail()\nexcept hq.ToolError as e:\n    caught = str(e)\ncaught";
        let result = tool
            .call(&serde_json::json!({ "code": code }).to_string())
            .await
            .unwrap();
        let output: PythonOutput = serde_json::from_str(&result).unwrap();

        assert_eq!(output.value, serde_json::json!("boom"));
    }

    #[tokio::test]
    async fn help_lists_bridged_tools() {
        let tool = code_tool(vec![echo_tool()]);
        let result = tool.call(r#"{"code": "hq.help()"}"#).await.unwrap();
        let output: PythonOutput = serde_json::from_str(&result).unwrap();

        let help = output.value.as_str().unwrap();
        assert!(help.contains("echo(*, value)"));
        assert!(help.contains("Echo a value."));
    }

    #[tokio::test]
    async fn sandbox_without_tools_has_empty_namespace() {
        let tool = code_tool(vec![]);
        let result = tool.call(r#"{"code": "hq.help()"}"#).await.unwrap();
        let output: PythonOutput = serde_json::from_str(&result).unwrap();

        assert_eq!(output.value, serde_json::json!(""));
    }

    #[tokio::test]
    async fn security_rules_apply_to_nested_calls() {
        let mut used = HashSet::new();
        used.insert("view_website".to_string());
        let workspace = std::env::temp_dir()
            .join("workspace")
            .join(Uuid::new_v4().to_string());
        let tool = CodeModeTool::build(workspace, vec![named_echo_tool("notify")], used).unwrap();

        let err = tool
            .call(r#"{"code": "await hq.notify(value=\"x\")"}"#)
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("cannot be called after"),
            "expected a security rejection, got: {err}"
        );
    }

    #[test]
    fn shell_sandbox_and_subagent_are_not_bridged() {
        assert!(!BRIDGED_TOOLS.contains(&"bash"));
        assert!(!BRIDGED_TOOLS.contains(&"python"));
        assert!(!BRIDGED_TOOLS.contains(&"codemode"));
        // Exposed as a separate top-level tool so its calls stay visible.
        assert!(!BRIDGED_TOOLS.contains(&"subagent"));
    }

    #[test]
    fn tool_name_is_codemode() {
        assert_eq!(CodeModeTool::NAME, "codemode");
    }

    async fn test_registry() -> ToolRegistry {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::core::db::async_db(dir.path().to_str().unwrap())
            .await
            .unwrap();
        db.call(|conn| {
            crate::core::db::initialize_db(conn).expect("Failed to migrate db");
            Ok::<_, tokio_rusqlite::rusqlite::Error>(())
        })
        .await
        .unwrap();

        ToolRegistry::builtin(ToolConfig {
            db,
            api_base_url: "http://localhost:2222".to_string(),
            storage_path: dir.path().to_string_lossy().to_string(),
            vapid_key_path: String::new(),
            session_id: "test-session".to_string(),
            notification_url: None,
            skill_registry: None,
            api_hostname: "https://api.openai.com".to_string(),
            api_key: "test-key".to_string(),
            model: "gpt-4".to_string(),
        })
    }

    #[tokio::test]
    async fn every_bridged_tool_name_exists() {
        let registry = test_registry().await;
        let available = registry.available_names();
        for name in BRIDGED_TOOLS {
            assert!(
                available.contains(&name.to_string()),
                "unknown tool in allowlist: {name}"
            );
        }
    }
}
