//! Runtime windows. Each is one workspace window, labelled with its id (`w-<n>`), so
//! `tauri-plugin-window-state` (which keys on labels) restores its position and size,
//! and the capability glob `"windows": ["w-*"]` gives it IPC.

use livespec_core::{Error, Result};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Window, WindowEvent};

use crate::events;
use crate::state::{cancel_task, AppState};

/// Builds the window for workspace window `id`. Call it from `setup` or an `async`
/// command only: on Windows, building a window in a synchronous command or an event
/// handler deadlocks (WebView2).
pub fn build(app: &AppHandle, id: &str) -> Result<WebviewWindow> {
    WebviewWindowBuilder::new(app, id, WebviewUrl::App("index.html".into()))
        .title("Live Spec")
        .inner_size(1280.0, 820.0)
        .min_inner_size(860.0, 560.0)
        .build()
        // `Provider` is the catch-all failure; its message is shown verbatim.
        .map_err(|e| Error::Provider(format!("Couldn't open a window: {e}")))
}

/// Registered with `Builder::on_window_event`.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { .. } = event {
        close_requested(window.app_handle(), window.label());
    }
}

/// With another window open, this window's tabs go to Recently closed and their runs stop.
/// The last window is left as it is: closing it quits, and the next launch restores it.
fn close_requested(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    match state.sessions.close_window_unless_last(id) {
        Ok(Some(closed)) => {
            for tab in &closed {
                cancel_task(&state.analyses, tab); // drops the claude children too (kill_on_drop)
            }
            state.evict_unused_projects();
            events::workspace_changed(app, vec![id.to_owned()]);
        }
        Ok(None) => {}
        // The window still closes; its tabs stay listed and come back at the next launch.
        Err(e) => eprintln!("Live Spec: couldn't move window {id}'s tabs to Recently closed: {e}"),
    }
}
