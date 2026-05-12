use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub prompt: String,
    pub expected: EvalExpected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalExpected {
    pub contains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalRun {
    pub id: String,
    pub name: String,
    pub model: String,
    pub status: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub id: String,
    pub run_id: String,
    pub case_id: String,
    pub input: String,
    pub output: Option<String>,
    pub passed: bool,
    pub error: Option<String>,
}
