//! IPC surface. Long-running work (analysis, briefs, downloads) returns
//! immediately and streams over a per-call `Channel`, so the UI never blocks
//! and each request's events can't leak into another's.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use livebrainstorm_core::agents;
use livebrainstorm_core::analysis::{self, PreInput, PreOutcome, PRE_BUDGET};
use livebrainstorm_core::instructions::{InstructionsView, ProjectInstructions, Template};
use livebrainstorm_core::jev::key::KeyStatus;
use livebrainstorm_core::jev::check::{self, CheckEvent, CheckResult};
use livebrainstorm_core::llm::Chunk;
use livebrainstorm_core::local::{self, catalog, download, runtime, LocalInventory};
use livebrainstorm_core::project::explore;
use livebrainstorm_core::project::{self, BriefStatus, StoredBrief};
use livebrainstorm_core::settings::{ProviderSettings, Settings, Tier};
use livebrainstorm_core::{prompt, Error, Result};
use serde::Serialize;
use tauri::ipc::Channel;
use tokio::sync::oneshot;
use tauri::{AppHandle, Manager, State};

use crate::provider;
use crate::secrets;
use crate::state::{replace_task, AppState};

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UiEvent {
    Text { text: String, lane: usize },
    Activity { label: String, lane: usize },
    /// `partial`: some sections couldn't be written (see the notice).
    Done { text: String, partial: bool },
    Error { message: String },
    Progress { downloaded: u64, total: Option<u64> },
    /// `count` of the `total` Faults name code that's being checked.
    Checking { count: usize, total: usize },
    Check(CheckResult),
    /// Non-fatal: the notes are fine, something around them failed.
    Notice { message: String },
    /// The fact-check failed; the remaining claims were dropped.
    ChecksStopped { message: String },
    /// The pre-phase passed: the model call is starting (clear old notes now).
    Started,
    /// The gate found no real change: keep the current notes. Final.
    Skipped,
}

impl From<CheckEvent> for UiEvent {
    fn from(ev: CheckEvent) -> Self {
        match ev {
            CheckEvent::Checking { count, total } => UiEvent::Checking { count, total },
            CheckEvent::Result(result) => UiEvent::Check(result),
            CheckEvent::Notice(message) => UiEvent::ChecksStopped { message },
        }
    }
}

fn send(ch: &Channel<UiEvent>, ev: UiEvent) {
    let _ = ch.send(ev); // window may be gone; nothing useful to do
}

/// The brief streams as one lane.
fn on_chunk(ch: &Channel<UiEvent>) -> impl FnMut(Chunk) + Send + '_ {
    move |c| match c {
        Chunk::Text(text) => send(ch, UiEvent::Text { text, lane: 0 }),
        Chunk::Activity(label) => send(ch, UiEvent::Activity { label, lane: 0 }),
    }
}

