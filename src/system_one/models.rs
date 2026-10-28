//! Typed request and response models for the System One HTTP API.
//!
//! The API takes a `state` (the content to evaluate) and a set of named
//! questions, and returns a typed answer for each question. These types mirror
//! the shapes described in the System One OpenAPI schema.

use std::collections::BTreeMap;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Content accepted by the API wherever a string, object, or array is allowed.
///
/// `state`, question `instructions`, and criteria values can all be any of
/// these three shapes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Object(Map<String, Value>),
    Array(Vec<Value>),
}

impl Content {
    /// A plain-text value.
    pub fn text(text: impl Into<String>) -> Self {
        Content::Text(text.into())
    }

    /// A JSON object value.
    pub fn object(map: Map<String, Value>) -> Self {
        Content::Object(map)
    }

    /// A JSON array value.
    pub fn array(items: Vec<Value>) -> Self {
        Content::Array(items)
    }
}

impl From<&str> for Content {
    fn from(value: &str) -> Self {
        Content::Text(value.to_string())
    }
}

impl From<String> for Content {
    fn from(value: String) -> Self {
        Content::Text(value)
    }
}

impl From<Value> for Content {
    fn from(value: Value) -> Self {
        match value {
            Value::String(text) => Content::Text(text),
            Value::Object(map) => Content::Object(map),
            Value::Array(items) => Content::Array(items),
            other => Content::Text(other.to_string()),
        }
    }
}

/// Criteria clarifying what counts as a yes or no answer for a noul question.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NoulCriteria {
    /// What counts as a yes answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#true: Option<Content>,
    /// What counts as a no answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#false: Option<Content>,
}

/// A question about the supplied content. The `type` field selects the variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Question {
    /// A yes/no question, answered with the probability of yes.
    Noul {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instructions: Option<Content>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
    /// A selection from a fixed set of options.
    Choice {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instructions: Option<Content>,
        /// Option names mapped to a description of when each applies. A `None`
        /// description means the option name is interpreted on its own.
        criteria: BTreeMap<String, Option<Content>>,
    },
    /// A rating against ordered levels. Position determines the score, starting
    /// at zero.
    Score {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instructions: Option<Content>,
        criteria: Vec<Content>,
    },
    /// Locates what the instructions describe in an image.
    Box {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instructions: Option<Content>,
    },
}

impl Question {
    /// A yes/no question.
    pub fn noul(instructions: impl Into<Content>) -> Self {
        Question::Noul {
            instructions: Some(instructions.into()),
            criteria: None,
        }
    }

    /// A yes/no question with explicit criteria for what yes and no mean.
    pub fn noul_with_criteria(
        instructions: impl Into<Content>,
        true_means: impl Into<Content>,
        false_means: impl Into<Content>,
    ) -> Self {
        Question::Noul {
            instructions: Some(instructions.into()),
            criteria: Some(NoulCriteria {
                r#true: Some(true_means.into()),
                r#false: Some(false_means.into()),
            }),
        }
    }

    /// A choice question. `options` maps option names to an optional description
    /// of when the option applies.
    pub fn choice<I, S>(instructions: impl Into<Content>, options: I) -> Self
    where
        I: IntoIterator<Item = (S, Option<Content>)>,
        S: Into<String>,
    {
        Question::Choice {
            instructions: Some(instructions.into()),
            criteria: options
                .into_iter()
                .map(|(name, description)| (name.into(), description))
                .collect(),
        }
    }

    /// A score question. `levels` is ordered from lowest to highest.
    pub fn score(instructions: impl Into<Content>, levels: Vec<Content>) -> Self {
        Question::Score {
            instructions: Some(instructions.into()),
            criteria: levels,
        }
    }

    /// A box question, locating something in an image.
    pub fn boxed(instructions: impl Into<Content>) -> Self {
        Question::Box {
            instructions: Some(instructions.into()),
        }
    }
}

/// The image format of an [`Asset`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Png,
    Jpg,
    Jpeg,
}

/// An image that replaces its placeholder wherever the placeholder appears in
/// the request's strings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<ImageFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
}

impl Asset {
    /// An image asset built from raw bytes, base64-encoded for the JSON API.
    pub fn from_bytes(format: ImageFormat, bytes: &[u8]) -> Self {
        Asset {
            r#type: "image".to_string(),
            format: Some(format),
            data_base64: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
            part: None,
        }
    }
}

