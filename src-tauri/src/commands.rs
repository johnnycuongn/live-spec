//! IPC surface. Long-running work (analysis, briefs, downloads) returns
//! immediately and streams over a per-call `Channel`, so the UI never blocks
//! and each request's events can't leak into another's.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
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
use livebrainstorm_core::sessions::{valid_id, LegacyDraft, Recorder, Tab, WindowSummary, WindowView};
use livebrainstorm_core::settings::{ProviderSettings, Settings, Tier};
use livebrainstorm_core::usage::{self, UsageBucket, UsageKind, UsageTag};
use livebrainstorm_core::{prompt, Error, Result};
use serde::Serialize;
use tauri::ipc::Channel;
use tokio::sync::oneshot;
use tauri::{AppHandle, Manager, State, Webview};

use crate::events::{self, BriefState};
use crate::provider;
use crate::secrets;
use crate::state::{cancel_task, replace_task, AppState, ProjectEntry};
use crate::windows;

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
        // Recorded by `usage::metered`, never shown.
        Chunk::Usage(_) => {}
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
    events::settings_changed(&app); // other windows call get_settings
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

impl ProjectSummary {
    fn of(entry: &ProjectEntry) -> Self {
        let index = &entry.index;
        Self {
            name: index.name.clone(),
            root: index.root.display().to_string(),
            indexed_files: index.files.len(),
            total_files: index.tree.len(),
            brief_status: BriefStatus::of(entry.brief.as_ref(), index),
            brief: entry.brief.clone(),
        }
    }
}

/// Scans `root` on a blocking thread, loads its cached brief, and puts it in
/// the registry under its canonical root, replacing any earlier entry. Other
/// projects stay loaded.
async fn load_project(state: &AppState, root: PathBuf) -> Result<Arc<ProjectEntry>> {
    let index = tauri::async_runtime::spawn_blocking(move || project::scan(&root))
        .await
        .map_err(|e| Error::Provider(format!("Scan failed: {e}")))??;
    let brief = state.briefs.load(&index.root);
    let entry = Arc::new(ProjectEntry { index: Arc::new(index), brief });
    let key = entry.index.root.display().to_string();
    state.projects.write().expect("projects lock").insert(key, entry.clone());
    Ok(entry)
}

/// The loaded project for a tab's root, scanned first if no window has loaded
/// it since launch.
async fn project_or_scan(state: &AppState, root: &str) -> Result<Arc<ProjectEntry>> {
    if let Some(entry) = state.project_entry(root) {
        return Ok(entry);
    }
    if !Path::new(root).is_dir() {
        return Err(Error::Precondition("That folder no longer exists. Open it again from its new place.".into()));
    }
    let entry = load_project(state, PathBuf::from(root)).await?;
    // A tab may have closed while this scanned: keep only roots an open tab uses.
    state.evict_unused_projects();
    Ok(entry)
}

/// "Open a project…", and explicit refreshes: always rescans.
#[tauri::command]
pub async fn open_project(app: AppHandle, path: String) -> Result<ProjectSummary> {
    let state = app.state::<AppState>();
    let entry = load_project(&state, PathBuf::from(path)).await?;
    Ok(ProjectSummary::of(&entry))
}

/// The summary a window shows for a tab's project. Scans only a project that
/// isn't loaded yet.
#[tauri::command]
pub async fn project_summary(app: AppHandle, root: String) -> Result<ProjectSummary> {
    let state = app.state::<AppState>();
    let entry = project_or_scan(&state, &root).await?;
    Ok(ProjectSummary::of(&entry))
}

