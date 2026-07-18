use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreferredApp {
    pub name: String,
    pub path: String,
    pub args: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub vscode_path: String,
    pub browser_path: String,
    pub terminal: String,
    pub theme: String,
    pub language: String,
    pub auto_save_profiles: bool,
    pub preferred_apps: Vec<PreferredApp>,
}
