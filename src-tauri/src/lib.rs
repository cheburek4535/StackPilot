mod core;
mod modules;

use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to get app data dir");

            std::fs::create_dir_all(&data_dir)
                .expect("Failed to create data dir");

            std::fs::create_dir_all(data_dir.join("profiles"))
                .expect("Failed to create profiles dir");

            // === Core services ===
            let settings_service: Arc<dyn core::settings::SettingsService> = Arc::new(
                core::settings::JsonSettingsService::new(data_dir.clone()),
            );

            // === Workspace module ===
            let workspace_state = modules::workspace::WorkspaceState::new(
                Arc::new(modules::workspace::process_manager::OsProcessManager::new()),
            );

            // === DevLauncher module ===
            let devlauncher_state = modules::devlauncher::DevLauncherState::new(
                data_dir.join("profiles"),
                Arc::new(modules::devlauncher::launch_engine::ProcessLaunchEngine),
                Arc::new(modules::devlauncher::analyzer::FsProjectAnalyzer),
            );

            // Register all states
            app.manage(devlauncher_state);
            app.manage(workspace_state);
            app.manage(core::settings::SettingsState(settings_service));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // DevLauncher commands
            modules::devlauncher::commands::ping_rust,
            modules::devlauncher::commands::get_demo_profile,
            modules::devlauncher::commands::list_profiles,
            modules::devlauncher::commands::get_profile,
            modules::devlauncher::commands::save_profile,
            modules::devlauncher::commands::delete_profile,
            modules::devlauncher::commands::execute_action,
            modules::devlauncher::commands::analyze_project,
            // Workspace commands
            modules::workspace::commands::spawn_process,
            modules::workspace::commands::list_processes,
            modules::workspace::commands::kill_process,
            modules::workspace::commands::refresh_process,
            // Core commands
            core::settings::get_settings,
            core::settings::update_settings,
            core::settings::reset_settings,
        ])
        .run(tauri::generate_context!())
        .expect("Error starting Tauri application");
}
