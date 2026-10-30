use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum Reasoning {
    None,
    Minimal,
    Low,
    #[default]
    Medium,
    High,
    Xhigh,
    Max,
    Ultra,
}

impl Reasoning {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
            Self::Ultra => "ultra",
        }
    }
}
