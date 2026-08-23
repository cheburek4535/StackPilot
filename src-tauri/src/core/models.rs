use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreferredApp {
    pub name: String,
    pub path: String,
    pub args: Option<String>,
}

/// AI provider preset (dead configuration — the AI layer itself is not
/// implemented yet, but the connection contract is captured here).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSettings {
    pub enabled: bool,
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub temperature: f64,
    pub max_tokens: u32,
    pub timeout_secs: u32,
    pub system_prompt: String,
    /// Future: run the assistant against the context of the current page.
    pub page_context: bool,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "openai".into(),
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
            temperature: 0.7,
            max_tokens: 2048,
            timeout_secs: 30,
            system_prompt: String::new(),
            page_context: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalSettings {
    pub name: String,
    pub username: String,
    pub email: String,
}

impl Default for PersonalSettings {
    fn default() -> Self {
        Self {
            name: String::new(),
            username: String::new(),
            email: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppSettings {
    pub vscode_path: String,
    pub browser_path: String,
    pub terminal: String,
    pub theme: String,
    pub language: String,
    pub auto_save_profiles: bool,
    pub preferred_apps: Vec<PreferredApp>,

    /// Persist settings immediately on every change (top-level switch).
    pub auto_save: bool,
    /// "sm" | "md" | "lg" — applied to the root font size.
    pub font_size: String,
    pub reduced_motion: bool,
    pub show_interface_hints: bool,
    /// Accent color: preset key ("violet", "green", ...) or "#rrggbb".
    pub accent_color: String,
    /// Restore the last visited route on app start.
    pub restore_last_route: bool,
    /// Ask for confirmation before a factory reset.
    pub confirm_before_reset: bool,

    pub personal: PersonalSettings,
    pub ai: AiSettings,
}