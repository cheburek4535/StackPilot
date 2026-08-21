use crate::modules::devlauncher::models::*;
use crate::modules::devlauncher::DevLauncherState;
use crate::modules::workspace::project::ProjectService;
use crate::modules::workspace::session::SessionService;
use crate::modules::workspace::WorkspaceState;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

/// Относительный working_dir действия (например "./backend") резолвится
/// относительно текущего проекта Workspace — иначе команда выполнится в
/// рабочей папке самого приложения и упадёт с "No such file or directory".
fn resolve_working_dir(action: &mut LaunchAction, workspace: &WorkspaceState) {
    let project_path = workspace
        .project
        .get_current()
        .and_then(|ctx| ctx.project_path);
    let ActionType::RunCommand { working_dir, .. } = &mut action.action_type else {
        return;
    };
    let Some(dir) = working_dir else { return };
    let Some(base) = project_path else { return };
    let path = Path::new(dir);
    if path.is_relative() {
        *dir = PathBuf::from(base)
            .join(path)
            .to_string_lossy()
            .into_owned();
    }
}

/// Выполняет действие профиля в фоне, НЕ на главном потоке.
///
/// Синхронные команды Tauri исполняются на главном потоке, а действия
/// профиля бывают блокирующими (Delay, WaitForPort/WaitForUrl до 60с,
/// ExecuteScript опрашивает процесс до завершения). Поэтому здесь команда
/// объявлена async и вся блокирующая работа уезжает в spawn_blocking —
/// UI остаётся отзывчивым во время долгих ожиданий.
#[tauri::command]
pub async fn execute_action(
    state: State<'_, DevLauncherState>,
    workspace: State<'_, WorkspaceState>,
    action: LaunchAction,
    session_id: Option<String>,
) -> Result<ActionStatus, String> {
    let engine = Arc::clone(&state.launch_engine);
    let mut action = action;
    resolve_working_dir(&mut action, &workspace);
    // Процессы, запущенные из профиля, привязываются к текущей сессии —
    // иначе таймер сессии никогда не увидит их завершения.
    let session_id =
        session_id.or_else(|| workspace.session.get_session().map(|s| s.started_at));
    let session_id_for_link = session_id.clone();

    let (status, proc_id) = tauri::async_runtime::spawn_blocking(move || {
        engine.execute_action(&action, session_id)
    })
    .await
    .map_err(|e| format!("Action task failed: {e}"))??;

    if let Some(proc_id) = proc_id {
        if session_id_for_link.is_some() {
            workspace.session.link_process(&proc_id);
        }
    }
    Ok(status)
}

#[tauri::command]
pub async fn analyze_project(
    state: State<'_, DevLauncherState>,
    path: String,
) -> Result<LaunchProfile, String> {
    let analyzer = Arc::clone(&state.analyzer);
    tauri::async_runtime::spawn_blocking(move || analyzer.analyze(&path))
        .await
        .map_err(|e| format!("Analyze task failed: {e}"))?
}
