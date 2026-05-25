---
name: add-new-tool
description: |
  Add a new AI-callable tool to the hq Rust codebase. Follows the project's tool pattern:
  ToolCall trait, parse_tool_args for JSON arg parsing, RecoverableToolError for retryable
  failures, module registration in mod.rs, and wiring in cli/chat.rs.
---

# Adding a New Tool

Tools are AI-callable functions that extend what the LLM can do. Each tool lives in its own file under `src/ai/tools/`.

## Step 1: Create the tool file

Create `src/ai/tools/your_tool.rs`. Follow this structure:

```rust
use crate::openai::{
    Function, Parameters, Property, ToolCall, ToolType, parse_tool_args,
};
use anyhow::{Error, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// Props struct — JSON Schema the LLM uses to generate arguments
#[derive(Serialize)]
pub struct YourToolProps {
    pub query: Property,
}

// Args struct — deserialized form of the LLM's JSON arguments
#[derive(Deserialize)]
pub struct YourToolArgs {
    pub query: String,
}

// Tool struct — function metadata + dependencies (DB, API URL, etc.)
#[derive(Serialize)]
pub struct YourTool {
    pub r#type: ToolType,
    pub function: Function<YourToolProps>,
    #[serde(skip)]
    api_base_url: String,
}

#[async_trait]
impl ToolCall for YourTool {
    async fn call(&self, args: &str) -> Result<String, Error> {
        let fn_args: YourToolArgs = parse_tool_args(args)?;
        // ... business logic ...
        Ok(format!("## Result\n\nFound: {}", fn_args.query))
    }

    fn function_name(&self) -> String {
        self.function.name.clone()
    }
}

impl YourTool {
    pub fn new(api_base_url: &str) -> Self {
        let function = Function {
            name: String::from("your_tool"),
            description: String::from("Describe what this tool does and when the AI should use it."),
            parameters: Parameters {
                r#type: String::from("object"),
                properties: YourToolProps {
                    query: Property {
                        r#type: String::from("string"),
                        description: String::from("Description of this parameter."),
                        r#enum: None,
                    },
                },
                required: vec![String::from("query")],
                additional_properties: false,
            },
            strict: true,
        };
        Self {
            r#type: ToolType::Function,
            function,
            api_base_url: api_base_url.to_string(),
        }
    }
}

impl Default for YourTool {
    fn default() -> Self {
        Self::new("http://localhost:2222")
    }
}
```

## Step 2: Register the module

Add to `src/ai/tools/mod.rs`:

```rust
pub mod your_tool;
pub use your_tool::YourTool;
```

## Step 3: Wire into the CLI

In `src/cli/chat.rs`:

1. Add to the import:
```rust
use crate::ai::tools::{
    CalendarTool, EmailUnreadTool, MeetingSearchTool, MemoryTool, NoteSearchTool, WebSearchTool,
    YourTool,
};
```

2. Create an instance and add it to the tools vec:
```rust
let your_tool = if let Ok(url) = &note_search_api_url {
    YourTool::new(url)
} else {
    YourTool::default()
};

let tools: Vec<BoxedToolCall> = vec![
    Box::new(note_search_tool),
    Box::new(meeting_search_tool),
    Box::new(web_search_tool),
    Box::new(email_unread_tool),
    Box::new(calendar_tool),
    Box::new(memory_tool),
    Box::new(your_tool),
];
```

## Returning results

Return `Result<String, Error>`. The string should be markdown or JSON:

- **Markdown** for human-readable results:
  ```rust
  Ok(format!("## Results\n\n- Item 1\n- Item 2\n\nTotal: {}", count))
  ```

- **JSON** for structured data the LLM can reason about:
  ```rust
  Ok(serde_json::to_string(&result)?)
  ```

## Handling errors

| Scenario | Approach |
|----------|----------|
| Bad LLM arguments | `parse_tool_args(args)?` — returns `RecoverableToolError`, LLM retries |
| Transient failures (timeout, 5xx) | `return Err(RecoverableToolError::new("msg").into())` |
| Permanent failures (not found, bad input) | Return `Ok("informative message".to_string())` |
| Unexpected bugs | `anyhow::bail!("message")` — fatal, crashes the chat loop |
