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

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use core::metadata::MetadataStore;
use models::*;

/// Глобальное состояние модуля Toolchain Manager.
/// Регистрируется в Tauri State (как ProjectCreatorState).
pub struct ToolchainState {
    definitions: Vec<ToolDefinition>,
    /// Живая сессия установки — обновляется фоновой задачей
    /// (Arc+Mutex, т.к. команда-run и команда-status живут отдельно).
    install_session: Arc<Mutex<Option<InstallSession>>>,
    /// Флаг отмены установки: tc_abort_install ставит его, installer
    /// опрашивает между задачами и в процессе стриминга (kill).
    abort_install: Arc<AtomicBool>,
    /// Постоянное состояние (state.json): установленные инструменты,
    /// секреты, настройки. Меняется по завершении установки.
    metadata: Arc<Mutex<MetadataStore>>,
}

impl ToolchainState {
    /// `dir` — каталог хранения состояния (app_data/toolchain).
    pub fn new(dir: PathBuf) -> Self {
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

        Self {
            definitions,
            install_session: Arc::new(Mutex::new(None)),
            abort_install: Arc::new(AtomicBool::new(false)),
            metadata: Arc::new(Mutex::new(MetadataStore::load(&dir))),
        }
    }

    /// Все определения инструментов (для фронтенда и сервисов).
    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }

    /// Найти определение по id.
    pub fn get_definition(&self, id: &str) -> Option<&ToolDefinition> {
        self.definitions.iter().find(|d| d.id == id)
    }

    /// Общая ссылка на сессию установки (для команд run/status).
    pub fn install_session(&self) -> Arc<Mutex<Option<InstallSession>>> {
        Arc::clone(&self.install_session)
    }

    /// Общий флаг отмены установки (для команд run/abort).
    pub fn abort_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.abort_install)
    }

    /// Общий доступ к хранилищу state.json (для команд и фоновой задачи).
    pub fn metadata(&self) -> Arc<Mutex<MetadataStore>> {
        Arc::clone(&self.metadata)
    }

    /// Информация об ОС и количестве известных инструментов.
    /// Версия ОС спрашивается у системы (быстрая команда с таймаутом).
    pub async fn environment_info(&self) -> EnvironmentInfo {
        EnvironmentInfo {
            os: std::env::consts::OS.to_string(),
            os_version: platforms::current_platform().os_version().await,
            package_managers: platforms::current_platform().package_managers(),
            tool_count: self.definitions.len(),
        }
    }
}
