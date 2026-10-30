use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use specta::Type;
use strum::{Display, EnumIter, EnumString, IntoStaticStr, VariantArray};

use crate::{
    local::LocalConfig,
    remote::{
        CaiyunConfig, ClaudeConfig, CodexConfig, DeepLConfig, DeepSeekConfig, GeminiConfig,
        GoogleCloudConfig, GrokConfig, LmStudioConfig, MiniMaxConfig, OpenAiCompatibleConfig,
        OpenAiConfig, OpenRouterConfig,
    },
};

macro_rules! define_providers {
    ($(
        $variant:ident {
            id: $id:literal,
            name: $name:literal,
            field: $field:ident,
            config: $config:ty,
            api_key: $api_key:literal,
        }
    )+) => {
        #[derive(
            Clone,
            Copy,
            Debug,
            Display,
            EnumIter,
            EnumString,
            Eq,
            Hash,
            IntoStaticStr,
            PartialEq,
            Serialize,
            Deserialize,
            Type,
            VariantArray,
        )]
        pub enum Provider {
            $(
                #[serde(rename = $id)]
                #[strum(serialize = $id)]
                $variant,
            )+
        }

        impl Provider {
            #[must_use]
            pub const fn accepts_api_key(self) -> bool {
                match self {
                    $(Self::$variant => $api_key,)+
                }
            }

            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)+
                }
            }
        }

        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
        #[serde(tag = "provider", content = "settings")]
        pub enum ProviderConfig {
            $(
                #[serde(rename = $id)]
                $variant($config),
            )+
        }

        impl ProviderConfig {
            #[must_use]
            pub const fn provider(&self) -> Provider {
                match self {
                    $(Self::$variant(_) => Provider::$variant,)+
                }
            }
        }

        #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
        #[serde(default)]
        pub struct ProvidersConfig {
            $(
                #[serde(rename = $id)]
                pub $field: $config,
            )+
        }

        impl ProvidersConfig {
            #[must_use]
            pub fn entries(&self) -> Vec<ProviderConfig> {
                vec![$(ProviderConfig::$variant(self.$field.clone()),)+]
            }

            pub fn from_entries(entries: impl IntoIterator<Item = ProviderConfig>) -> Result<Self> {
                let mut config = Self::default();
                let mut seen = std::collections::HashSet::new();
                for entry in entries {
                    let provider = entry.provider();
                    if !seen.insert(provider) {
                        bail!("duplicate provider configuration for {provider}");
                    }
                    match entry {
                        $(ProviderConfig::$variant(settings) => config.$field = settings,)+
                    }
                }
                if seen.len() != Provider::VARIANTS.len() {
                    let missing = Provider::VARIANTS
                        .iter()
                        .filter(|provider| !seen.contains(provider))
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    bail!("missing provider configurations: {missing}");
                }
                Ok(config)
            }
        }
    };
}

define_providers! {
    Local {
        id: "local",
        name: "Local",
        field: local,
        config: LocalConfig,
        api_key: false,
    }
    Codex {
        id: "codex",
        name: "Codex",
        field: codex,
        config: CodexConfig,
        api_key: false,
    }
    OpenAi {
        id: "openai",
        name: "OpenAI",
        field: openai,
        config: OpenAiConfig,
        api_key: true,
    }
    Gemini {
        id: "gemini",
        name: "Gemini",
        field: gemini,
        config: GeminiConfig,
        api_key: true,
    }
    Claude {
        id: "claude",
        name: "Claude",
        field: claude,
        config: ClaudeConfig,
        api_key: true,
    }
    Grok {
        id: "grok",
        name: "Grok",
        field: grok,
        config: GrokConfig,
        api_key: true,
    }
    MiniMax {
        id: "minimax",
        name: "MiniMax",
        field: minimax,
        config: MiniMaxConfig,
        api_key: true,
    }
    DeepSeek {
        id: "deepseek",
        name: "DeepSeek",
        field: deepseek,
        config: DeepSeekConfig,
        api_key: true,
    }
    OpenAiCompatible {
        id: "openai-compatible",
        name: "OpenAI-compatible",
        field: openai_compatible,
        config: OpenAiCompatibleConfig,
        api_key: true,
    }
    OpenRouter {
        id: "openrouter",
        name: "OpenRouter",
        field: openrouter,
        config: OpenRouterConfig,
        api_key: true,
    }
    LmStudio {
        id: "lm-studio",
        name: "LM Studio",
        field: lm_studio,
        config: LmStudioConfig,
        api_key: true,
    }
    DeepL {
        id: "deepl",
        name: "DeepL",
        field: deepl,
        config: DeepLConfig,
        api_key: true,
    }
    GoogleCloudTranslation {
        id: "google-cloud-translation",
        name: "Google Cloud Translation",
        field: google_cloud_translation,
        config: GoogleCloudConfig,
        api_key: true,
    }
    Caiyun {
        id: "caiyun",
        name: "Caiyun",
        field: caiyun,
        config: CaiyunConfig,
        api_key: true,
    }
}

impl Provider {
    pub(crate) const fn supports_dialogue_analysis(self) -> bool {
        match self {
            Self::DeepL | Self::GoogleCloudTranslation | Self::Caiyun => false,
            Self::Local
            | Self::Codex
            | Self::OpenAi
            | Self::Gemini
            | Self::Claude
            | Self::Grok
            | Self::MiniMax
            | Self::DeepSeek
            | Self::OpenAiCompatible
            | Self::OpenRouter
            | Self::LmStudio => true,
        }
    }
}

impl ProvidersConfig {
    pub fn load() -> anyhow::Result<koharu_config::Config<Self>> {
        koharu_config::load("providers")
    }
}