/// A request to answer one or more questions about `state`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemOneRequest {
    pub model: String,
    pub state: Content,
    pub questions: BTreeMap<String, Question>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub assets: BTreeMap<String, Asset>,
}

impl SystemOneRequest {
    /// A request with no questions yet.
    pub fn new(model: impl Into<String>, state: impl Into<Content>) -> Self {
        Self {
            model: model.into(),
            state: state.into(),
            questions: BTreeMap::new(),
            assets: BTreeMap::new(),
        }
    }

    /// Add a question under the given id.
    pub fn question(mut self, id: impl Into<String>, question: Question) -> Self {
        self.questions.insert(id.into(), question);
        self
    }

    /// Add an image asset under the given placeholder.
    pub fn asset(mut self, placeholder: impl Into<String>, asset: Asset) -> Self {
        self.assets.insert(placeholder.into(), asset);
        self
    }

    /// Whether this request needs the `TypeSafe-Danger-Zone-Images` header: it
    /// carries assets or asks a box question.
    pub fn has_images(&self) -> bool {
        !self.assets.is_empty()
            || self
                .questions
                .values()
                .any(|question| matches!(question, Question::Box { .. }))
    }
}

/// Normalized box edges, as fractions of the image's width and height.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoxCoordinates {
    pub top: f64,
    pub bottom: f64,
    pub left: f64,
    pub right: f64,
}

/// An answer whose type matches the corresponding question. The `type` field
/// selects the variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Answer {
    /// The probability of a yes answer or true statement, from 0 to 1.
    Noul { noul: f64 },
    /// The selected option, with confidence and the full probability spread.
    Choice {
        choice: String,
        confidence: f64,
        probabilities: BTreeMap<String, f64>,
    },
    /// The expected score along the rubric, with confidence and probabilities.
    Score {
        score: f64,
        confidence: f64,
        legend: BTreeMap<String, Content>,
        probabilities: BTreeMap<String, f64>,
    },
    /// A located box, with normalized coordinates and optional confidence.
    Box {
        #[serde(rename = "box")]
        coordinates: BoxCoordinates,
        confidence: Option<f64>,
    },
}

impl Answer {
    /// The probability of a yes answer, for a noul answer.
    pub fn noul(&self) -> Option<f64> {
        match self {
            Answer::Noul { noul } => Some(*noul),
            _ => None,
        }
    }

    /// The selected option, for a choice answer.
    pub fn choice(&self) -> Option<&str> {
        match self {
            Answer::Choice { choice, .. } => Some(choice),
            _ => None,
        }
    }

    /// The expected score, for a score answer.
    pub fn score(&self) -> Option<f64> {
        match self {
            Answer::Score { score, .. } => Some(*score),
            _ => None,
        }
    }

    /// Confidence in the answer, when the answer's type reports one.
    pub fn confidence(&self) -> Option<f64> {
        match self {
            Answer::Choice { confidence, .. } | Answer::Score { confidence, .. } => {
                Some(*confidence)
            }
            Answer::Box { confidence, .. } => *confidence,
            Answer::Noul { .. } => None,
        }
    }

    /// The probability distribution across options or levels, when present.
    pub fn probabilities(&self) -> Option<&BTreeMap<String, f64>> {
        match self {
            Answer::Choice { probabilities, .. } | Answer::Score { probabilities, .. } => {
                Some(probabilities)
            }
            _ => None,
        }
    }

    /// The located box, for a box answer.
    pub fn box_coordinates(&self) -> Option<&BoxCoordinates> {
        match self {
            Answer::Box { coordinates, .. } => Some(coordinates),
            _ => None,
        }
    }
}

/// Token usage for a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Answers keyed by the question ids supplied in the request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemOneResponse {
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
    pub usage: Usage,
}

impl SystemOneResponse {
    /// The raw answer for a question id.
    pub fn answer(&self, id: &str) -> Option<&Answer> {
        self.answers.get(id)
    }

    /// The yes-probability for a noul question id.
    pub fn noul(&self, id: &str) -> Option<f64> {
        self.answer(id).and_then(Answer::noul)
    }

    /// The selected option for a choice question id.
    pub fn choice(&self, id: &str) -> Option<&str> {
        self.answer(id).and_then(Answer::choice)
    }

    /// The expected score for a score question id.
    pub fn score(&self, id: &str) -> Option<f64> {
        self.answer(id).and_then(Answer::score)
    }

    /// The confidence for a question id, when its answer reports one.
    pub fn confidence(&self, id: &str) -> Option<f64> {
        self.answer(id).and_then(Answer::confidence)
    }
}

