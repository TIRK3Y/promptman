//! Provider registry and model references ("provider/model").
use serde::{Deserialize, Serialize};
use std::fmt;

/// Wire protocol spoken by a provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// OpenAI chat completions (also Groq, Mistral, LM Studio, vLLM, llama.cpp).
    OpenAi,
    Anthropic,
    Gemini,
    Ollama,
}

pub struct ProviderInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub short: &'static str,
    pub protocol: Protocol,
    pub default_base_url: &'static str,
    pub needs_key: bool,
    /// Runs on this machine: free, and hardware stats are sampled.
    pub local: bool,
}

pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        id: "openai",
        short: "OpenAI",
        name: "OpenAI",
        protocol: Protocol::OpenAi,
        default_base_url: "https://api.openai.com/v1",
        needs_key: true,
        local: false,
    },
    ProviderInfo {
        id: "anthropic",
        short: "Anthropic",
        name: "Anthropic",
        protocol: Protocol::Anthropic,
        default_base_url: "https://api.anthropic.com/v1",
        needs_key: true,
        local: false,
    },
    ProviderInfo {
        id: "gemini",
        short: "Gemini",
        name: "Google Gemini",
        protocol: Protocol::Gemini,
        default_base_url: "https://generativelanguage.googleapis.com/v1beta",
        needs_key: true,
        local: false,
    },
    ProviderInfo {
        id: "groq",
        short: "Groq",
        name: "Groq",
        protocol: Protocol::OpenAi,
        default_base_url: "https://api.groq.com/openai/v1",
        needs_key: true,
        local: false,
    },
    ProviderInfo {
        id: "mistral",
        short: "Mistral",
        name: "Mistral",
        protocol: Protocol::OpenAi,
        default_base_url: "https://api.mistral.ai/v1",
        needs_key: true,
        local: false,
    },
    ProviderInfo {
        id: "custom",
        short: "Custom",
        name: "OpenAI-compatible (LM Studio, vLLM, llama.cpp)",
        protocol: Protocol::OpenAi,
        default_base_url: "http://localhost:1234/v1",
        needs_key: false,
        local: true,
    },
    ProviderInfo {
        id: "ollama",
        short: "Ollama",
        name: "Ollama",
        protocol: Protocol::Ollama,
        default_base_url: "http://localhost:11434",
        needs_key: false,
        local: true,
    },
];

pub fn provider(id: &str) -> Option<&'static ProviderInfo> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// A model on a provider, written as `provider/model` (e.g. `ollama/llama3:8b`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
}

impl ModelRef {
    pub fn new(provider: &str, model: &str) -> Self {
        Self { provider: provider.to_string(), model: model.to_string() }
    }

    /// Parses `provider/model`; the model part may itself contain `/`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let (p, m) = s.split_once('/').ok_or_else(|| format!("`{s}` is not in provider/model form"))?;
        if provider(p).is_none() {
            let ids: Vec<_> = PROVIDERS.iter().map(|p| p.id).collect();
            return Err(format!("unknown provider `{p}` (expected one of {})", ids.join(", ")));
        }
        if m.trim().is_empty() {
            return Err(format!("`{s}` has an empty model name"));
        }
        Ok(Self::new(p, m.trim()))
    }

    pub fn info(&self) -> &'static ProviderInfo {
        provider(&self.provider).unwrap_or(&PROVIDERS[0])
    }

    pub fn is_local(&self) -> bool {
        self.info().local
    }
}

impl fmt::Display for ModelRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.provider, self.model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_table_is_consistent() {
        let mut ids: Vec<_> = PROVIDERS.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PROVIDERS.len(), "provider ids are unique");
        for p in PROVIDERS {
            if p.local {
                assert!(p.default_base_url.starts_with("http://localhost"), "{}", p.id);
            } else {
                // Cloud keys must never travel over plain http by default.
                assert!(p.default_base_url.starts_with("https://"), "{}", p.id);
                assert!(p.needs_key, "{} is cloud but needs no key", p.id);
            }
            assert_eq!(ModelRef::new(p.id, "x").info().id, p.id);
        }
    }

    #[test]
    fn parse_model_refs() {
        assert_eq!(ModelRef::parse("openai/gpt-4o").unwrap(), ModelRef::new("openai", "gpt-4o"));
        assert_eq!(ModelRef::parse("custom/meta-llama/Llama-3-8B").unwrap().model, "meta-llama/Llama-3-8B");
        assert!(ModelRef::parse("gpt-4o").is_err());
        assert!(ModelRef::parse("nope/x").is_err());
        assert!(ModelRef::parse("openai/ ").is_err());
        assert!(ModelRef::parse("ollama/llama3").unwrap().is_local());
    }
}