/// One build per project. While one runs for `root`, another request (from
/// any window) returns at once and follows it through `brief-status`, which
/// every window receives. The window that started it also gets the live text
/// and activity over its Channel.
#[tauri::command]
pub async fn build_brief(app: AppHandle, root: String, on_event: Channel<UiEvent>) -> Result<()> {
    let state = app.state::<AppState>();
    let index = project_or_scan(&state, &root).await?.index.clone();
    let mut tasks = state.brief_tasks.lock().expect("task lock");
    if tasks.get(&root).is_some_and(|task| !task.inner().is_finished()) {
        return Ok(());
    }
    events::brief_status(&app, &root, BriefState::Building, None);
    let task_app = app.clone();
    let task_root = root.clone();

    let task = tauri::async_runtime::spawn(async move {
        let result: Result<StoredBrief> = async {
            let settings = task_app.state::<AppState>().settings();
            let provider = provider::resolve(&task_app, Tier::Deep).await?;
            let slices = explore::slices(&index, settings.effective_brief_agents());
            let ledger = task_app.state::<AppState>().usage.clone();
            let tag = UsageTag { root: Some(index.root.display().to_string()), kind: UsageKind::Brief };
            let (text, agents) = if slices.len() <= 1 {
                // One explorer: today's path, unchanged.
                let req = prompt::brief_request(&index, &provider);
                (usage::metered(&provider, &req, ledger, tag, &mut on_chunk(&on_event)).await?, 1)
            } else {
                // Explorers map their slices at once; their activity shows as
                // "Agent k". A `for` loop, not `.map(…)`, inside this spawned task.
                let n = slices.len();
                let mut explorers = Vec::new();
                for (k, slice) in slices.iter().enumerate() {
                    let req = prompt::explorer_request(&index, &provider, slice, k + 1, n);
                    let (provider, on_event) = (&provider, &on_event);
                    let (ledger, tag) = (ledger.clone(), tag.clone());
                    explorers.push(async move {
                        let lane = k + 1;
                        let mut sink = |chunk: Chunk| {
                            if let Chunk::Activity(label) = chunk {
                                send(on_event, UiEvent::Activity { label, lane });
                            }
                        };
                        usage::metered(provider, &req, ledger, tag, &mut sink).await
                    });
                }
                let (notes, unmapped) = explore::collect_notes(&slices, agents::run_limited(n, explorers).await)?;
                let req = prompt::merge_request(&index, &provider, &notes, &unmapped);
                (usage::metered(&provider, &req, ledger, tag, &mut on_chunk(&on_event)).await?, n)
            };
            let brief = StoredBrief {
                root: index.root.display().to_string(),
                fingerprint: index.fingerprint.clone(),
                text: text.trim().to_owned(),
                created_at: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
                source: explore::brief_source(&settings.provider.label(), agents),
            };
            task_app.state::<AppState>().briefs.save(&brief)?;
            Ok(brief)
        }
        .await;
        let state = task_app.state::<AppState>();
        match result {
            Ok(brief) => {
                // Rescan, so every tab of the project gets the new brief and
                // the files changed while it was written.
                if load_project(&state, index.root.clone()).await.is_err() {
                    let entry = ProjectEntry { index: index.clone(), brief: Some(brief.clone()) };
                    state.projects.write().expect("projects lock").insert(task_root.clone(), Arc::new(entry));
                }
                // The project's last tab may have closed while the brief was written.
                state.evict_unused_projects();
                events::brief_status(&task_app, &task_root, BriefState::Done, None);
                send(&on_event, UiEvent::Done { text: brief.text, partial: false });
            }
            Err(e) => {
                events::brief_status(&task_app, &task_root, BriefState::Error, Some(e.to_string()));
                send(&on_event, UiEvent::Error { message: e.to_string() });
            }
        }
    });
    tasks.insert(root, task);
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
pub fn save_project_instructions(
    state: State<'_, AppState>,
    webview: Webview,
    root: String,
    instructions: ProjectInstructions,
) -> Result<()> {
    state.instructions.save_project(&root, instructions)?;
    events::instructions_changed(&webview, root);
    Ok(())
}

#[tauri::command]
pub fn save_templates(state: State<'_, AppState>, webview: Webview, templates: Vec<Template>) -> Result<()> {
    state.instructions.save_templates(templates)?;
    events::instructions_changed(&webview, String::new());
    Ok(())
}

#[tauri::command]
pub fn restore_starter_templates(state: State<'_, AppState>, webview: Webview) -> Result<Vec<Template>> {
    let templates = state.instructions.restore_starters()?;
    events::instructions_changed(&webview, String::new());
    Ok(templates)
}

// ---------- sessions ----------

/// This window's tabs, its full active tab, the shared closed list, and `fresh`.
#[tauri::command]
pub fn list_workspace(state: State<'_, AppState>, window: String) -> Result<WindowView> {
    check_id(&window)?;
    state.sessions.window_view(&window)
}

/// The full tab, when a window shows it. A closed tab must be reopened first.
#[tauri::command]
pub fn get_tab(state: State<'_, AppState>, tab_id: String) -> Result<Tab> {
    check_id(&tab_id)?;
    state.sessions.open_tab_root(&tab_id)?;
    state.sessions.tab(&tab_id)
}

#[tauri::command]
pub fn create_tab(app: AppHandle, window: String, root: Option<String>) -> Result<Tab> {
    check_id(&window)?;
    let tab = app.state::<AppState>().sessions.create_tab(&window, root)?;
    events::workspace_changed(&app, vec![window]);
    Ok(tab)
}

/// The frontend saves here 500 ms after typing stops. A closed tab still takes
/// a save that was already on its way. Only a new first line changes a list, so
/// only that announces itself: every pause would otherwise refetch every window.
#[tauri::command]
pub fn save_draft(app: AppHandle, tab_id: String, draft: String) -> Result<()> {
    check_id(&tab_id)?;
    let state = app.state::<AppState>();
    if state.sessions.save_draft(&tab_id, draft)? {
        events::workspace_changed(&app, state.sessions.window_of(&tab_id).into_iter().collect());
    }
    Ok(())
}

/// `None` or a blank title returns the tab to its automatic title.
#[tauri::command]
pub fn rename_tab(app: AppHandle, tab_id: String, title: Option<String>) -> Result<()> {
    check_id(&tab_id)?;
    let state = app.state::<AppState>();
    state.sessions.rename_tab(&tab_id, title)?;
    events::workspace_changed(&app, state.sessions.window_of(&tab_id).into_iter().collect());
    Ok(())
}

#[tauri::command]
pub fn activate_tab(app: AppHandle, window: String, tab_id: String) -> Result<()> {
    check_id(&window)?;
    check_id(&tab_id)?;
    app.state::<AppState>().sessions.activate(&window, &tab_id)?;
    events::workspace_changed(&app, vec![window]);
    Ok(())
}

/// Stops the tab's run, keeps its file for Recently closed, and unloads its
/// project if no open tab uses it any more.
#[tauri::command]
pub fn close_tab(app: AppHandle, tab_id: String) -> Result<()> {
    check_id(&tab_id)?;
    let state = app.state::<AppState>();
    // Close first, then cancel: `analyze` re-checks the tab after registering its task, so
    // whichever order they interleave in, a run on a closed tab is cancelled by one of the two.
    let window = state.sessions.close_tab(&tab_id)?;
    cancel_task(&state.analyses, &tab_id); // drops the Claude CLI children too (kill_on_drop)
    state.evict_unused_projects();
    events::workspace_changed(&app, window.into_iter().collect());
    Ok(())
}

#[tauri::command]
pub fn reopen_tab(app: AppHandle, tab_id: String, window: String) -> Result<()> {
    check_id(&tab_id)?;
    check_id(&window)?;
    app.state::<AppState>().sessions.reopen_tab(&tab_id, &window)?;
    events::workspace_changed(&app, vec![window]);
    Ok(())
}

/// Closed tabs only. Every window refetches its closed list.
#[tauri::command]
pub fn delete_tab(app: AppHandle, tab_id: String) -> Result<()> {
    check_id(&tab_id)?;
    let state = app.state::<AppState>();
    state.sessions.delete_tab(&tab_id)?;
    state.evict_unused_projects();
    events::workspace_changed(&app, Vec::new());
    Ok(())
}

/// The first launch after the upgrade: the old build's drafts become tabs in
/// `window`. Runs once; later calls do nothing.
#[tauri::command]
pub fn import_legacy_drafts(
    app: AppHandle,
    window: String,
    drafts: Vec<LegacyDraft>,
    active_root: Option<String>,
) -> Result<()> {
    check_id(&window)?;
    app.state::<AppState>().sessions.import_legacy(&window, drafts, active_root)?;
    events::workspace_changed(&app, vec![window]);
    Ok(())
}

// ---------- windows ----------

/// A new, empty window ("Open a project" shows in it). `async`: building a window in a
/// synchronous command deadlocks on Windows.
#[tauri::command]
pub async fn open_window(app: AppHandle) -> Result<String> {
    let state = app.state::<AppState>();
    let id = state.sessions.add_window()?;
    if let Err(e) = windows::build(&app, &id) {
        let _ = state.sessions.close_window(&id); // it has no tabs: this only drops the entry
        return Err(e);
    }
    events::workspace_changed(&app, vec![id.clone()]);
    Ok(id)
}

/// Moves a tab to another window, or to a new one (`to: None`), and makes it active there.
/// Its running analysis stops: the run's Channel belongs to the old window, and the new
/// window re-runs at the next pause. Returns the target window's id. `async`: it may build
/// a window, which deadlocks in a synchronous command on Windows.
#[tauri::command]
pub async fn move_tab(app: AppHandle, tab_id: String, to: Option<String>) -> Result<String> {
    check_id(&tab_id)?;
    if let Some(window) = &to {
        check_id(window)?;
    }
    let state = app.state::<AppState>();
    let new_window = to.is_none();
    let (from, to) = state.sessions.move_tab(&tab_id, to.as_deref())?;
    if from == to {
        return Ok(to); // already there: nothing moved, and its run goes on
    }
    cancel_task(&state.analyses, &tab_id); // drops the claude children too (kill_on_drop)
    if let Err(e) = state.sessions.activate(&to, &tab_id) {
        eprintln!("LiveBrainstorm: moved tab {tab_id} to {to} but couldn't show it there: {e}");
    }
    match app.get_webview_window(&to) {
        Some(window) => {
            let _ = window.set_focus();
        }
        None => {
            if let Err(e) = windows::build(&app, &to) {
                // Put it back rather than strand it in a window that isn't on screen.
                let _ = state.sessions.move_tab(&tab_id, Some(from.as_str()));
                let _ = state.sessions.activate(&from, &tab_id);
                // Only a window this call made is dropped: closing one that existed (its
                // window is just not on screen) would send its other tabs to Recently closed.
                if new_window {
                    let _ = state.sessions.close_window(&to);
                }
                events::workspace_changed(&app, vec![from]);
                return Err(e);
            }
        }
    }
    state.evict_unused_projects();
    events::workspace_changed(&app, vec![from, to.clone()]);
    Ok(to)
}

/// Every window, named by its active tab, for the *Move to* items.
#[tauri::command]
pub fn list_windows(state: State<'_, AppState>) -> Result<Vec<WindowSummary>> {
    state.sessions.windows()
}

// ---------- live analysis ----------

/// Every tab or window id from the webview passes this before it is used: a
/// tab id becomes a file name.
fn check_id(id: &str) -> Result<()> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(Error::Precondition("Unknown tab.".into()))
    }
}

