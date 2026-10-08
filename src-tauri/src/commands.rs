//! IPC surface. Long-running work (analysis, briefs, downloads) returns
//! immediately and streams over a per-call `Channel`, so the UI never blocks
//! and each request's events can't leak into another's.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use livebrainstorm_core::llm::Chunk;
use livebrainstorm_core::local::{self, catalog, download, runtime, LocalInventory};
use livebrainstorm_core::project::{self, BriefStatus, StoredBrief};
use livebrainstorm_core::settings::{ProviderSettings, Settings, Tier};
use livebrainstorm_core::{prompt, Error, Result};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::provider;
use crate::secrets;
use crate::state::{replace_task, AppState};

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UiEvent {
    Text { text: String },
    Activity { label: String },
    Done { text: String },
    Error { message: String },
    Progress { downloaded: u64, total: Option<u64> },
}

fn send(ch: &Channel<UiEvent>, ev: UiEvent) {
    let _ = ch.send(ev); // window may be gone; nothing useful to do
}

fn on_chunk(ch: &Channel<UiEvent>) -> impl FnMut(Chunk) + Send + '_ {
    move |c| match c {
        Chunk::Text(text) => send(ch, UiEvent::Text { text }),
        Chunk::Activity(label) => send(ch, UiEvent::Activity { label }),
    }
}

fn finish(ch: &Channel<UiEvent>, result: Result<String>) {
    match result {
        Ok(text) => send(ch, UiEvent::Done { text }),
        Err(e) => send(ch, UiEvent::Error { message: e.to_string() }),
    }
}

// ---------- settings ----------

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings()
}

#[tauri::command]
pub async fn save_settings(app: AppHandle, settings: Settings) -> Result<()> {
    let state = app.state::<AppState>();
    settings.save(&state.settings_path)?;
    let leaving_local = !matches!(settings.provider, ProviderSettings::Local { .. });
    *state.settings.write().expect("settings lock") = settings;
    if leaving_local {
        if let Some(server) = state.local.lock().await.take() {
            server.stop().await; // free the RAM
        }
    }
    warm_up(app).await;
    Ok(())
}

#[tauri::command]
pub fn set_api_key(key: String) -> Result<()> {
    secrets::set_api_key(&key)
}

#[tauri::command]
pub fn has_api_key() -> bool {
    matches!(secrets::api_key(), Ok(Some(_)))
}

/// Returns the CLI version, proving it's installed and runnable.
#[tauri::command]
pub async fn check_claude(binary_path: Option<String>) -> Result<String> {
    provider::claude_cli(binary_path).await?.version().await
}

/// Pre-pays slow setup (login-shell PATH lookup, loading a local model) so
/// the first analysis is fast. Fire-and-forget.
#[tauri::command]
pub async fn warm_up(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let _ = provider::resolve(&app, Tier::Fast).await;
    });
}

// ---------- project ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    name: String,
    root: String,
    indexed_files: usize,
    total_files: usize,
    brief_status: BriefStatus,
    brief: Option<StoredBrief>,
}

#[tauri::command]
pub async fn open_project(app: AppHandle, path: String) -> Result<ProjectSummary> {
    let root = PathBuf::from(path);
    let index = tauri::async_runtime::spawn_blocking(move || project::scan(&root))
        .await
        .map_err(|e| Error::Provider(format!("Scan failed: {e}")))??;

    let state = app.state::<AppState>();
    let brief = state.briefs.load(&index.root);
    let summary = ProjectSummary {
        name: index.name.clone(),
        root: index.root.display().to_string(),
        indexed_files: index.files.len(),
        total_files: index.tree.len(),
        brief_status: BriefStatus::of(brief.as_ref(), &index),
        brief: brief.clone(),
    };
    *state.project.write().expect("project lock") = Some(index.into());
    *state.brief.write().expect("brief lock") = brief;
    Ok(summary)
}

#[tauri::command]
pub async fn build_brief(app: AppHandle, on_event: Channel<UiEvent>) -> Result<()> {
    let state = app.state::<AppState>();
    let index = state.project().ok_or_else(|| Error::Precondition("Open a project first.".into()))?;
    let task_app = app.clone();

    let task = tauri::async_runtime::spawn(async move {
        let result = async {
            let provider = provider::resolve(&task_app, Tier::Deep).await?;
            let req = prompt::brief_request(&index, &provider);
            let text = provider.stream(&req, &mut on_chunk(&on_event)).await?;
            let brief = StoredBrief {
                root: index.root.display().to_string(),
                fingerprint: index.fingerprint.clone(),
                text: text.trim().to_owned(),
                created_at: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
                source: task_app.state::<AppState>().settings().provider.label(),
            };
            let state = task_app.state::<AppState>();
            state.briefs.save(&brief)?;
            // Only adopt it if the user hasn't switched projects meanwhile.
            if state.project().is_some_and(|p| p.root == index.root) {
                *state.brief.write().expect("brief lock") = Some(brief.clone());
            }
            Ok(brief.text)
        }
        .await;
        finish(&on_event, result);
    });
    replace_task(&state.brief_task, task);
    Ok(())
}

