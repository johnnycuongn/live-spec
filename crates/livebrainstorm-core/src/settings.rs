use std::path::Path;

use serde::{Deserialize, Serialize};

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
        }
    }
}

impl Settings {
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

        let s = Settings { provider: ProviderSettings::Local { model_id: "qwen3-4b".into() }, debounce_ms: 900 };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);

        std::fs::write(&path, "{nope").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
    }
}
