use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::jev::DEFAULT_MODEL;
use crate::Result;

/// Which model answers. Fast = live analysis while typing; Deep = project brief.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Fast,
    Deep,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProviderSettings {
    /// Your Claude subscription, through the installed Claude Code CLI.
    #[serde(rename_all = "camelCase")]
    ClaudeCli { binary_path: Option<String>, fast_model: String, deep_model: String },
    /// Anthropic API key (stored in the OS keychain, never in this file).
    #[serde(rename_all = "camelCase")]
    AnthropicApi { fast_model: String, deep_model: String },
    /// A downloaded GGUF model served by the managed llama.cpp runtime.
    #[serde(rename_all = "camelCase")]
    Local { model_id: String },
}

impl ProviderSettings {
    pub fn label(&self) -> String {
        match self {
            Self::ClaudeCli { fast_model, .. } => format!("Claude Code ({fast_model})"),
            Self::AnthropicApi { fast_model, .. } => format!("Anthropic API ({fast_model})"),
            Self::Local { model_id } => format!("Local ({model_id})"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub provider: ProviderSettings,
    /// Idle time after typing before the notes re-think.
    pub debounce_ms: u64,
    pub jev: JevSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: ProviderSettings::ClaudeCli {
                binary_path: None,
                // Claude Code accepts aliases, which track the latest models.
                fast_model: "haiku".into(),
                deep_model: "sonnet".into(),
            },
            debounce_ms: 1200,
            jev: JevSettings::default(),
        }
    }
}

/// Which Jev judgments run. They only run when a TypeSafe key is found.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct JevSettings {
    /// Skip re-analysis when an edit doesn't change the idea.
    pub gate: bool,
    /// Rerank the keyword shortlist of code snippets.
    pub rerank: bool,
    /// Fact-check the Faults against the code.
    pub check: bool,
    /// Pin a versioned ID (e.g. `jev-1.13.0`) once thresholds are tuned.
    pub model: String,
}

impl Default for JevSettings {
    fn default() -> Self {
        Self { gate: true, rerank: true, check: true, model: DEFAULT_MODEL.into() }
    }
}

impl Settings {
    /// Whether the Faults may be fact-checked by Jev. That sends claims and
    /// code excerpts to TypeSafe, so the Local option ("private and offline")
    /// never does, whatever the switch says.
    pub fn jev_check_enabled(&self) -> bool {
        self.jev.check && !matches!(self.provider, ProviderSettings::Local { .. })
    }

    /// Missing or corrupt files fall back to defaults instead of blocking startup.
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Atomic write: never leaves a half-written settings file behind.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default());

        let s = Settings { provider: ProviderSettings::Local { model_id: "qwen3-4b".into() }, debounce_ms: 900, ..Default::default() };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);

        std::fs::write(&path, "{nope").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
    }

    #[test]
    fn files_from_before_jev_get_its_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"provider":{"kind":"local","modelId":"qwen3-4b"},"debounceMs":900}"#).unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.debounce_ms, 900);
        assert_eq!(s.jev, JevSettings::default());
        assert!(s.jev.check && s.jev.model == "jev-latest");
    }

    #[test]
    fn local_models_never_send_code_to_jev() {
        let mut s = Settings::default();
        assert!(s.jev_check_enabled());
        s.provider = ProviderSettings::Local { model_id: "qwen3-4b-instruct".into() };
        assert!(!s.jev_check_enabled(), "the Local option promises nothing leaves this computer");
        s.provider = Settings::default().provider;
        s.jev.check = false;
        assert!(!s.jev_check_enabled());
    }
}
