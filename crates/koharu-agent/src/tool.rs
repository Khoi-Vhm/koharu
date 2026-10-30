use anyhow::Result;
use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use koharu_codex::{Control, Tool, ToolCall};

#[derive(Clone, Debug)]
pub struct ToolImage {
    pub label: String,
    pub data_url: String,
}

#[derive(Clone, Debug)]
pub struct Invocation {
    pub value: Value,
    pub changed: bool,
    pub images: Vec<ToolImage>,
}

impl Invocation {
    pub fn read(value: impl Serialize) -> Result<Self> {
        Ok(Self {
            value: serde_json::to_value(value)?,
            changed: false,
            images: Vec::new(),
        })
    }

    pub fn changed(value: impl Serialize) -> Result<Self> {
        Ok(Self {
            value: serde_json::to_value(value)?,
            changed: true,
            images: Vec::new(),
        })
    }

    #[must_use]
    pub fn with_image(mut self, label: impl Into<String>, data_url: impl Into<String>) -> Self {
        self.images.push(ToolImage {
            label: label.into(),
            data_url: data_url.into(),
        });
        self
    }
}

#[async_trait]
pub trait Host: Send + Sync + 'static {
    async fn context(&self) -> Result<Value>;

    fn tools(&self) -> Vec<Tool>;

    async fn invoke(&self, call: ToolCall, control: &Control) -> Result<Invocation>;
}
