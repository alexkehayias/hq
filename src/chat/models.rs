//! The core models for managing a stateful chat with an LLM.
use crate::openai::Message;

// TODO: Should there be an app specific `Message` object instead of
// building around OpenAI?

#[derive(Default)]
pub struct Transcript(Vec<Message>);

impl Transcript {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn new_with_messages(messages: Vec<Message>) -> Self {
        Self(messages)
    }

    pub fn messages(&self) -> Vec<Message> {
        self.0.clone()
    }

    pub fn push(&mut self, msg: Message) {
        self.0.push(msg)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Message> {
        self.0.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Message> {
        self.0.iter_mut()
    }
}

// TODO: Consider a session model to keep track of things like
// metrics, rate limits, registries.
// pub struct Session {
//     id: String,
//     transcript: Transcript,
// }
