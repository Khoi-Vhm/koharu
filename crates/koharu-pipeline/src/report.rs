use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

use crate::Stage;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunStatus {
    Completed,
    Stopped,
}

#[derive(Debug)]
pub struct Report {
    pub status: RunStatus,
    pub base: koharu_scene::Revision,
    pub final_revision: koharu_scene::Revision,
    pub completed: usize,
    pub total: usize,
    pub elapsed: Duration,
}

#[derive(Debug)]
pub struct StageOutput {
    pub page: koharu_scene::EntityId,
    pub stage: Stage,
    pub patch: koharu_scene::Patch,
    pub(crate) translation_input: Option<crate::stages::TranslationInput>,
}

impl StageOutput {
    /// Validate context against the current snapshot while holding the commit lock.
    pub fn validate_on(&self, current: &koharu_scene::Snapshot) -> Result<()> {
        anyhow::ensure!(
            self.stage != Stage::Translation || self.translation_input.is_some(),
            "translation input snapshot is missing"
        );
        if let Some(input) = &self.translation_input {
            input.validate_on(current)?;
        }
        Ok(())
    }
}

#[async_trait]
pub trait Committer: Send {
    /// Call `output.validate_on` against the latest snapshot before committing.
    async fn commit(&mut self, output: StageOutput) -> Result<koharu_scene::Snapshot>;
}
