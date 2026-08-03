// ============================================================
// Toolchain Manager — точка входа модуля
// ============================================================
// Модуль управляет локальным окружением разработчика:
//  1. узнаёт, что установлено и какие версии (Discovery/Version);
//  2. собирает план установки недостающего (Planner/Installer);
//  3. следит за PATH, диском и здоровьем окружения (Path/Health);
//  4. хранит локальную мета-информацию (Metadata).
//
// Модуль самодостаточен: не зависит от project_creator, devlauncher
// и workspace. Интеграция с ними — только через фронтенд и события,
// бэкенд соседних модулей не трогается.

pub mod commands;
pub mod core;
pub mod defs;
pub mod models;
pub mod platforms;

use models::*;

/// Глобальное состояние модуля Toolchain Manager.
/// Регистрируется в Tauri State (как ProjectCreatorState).
pub struct ToolchainState {
    definitions: Vec<ToolDefinition>,
}

impl ToolchainState {
    pub fn new() -> Self {
        let definitions = defs::load_definitions();

        // Дубликаты id ломают lookup по id — это ошибка разработчика,
        // падаем громко и сразу, а не тихо при первом обращении.
        let warnings = defs::validate(&definitions);
        let has_duplicates = warnings.iter().any(|w| w.starts_with("Дубликат"));
        assert!(
            !has_duplicates,
            "tools.json содержит дубликаты id: {:?}",
            warnings
        );

        Self { definitions }
    }

    /// Все определения инструментов (для фронтенда и сервисов).
    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }

    /// Найти определение по id.
    pub fn get_definition(&self, id: &str) -> Option<&ToolDefinition> {
        self.definitions.iter().find(|d| d.id == id)
    }

    /// Информация об ОС и количестве известных инструментов.
    pub fn environment_info(&self) -> EnvironmentInfo {
        EnvironmentInfo {
            os: std::env::consts::OS.to_string(),
            os_version: String::new(), // наполняется на этапе 5 системными командами
            package_managers: platforms::current_platform().package_managers(),
            tool_count: self.definitions.len(),
        }
    }
}
