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

/// What a provider emits while streaming.
#[derive(Debug, Clone, PartialEq)]
pub enum Chunk {
    /// Answer text, in order.
    Text(String),
    /// Side activity worth surfacing, e.g. "Read src/main.rs".
    Activity(String),
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

    /// Rough character budget for project context we inline into prompts.
    /// Local models get far less room than hosted Claude.
    pub fn context_budget_chars(&self) -> usize {
        match self {
            Provider::ClaudeCli(_) | Provider::AnthropicApi(_) => 120_000,
            Provider::OpenAiCompat(p) => (p.context_tokens as usize).saturating_mul(3) / 2,
        }
    }
}
