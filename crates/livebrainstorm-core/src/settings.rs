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

/// How hard the model thinks: `claude --effort`, or `output_config.effort` in
/// the Messages API. `None` in settings means the model's own default.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

/// How the Claude Code CLI authenticates.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ClaudeAuth {
    /// Claude Code's own login (the subscription).
    #[default]
    Login,
    /// The Anthropic API key from the keychain, given to `claude` as ANTHROPIC_API_KEY.
    ApiKey,
}

/// Reads a field that must never reset the whole file: `Settings::load` falls
/// back to defaults on any error, so an unknown value (a typo, or a level a
/// newer version wrote) becomes the field's default instead.
fn lenient<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProviderSettings {
    /// Your installed Claude Code CLI, with its login or your API key.
    #[serde(rename_all = "camelCase")]
    ClaudeCli {
        binary_path: Option<String>,
        fast_model: String,
        deep_model: String,
        #[serde(default, deserialize_with = "lenient")]
        fast_effort: Option<Effort>,
        #[serde(default, deserialize_with = "lenient")]
        deep_effort: Option<Effort>,
        #[serde(default, deserialize_with = "lenient")]
        auth: ClaudeAuth,
    },
    /// Anthropic API key (stored in the OS keychain, never in this file).
    #[serde(rename_all = "camelCase")]
    AnthropicApi {
        fast_model: String,
        deep_model: String,
        #[serde(default, deserialize_with = "lenient")]
        fast_effort: Option<Effort>,
        #[serde(default, deserialize_with = "lenient")]
        deep_effort: Option<Effort>,
    },
    /// A downloaded GGUF model served by the managed llama.cpp runtime.
    #[serde(rename_all = "camelCase")]
    Local { model_id: String },
}

