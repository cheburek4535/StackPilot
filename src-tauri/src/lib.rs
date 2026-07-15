// ============================================================
// Точка сборки приложения DevLauncher.
//
// Здесь происходит "сборка" (wiring):
//   1. Объявляем модули (каждый в своём файле)
//   2. В setup() создаём экземпляры модулей
//   3. Регистрируем состояние (State) и команды
//   4. Запускаем Tauri
//
// Это единственное место, где все модули соединяются.
// Каждый модуль ничего не знает о других модулях.
// ============================================================

mod models;
mod profile_manager;
mod launch_engine;
mod analyzer;
mod commands;
mod settings;

use std::sync::Arc;
use tauri::Manager;
use commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // --- Плагины ---
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())

        // --- Настройка приложения ---
        .setup(|app| {
            // Получаем стандартную папку для данных приложения
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("Не удалось получить папку данных приложения");

            // Папка для JSON-файлов профилей
            let profiles_dir = data_dir.join("profiles");

            // Создаём папку, если её нет
            std::fs::create_dir_all(&profiles_dir)
                .expect("Не удалось создать папку профилей");

            // Создаём состояние приложения с реальными реализациями
            let app_state = AppState {
                profile_manager: Arc::new(
                    profile_manager::JsonProfileManager::new(profiles_dir),
                ),
                settings: Arc::new(
                    settings::JsonSettingsService::new(data_dir),
                ),
                launch_engine: Arc::new(launch_engine::ProcessLaunchEngine),
                analyzer: Arc::new(analyzer::FsProjectAnalyzer),
            };

            // Регистрируем состояние — оно будет доступно
            // во всех командах через State<'_, AppState>
            app.manage(app_state);

            Ok(())
        })

        // --- Команды ---
        .invoke_handler(tauri::generate_handler![
            commands::ping_rust,
            commands::get_demo_profile,
            commands::list_profiles,
            commands::get_profile,
            commands::save_profile,
            commands::delete_profile,
            commands::execute_action,
            commands::get_settings,
            commands::update_settings,
            commands::reset_settings,
            commands::analyze_project,
        ])

        // --- Запуск ---
        .run(tauri::generate_context!())
        .expect("Ошибка при запуске Tauri приложения");
}