fn finish(ch: &Channel<UiEvent>, result: Result<String>) {
    match result {
        Ok(text) => send(ch, UiEvent::Done { text, partial: false }),
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

#[tauri::command]
pub fn set_jev_key(key: String) -> Result<()> {
    secrets::set_jev_key(&key)
}

/// Where the TypeSafe key comes from, never the key itself.
#[tauri::command]
pub fn jev_status() -> KeyStatus {
    KeyStatus { source: secrets::jev_key_with_source().map(|(_, source)| source) }
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
            let settings = task_app.state::<AppState>().settings();
            let provider = provider::resolve(&task_app, Tier::Deep).await?;
            let slices = explore::slices(&index, settings.effective_brief_agents());
            let (text, agents) = if slices.len() <= 1 {
                // One explorer: today's path, unchanged.
                let req = prompt::brief_request(&index, &provider);
                (provider.stream(&req, &mut on_chunk(&on_event)).await?, 1)
            } else {
                // Explorers map their slices at once; their activity shows as
                // "Agent k". A `for` loop, not `.map(…)`, inside this spawned task.
                let n = slices.len();
                let mut explorers = Vec::new();
                for (k, slice) in slices.iter().enumerate() {
                    let req = prompt::explorer_request(&index, &provider, slice, k + 1, n);
                    let (provider, on_event) = (&provider, &on_event);
                    explorers.push(async move {
                        let lane = k + 1;
                        let mut sink = |chunk: Chunk| {
                            if let Chunk::Activity(label) = chunk {
                                send(on_event, UiEvent::Activity { label, lane });
                            }
                        };
                        provider.stream(&req, &mut sink).await
                    });
                }
                let (notes, unmapped) = explore::collect_notes(&slices, agents::run_limited(n, explorers).await)?;
                let req = prompt::merge_request(&index, &provider, &notes, &unmapped);
                (provider.stream(&req, &mut on_chunk(&on_event)).await?, n)
            };
            let brief = StoredBrief {
                root: index.root.display().to_string(),
                fingerprint: index.fingerprint.clone(),
                text: text.trim().to_owned(),
                created_at: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
                source: explore::brief_source(&settings.provider.label(), agents),
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

// ---------- session instructions ----------

/// Never fails: a missing or unreadable file gives the starter templates, so
/// instructions can't block opening a project.
#[tauri::command]
pub fn get_instructions(state: State<'_, AppState>, root: String) -> InstructionsView {
    state.instructions.view(&root)
}

#[tauri::command]
pub fn save_project_instructions(state: State<'_, AppState>, root: String, instructions: ProjectInstructions) -> Result<()> {
    state.instructions.save_project(&root, instructions)
}

#[tauri::command]
pub fn save_templates(state: State<'_, AppState>, templates: Vec<Template>) -> Result<()> {
    state.instructions.save_templates(templates)
}

#[tauri::command]
pub fn restore_starter_templates(state: State<'_, AppState>) -> Result<Vec<Template>> {
    state.instructions.restore_starters()
}

// ---------- live analysis ----------

#[tauri::command]
pub async fn analyze(
    app: AppHandle,
    draft: String,
    previous_draft: Option<String>,
    instructions: String,
    on_event: Channel<UiEvent>,
) -> Result<()> {
    let task_app = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        let state = task_app.state::<AppState>();
        let settings = state.settings();
        let index = state.project();
        let provider = match provider::resolve(&task_app, Tier::Fast).await {
            Ok(provider) => provider,
            Err(e) => return send(&on_event, UiEvent::Error { message: e.to_string() }),
        };
        let brief = state.brief.read().expect("brief lock").as_ref().map(|b| b.text.clone());
        let budget = provider.context_budget_chars();
        let max_files = if budget < 40_000 { 3 } else { 5 };

        // Jev sends text to TypeSafe, so never for the Local provider.
        let jev = if settings.jev_allowed() { provider::jev(&task_app) } else { None };
        let pre = analysis::pre_phase(
            PreInput {
                draft: &draft,
                previous_draft: previous_draft.as_deref(),
                index: index.as_deref(),
                max_files,
                budget_chars: budget / 4,
                jev: jev.as_ref(),
                gate: settings.jev_gate_enabled(),
                rerank: settings.jev_rerank_enabled(),
            },
            PRE_BUDGET,
        )
        .await;
        let PreOutcome::Analyze { snippets, notice } = pre else { return send(&on_event, UiEvent::Skipped) };
        send(&on_event, UiEvent::Started);
        if let Some(message) = notice {
            send(&on_event, UiEvent::Notice { message });
        }

        // Each lane writes some of the sections; up to one agent per section.
        // A plain `for` loop, not `.map(…)`: closures over borrowed items trip
        // rustc's Send check inside this spawned task.
        let groups = prompt::section_groups(settings.effective_note_agents());
        let (faults_tx, faults_rx) = oneshot::channel::<String>();
        let mut faults_tx = Some(faults_tx);
        let mut lanes = Vec::new();
        for (lane, sections) in groups.iter().enumerate() {
            let req = prompt::analysis_request(&draft, index.as_deref(), brief.as_deref(), &snippets, &provider, sections, &instructions);
            let tx = if sections.contains(&"Faults") { faults_tx.take() } else { None };
            let (provider, on_event) = (&provider, &on_event);
            lanes.push(async move {
                let mut sink = |chunk: Chunk| match chunk {
                    Chunk::Text(text) => send(on_event, UiEvent::Text { text, lane }),
                    Chunk::Activity(label) => send(on_event, UiEvent::Activity { label, lane }),
                };
                let result = provider.stream(&req, &mut sink).await;
                if let (Some(tx), Ok(text)) = (tx, &result) {
                    let _ = tx.send(text.clone());
                }
                result
            });
        }
        drop(faults_tx);

        let write = async {
            match analysis::merge_lanes(&groups, agents::run_limited(groups.len(), lanes).await) {
                Ok((text, notice)) => {
                    let partial = notice.is_some();
                    if let Some(message) = notice {
                        send(&on_event, UiEvent::Notice { message });
                    }
                    send(&on_event, UiEvent::Done { text, partial });
                }
                Err(e) => send(&on_event, UiEvent::Error { message: e.to_string() }),
            }
        };
        // "Checked in code" starts as soon as the Faults lane finishes, while
        // other lanes may still stream; `done` never waits for it. The next
        // analysis aborts this task, lanes and checks included.
        let check = async {
            let Ok(text) = faults_rx.await else { return }; // that lane failed
            if !settings.jev_check_enabled() {
                return;
            }
            let (Some(index), Some(jev)) = (index.as_deref(), jev.as_ref()) else { return };
            if let Some(faults) = prompt::section_body(&text, "Faults") {
                check::run(jev, index, &faults, |ev| send(&on_event, ev.into())).await;
            }
        };
        tokio::join!(write, check);
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
