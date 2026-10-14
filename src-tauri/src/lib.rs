//! Tauri shell: wires `livebrainstorm-core` to the webview. Kept thin on purpose;
//! logic that doesn't need Tauri lives in the core crate.

mod commands;
mod events;
mod provider;
mod secrets;
mod state;

use tauri::{Manager, RunEvent};

use state::AppState;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let config_dir = app.path().app_config_dir()?;
            app.manage(AppState::new(&data_dir, &config_dir));
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
