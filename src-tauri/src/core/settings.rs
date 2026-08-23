use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::State;

use crate::core::models::*;

pub trait SettingsService: Send + Sync {
    fn get_settings(&self) -> Result<AppSettings, String>;
    /// Validates and normalizes the given settings, persists them and
    /// returns the canonical (normalized) copy.
    fn update_settings(&self, settings: &AppSettings) -> Result<AppSettings, String>;
    fn reset_to_defaults(&self) -> Result<AppSettings, String>;
    fn app_data_dir(&self) -> PathBuf;
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
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse settings: {}", e))
    }

    /// Atomic write: serialize to a temp file first, then rename over the
    /// target. A crashed write can never leave a truncated settings.json.
    fn save(&self, settings: &AppSettings) -> Result<(), String> {
        let content = serde_json::to_string_pretty(settings)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;
        let parent = self
            .settings_path
            .parent()
            .ok_or_else(|| "Settings path has no parent directory".to_string())?;
        let tmp = parent.join("settings.json.tmp");
        fs::write(&tmp, &content).map_err(|e| format!("Failed to write settings: {}", e))?;
        fs::rename(&tmp, &self.settings_path)
            .map_err(|e| format!("Failed to commit settings: {}", e))
    }

    fn defaults() -> AppSettings {
        AppSettings {
            vscode_path: "code".into(),
            browser_path: String::new(),
            terminal: String::new(),
            theme: "dark".into(),
            language: "ru".into(),
            auto_save_profiles: true,
            preferred_apps: vec![PreferredApp {
                name: "VS Code".into(),
                path: "code".into(),
                args: None,
            }],
            auto_save: true,
            font_size: "md".into(),
            reduced_motion: false,
            show_interface_hints: true,
            accent_color: "violet".into(),
            restore_last_route: true,
            confirm_before_reset: true,
            personal: PersonalSettings::default(),
            ai: AiSettings::default(),
        }
    }

    fn is_accent_preset(value: &str) -> bool {
        matches!(
            value,
            "violet" | "blue" | "green" | "orange" | "red" | "cyan"
        )
    }

    fn is_valid_hex(value: &str) -> bool {
        let hex = value.trim_start_matches('#');
        hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())
    }

    /// Normalizes free-form values into canonical ones. Used both on
    /// `update` (persist the normalized form) and before returning data.
    fn normalize(mut s: AppSettings) -> AppSettings {
        s.vscode_path = s.vscode_path.trim().to_string();
        s.browser_path = s.browser_path.trim().to_string();
        s.terminal = s.terminal.trim().to_string();

        if !matches!(s.theme.as_str(), "system" | "light" | "dark") {
            s.theme = "dark".into();
        }
        if !matches!(s.language.as_str(), "ru" | "en") {
            s.language = "ru".into();
        }
        if !matches!(s.font_size.as_str(), "sm" | "md" | "lg") {
            s.font_size = "md".into();
        }

        let accent = s.accent_color.trim().to_string();
        s.accent_color = if Self::is_accent_preset(&accent) || Self::is_valid_hex(&accent) {
            accent
        } else {
            "violet".into()
        };

        s.personal.name = s.personal.name.trim().to_string();
        s.personal.username = s.personal.username.trim().to_string();
        s.personal.email = s.personal.email.trim().to_string();

        s.ai.provider = match s.ai.provider.as_str() {
            "openai" | "anthropic" | "ollama" | "custom" => s.ai.provider,
            _ => "custom".into(),
        };
        s.ai.base_url = s.ai.base_url.trim().to_string();
        s.ai.api_key = s.ai.api_key.trim().to_string();
        s.ai.model = s.ai.model.trim().to_string();
        s.ai.temperature = s.ai.temperature.clamp(0.0, 2.0);
        s.ai.max_tokens = s.ai.max_tokens.clamp(1, 128_000);
        s.ai.timeout_secs = s.ai.timeout_secs.clamp(5, 600);
        s.ai.system_prompt = s.ai.system_prompt.trim().to_string();

        s.preferred_apps.retain(|a| !a.name.trim().is_empty() || !a.path.trim().is_empty());
        for app in &mut s.preferred_apps {
            app.name = app.name.trim().to_string();
            app.path = app.path.trim().to_string();
            if let Some(args) = &app.args {
                app.args = Some(args.trim().to_string());
            }
        }

        s
    }

    /// Loads existing settings if present, otherwise creates defaults.
    fn load_or_default(&self) -> Result<AppSettings, String> {
        if !self.settings_path.exists() {
            let defaults = Self::defaults();
            self.save(&defaults)?;
            return Ok(defaults);
        }
        let loaded = self.load()?;
        Ok(Self::normalize(loaded))
    }
}

