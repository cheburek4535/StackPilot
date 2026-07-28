mod core;
mod modules;

use std::sync::Arc;
use tauri::{Listener, Manager};

#[cfg(feature = "plugins")]
use modules::plugins::mini_ide;
use modules::workspace::session::SessionService;
use modules::project_creator::analysis::DefaultProjectAnalyzer;
use modules::project_creator::engine::DefaultRecipeEngine;
use modules::project_creator::generators::GeneratorRegistry;
use modules::project_creator::knowledge::DefaultKnowledgeBase;
use modules::project_creator::packs::DefaultPackRegistry;

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

            // === Plugins ===
            #[cfg(feature = "plugins")]
            {
                let ide_state = mini_ide::commands::IdeState::new();
                app.manage(ide_state);
            }

            // === Core services ===
            let settings_service: Arc<dyn core::settings::SettingsService> = Arc::new(
                core::settings::JsonSettingsService::new(data_dir.clone()),
            );

            // === Workspace module ===
            let os_pm = modules::workspace::process_manager::OsProcessManager::new();
            os_pm.set_app_handle(app.handle().clone());
            let process_manager: Arc<dyn modules::workspace::process_manager::ProcessManager> =
                Arc::new(os_pm);
            let workspace_state = modules::workspace::WorkspaceState::new(process_manager.clone());
            // === DevLauncher module ===
            let devlauncher_state = modules::devlauncher::DevLauncherState::new(
                data_dir.join("profiles"),
                Arc::new(modules::devlauncher::launch_engine::ProcessLaunchEngine::new(process_manager)),
                Arc::new(modules::devlauncher::analyzer::FsProjectAnalyzer),
            );

            // === ProjectCreator module ===
            let project_creator_state = modules::project_creator::ProjectCreatorState::new(
                Arc::new(DefaultProjectAnalyzer::new()),
                Arc::new(DefaultRecipeEngine::new()),
                Arc::new(GeneratorRegistry::new()),
                Arc::new(DefaultPackRegistry::new()),
                Arc::new(DefaultKnowledgeBase::new()),
            );

            // Register all states
            app.manage(devlauncher_state);
            app.manage(workspace_state);
            app.manage(project_creator_state);
            app.manage(core::settings::SettingsState(settings_service));

            // Auto-track process errors in the session
            let handle = app.handle().clone();
            app.listen("process-status", move |event| {
                if let Ok(payload) = serde_json::from_str::<modules::workspace::models::ProcessStatusEvent>(event.payload()) {
                    let is_error = match &payload.status {
                        modules::workspace::models::ProcessStatus::Crashed => true,
                        modules::workspace::models::ProcessStatus::Exited(c) if *c != 0 => true,
                        _ => false,
                    };
                    if is_error {
                        if let Some(session) = handle.try_state::<modules::workspace::WorkspaceState>() {
                            session.inner().session.increment_errors();
                        }
                    }
                }
            });

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
            modules::workspace::commands::get_process_logs,
            modules::workspace::commands::set_current_project,
            modules::workspace::commands::get_current_project,
            modules::workspace::commands::clear_current_project,
            modules::workspace::commands::get_session_info,
            modules::workspace::commands::open_project_from_path,
            modules::workspace::commands::list_directory,
            modules::workspace::commands::read_file,
            modules::workspace::commands::write_file,
            modules::workspace::commands::open_in_vscode,
            modules::workspace::commands::get_workspace_overview,
            modules::workspace::commands::get_problems,
            modules::workspace::commands::clear_problems,
            // ProjectCreator commands
            modules::project_creator::commands::ping_project_creator,
            modules::project_creator::commands::get_wizard_tree,
            modules::project_creator::commands::get_project_types,
            modules::project_creator::commands::start_wizard,
            modules::project_creator::commands::submit_wizard_answer,
            modules::project_creator::commands::analyze_project_technologies,
            modules::project_creator::commands::preview_project_recipe,
            modules::project_creator::commands::start_project_execution,
            // Core commands
            core::settings::get_settings,
            core::settings::update_settings,
            core::settings::reset_settings,
            // Plugin commands
            #[cfg(feature = "plugins")]
            mini_ide::commands::get_completions,
            #[cfg(feature = "plugins")]
            mini_ide::commands::get_diagnostics,
            #[cfg(feature = "plugins")]
            mini_ide::commands::get_hover,
            #[cfg(feature = "plugins")]
            mini_ide::commands::go_to_definition,
            #[cfg(feature = "plugins")]
            mini_ide::commands::format_code,
        ])
        .run(tauri::generate_context!())
        .expect("Error starting Tauri application");
}
