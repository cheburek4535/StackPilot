// ============================================================
// Модели данных DevLauncher
//
// Все структуры данных, которыми обмениваются фронтенд и бэкенд.
// Сериализация/десериализация JSON — через serde.
// ============================================================

use serde::{Deserialize, Serialize};

// --------------------------------------------------
// ActionType — тип действия в профиле запуска.
//
// Rust-перечисление (enum). В JSON превращается в:
//   {"RunCommand": {"command": "...", "working_dir": null}}
// --------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionType {
    /// Запустить команду в терминале
    RunCommand {
        command: String,
        working_dir: Option<String>,
    },
    /// Открыть приложение (VS Code, браузер и т.д.)
    OpenApplication {
        path: String,
        args: Option<String>,
    },
    /// Открыть URL в браузере
    OpenUrl {
        url: String,
    },
    /// Ждать, пока URL станет доступен (HTTP GET)
    WaitForUrl {
        url: String,
        timeout_secs: u64,
    },
    /// Ждать, пока TCP-порт откроется
    WaitForPort {
        host: String,
        port: u16,
        timeout_secs: u64,
    },
    /// Подождать указанное количество секунд
    Delay {
        seconds: u64,
    },
    /// Выполнить произвольный скрипт
    ExecuteScript {
        script: String,
        shell: Option<String>,
    },
}

// --------------------------------------------------
// LaunchAction — одно действие в профиле.
//
// У каждого действия есть:
//   • id      — уникальный идентификатор
//   • label   — человекочитаемое название
//   • enabled — включено/выключено
//   • action_type — что именно делать
// --------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchAction {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub action_type: ActionType,
}

// --------------------------------------------------
// LaunchProfile — профиль запуска проекта.
//
// Упорядоченный список действий для запуска проекта.
// Сохраняется в JSON-файл.
// --------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchProfile {
    pub name: String,
    pub description: String,
    pub project_path: Option<String>,
    pub actions: Vec<LaunchAction>,
}

// --------------------------------------------------
// ActionStatus — результат выполнения одного действия.
//
// Возвращается из LaunchEngine на фронтенд.
// --------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionStatus {
    Success {
        message: String,
    },
    Failed {
        error: String,
    },
    Skipped {
        reason: String,
    },
}

// --------------------------------------------------
// PreferredApp — предпочитаемое приложение.
//
// Используется в настройках: список приложений,
// которые DevLauncher может открывать.
// --------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreferredApp {
    pub name: String,
    pub path: String,
    pub args: Option<String>,
}

// --------------------------------------------------
// AppSettings — глобальные настройки приложения.
//
// Хранятся в JSON-файле {app_data}/settings.json.
// --------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// Путь к VS Code (или просто "code" если в PATH)
    pub vscode_path: String,
    /// Путь к браузеру (пусто = системный по умолчанию)
    pub browser_path: String,
    /// Терминал (пусто = системный по умолчанию)
    pub terminal: String,
    /// Тема: "system" | "light" | "dark"
    pub theme: String,
    /// Язык интерфейса: "ru" | "en"
    pub language: String,
    /// Автоматически сохранять профили при изменении
    pub auto_save_profiles: bool,
    /// Список предпочитаемых приложений
    pub preferred_apps: Vec<PreferredApp>,
}
