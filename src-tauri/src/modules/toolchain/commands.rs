// ============================================================
// Tauri-команды Toolchain Manager
// ============================================================
// Команды вызываются с фронтенда через invoke(). Префикс tc_
// (toolchain) — чтобы имена не пересекались с другими модулями.
// На этапе 1 доступны «пассивные» команды: ping, список определений,
// информация об окружении. Проверка/установка появится с этапа 2.

use tauri::State;

use super::core;
use super::models::*;
use super::ToolchainState;

#[tauri::command]
pub fn ping_toolchain() -> Result<String, String> {
    Ok("ToolchainManager module is loaded".to_string())
}

/// Список всех известных инструментов (для отладки и будущей страницы окружения).
#[tauri::command]
pub fn tc_get_tool_definitions(state: State<'_, ToolchainState>) -> Vec<ToolDefinition> {
    state.definitions().to_vec()
}

/// Информация об ОС и менеджерах пакетов.
#[tauri::command]
pub fn tc_get_environment_info(state: State<'_, ToolchainState>) -> EnvironmentInfo {
    state.environment_info()
}

/// Проверка окружения под требования проекта (шаг «Environment» в мастере).
///
/// На вход — ProjectRequirements: фронтенд собирает его из WizardContext
/// (языки, фреймворки, тулы, флаги git/vscode/docker).
/// На выходе — EnvironmentCheck: статус каждого инструмента, объёмы
/// загрузки, нужны ли права администратора, всё ли готово к генерации.
#[tauri::command]
pub async fn tc_check_environment(
    state: State<'_, ToolchainState>,
    requirements: ProjectRequirements,
) -> Result<EnvironmentCheck, String> {
    let requested = core::requirements::resolve(&requirements);
    // free_space_mb = 0 — проверка диска появится на этапе 5 (disk.rs)
    let check = core::check::run_check(state.definitions(), &requested, 0).await;
    Ok(check)
}
