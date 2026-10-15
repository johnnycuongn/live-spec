//! Provider abstraction.
//!
//! Enum dispatch instead of `dyn Trait`: we have a closed set of three
//! providers, so this avoids `async_trait` boxing and keeps call sites simple.

mod anthropic_api;
mod claude_cli;
mod openai_compat;
mod sse;

pub use anthropic_api::AnthropicApi;
pub use claude_cli::ClaudeCli;
pub use openai_compat::OpenAiCompat;

/// An API key. `Debug` never prints it, so a derived `Debug` on a provider or
/// a stray `{:?}` can't leak it into a log.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

use std::path::PathBuf;

use crate::Result;

/// One model call. Providers that can't use some fields simply ignore them.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub system: String,
    pub user: String,
    pub max_tokens: u32,
    /// Working directory (Claude CLI only): lets it read CLAUDE.md and, if
    /// `allow_repo_tools` is set, explore the repo itself.
    pub cwd: Option<PathBuf>,
    /// Allow read-only repo tools (Read/Glob/Grep). Claude CLI only.
    pub allow_repo_tools: bool,
    /// Claude CLI turn limit; None means 40 with repo tools, 1 without.
    pub max_turns: Option<u32>,
}

/// What one model call used, as the provider reported it. `cost_usd` is the
/// call's list-price cost, or None when the price isn't known.
#[derive(Debug, Clone, PartialEq)]
pub struct Usage {
    pub model: String,
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    pub cost_usd: Option<f64>,
}

/// What a provider emits while streaming.
#[derive(Debug, Clone, PartialEq)]
pub enum Chunk {
    /// Answer text, in order.
    Text(String),
    /// Side activity worth surfacing, e.g. "Read src/main.rs".
    Activity(String),
    /// What the call used, once per model at the end of any call the provider
    /// reports usage for (failed calls included). `usage::metered` records it;
    /// it is never shown.
    Usage(Usage),
}

pub type OnChunk<'a> = &'a mut (dyn FnMut(Chunk) + Send);

#[derive(Debug, Clone)]
pub enum Provider {
    ClaudeCli(ClaudeCli),
    AnthropicApi(AnthropicApi),
    OpenAiCompat(OpenAiCompat),
}

impl Provider {
    /// Streams the answer through `on_chunk` and returns the final answer text.
    pub async fn stream(&self, req: &ChatRequest, on_chunk: OnChunk<'_>) -> Result<String> {
        match self {
            Provider::ClaudeCli(p) => p.stream(req, on_chunk).await,
            Provider::AnthropicApi(p) => p.stream(req, on_chunk).await,
            Provider::OpenAiCompat(p) => p.stream(req, on_chunk).await,
        }
    }

    /// Whether the provider can explore the repository on its own (agentic).
    pub fn can_explore_repo(&self) -> bool {
        matches!(self, Provider::ClaudeCli(_))
    }

    /// The model this provider was asked for: an alias ("haiku") or a model id
    /// for Claude, a catalog id for local models. Usage estimates are priced by it.
    pub fn model(&self) -> &str {
        match self {
            Provider::ClaudeCli(p) => &p.model,
            Provider::AnthropicApi(p) => &p.model,
            Provider::OpenAiCompat(p) => &p.model,
        }
    }

    /// Rough character budget for project context we inline into prompts.
    /// Local models get far less room than hosted Claude.
    pub fn context_budget_chars(&self) -> usize {
        match self {
            Provider::ClaudeCli(_) | Provider::AnthropicApi(_) => 120_000,
            Provider::OpenAiCompat(p) => (p.context_tokens as usize).saturating_mul(3) / 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_never_prints() {
        let s = Secret::new("sk-ant-api03-abc");
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert_eq!(s.expose(), "sk-ant-api03-abc");
    }

    #[test]
    fn model_is_what_the_provider_was_asked_for() {
        let cli = Provider::ClaudeCli(ClaudeCli::new("claude".into(), "haiku", None));
        assert_eq!(cli.model(), "haiku");
        let api = Provider::AnthropicApi(AnthropicApi {
            api_key: Secret::new("k"),
            model: "claude-haiku-5-5".into(),
            effort: None,
            http: reqwest::Client::new(),
        });
        assert_eq!(api.model(), "claude-haiku-5-5");
        let local = Provider::OpenAiCompat(OpenAiCompat {
            base_url: "http://127.0.0.1:1".into(),
            model: "qwen3-4b-instruct".into(),
            context_tokens: 4096,
            http: reqwest::Client::new(),
        });
        assert_eq!(local.model(), "qwen3-4b-instruct");
    }
}
