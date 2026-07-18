use std::path::PathBuf;
use std::fs;
use std::sync::Arc;
use tauri::State;

use crate::core::models::*;

pub trait SettingsService: Send + Sync {
    fn get_settings(&self) -> Result<AppSettings, String>;
    fn update_settings(&self, settings: &AppSettings) -> Result<(), String>;
    fn reset_to_defaults(&self) -> Result<AppSettings, String>;
}

pub struct JsonSettingsService {
    settings_path: PathBuf,
}

impl JsonSettingsService {
    pub fn new(app_data_dir: PathBuf) -> Self {
        Self {
            settings_path: app_data_dir.join("settings.json"),
        }
    }

    fn load(&self) -> Result<AppSettings, String> {
        let content = fs::read_to_string(&self.settings_path)
            .map_err(|e| format!("Failed to read settings: {}", e))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse settings: {}", e))
    }

    fn save(&self, settings: &AppSettings) -> Result<(), String> {
        let content = serde_json::to_string_pretty(settings)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;
        fs::write(&self.settings_path, content)
            .map_err(|e| format!("Failed to write settings: {}", e))
    }

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

// ===== State wrapper + Tauri commands =====

/// Обёртка для регистрации в Tauri State
pub struct SettingsState(pub Arc<dyn SettingsService>);

#[tauri::command]
pub fn get_settings(state: State<'_, SettingsState>) -> Result<AppSettings, String> {
    state.0.get_settings()
}

#[tauri::command]
pub fn update_settings(state: State<'_, SettingsState>, settings: AppSettings) -> Result<(), String> {
    state.0.update_settings(&settings)
}

#[tauri::command]
pub fn reset_settings(state: State<'_, SettingsState>) -> Result<AppSettings, String> {
    state.0.reset_to_defaults()
}
