//! Events every window receives (`app.emit`), unlike a command's own Channel.
//! Names and payloads mirror `src/lib/events.ts`.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const BRIEF_STATUS: &str = "brief-status";

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

pub fn brief_status(app: &AppHandle, root: &str, state: BriefState, message: Option<String>) {
    let _ = app.emit(BRIEF_STATUS, BriefStatus { root: root.to_owned(), state, message }); // a window may be gone
}
