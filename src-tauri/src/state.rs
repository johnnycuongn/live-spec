use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use livebrainstorm_core::local::{LocalPaths, LocalServer};
use livebrainstorm_core::project::{BriefStore, ProjectIndex, StoredBrief};
use livebrainstorm_core::settings::Settings;
use tauri::async_runtime::JoinHandle;

pub struct AppState {
    pub http: reqwest::Client,
    pub settings_path: std::path::PathBuf,
    pub settings: RwLock<Settings>,
    pub briefs: BriefStore,
    pub local_paths: LocalPaths,
    pub project: RwLock<Option<Arc<ProjectIndex>>>,
    pub brief: RwLock<Option<StoredBrief>>,
    /// At most one live analysis: a new keystroke-triggered run aborts the old.
    pub analysis: Mutex<Option<JoinHandle<()>>>,
    pub brief_task: Mutex<Option<JoinHandle<()>>>,
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
            local_paths: LocalPaths::new(data_dir),
            project: RwLock::new(None),
            brief: RwLock::new(None),
            analysis: Mutex::new(None),
            brief_task: Mutex::new(None),
            downloads: Mutex::new(HashMap::new()),
            local: tokio::sync::Mutex::new(None),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().expect("settings lock").clone()
    }

    pub fn project(&self) -> Option<Arc<ProjectIndex>> {
        self.project.read().expect("project lock").clone()
    }
}

/// Swaps in a new task, aborting whatever was running in that slot.
pub fn replace_task(slot: &Mutex<Option<JoinHandle<()>>>, task: JoinHandle<()>) {
    if let Some(old) = slot.lock().expect("task lock").replace(task) {
        old.abort();
    }
}
