//! Integration test that exercises image attachments against a real
//! completions API.
//!
//! Ignored by default because it makes a network call. Run with:
//!
//! ```sh
//! cargo test --test llm_attachments_test -- --ignored --nocapture
//! ```
//!
//! Uses `HQ_LOCAL_LLM_HOST` and `HQ_LOCAL_LLM_MODEL`, with the API key taken
//! from `OPENAI_API_KEY` (falling back to `ANTHROPIC_API_KEY`).

use hq::ai::chat::{ChatBuilder, user_message_with_attachments};
use hq::openai::{Message, Role};

/// A solid red image so the model has something unambiguous to identify.
fn write_red_png(path: &std::path::Path) {
    let mut image = image::RgbImage::new(64, 64);
    for pixel in image.pixels_mut() {
        *pixel = image::Rgb([255, 0, 0]);
    }
    image.save(path).expect("failed to write test image");
}

#[tokio::test]
#[ignore = "makes a real completions API call; run with --ignored"]
async fn real_completion_identifies_attached_image() {
    let Ok(host) = std::env::var("HQ_LOCAL_LLM_HOST") else {
        eprintln!("HQ_LOCAL_LLM_HOST not set, skipping");
        return;
    };
    let model = std::env::var("HQ_LOCAL_LLM_MODEL").unwrap_or_else(|_| "gpt-4.1-mini".to_string());
    let api_key = std::env::var("OPENAI_API_KEY")
        .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
        .unwrap_or_default();

    let temp = tempfile::TempDir::new().unwrap();
    let files = temp.path().join("files");
    std::fs::create_dir_all(&files).unwrap();
    write_red_png(&files.join("red.png"));

    let mut chat = ChatBuilder::new(&host, &api_key, &model)
        .transcript(vec![Message::new(
            Role::System,
            "Answer with a single word.",
        )])
        .workspace_path(temp.path())
        .build();

    let user = user_message_with_attachments("What color is this image?", &["red.png".to_string()]);
    let messages = chat
        .next_msg(user)
        .await
        .expect("real completion call failed");
    let reply = messages.last().and_then(|m| m.text()).unwrap_or_default();

    println!("model replied: {reply}");
    assert!(
        reply.to_lowercase().contains("red"),
        "expected the model to identify the red image, got: {reply}"
    );
}