impl ProviderSettings {
    pub fn label(&self) -> String {
        match self {
            Self::ClaudeCli { fast_model, auth: ClaudeAuth::ApiKey, .. } => format!("Claude Code (API key, {fast_model})"),
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
    pub agents: AgentSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: ProviderSettings::ClaudeCli {
                binary_path: None,
                // Claude Code accepts aliases, which track the latest models.
                fast_model: "haiku".into(),
                deep_model: "sonnet".into(),
                fast_effort: None,
                deep_effort: None,
                auth: ClaudeAuth::Login,
            },
            debounce_ms: 1200,
            jev: JevSettings::default(),
            agents: AgentSettings::default(),
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

/// How many agents work at once. Notes use at most one per section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentSettings {
    /// The cap on agents running at once; clamped to 1..=8.
    pub max_agents: u8,
    /// Map the project with parallel explorers (opt-in; measured slower).
    pub parallel_brief: bool,
    /// Write the four note sections with parallel agents.
    pub parallel_notes: bool,
}

impl Default for AgentSettings {
    fn default() -> Self {
        // Parallel notes and briefs are opt-in: neither measured faster
        // than one agent with Claude Code (see the spec's measurements).
        Self { max_agents: 4, parallel_brief: false, parallel_notes: false }
    }
}

impl Settings {
    /// Jev sends drafts and code excerpts to TypeSafe, so the Local option
    /// ("private and offline") never uses it, whatever the switches say.
    pub fn jev_allowed(&self) -> bool {
        !self.is_local()
    }

    fn is_local(&self) -> bool {
        matches!(self.provider, ProviderSettings::Local { .. })
    }

    /// The user's cap on agents running at once.
    pub fn max_agents(&self) -> usize {
        usize::from(self.agents.max_agents.clamp(1, 8))
    }

    /// How many explorers map the project for the brief. The Local model gets
    /// one, because a single llama-server would queue the rest.
    pub fn effective_brief_agents(&self) -> usize {
        if self.agents.parallel_brief && !self.is_local() {
            self.max_agents()
        } else {
            1
        }
    }

    /// How many agents write the notes: up to one per section. The Local
    /// model gets one, because a single llama-server would queue the rest.
    pub fn effective_note_agents(&self) -> usize {
        if self.agents.parallel_notes && !self.is_local() {
            self.max_agents().min(4)
        } else {
            1
        }
    }

    pub fn jev_check_enabled(&self) -> bool {
        self.jev.check && self.jev_allowed()
    }

    pub fn jev_gate_enabled(&self) -> bool {
        self.jev.gate && self.jev_allowed()
    }

    pub fn jev_rerank_enabled(&self) -> bool {
        self.jev.rerank && self.jev_allowed()
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
        assert!(s.jev_allowed() && s.jev_check_enabled() && s.jev_gate_enabled() && s.jev_rerank_enabled());
        s.provider = ProviderSettings::Local { model_id: "qwen3-4b-instruct".into() };
        assert!(!s.jev_allowed(), "the Local option promises nothing leaves this computer");
        assert!(!s.jev_check_enabled() && !s.jev_gate_enabled() && !s.jev_rerank_enabled());
        s.provider = Settings::default().provider;
        s.jev = JevSettings { gate: false, rerank: false, check: false, ..JevSettings::default() };
        assert!(s.jev_allowed());
        assert!(!s.jev_check_enabled() && !s.jev_gate_enabled() && !s.jev_rerank_enabled());
    }

    #[test]
    fn note_agents_follow_the_settings() {
        let mut s = Settings::default();
        assert_eq!((s.max_agents(), s.effective_note_agents()), (4, 1), "parallel notes are opt-in: measured slower with Claude Code");
        s.agents.parallel_notes = true;
        assert_eq!(s.effective_note_agents(), 4);
        s.agents.max_agents = 2;
        assert_eq!(s.effective_note_agents(), 2);
        s.agents.max_agents = 8;
        assert_eq!((s.max_agents(), s.effective_note_agents()), (8, 4), "at most one agent per section");
        s.agents.max_agents = 0;
        assert_eq!((s.max_agents(), s.effective_note_agents()), (1, 1), "clamped to at least one");
        s.agents.max_agents = 200;
        assert_eq!(s.max_agents(), 8, "clamped to at most eight");
        s.agents = AgentSettings { parallel_notes: false, ..AgentSettings::default() };
        assert_eq!(s.effective_note_agents(), 1);
        s.agents = AgentSettings { parallel_notes: true, ..AgentSettings::default() };
        s.provider = ProviderSettings::Local { model_id: "qwen3-4b-instruct".into() };
        assert_eq!(s.effective_note_agents(), 1, "one local server would just queue the requests");
    }

    #[test]
    fn files_from_before_agents_get_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"provider":{"kind":"claudeCli","binaryPath":null,"fastModel":"haiku","deepModel":"sonnet"},"debounceMs":900}"#).unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.agents, AgentSettings { max_agents: 4, parallel_brief: false, parallel_notes: false });
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["agents"], serde_json::json!({"maxAgents": 4, "parallelBrief": false, "parallelNotes": false}));
    }

    #[test]
    fn brief_agents_follow_the_settings() {
        let mut s = Settings::default();
        assert_eq!(s.effective_brief_agents(), 1, "parallel briefs are opt-in");
        s.agents.parallel_brief = true;
        assert_eq!(s.effective_brief_agents(), 4);
        s.agents.max_agents = 6;
        assert_eq!(s.effective_brief_agents(), 6, "explorers are not capped at four");
        s.agents.max_agents = 1;
        assert_eq!(s.effective_brief_agents(), 1);
        s.agents = AgentSettings { parallel_brief: false, ..AgentSettings::default() };
        assert_eq!(s.effective_brief_agents(), 1);
        s.agents = AgentSettings { parallel_brief: true, ..AgentSettings::default() };
        s.provider = ProviderSettings::Local { model_id: "qwen3-4b-instruct".into() };
        assert_eq!(s.effective_brief_agents(), 1);
    }

    #[test]
    fn files_from_before_effort_get_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"provider":{"kind":"claudeCli","binaryPath":null,"fastModel":"haiku","deepModel":"sonnet"},"debounceMs":900}"#).unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.debounce_ms, 900);
        assert_eq!(
            s.provider,
            ProviderSettings::ClaudeCli {
                binary_path: None,
                fast_model: "haiku".into(),
                deep_model: "sonnet".into(),
                fast_effort: None,
                deep_effort: None,
                auth: ClaudeAuth::Login,
            }
        );
        std::fs::write(&path, r#"{"provider":{"kind":"anthropicApi","fastModel":"a","deepModel":"b"}}"#).unwrap();
        assert_eq!(
            Settings::load(&path).provider,
            ProviderSettings::AnthropicApi { fast_model: "a".into(), deep_model: "b".into(), fast_effort: None, deep_effort: None }
        );
    }

    #[test]
    fn effort_and_auth_serialize_as_the_ui_expects() {
        let p = ProviderSettings::ClaudeCli {
            binary_path: None,
            fast_model: "haiku".into(),
            deep_model: "opus".into(),
            fast_effort: Some(Effort::Low),
            deep_effort: Some(Effort::Xhigh),
            auth: ClaudeAuth::ApiKey,
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "claudeCli", "binaryPath": null, "fastModel": "haiku", "deepModel": "opus",
                               "fastEffort": "low", "deepEffort": "xhigh", "auth": "apiKey"})
        );
        assert_eq!(serde_json::from_value::<ProviderSettings>(json).unwrap(), p);
        for e in [Effort::Low, Effort::Medium, Effort::High, Effort::Xhigh, Effort::Max] {
            assert_eq!(serde_json::to_value(e).unwrap(), e.as_str(), "the CLI flag and the JSON agree");
        }
    }

    #[test]
    fn an_unknown_effort_or_auth_never_resets_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"provider":{"kind":"claudeCli","binaryPath":null,"fastModel":"opus","deepModel":"sonnet",
                "fastEffort":"extreme","deepEffort":"max","auth":"sso"},"debounceMs":900}"#,
        )
        .unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.debounce_ms, 900, "the rest of the file still loads");
        let ProviderSettings::ClaudeCli { fast_model, fast_effort, deep_effort, auth, .. } = s.provider else {
            panic!("kept the provider")
        };
        assert_eq!((fast_model.as_str(), fast_effort, deep_effort, auth), ("opus", None, Some(Effort::Max), ClaudeAuth::Login));
    }

    #[test]
    fn the_label_says_when_claude_code_uses_an_api_key() {
        let mut s = Settings::default();
        assert_eq!(s.provider.label(), "Claude Code (haiku)");
        if let ProviderSettings::ClaudeCli { auth, .. } = &mut s.provider {
            *auth = ClaudeAuth::ApiKey;
        }
        assert_eq!(s.provider.label(), "Claude Code (API key, haiku)");
    }
}
