use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionType {
    RunCommand { command: String, working_dir: Option<String> },
    OpenApplication { path: String, args: Option<String> },
    OpenUrl { url: String },
    WaitForUrl { url: String, timeout_secs: u64 },
    WaitForPort { host: String, port: u16, timeout_secs: u64 },
    Delay { seconds: u64 },
    ExecuteScript { script: String, shell: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchAction {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub action_type: ActionType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchProfile {
    pub name: String,
    pub description: String,
    pub project_path: Option<String>,
    pub actions: Vec<LaunchAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionStatus {
    Success { message: String },
    Failed { error: String },
    Skipped { reason: String },
}
