// ============================================================
// Tauri-команды приложения.
//
// Каждая команда — тонкая обёртка: получает состояние (State),
// вызывает метод нужного модуля, возвращает результат.
//
// Команды НЕ содержат бизнес-логики.
// Вся логика — в модулях (profile_manager, launch_engine, analyzer).
// ============================================================

use std::sync::Arc;
use tauri::State;

use crate::models::*;
use crate::profile_manager::ProfileManager;
use crate::launch_engine::LaunchEngine;
use crate::analyzer::ProjectAnalyzer;
use crate::settings::SettingsService;

// --------------------------------------------------
// AppState — глобальное состояние приложения.
//
// Tauri внедряет его в команды через State<'_, AppState>.
// Arc<dyn Trait> позволяет подменять реализации
// (например, для тестов или разных платформ).
// --------------------------------------------------
pub struct AppState {
    pub profile_manager: Arc<dyn ProfileManager>,
    pub launch_engine: Arc<dyn LaunchEngine>,
    pub analyzer: Arc<dyn ProjectAnalyzer>,
    pub settings: Arc<dyn SettingsService>,
}

// ===== Системные команды =====

/// Проверка связи с Rust-бэкендом
#[tauri::command]
pub fn ping_rust() -> String {
    "pong".into()
}

/// Получить демо-профиль (для ознакомления с UI)
#[tauri::command]
pub fn get_demo_profile() -> LaunchProfile {
    LaunchProfile {
        name: "демо-проект".into(),
        description: "Пример профиля для веб-разработки".into(),
        project_path: None,
        actions: vec![
            LaunchAction {
                id: generate_id(),
                label: "Запустить Docker".into(),
                enabled: true,
                action_type: ActionType::RunCommand {
                    command: "docker compose up -d".into(),
                    working_dir: None,
                },
            },
            LaunchAction {
                id: generate_id(),
                label: "Запустить бэкенд".into(),
                enabled: true,
                action_type: ActionType::RunCommand {
                    command: "npm run dev".into(),
                    working_dir: Some("./backend".into()),
                },
            },
            LaunchAction {
                id: generate_id(),
                label: "Открыть Swagger".into(),
                enabled: true,
                action_type: ActionType::OpenUrl {
                    url: "http://localhost:3000/swagger".into(),
                },
            },
        ],
    }
}

/// Генерация простого ID на основе времени
fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("act_{}", nanos)
}

// ===== ProfileManager команды =====

/// Получить список всех сохранённых профилей
#[tauri::command]
pub fn list_profiles(state: State<'_, AppState>) -> Result<Vec<LaunchProfile>, String> {
    state.profile_manager.list_profiles()
}

/// Получить один профиль по имени
#[tauri::command]
pub fn get_profile(state: State<'_, AppState>, name: String) -> Result<LaunchProfile, String> {
    state.profile_manager.get_profile(&name)
}

/// Сохранить профиль (создать или перезаписать)
#[tauri::command]
pub fn save_profile(state: State<'_, AppState>, profile: LaunchProfile) -> Result<(), String> {
    state.profile_manager.save_profile(&profile)
}

/// Удалить профиль по имени
#[tauri::command]
pub fn delete_profile(state: State<'_, AppState>, name: String) -> Result<(), String> {
    state.profile_manager.delete_profile(&name)
}

// ===== LaunchEngine команды =====

/// Выполнить одно действие
#[tauri::command]
pub fn execute_action(
    state: State<'_, AppState>,
    action: LaunchAction,
) -> Result<ActionStatus, String> {
    state.launch_engine.execute_action(&action)
}

// ===== Settings команды =====

/// Получить текущие настройки приложения
#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    state.settings.get_settings()
}

/// Обновить настройки приложения
#[tauri::command]
pub fn update_settings(
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<(), String> {
    state.settings.update_settings(&settings)
}

/// Сбросить настройки на значения по умолчанию
#[tauri::command]
pub fn reset_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    state.settings.reset_to_defaults()
}

// ===== Analyzer команды =====

/// Проанализировать проект и получить предложенный профиль
#[tauri::command]
pub fn analyze_project(
    state: State<'_, AppState>,
    path: String,
) -> Result<LaunchProfile, String> {
    state.analyzer.analyze(&path)
}
