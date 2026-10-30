use serde::Serialize;
use serde_json::{Value, json};

use crate::{Reasoning, Tool};

#[derive(Debug, Serialize)]
pub struct Request {
    pub(super) model: String,
    pub(super) instructions: String,
    pub(super) input: Vec<Value>,
    pub(super) tools: Vec<Tool>,
    pub(super) tool_choice: &'static str,
    pub(super) parallel_tool_calls: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) reasoning: Option<ReasoningOptions>,
    pub(super) text: TextOptions,
    pub(super) include: [&'static str; 1],
    pub(super) stream: bool,
    pub(super) store: bool,
    pub(super) prompt_cache_key: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ReasoningOptions {
    effort: &'static str,
    summary: &'static str,
}

#[derive(Debug, Serialize)]
pub(super) struct TextOptions {
    verbosity: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<Value>,
}

impl Request {
    pub fn new(
        model: String,
        instructions: String,
        input: Vec<Value>,
        tools: Vec<Tool>,
        reasoning: Option<Reasoning>,
        session: String,
    ) -> Self {
        Self {
            model,
            instructions,
            input,
            tool_choice: if tools.is_empty() { "none" } else { "auto" },
            tools,
            parallel_tool_calls: false,
            reasoning: reasoning.map(|reasoning| ReasoningOptions {
                effort: reasoning.as_str(),
                summary: "auto",
            }),
            text: TextOptions {
                verbosity: "low",
                format: None,
            },
            include: ["reasoning.encrypted_content"],
            stream: true,
            store: false,
            prompt_cache_key: session,
        }
    }

    pub fn with_output_schema(mut self, schema: Value) -> Self {
        self.text.format = Some(json!({
            "type": "json_schema",
            "name": "response",
            "strict": true,
            "schema": schema,
        }));
        self
    }
}

pub fn message(role: &str, text: impl Into<String>) -> Value {
    json!({
        "type": "message",
        "role": role,
        "content": [{
            "type": "input_text",
            "text": text.into(),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_without_tools_disable_tool_calls() {
        let request = Request::new(
            "codex".to_owned(),
            "Translate manga.".to_owned(),
            vec![message("user", "hello")],
            Vec::new(),
            Some(Reasoning::Low),
            "translation".to_owned(),
        );
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["tool_choice"], "none");
        assert_eq!(value["tools"], json!([]));
        assert_eq!(value["stream"], true);
        assert_eq!(value["store"], false);
        assert!(value["text"].get("format").is_none());
    }
}
