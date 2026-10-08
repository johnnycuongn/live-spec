//! Turns settings into a ready-to-call provider.

use std::path::PathBuf;

use livebrainstorm_core::llm::{AnthropicApi, ClaudeCli, OpenAiCompat, Provider};
use livebrainstorm_core::local::catalog;
use livebrainstorm_core::local::LocalServer;
use livebrainstorm_core::settings::{ProviderSettings, Tier};
use livebrainstorm_core::{shell_env, Error, Result};
use tauri::{AppHandle, Manager};

use crate::secrets;
use crate::state::AppState;

pub async fn resolve(app: &AppHandle, tier: Tier) -> Result<Provider> {
    let state = app.state::<AppState>();
    let pick = |fast: String, deep: String| if tier == Tier::Fast { fast } else { deep };

    match state.settings().provider {
        ProviderSettings::ClaudeCli { binary_path, fast_model, deep_model } => {
            let mut cli = claude_cli(binary_path).await?;
            cli.model = pick(fast_model, deep_model);
            Ok(Provider::ClaudeCli(cli))
        }
        ProviderSettings::AnthropicApi { fast_model, deep_model } => {
            let api_key = secrets::api_key()?
                .ok_or_else(|| Error::Precondition("Add your Anthropic API key in Settings.".into()))?;
            Ok(Provider::AnthropicApi(AnthropicApi {
                api_key,
                model: pick(fast_model, deep_model),
                http: state.http.clone(),
            }))
        }
        ProviderSettings::Local { model_id } => {
            let (base_url, context_tokens) = ensure_local(app, &model_id).await?;
            Ok(Provider::OpenAiCompat(OpenAiCompat {
                base_url,
                model: model_id,
                context_tokens,
                http: state.http.clone(),
            }))
        }
    }
}

pub async fn claude_cli(binary_path: Option<String>) -> Result<ClaudeCli> {
    // Spawning a login shell blocks; keep it off the async runtime.
    let path_env = tokio::task::spawn_blocking(shell_env::login_shell_path).await.ok().flatten();
    let binary = match binary_path.filter(|p| !p.trim().is_empty()) {
        Some(p) => PathBuf::from(p),
        None => shell_env::find_executable("claude", path_env.as_deref()).ok_or_else(|| {
            Error::Precondition(
                "Claude Code isn't installed or isn't on your PATH. Install it, run `claude` once \
                 to sign in, or set its path in Settings."
                    .into(),
            )
        })?,
    };
    Ok(ClaudeCli { binary, model: String::new(), path_env })
}

/// Starts (or reuses) the local server for `model_id`.
///
/// Startup runs in its own task: the caller is usually an analysis that gets
/// aborted on the next keystroke, and an aborted caller must not kill a model
/// that's halfway through loading. Awaiting a JoinHandle doesn't own the task.
pub async fn ensure_local(app: &AppHandle, model_id: &str) -> Result<(String, u32)> {
    let app = app.clone();
    let model_id = model_id.to_owned();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let mut guard = state.local.lock().await;
        if let Some(server) = guard.as_mut() {
            if server.model_id == model_id && server.is_alive() {
                return Ok((server.base_url(), server.context_tokens));
            }
        }
        if let Some(old) = guard.take() {
            old.stop().await;
        }
        let spec = catalog::find(&model_id)
            .ok_or_else(|| Error::Precondition(format!("Unknown local model '{model_id}'.")))?;
        let path = state.local_paths.model_path(spec);
        let server =
            LocalServer::start(&state.http, &state.local_paths, spec.id, &path, spec.context_tokens).await?;
        let out = (server.base_url(), server.context_tokens);
        *guard = Some(server);
        Ok(out)
    })
    .await
    .map_err(|e| Error::Provider(format!("Local model task failed: {e}")))?
}
