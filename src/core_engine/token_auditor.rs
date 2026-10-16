//! Token counting and pricing.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tiktoken_rs::{cl100k_base_singleton, o200k_base_singleton};

use super::providers::ModelRef;

#[derive(Debug, Clone, Default)]
pub struct TokenMetrics {
    pub char_count: usize,
    pub word_count: usize,
    pub token_count: usize,
    pub bpe_encoding: &'static str,
}

/// Counts tokens offline. OpenAI models use their own encoding; for other
/// vendors this is an estimate (their tokenizers aren't public).
pub fn count_tokens(text: &str, model: &str) -> (usize, &'static str) {
    let m = model.to_lowercase();
    let modern_openai = ["gpt-4o", "gpt-4.1", "gpt-5", "o1", "o3", "o4"].iter().any(|p| m.starts_with(p));
    if modern_openai {
        (o200k_base_singleton().encode_with_special_tokens(text).len(), "o200k_base")
    } else {
        (cl100k_base_singleton().encode_with_special_tokens(text).len(), "cl100k_base")
    }
}

pub fn audit_prompt(text: &str, model: &str) -> TokenMetrics {
    let (token_count, bpe_encoding) = count_tokens(text, model);
    TokenMetrics {
        char_count: text.chars().count(),
        word_count: text.split_whitespace().count(),
        token_count,
        bpe_encoding,
    }
}

/// Price per million tokens for one model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PriceEntry {
    pub provider: String,
    pub model: String,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

/// Prices loaded from `pricing.json`, which users can edit.
pub struct Pricing {
    pub path: PathBuf,
    pub entries: Vec<PriceEntry>,
}

impl Pricing {
    /// Loads the pricing file, writing the defaults first if it doesn't exist.
    pub fn load(data_dir: &Path) -> Result<Self, String> {
        let path = data_dir.join("pricing.json");
        if !path.exists() {
            let json = serde_json::to_string_pretty(&default_prices()).map_err(|e| e.to_string())?;
            std::fs::write(&path, json).map_err(|e| e.to_string())?;
        }
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let entries =
            serde_json::from_str(&text).map_err(|e| format!("{} is not valid pricing JSON: {e}", path.display()))?;
        Ok(Self { path, entries })
    }

    /// Like [`Pricing::load`], but a broken file falls back to the built-in prices
    /// (the file itself is left untouched) with a message explaining why.
    pub fn load_or_default(data_dir: &Path) -> (Self, Option<String>) {
        match Self::load(data_dir) {
            Ok(p) => (p, None),
            Err(e) => (
                Self { path: data_dir.join("pricing.json"), entries: default_prices() },
                Some(format!(
                    "Could not read prices ({e}). Using built-in prices until the file is fixed and reloaded in Settings."
                )),
            ),
        }
    }

    pub fn find(&self, m: &ModelRef) -> Option<&PriceEntry> {
        self.entries.iter().find(|e| e.provider == m.provider && e.model == m.model)
    }

    /// Known models for a provider, for model pickers.
    pub fn models_for(&self, provider: &str) -> Vec<String> {
        self.entries.iter().filter(|e| e.provider == provider).map(|e| e.model.clone()).collect()
    }

    /// Cost in USD; `None` when a cloud model has no price entry.
    pub fn cost(&self, m: &ModelRef, prompt_tokens: usize, completion_tokens: usize) -> Option<f64> {
        if m.is_local() {
            return Some(0.0);
        }
        self.find(m).map(|p| {
            (prompt_tokens as f64 * p.input_per_mtok + completion_tokens as f64 * p.output_per_mtok) / 1_000_000.0
        })
    }
}

/// Starting prices in USD per million tokens. Providers change these; edit
/// `pricing.json` to keep them current.
pub fn default_prices() -> Vec<PriceEntry> {
    let p = |provider: &str, model: &str, input: f64, output: f64| PriceEntry {
        provider: provider.into(),
        model: model.into(),
        input_per_mtok: input,
        output_per_mtok: output,
    };
    vec![
        p("openai", "gpt-4o", 2.50, 10.00),
        p("openai", "gpt-4o-mini", 0.15, 0.60),
        p("openai", "gpt-4.1", 2.00, 8.00),
        p("openai", "gpt-4.1-mini", 0.40, 1.60),
        p("anthropic", "claude-opus-5-5", 4.00, 20.00),
        p("anthropic", "claude-sonnet-5-5", 2.00, 10.00),
        p("anthropic", "claude-haiku-5-5", 0.10, 0.50),
        p("anthropic", "claude-fable-5-1", 10.00, 50.00),
        p("gemini", "gemini-2.5-pro", 1.25, 10.00),
        p("gemini", "gemini-2.5-flash", 0.30, 2.50),
        p("groq", "llama-3.3-70b-versatile", 0.59, 0.79),
        p("groq", "llama-3.1-8b-instant", 0.05, 0.08),
        p("mistral", "mistral-large-latest", 2.00, 6.00),
        p("mistral", "mistral-small-latest", 0.20, 0.60),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_tokens() {
        let m = audit_prompt("Hello world, how are you?", "gpt-4o");
        assert_eq!(m.word_count, 5);
        assert_eq!(m.bpe_encoding, "o200k_base");
        assert!(m.token_count > 0 && m.token_count < 10);
        assert_eq!(audit_prompt("", "claude-opus-5-5").token_count, 0);
    }

    #[test]
    fn costs() {
        let pricing = Pricing { path: PathBuf::new(), entries: default_prices() };
        let gpt = ModelRef::new("openai", "gpt-4o");
        let cost = pricing.cost(&gpt, 1_000_000, 1_000_000).unwrap();
        assert!((cost - 12.5).abs() < 1e-9);
        assert_eq!(pricing.cost(&ModelRef::new("ollama", "llama3"), 500, 500), Some(0.0));
        assert_eq!(pricing.cost(&ModelRef::new("openai", "unknown-model"), 1, 1), None);
        assert!(pricing.models_for("anthropic").contains(&"claude-opus-5-5".to_string()));
    }

    #[test]
    fn broken_pricing_file_falls_back() {
        let dir = std::env::temp_dir().join(format!("promptman-badprice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pricing.json"), "{ not json").unwrap();
        let (pricing, note) = Pricing::load_or_default(&dir);
        assert_eq!(pricing.entries, default_prices());
        assert!(note.unwrap().starts_with("Could not read prices"));
        assert_eq!(std::fs::read_to_string(dir.join("pricing.json")).unwrap(), "{ not json");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_cloud_provider_has_priced_models() {
        let pricing = Pricing { path: PathBuf::new(), entries: default_prices() };
        for p in crate::core_engine::providers::PROVIDERS.iter().filter(|p| !p.local) {
            assert!(!pricing.models_for(p.id).is_empty(), "{} has no models in the default price list", p.id);
        }
        for e in &pricing.entries {
            assert!(e.input_per_mtok >= 0.0 && e.output_per_mtok >= e.input_per_mtok, "{}", e.model);
        }
    }

    #[test]
    fn counts_unicode_and_long_text() {
        assert!(count_tokens("日本語のテキスト ✓ émoji 🚀", "claude-opus-5-5").0 > 0);
        let long = "word ".repeat(10_000);
        let (n, enc) = count_tokens(&long, "gpt-4.1-mini");
        assert_eq!(enc, "o200k_base");
        assert!(n >= 10_000);
    }

    #[test]
    fn pricing_file_roundtrip() {
        let dir = std::env::temp_dir().join(format!("promptman-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let loaded = Pricing::load(&dir).unwrap();
        assert_eq!(loaded.entries, default_prices());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
