use anyhow::Result;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::env;

use crate::ai::tools::{CalendarTool, EmailUnreadTool, NoteSearchTool, WebSearchTool};
use crate::openai::chat;
use crate::openai::{Message, Role, ToolCall};

pub async fn run() -> Result<()> {
    let mut rl = DefaultEditor::new().expect("Editor failed");

    // Create tools
    let note_search_api_url = env::var("INDEXER_NOTE_SEARCH_API_URL");
    let note_search_tool = if let Ok(url) = &note_search_api_url {
        NoteSearchTool::new(url)
    } else {
        NoteSearchTool::default()
    };

    let email_unread_tool = if let Ok(url) = &note_search_api_url {
        EmailUnreadTool::new(url)
    } else {
        EmailUnreadTool::default()
    };

    let web_search_tool = if let Ok(url) = &note_search_api_url {
        WebSearchTool::new(url)
    } else {
        WebSearchTool::default()
    };

    let calendar_tool = if let Ok(url) = &note_search_api_url {
        CalendarTool::new(url)
    } else {
        CalendarTool::default()
    };

    let tools: Option<Vec<Box<dyn ToolCall + Send + Sync + 'static>>> = Some(vec![
        Box::new(note_search_tool),
        Box::new(web_search_tool),
        Box::new(email_unread_tool),
        Box::new(calendar_tool),
    ]);

    // Get OpenAI API configuration from environment variables (similar to AppConfig)
    let openai_api_hostname =
        env::var("INDEXER_LOCAL_LLM_HOST").unwrap_or_else(|_| "https://api.openai.com".to_string());
    let openai_api_key =
        env::var("OPENAI_API_KEY").unwrap_or_else(|_| "thiswontworkforopenai".to_string());
    let openai_model =
        env::var("INDEXER_LOCAL_LLM_MODEL").unwrap_or_else(|_| "gpt-4.1-mini".to_string());

    let mut history = vec![Message::new(Role::System, "You are a helpful assistant.")];

    loop {
        let readline = rl.readline(">>> ");
        match readline {
            Ok(line) => {
                history.push(Message::new(Role::User, line.as_str()));
                let resp = chat(
                    &tools,
                    &history,
                    &openai_api_hostname,
                    &openai_api_key,
                    &openai_model,
                )
                .await?;
                let msg = resp.last().unwrap();
                println!("{}", msg.content.clone().unwrap());
            }
            Err(ReadlineError::Interrupted) => break,
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }

    Ok(())
}
