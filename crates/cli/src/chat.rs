use anyhow::Result;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

use hq::ai::chat::{
    ChatBuilder, InfiniteLoopDetector, InvisibleCharFilter, ToolSecurityMiddleware,
};
use hq::ai::tools::{
    CalendarTool, DateTimeTool, EmailUnreadTool, MeetingSearchTool, NoteSearchTool, SubagentTool,
    WebSearchTool,
};
use hq::core::db::async_db;
use hq::core::markdown_render;
use hq::openai::{BoxedToolCall, Message, Role};

pub async fn run(
    vec_db_path: &str,
    note_search_api_url: Option<&str>,
    api_hostname: &str,
    api_key: &str,
    model: &str,
) -> Result<()> {
    let db = async_db(vec_db_path)
        .await
        .expect("Failed to connect to db");
    let mut rl = DefaultEditor::new().expect("Editor failed");

    // Create tools
    let note_search_tool = if let Some(url) = note_search_api_url {
        NoteSearchTool::new(url)
    } else {
        NoteSearchTool::default()
    };

    let meeting_search_tool = if let Some(url) = note_search_api_url {
        MeetingSearchTool::new(url)
    } else {
        MeetingSearchTool::default()
    };

    let email_unread_tool = if let Some(url) = note_search_api_url {
        EmailUnreadTool::new(url)
    } else {
        EmailUnreadTool::default()
    };

    let web_search_tool = if let Some(url) = note_search_api_url {
        WebSearchTool::new(url)
    } else {
        WebSearchTool::default()
    };

    let calendar_tool = if let Some(url) = note_search_api_url {
        CalendarTool::new(db.clone(), url)
    } else {
        // This shouldn't happen - we always have a db now
        CalendarTool::new(db.clone(), "http://localhost:2222")
    };

    let datetime_tool = DateTimeTool::default();
    let subagent_tool = SubagentTool::new(api_hostname, api_key, model);

    let tools: Vec<BoxedToolCall> = vec![
        Box::new(note_search_tool),
        Box::new(meeting_search_tool),
        Box::new(web_search_tool),
        Box::new(email_unread_tool),
        Box::new(calendar_tool),
        Box::new(datetime_tool),
        Box::new(subagent_tool),
    ];

    let mut chat = ChatBuilder::new(api_hostname, api_key, model)
        .transcript(vec![Message::new(
            Role::System,
            "You are a helpful assistant.",
        )])
        .tools(tools)
        .middleware(vec![
            Box::new(InfiniteLoopDetector::new(3)),
            Box::new(ToolSecurityMiddleware::default()),
            Box::new(InvisibleCharFilter),
        ])
        .build();

    loop {
        let readline = rl.readline(">>> ");
        match readline {
            Ok(line) => {
                let user_msg = Message::new(Role::User, line.as_str());
                let resp = chat.next_msg(user_msg).await?;
                if let Some(content) = resp.last().and_then(|msg| msg.text()) {
                    print!("{}", markdown_render::render_for_stdout(content));
                }
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