/// A notice about this run's notes: shown now, and saved with them.
fn note(ch: &Channel<UiEvent>, recorder: &Mutex<Recorder<'_>>, message: String) {
    let _ = recorder.lock().expect("recorder lock").notice(&message); // saving never fails a run
    send(ch, UiEvent::Notice { message });
}

/// One run for one tab. A new run for a tab aborts only that tab's previous
/// run, so tabs analyse in parallel. The notes are saved to the tab's history
/// before `done` is sent, so a `get_tab` right after `done` includes them.
#[tauri::command]
pub async fn analyze(
    app: AppHandle,
    tab_id: String,
    draft: String,
    previous_draft: Option<String>,
    instructions: String,
    context: String,
    on_event: Channel<UiEvent>,
) -> Result<()> {
    check_id(&tab_id)?;
    // A closed or unknown tab can't analyse. Its project is read once, as the run starts.
    let root = app.state::<AppState>().sessions.open_tab_root(&tab_id)?;
    let task_app = app.clone();
    let task_tab = tab_id.clone();
    let task = tauri::async_runtime::spawn(async move {
        let state = task_app.state::<AppState>();
        let settings = state.settings();
        // A project no window has loaded yet is scanned now. One that can't be
        // read analyses without code context, like a tab with no project.
        let entry = match root.as_deref() {
            Some(root) => project_or_scan(&state, root).await.ok(),
            None => None,
        };
        let (index, brief) = match &entry {
            Some(entry) => (Some(entry.index.clone()), entry.brief.as_ref().map(|b| b.text.clone())),
            None => (None, None),
        };
        let provider = match provider::resolve(&task_app, Tier::Fast).await {
            Ok(provider) => provider,
            Err(e) => return send(&on_event, UiEvent::Error { message: e.to_string() }),
        };
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
        // This run's snapshot, saved at `done`. Checks that arrive first are
        // kept until then; later checks and notices update the saved snapshot.
        let recorder = Mutex::new(Recorder::new(&state.sessions, &task_tab, &draft, &context));
        if let Some(message) = notice {
            note(&on_event, &recorder, message);
        }

        // Each lane writes some of the sections; up to one agent per section.
        // A plain `for` loop, not `.map(…)`: closures over borrowed items trip
        // rustc's Send check inside this spawned task.
        // Each lane is metered from the moment its request starts (after the
        // pre-phase), so a keystroke during the Jev gate records nothing.
        let ledger = state.usage.clone();
        let tag = UsageTag { root: root.clone(), kind: UsageKind::Notes };
        let groups = prompt::section_groups(settings.effective_note_agents());
        let (faults_tx, faults_rx) = oneshot::channel::<String>();
        let mut faults_tx = Some(faults_tx);
        let mut lanes = Vec::new();
        for (lane, sections) in groups.iter().enumerate() {
            let req = prompt::analysis_request(&draft, index.as_deref(), brief.as_deref(), &snippets, &provider, sections, &instructions);
            let tx = if sections.contains(&"Faults") { faults_tx.take() } else { None };
            let (provider, on_event) = (&provider, &on_event);
            let (ledger, tag) = (ledger.clone(), tag.clone());
            lanes.push(async move {
                let mut sink = |chunk: Chunk| match chunk {
                    Chunk::Text(text) => send(on_event, UiEvent::Text { text, lane }),
                    Chunk::Activity(label) => send(on_event, UiEvent::Activity { label, lane }),
                    Chunk::Usage(_) => {}
                };
                let result = usage::metered(provider, &req, ledger, tag, &mut sink).await;
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
                        note(&on_event, &recorder, message);
                    }
                    // Saved before `done`, so a `get_tab` right after `done` includes it.
                    let saved = recorder.lock().expect("recorder lock").done(&text, partial);
                    if let Err(e) = saved {
                        send(&on_event, UiEvent::Notice { message: format!("Couldn't save these notes: {e}") });
                    }
                    send(&on_event, UiEvent::Done { text, partial });
                }
                Err(e) => send(&on_event, UiEvent::Error { message: e.to_string() }),
            }
        };
        // "Checked in code" starts as soon as the Faults lane finishes, while
        // other lanes may still stream; `done` never waits for it. The next
        // analysis of this tab aborts this task, lanes and checks included.
        let check = async {
            let Ok(text) = faults_rx.await else { return }; // that lane failed
            if !settings.jev_check_enabled() {
                return;
            }
            let (Some(index), Some(jev)) = (index.as_deref(), jev.as_ref()) else { return };
            if let Some(faults) = prompt::section_body(&text, "Faults") {
                check::run(jev, index, &faults, |ev| {
                    let _ = recorder.lock().expect("recorder lock").check(&ev); // saving never fails a run
                    send(&on_event, ev.into());
                })
                .await;
            }
        };
        tokio::join!(write, check);
    });
    let state = app.state::<AppState>();
    replace_task(&state.analyses, &tab_id, task);
    // The tab may have closed since the check above, after `close_tab` looked for
    // a run to cancel: stop this one, so it records nothing into a closed tab.
    if state.sessions.open_tab_root(&tab_id).is_err() {
        cancel_task(&state.analyses, &tab_id);
    }
    Ok(())
}

#[tauri::command]
pub fn cancel_analysis(state: State<'_, AppState>, tab_id: String) -> Result<()> {
    check_id(&tab_id)?;
    cancel_task(&state.analyses, &tab_id); // drops the Claude CLI children too (kill_on_drop)
    Ok(())
}

// ---------- usage ----------

/// The ledger's hourly buckets (UTC) from `since_hour` on; the UI groups them
/// by the user's local day.
#[tauri::command]
pub fn usage_report(state: State<'_, AppState>, since_hour: u64) -> Vec<UsageBucket> {
    state.usage.report(since_hour)
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
