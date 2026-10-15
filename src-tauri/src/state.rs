use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use livespec_core::instructions::InstructionStore;
use livespec_core::local::{LocalPaths, LocalServer};
use livespec_core::project::{BriefStore, ProjectIndex, StoredBrief};
use livespec_core::sessions::SessionStore;
use livespec_core::settings::Settings;
use livespec_core::usage::UsageLedger;
use tauri::async_runtime::JoinHandle;

/// One loaded project: its index (which keeps file contents) and its cached brief.
pub struct ProjectEntry {
    pub index: Arc<ProjectIndex>,
    pub brief: Option<StoredBrief>,
}

pub struct AppState {
    pub http: reqwest::Client,
    pub settings_path: std::path::PathBuf,
    pub settings: RwLock<Settings>,
    pub briefs: BriefStore,
    /// Session instructions and templates (`instructions.json`).
    pub instructions: InstructionStore,
    pub local_paths: LocalPaths,
    /// Loaded projects, by root, shared by every tab and window.
    pub projects: RwLock<HashMap<String, Arc<ProjectEntry>>>,
    /// One live analysis per tab: a new run for a tab aborts only that tab's previous run.
    pub analyses: Mutex<HashMap<String, JoinHandle<()>>>,
    /// One brief build per project root.
    pub brief_tasks: Mutex<HashMap<String, JoinHandle<()>>>,
    /// Windows, tabs, drafts and saved analyses (`<app_data>/sessions`).
    pub sessions: SessionStore,
    /// Every model call's tokens and cost (`usage.json`); shared with each `Meter`.
    pub usage: Arc<UsageLedger>,
    /// Keyed by "runtime" or a model id.
    pub downloads: Mutex<HashMap<String, JoinHandle<()>>>,
    /// tokio Mutex: held across the (slow) server startup.
    pub local: tokio::sync::Mutex<Option<LocalServer>>,
}

impl AppState {
    pub fn new(data_dir: &Path, config_dir: &Path) -> Self {
        let settings_path = config_dir.join("settings.json");
        Self {
            http: reqwest::Client::new(),
            settings: RwLock::new(Settings::load(&settings_path)),
            settings_path,
            briefs: BriefStore::new(data_dir.join("briefs")),
            instructions: InstructionStore::new(config_dir.join("instructions.json")),
            local_paths: LocalPaths::new(data_dir),
            projects: RwLock::new(HashMap::new()),
            analyses: Mutex::new(HashMap::new()),
            brief_tasks: Mutex::new(HashMap::new()),
            sessions: SessionStore::new(data_dir.join("sessions")),
            usage: Arc::new(UsageLedger::new(data_dir.join("usage.json"))),
            downloads: Mutex::new(HashMap::new()),
            local: tokio::sync::Mutex::new(None),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().expect("settings lock").clone()
    }

    /// The loaded project for `root`, if any window has loaded it.
    pub fn project_entry(&self, root: &str) -> Option<Arc<ProjectEntry>> {
        self.projects.read().expect("projects lock").get(root).cloned()
    }

    /// Drops every loaded project that no open tab, in any window, uses: an
    /// index keeps file contents. Call after closing, moving or deleting tabs.
    pub fn evict_unused_projects(&self) {
        let open = self.sessions.open_roots(); // read before taking the projects lock
        self.projects.write().expect("projects lock").retain(|root, _| open.contains(root));
    }
}

/// Swaps in a new task for `key`, aborting whatever was running for that key only.
pub fn replace_task(slots: &Mutex<HashMap<String, JoinHandle<()>>>, key: &str, task: JoinHandle<()>) {
    if let Some(old) = slots.lock().expect("task lock").insert(key.to_owned(), task) {
        old.abort();
    }
}

/// Aborts the task for `key`, if any. Aborting an analysis drops its `claude`
/// children too (`kill_on_drop`).
pub fn cancel_task(slots: &Mutex<HashMap<String, JoinHandle<()>>>, key: &str) {
    if let Some(task) = slots.lock().expect("task lock").remove(key) {
        task.abort();
    }
}