impl SettingsService for JsonSettingsService {
    fn get_settings(&self) -> Result<AppSettings, String> {
        self.load_or_default()
    }

    fn update_settings(&self, settings: &AppSettings) -> Result<AppSettings, String> {
        let normalized = Self::normalize(settings.clone());
        self.save(&normalized)?;
        Ok(normalized)
    }

    fn reset_to_defaults(&self) -> Result<AppSettings, String> {
        let defaults = Self::defaults();
        self.save(&defaults)?;
        Ok(defaults)
    }

    fn app_data_dir(&self) -> PathBuf {
        self.settings_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
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
pub fn update_settings(
    state: State<'_, SettingsState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    state.0.update_settings(&settings)
}

#[tauri::command]
pub fn reset_settings(state: State<'_, SettingsState>) -> Result<AppSettings, String> {
    state.0.reset_to_defaults()
}

/// True when a file or directory exists at the given path (empty → false).
/// Used by the UI to flag broken paths before they fail silently.
#[tauri::command]
pub fn settings_check_path(path: String) -> Result<bool, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }
    Ok(std::fs::metadata(trimmed).is_ok())
}

/// The app data folder (contains settings.json and profiles).
#[tauri::command]
pub fn get_app_data_dir(state: State<'_, SettingsState>) -> Result<String, String> {
    Ok(state.0.app_data_dir().to_string_lossy().into_owned())
}

// ===== Tests =====

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sp_settings_test_{}_{}",
            std::process::id(),
            name
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn creates_defaults_when_missing() {
        let dir = temp_dir("defaults");
        let svc = JsonSettingsService::new(dir.clone());
        let got = svc.get_settings().unwrap();
        assert_eq!(got.theme, "dark");
        assert_eq!(got.accent_color, "violet");
        assert!(got.auto_save);
        assert!(svc.settings_path.exists());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn normalizes_invalid_values_on_update() {
        let dir = temp_dir("normalize");
        let svc = JsonSettingsService::new(dir.clone());
        let mut s = svc.get_settings().unwrap();
        s.theme = "weird".into();
        s.font_size = "xxl".into();
        s.accent_color = "not-a-color".into();
        s.ai.temperature = 99.0;
        s.ai.max_tokens = 0;
        s.ai.provider = "nope".into();
        let got = svc.update_settings(&s).unwrap();
        assert_eq!(got.theme, "dark");
        assert_eq!(got.font_size, "md");
        assert_eq!(got.accent_color, "violet");
        assert_eq!(got.ai.temperature, 2.0);
        assert_eq!(got.ai.max_tokens, 1);
        assert_eq!(got.ai.provider, "custom");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn keeps_valid_accent_hex() {
        let dir = temp_dir("hex");
        let svc = JsonSettingsService::new(dir.clone());
        let mut s = svc.get_settings().unwrap();
        s.accent_color = "#12ab34".into();
        let got = svc.update_settings(&s).unwrap();
        assert_eq!(got.accent_color, "#12ab34");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn reset_restores_defaults() {
        let dir = temp_dir("reset");
        let svc = JsonSettingsService::new(dir.clone());
        let mut s = svc.get_settings().unwrap();
        s.theme = "light".into();
        s.preferred_apps.push(PreferredApp {
            name: "Custom".into(),
            path: "/bin/x".into(),
            args: None,
        });
        svc.update_settings(&s).unwrap();
        let reset = svc.reset_to_defaults().unwrap();
        assert_eq!(reset.theme, "dark");
        assert_eq!(reset.preferred_apps.len(), 1);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn atomic_save_leaves_no_temp_file() {
        let dir = temp_dir("atomic");
        let svc = JsonSettingsService::new(dir.clone());
        svc.get_settings().unwrap();
        assert!(!Path::new(&dir.join("settings.json.tmp")).exists());
        assert!(svc.settings_path.exists());
        fs::remove_dir_all(dir).ok();
    }
}