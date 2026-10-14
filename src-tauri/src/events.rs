//! Events every window receives (`app.emit`), unlike a command's own Channel.
//! Names and payloads mirror `src/lib/events.ts`.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Webview};

pub const BRIEF_STATUS: &str = "brief-status";
pub const WORKSPACE_CHANGED: &str = "workspace-changed";
pub const MENU: &str = "menu";
pub const SETTINGS_CHANGED: &str = "settings-changed";
pub const INSTRUCTIONS_CHANGED: &str = "instructions-changed";

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BriefState {
    Building,
    Done,
    Error,
}

/// A brief build for `root` started, finished or failed. On `done`, windows
/// showing a tab of that project refetch its summary.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BriefStatus {
    pub root: String,
    pub state: BriefState,
    pub message: Option<String>,
}

/// A session changed. `windows` lists the windows whose tab list changed
/// (empty when only the shared closed list did). Every window refetches the
/// closed list.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceChanged {
    pub windows: Vec<String>,
}

/// An app-menu item, sent to the focused window only (`emit_to` its label). `id` is an `ActionId`.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuEvent {
    pub id: String,
}

pub fn brief_status(app: &AppHandle, root: &str, state: BriefState, message: Option<String>) {
    let _ = app.emit(BRIEF_STATUS, BriefStatus { root: root.to_owned(), state, message }); // a window may be gone
}

pub fn workspace_changed(app: &AppHandle, windows: Vec<String>) {
    let _ = app.emit(WORKSPACE_CHANGED, WorkspaceChanged { windows });
}

/// After saving instructions (`root` is that project, "" for no project) or templates
/// (`root` ""). `source` is the saving window's label: it skips its own event.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionsChanged {
    pub root: String,
    pub source: String,
}

/// Every window calls `get_settings`. No payload.
pub fn settings_changed(app: &AppHandle) {
    let _ = app.emit(SETTINGS_CHANGED, ()); // a window may be gone
}

/// Other windows refetch; `source` lets the saving window skip its own event, whose copy
/// can be older than what was typed since. `Webview::emit` reaches every window, like `app.emit`.
pub fn instructions_changed(from: &Webview, root: String) {
    let _ = from.emit(INSTRUCTIONS_CHANGED, InstructionsChanged { root, source: from.label().to_owned() });
}
