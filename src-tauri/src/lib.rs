//! Tauri shell: wires `livebrainstorm-core` to the webview. Kept thin on purpose;
//! logic that doesn't need Tauri lives in the core crate.

mod commands;
mod events;
mod menu;
mod provider;
mod secrets;
mod state;
mod windows;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, RunEvent};

use state::AppState;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // Restores each window's position and size by label (= workspace window id).
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .build(),
        )
        .on_window_event(windows::on_window_event)
        .menu(menu::build)
        .on_menu_event(menu::on_event)
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let config_dir = app.path().app_config_dir()?;
            app.manage(AppState::new(&data_dir, &config_dir));
            start_usage_tasks(app.handle());
            // One window per workspace window; tauri.conf.json declares none.
            let ids = app.state::<AppState>().sessions.windows_to_open().unwrap_or_else(|e| {
                eprintln!("LiveBrainstorm: couldn't read the saved windows: {e}");
                vec!["w-1".to_owned()]
            });
            // One window failing to build must not stop the rest; with none on screen the
            // app would run invisible, so then the last error ends the launch.
            let mut built = 0;
            let mut last_error = None;
            for id in ids {
                match windows::build(app.handle(), &id) {
                    Ok(_) => built += 1,
                    Err(e) => {
                        eprintln!("LiveBrainstorm: couldn't open window {id}: {e}");
                        last_error = Some(e);
                    }
                }
            }
            match last_error {
                Some(e) if built == 0 => Err(e.into()),
                _ => Ok(()),
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::set_api_key,
            commands::has_api_key,
            commands::set_jev_key,
            commands::jev_status,
            commands::check_claude,
            commands::warm_up,
            commands::open_project,
            commands::project_summary,
            commands::get_instructions,
            commands::save_project_instructions,
            commands::save_templates,
            commands::restore_starter_templates,
            commands::build_brief,
            commands::analyze,
            commands::cancel_analysis,
            commands::usage_report,
            commands::list_workspace,
            commands::get_tab,
            commands::create_tab,
            commands::save_draft,
            commands::rename_tab,
            commands::activate_tab,
            commands::close_tab,
            commands::reopen_tab,
            commands::delete_tab,
            commands::import_legacy_drafts,
            commands::open_window,
            commands::move_tab,
            commands::list_windows,
            commands::local_inventory,
            commands::install_runtime,
            commands::download_model,
            commands::cancel_download,
            commands::stop_local,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build LiveBrainstorm")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                // Don't leave a multi-GB model server running after quit.
                if let Ok(mut local) = app.state::<AppState>().local.try_lock() {
                    local.take(); // kill_on_drop
                }
                // The last few seconds of usage, and the 90-day prune.
                if let Err(e) = app.state::<AppState>().usage.flush() {
                    eprintln!("Couldn't save usage.json: {e}");
                }
            }
        });
}

/// Emits `usage-changed` at most once a second after a record, and saves the
/// ledger every 5 s when it changed. Ledger failures are logged once per
/// streak and never reach an analysis.
fn start_usage_tasks(app: &AppHandle) {
    let ledger = app.state::<AppState>().usage.clone();
    let changed = Arc::new(AtomicBool::new(false));
    let flag = changed.clone();
    ledger.set_on_record(Box::new(move || flag.store(true, Ordering::Relaxed)));
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let (mut seconds, mut failing) = (0u32, false);
        loop {
            tick.tick().await;
            if changed.swap(false, Ordering::Relaxed) {
                let _ = app.emit(events::USAGE_CHANGED, ());
            }
            seconds += 1;
            if seconds < 5 {
                continue;
            }
            seconds = 0;
            match ledger.flush_if_dirty() {
                Ok(()) => failing = false,
                Err(e) if !failing => {
                    eprintln!("Couldn't save usage.json: {e}");
                    failing = true;
                }
                Err(_) => {}
            }
        }
    });
}
