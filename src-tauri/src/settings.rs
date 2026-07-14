// ============================================================
// SettingsService — управление настройками приложения.
//
// Настройки хранятся в одном JSON-файле {app_data}/settings.json
// и содержат глобальные параметры DevLauncher.
//
// # Архитектура
//
// SettingsService — трейт (интерфейс), чтобы можно было
// подменять реализацию (тесты, другой формат хранения).
//
// JsonSettingsService — конкретная реализация:
//   • читает/пишет JSON-файл
//   • при отсутствии файла создаёт с настройками по умолчанию
//   • не валидирует значения (пока)
// ============================================================

use std::path::PathBuf;
use std::fs;

use crate::models::*;

// --------------------------------------------------
// Трейт SettingsService
// --------------------------------------------------
pub trait SettingsService: Send + Sync {
    /// Получить текущие настройки.
    /// Если файла нет — возвращает настройки по умолчанию.
    fn get_settings(&self) -> Result<AppSettings, String>;

    /// Сохранить настройки (полная замена).
    fn update_settings(&self, settings: &AppSettings) -> Result<(), String>;

    /// Сбросить настройки на значения по умолчанию.
    fn reset_to_defaults(&self) -> Result<AppSettings, String>;
}

// --------------------------------------------------
// JsonSettingsService
// --------------------------------------------------
pub struct JsonSettingsService {
    /// Путь к файлу settings.json
    settings_path: PathBuf,
}

impl JsonSettingsService {
    /// Создаёт сервис, работающий с файлом {app_data_dir}/settings.json
    pub fn new(app_data_dir: PathBuf) -> Self {
        Self {
            settings_path: app_data_dir.join("settings.json"),
        }
    }

    /// Прочитать настройки из файла
    fn load(&self) -> Result<AppSettings, String> {
        let content = fs::read_to_string(&self.settings_path)
            .map_err(|e| format!("Не удалось прочитать файл настроек: {}", e))?;

        serde_json::from_str(&content)
            .map_err(|e| format!("Ошибка парсинга файла настроек: {}", e))
    }

    /// Записать настройки в файл
    fn save(&self, settings: &AppSettings) -> Result<(), String> {
        let content = serde_json::to_string_pretty(settings)
            .map_err(|e| format!("Ошибка сериализации настроек: {}", e))?;

        fs::write(&self.settings_path, content)
            .map_err(|e| format!("Не удалось записать файл настроек: {}", e))
    }

    /// Настройки по умолчанию
    fn defaults() -> AppSettings {
        AppSettings {
            vscode_path: "code".into(),
            browser_path: String::new(),
            terminal: String::new(),
            theme: "system".into(),
            language: "ru".into(),
            auto_save_profiles: true,
            preferred_apps: vec![
                PreferredApp {
                    name: "VS Code".into(),
                    path: "code".into(),
                    args: None,
                },
            ],
        }
    }
}

impl SettingsService for JsonSettingsService {
    fn get_settings(&self) -> Result<AppSettings, String> {
        // Если файла нет — создаём с настройками по умолчанию
        if !self.settings_path.exists() {
            let defaults = Self::defaults();
            self.save(&defaults)?;
            return Ok(defaults);
        }
        self.load()
    }

    fn update_settings(&self, settings: &AppSettings) -> Result<(), String> {
        self.save(settings)
    }

    fn reset_to_defaults(&self) -> Result<AppSettings, String> {
        let defaults = Self::defaults();
        self.save(&defaults)?;
        Ok(defaults)
    }
}
