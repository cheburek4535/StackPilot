use crate::modules::devlauncher::models::*;
use crate::modules::devlauncher::DevLauncherState;
use tauri::State;

#[tauri::command]
pub fn ping_rust() -> String {
    "pong".into()
}

#[tauri::command]
pub fn get_demo_profile() -> LaunchProfile {
    LaunchProfile {
        name: "Demo Project".into(),
        description: "Example profile for web development".into(),
        project_path: None,
        actions: vec![
            LaunchAction {
                id: generate_id(),
                label: "Start Docker".into(),
                enabled: true,
                action_type: ActionType::RunCommand {
                    command: "docker compose up -d".into(),
                    working_dir: None,
                },
            },
            LaunchAction {
                id: generate_id(),
                label: "Start Backend".into(),
                enabled: true,
                action_type: ActionType::RunCommand {
                    command: "npm run dev".into(),
                    working_dir: Some("./backend".into()),
                },
            },
            LaunchAction {
                id: generate_id(),
                label: "Open Swagger".into(),
                enabled: true,
                action_type: ActionType::OpenUrl {
                    url: "http://localhost:3000/swagger".into(),
                },
            },
        ],
    }
}

fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("act_{}", nanos)
}

#[tauri::command]
pub fn list_profiles(state: State<'_, DevLauncherState>) -> Result<Vec<LaunchProfile>, String> {
    state.profile_manager.list_profiles()
}

#[tauri::command]
pub fn get_profile(
    state: State<'_, DevLauncherState>,
    name: String,
) -> Result<LaunchProfile, String> {
    state.profile_manager.get_profile(&name)
}

#[tauri::command]
pub fn save_profile(
    state: State<'_, DevLauncherState>,
    profile: LaunchProfile,
) -> Result<(), String> {
    state.profile_manager.save_profile(&profile)
}

#[tauri::command]
pub fn delete_profile(state: State<'_, DevLauncherState>, name: String) -> Result<(), String> {
    state.profile_manager.delete_profile(&name)
}

#[tauri::command]
pub fn execute_action(
    state: State<'_, DevLauncherState>,
    action: LaunchAction,
    session_id: Option<String>,
) -> Result<ActionStatus, String> {
    state.launch_engine.execute_action(&action, session_id)
}

#[tauri::command]
pub fn analyze_project(
    state: State<'_, DevLauncherState>,
    path: String,
) -> Result<LaunchProfile, String> {
    state.analyzer.analyze(&path)
}
