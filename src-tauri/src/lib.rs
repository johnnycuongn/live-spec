//! Tauri shell: wires `livebrainstorm-core` to the webview. Kept thin on purpose;
//! logic that doesn't need Tauri lives in the core crate.

mod commands;
mod events;
mod menu;
mod provider;
mod secrets;
mod state;
mod windows;

use tauri::{Manager, RunEvent};

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
            // One window per workspace window; tauri.conf.json declares none.
            let ids = app.state::<AppState>().sessions.windows_to_open().unwrap_or_else(|e| {
                eprintln!("LiveBrainstorm: couldn't read the saved windows: {e}");
                vec!["w-1".to_owned()]
            });
            for id in ids {
                windows::build(app.handle(), &id)?;
            }
            Ok(())
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
            }
        });
}