// ---------- live analysis ----------

#[tauri::command]
pub async fn analyze(app: AppHandle, draft: String, on_event: Channel<UiEvent>) -> Result<()> {
    let task_app = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        let result = async {
            let provider = provider::resolve(&task_app, Tier::Fast).await?;
            let state = task_app.state::<AppState>();
            let index = state.project();
            let brief = state.brief.read().expect("brief lock").as_ref().map(|b| b.text.clone());

            let budget = provider.context_budget_chars();
            let max_files = if budget < 40_000 { 3 } else { 5 };
            let snippets = index
                .as_deref()
                .map(|i| project::retrieve(i, &draft, max_files, budget / 4))
                .unwrap_or_default();

            let req = prompt::analysis_request(&draft, index.as_deref(), brief.as_deref(), &snippets, &provider);
            provider.stream(&req, &mut on_chunk(&on_event)).await
        }
        .await;
        finish(&on_event, result);
    });
    replace_task(&app.state::<AppState>().analysis, task);
    Ok(())
}

#[tauri::command]
pub fn cancel_analysis(state: State<'_, AppState>) {
    if let Some(task) = state.analysis.lock().expect("task lock").take() {
        task.abort(); // drops the Claude CLI child too (kill_on_drop)
    }
}

// ---------- local models ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalStatus {
    #[serde(flatten)]
    inventory: LocalInventory,
    running_model: Option<String>,
    downloading: Vec<String>,
}

#[tauri::command]
pub async fn local_inventory(state: State<'_, AppState>) -> Result<LocalStatus> {
    let mut local = state.local.lock().await;
    let running_model = local.as_mut().and_then(|s| s.is_alive().then(|| s.model_id.clone()));
    drop(local);
    let downloading = state
        .downloads
        .lock()
        .expect("downloads lock")
        .iter()
        .filter(|(_, t)| !t.inner().is_finished())
        .map(|(k, _)| k.clone())
        .collect();
    Ok(LocalStatus { inventory: local::inventory(&state.local_paths), running_model, downloading })
}

fn track_download(state: &AppState, key: &str, task: tauri::async_runtime::JoinHandle<()>) {
    if let Some(old) = state.downloads.lock().expect("downloads lock").insert(key.to_owned(), task) {
        old.abort();
    }
}

fn progress(ch: &Channel<UiEvent>) -> impl FnMut(u64, Option<u64>) + Send + '_ {
    move |downloaded, total| send(ch, UiEvent::Progress { downloaded, total })
}

#[tauri::command]
pub async fn install_runtime(app: AppHandle, on_event: Channel<UiEvent>) -> Result<()> {
    let task_app = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        let state = task_app.state::<AppState>();
        let result = runtime::install(&state.http, &state.local_paths, progress(&on_event))
            .await
            .map(|p| p.display().to_string());
        finish(&on_event, result);
    });
    track_download(&app.state::<AppState>(), "runtime", task);
    Ok(())
}

#[tauri::command]
pub async fn download_model(app: AppHandle, model_id: String, on_event: Channel<UiEvent>) -> Result<()> {
    let spec = catalog::find(&model_id)
        .ok_or_else(|| Error::Precondition(format!("Unknown local model '{model_id}'.")))?;
    let task_app = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        let state = task_app.state::<AppState>();
        let dest = state.local_paths.model_path(spec);
        let result = download::download(&state.http, spec.url, &dest, progress(&on_event))
            .await
            .map(|()| dest.display().to_string());
        finish(&on_event, result);
    });
    track_download(&app.state::<AppState>(), &model_id, task);
    Ok(())
}

/// Pauses a download; the `.part` file stays, so starting again resumes.
#[tauri::command]
pub fn cancel_download(state: State<'_, AppState>, key: String) {
    if let Some(task) = state.downloads.lock().expect("downloads lock").remove(&key) {
        task.abort();
    }
}

#[tauri::command]
pub async fn stop_local(state: State<'_, AppState>) -> Result<()> {
    if let Some(server) = state.local.lock().await.take() {
        server.stop().await;
    }
    Ok(())
}