/// A model or model alias available to the account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelMetadata {
    pub name: String,
    pub description: String,
    pub release_date: String,
}

/// The list-models response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelList {
    pub models: Vec<ModelMetadata>,
}

/// The body of an error response. `detail` is either a message string (400) or
/// a list of validation errors (422).
#[derive(Clone, Debug, Deserialize)]
pub struct ApiErrorBody {
    #[serde(default)]
    pub detail: Option<Value>,
}

impl ApiErrorBody {
    /// A human-readable message summarizing the error detail.
    pub fn message(&self) -> Option<String> {
        match self.detail.as_ref()? {
            Value::String(text) => Some(text.clone()),
            other => Some(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_noul_question() {
        let request = SystemOneRequest::new("jev-latest", "I was charged twice.")
            .question("billing", Question::noul("Is this message about billing?"));
        let value = serde_json::to_value(&request).unwrap();

        assert_eq!(value["model"], json!("jev-latest"));
        assert_eq!(value["state"], json!("I was charged twice."));
        assert_eq!(
            value["questions"]["billing"],
            json!({"type": "noul", "instructions": "Is this message about billing?"})
        );
        assert!(value.get("assets").is_none());
    }

    #[test]
    fn serializes_choice_and_score_questions() {
        let request = SystemOneRequest::new("m", "s")
            .question(
                "tone",
                Question::choice(
                    "What is the tone?",
                    [
                        ("angry", Some(Content::text("An upset message"))),
                        ("calm", None),
                    ],
                ),
            )
            .question(
                "urgency",
                Question::score(
                    "How urgent?",
                    vec![Content::text("Can wait"), Content::text("Today")],
                ),
            );
        let value = serde_json::to_value(&request).unwrap();

        assert_eq!(value["questions"]["tone"]["type"], json!("choice"));
        assert_eq!(
            value["questions"]["tone"]["criteria"]["angry"],
            json!("An upset message")
        );
        assert_eq!(value["questions"]["tone"]["criteria"]["calm"], Value::Null);
        assert_eq!(
            value["questions"]["urgency"]["criteria"],
            json!(["Can wait", "Today"])
        );
    }

    #[test]
    fn parses_mixed_answers() {
        let response: SystemOneResponse = serde_json::from_value(json!({
            "model": "jev-latest",
            "answers": {
                "billing": {"type": "noul", "noul": 0.98},
                "tone": {
                    "type": "choice",
                    "choice": "calm",
                    "confidence": 0.7,
                    "probabilities": {"angry": 0.15, "calm": 0.8, "excited": 0.05}
                },
                "urgency": {
                    "type": "score",
                    "score": 1.7,
                    "confidence": 0.55,
                    "legend": {"0": "Can wait", "1": "This week", "2": "Today"},
                    "probabilities": {"0": 0.1, "1": 0.1, "2": 0.8}
                }
            },
            "usage": {"input_tokens": 120, "output_tokens": 12}
        }))
        .unwrap();

        assert_eq!(response.noul("billing"), Some(0.98));
        assert_eq!(response.choice("tone"), Some("calm"));
        assert_eq!(response.score("urgency"), Some(1.7));
        assert_eq!(response.confidence("tone"), Some(0.7));
        assert_eq!(response.usage.input_tokens, 120);
        assert_eq!(response.usage.output_tokens, 12);
        assert_eq!(
            response.answer("tone").unwrap().probabilities().unwrap()["calm"],
            0.8
        );
    }

    #[test]
    fn parses_box_answer_with_null_confidence() {
        let answer: Answer = serde_json::from_value(json!({
            "type": "box",
            "box": {"top": 0.2, "bottom": 0.66, "left": 0.19, "right": 0.68},
            "confidence": null
        }))
        .unwrap();

        assert_eq!(answer.confidence(), None);
        assert_eq!(answer.box_coordinates().unwrap().top, 0.2);
        assert_eq!(answer.box_coordinates().unwrap().right, 0.68);
    }

    #[test]
    fn has_images_detects_assets_and_box_questions() {
        assert!(!SystemOneRequest::new("m", "s").has_images());

        let with_box = SystemOneRequest::new("m", "s").question("b", Question::boxed("Locate it."));
        assert!(with_box.has_images());

        let with_asset = SystemOneRequest::new("m", "s")
            .asset("photo", Asset::from_bytes(ImageFormat::Png, &[1, 2, 3]));
        assert!(with_asset.has_images());
    }
}
