use crate::ai::tools::{CalendarTool, TasksDueTodayTool, TasksScheduledTodayTool};
use crate::openai::chat;
use crate::openai::{Message, Role, ToolCall};

/// Daily agenda creator agent.
pub async fn daily_agenda_response(
    api_base_url: &str,
    calendar_emails: Vec<String>,
    openai_api_hostname: &str,
    openai_api_key: &str,
    openai_model: &str,
) -> Vec<Message> {
    let tasks_due_today_tool = TasksDueTodayTool::new(api_base_url);
    let tasks_scheduled_today_tool = TasksScheduledTodayTool::new(api_base_url);
    let calendar_tool = CalendarTool::new(api_base_url);

    let tools: Option<Vec<Box<dyn ToolCall + Send + Sync + 'static>>> = Some(vec![
        Box::new(tasks_due_today_tool),
        Box::new(tasks_scheduled_today_tool),
        Box::new(calendar_tool),
    ]);

    let system_msg = r#"You are a daily agenda assistant. Create an easy-to-read digest of what needs to happen today.

Use the available tools to gather:
1. Tasks due today
2. Tasks scheduled for today
3. Today's calendar events

Format the output as a short, scannable summary with:
- A brief overview of today's priorities
- Time-sensitive calendar events (with times)
- Tasks grouped by urgency or category
- Keep it concise and actionable

Avoid verbose descriptions. Focus on what's most important for the user to know."#;

    let user_msg = format!(
        r#"Create my daily agenda. My calendar emails are {}."#,
        calendar_emails.join("and ")
    );

    let history = vec![
        Message::new(Role::System, system_msg),
        Message::new(Role::User, &user_msg),
    ];

    chat(
        &tools,
        &history,
        openai_api_hostname,
        openai_api_key,
        openai_model,
    )
    .await
    .expect("Chat session failed")
}
