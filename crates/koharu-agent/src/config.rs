use anyhow::Result;
use koharu_codex::Reasoning;
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct Config {
    pub model: Option<String>,
    pub reasoning: Reasoning,
}

impl Config {
    pub fn load() -> Result<koharu_config::Config<Self>> {
        koharu_config::load("agent")
    }
}
