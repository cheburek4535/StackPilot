mod core;
mod modules;
pub mod platform;

use std::sync::Arc;
use tauri::{Listener, Manager};

#[cfg(feature = "plugins")]
use modules::plugins::mini_ide;
use modules::project_creator::analysis::DefaultProjectAnalyzer;
use modules::project_creator::engine::DefaultRecipeEngine;
use modules::project_creator::generators::GeneratorRegistry;
use modules::project_creator::knowledge::DefaultKnowledgeBase;
use modules::project_creator::packs::DefaultPackRegistry;
use modules::toolchain::ToolchainState;
use modules::workspace::session::SessionService;

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

            std::fs::create_dir_all(&data_dir).expect("Failed to create data dir");

            std::fs::create_dir_all(data_dir.join("profiles"))
                .expect("Failed to create profiles dir");

            // Project environments directory
            std::fs::create_dir_all(data_dir.join("project_environments"))
                .expect("Failed to create project_environments dir");

            // === Plugins ===
            #[cfg(feature = "plugins")]
            {
                let ide_state = mini_ide::commands::IdeState::new();
                app.manage(ide_state);
            }

            // === Core services ===
            let settings_service: Arc<dyn core::settings::SettingsService> =
                Arc::new(core::settings::JsonSettingsService::new(data_dir.clone()));

            // === Workspace module ===
            let os_pm = modules::workspace::process_manager::OsProcessManager::new();
            os_pm.set_app_handle(app.handle().clone());
            let process_manager: Arc<dyn modules::workspace::process_manager::ProcessManager> =
                Arc::new(os_pm);
            let workspace_state = modules::workspace::WorkspaceState::new(process_manager.clone());
            // === DevLauncher module ===
            let devlauncher_state = modules::devlauncher::DevLauncherState::new(
                data_dir.join("profiles"),
                Arc::new(
                    modules::devlauncher::launch_engine::ProcessLaunchEngine::new(process_manager),
                ),
                Arc::new(modules::devlauncher::analyzer::FsProjectAnalyzer),
            );

            // === ProjectEnvironment module ===
            let project_env_state = modules::project_environment::ProjectEnvironmentState::new(
                data_dir.join("project_environments"),
            );

            // Wire up binding service to DevLauncher
            let devlauncher_state =
                devlauncher_state.with_binding_service(project_env_state.binding_service.clone());

            // === ProjectCreator module ===
            let project_creator_state = modules::project_creator::ProjectCreatorState::new(
                Arc::new(DefaultProjectAnalyzer::new()),
                Arc::new(DefaultRecipeEngine::new()),
                Arc::new(GeneratorRegistry::with_defaults()),
                Arc::new(DefaultPackRegistry::new()),
                Arc::new(DefaultKnowledgeBase::new()),
            );

            // === ToolchainManager module ===
            let toolchain_state = ToolchainState::new(data_dir.join("toolchain"));
            app.manage(toolchain_state);

            // Канонический движок заданий: глобальный приёмник событий
            // (toolchainx:job_event) регистрируется один раз на приложение.
            if let Some(tc_state) = app.try_state::<ToolchainState>() {
                modules::toolchain::commands::register_tcx_event_sink(
                    &tc_state,
                    app.handle().clone(),
                );
            }

            // Приложение стартует с PATH момента запуска — инструменты,
            // установленные в прошлой сессии (npm-global в %APPDATA%\npm,
            // winget, SDK), могут остаться вне него. Подтягиваем свежий
            // пользовательский PATH из системы, пока UI ещё инициализируется.
            tauri::async_runtime::spawn(async move {
                let _ = modules::toolchain::core::path_service::sync_process_path().await;
            });

            // Register all states
            app.manage(devlauncher_state);
            app.manage(workspace_state);
            app.manage(project_creator_state);
            app.manage(project_env_state);
            app.manage(core::settings::SettingsState(settings_service));

            // Auto-track process errors in the session
            let handle = app.handle().clone();
            app.listen("process-status", move |event| {
                if let Ok(payload) = serde_json::from_str::<
                    modules::workspace::models::ProcessStatusEvent,
                >(event.payload())
                {
                    let is_error = match &payload.status {
                        modules::workspace::models::ProcessStatus::Crashed => true,
                        modules::workspace::models::ProcessStatus::Exited(c) if *c != 0 => true,
                        _ => false,
                    };
                    if is_error {
                        if let Some(session) =
                            handle.try_state::<modules::workspace::WorkspaceState>()
                        {
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
            modules::devlauncher::commands::launch_ide,
            modules::devlauncher::commands::run_profile,
            modules::devlauncher::commands::build_profile_from_context,
            modules::devlauncher::commands::start_file_watcher,
            modules::devlauncher::commands::stop_file_watcher,
            modules::devlauncher::commands::is_file_watching,
            // Workspace commands
            modules::workspace::commands::spawn_process,
            modules::workspace::commands::list_processes,
            modules::workspace::commands::kill_process,
            modules::workspace::commands::refresh_process,
            modules::workspace::commands::get_process_logs,
            modules::workspace::commands::spawn_process_visible,
            modules::workspace::commands::launch_application_detached,
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
            modules::project_creator::commands::preview_project_files,
            modules::project_creator::commands::start_project_execution,
            modules::project_creator::commands::project_execution_snapshot,
            modules::project_creator::commands::check_project_folder_exists,
            modules::project_creator::commands::get_host_platform,
            modules::project_creator::commands::validate_project_stack,
            modules::project_creator::commands::validate_project_stack_error,
            modules::project_creator::commands::get_stack_recommendations,
            // ToolchainManager commands
            modules::toolchain::commands::ping_toolchain,
            modules::toolchain::commands::tc_get_tool_definitions,
            modules::toolchain::commands::tc_get_environment_info,
            modules::toolchain::commands::tc_check_environment,
            modules::toolchain::commands::tc_build_install_plan,
            modules::toolchain::commands::tc_run_install,
            modules::toolchain::commands::tc_get_install_status,
            modules::toolchain::commands::tc_abort_install,
            modules::toolchain::commands::tc_take_new_secrets,
            modules::toolchain::commands::tc_get_metadata,
            modules::toolchain::commands::tc_get_health_report,
            // ToolchainManager: read-only scan/diagnostics engine (tcx_*)
            modules::toolchain::commands::tcx_get_catalog,
            modules::toolchain::commands::tcx_get_environment_snapshot,
            modules::toolchain::commands::tcx_start_scan,
            modules::toolchain::commands::tcx_get_scan_job,
            modules::toolchain::commands::tcx_get_latest_scan_job,
            modules::toolchain::commands::tcx_cancel_scan,
            modules::toolchain::commands::tcx_get_tool_details,
            modules::toolchain::commands::tcx_run_health_checks,
            modules::toolchain::commands::tcx_uninstall_tool,
            modules::toolchain::commands::tcx_profile_resolve,
            // ToolchainManager: canonical job engine (tcx_*)
            modules::toolchain::commands::tcx_build_plan,
            modules::toolchain::commands::tcx_start_job,
            modules::toolchain::commands::tcx_get_job,
            modules::toolchain::commands::tcx_list_jobs,
            modules::toolchain::commands::tcx_cancel_job,
            modules::toolchain::commands::tcx_retry_job,
            modules::toolchain::commands::tcx_adopt_tool,
            // Core commands
            core::settings::get_settings,
            core::settings::update_settings,
            core::settings::reset_settings,
            core::settings::settings_check_path,
            core::settings::get_app_data_dir,
            // ProjectEnvironment commands
            modules::project_environment::commands::pe_list_bindings,
            modules::project_environment::commands::pe_get_binding,
            modules::project_environment::commands::pe_save_binding,
            modules::project_environment::commands::pe_delete_binding,
            modules::project_environment::commands::pe_find_binding_for_project,
            modules::project_environment::commands::pe_validate_binding,
            modules::project_environment::commands::pe_resolve_overlay,
            modules::project_environment::commands::pe_create_binding,
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
