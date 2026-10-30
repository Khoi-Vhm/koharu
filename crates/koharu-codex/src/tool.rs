use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize)]
pub struct Tool {
    #[serde(rename = "type")]
    kind: &'static str,
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub strict: bool,
}

impl Tool {
    #[must_use]
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: Value) -> Self {
        Self {
            kind: "function",
            name: name.into(),
            description: description.into(),
            parameters,
            strict: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}
