use crate::core::settings::SettingsState;
use crate::modules::devlauncher::models::*;
use crate::modules::devlauncher::DevLauncherState;
use crate::modules::project_creator::models::WizardContext;
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
                    persistent: Some(true),
                },
            },
            LaunchAction {
                id: generate_id(),
                label: "Start Backend".into(),
                enabled: true,
                action_type: ActionType::RunCommand {
                    command: "npm run dev".into(),
                    working_dir: Some("./backend".into()),
                    persistent: Some(true),
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
        environment_binding_id: None,
        preferred_ide: Some(PreferredIde::Vscode),
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
///
/// If `environment_binding_id` is provided, the binding is resolved into
/// an EnvironmentOverlay and applied to the action. When absent, the
/// host environment is used (backward compatible).
#[tauri::command]
pub async fn execute_action(
    state: State<'_, DevLauncherState>,
    workspace: State<'_, WorkspaceState>,
    action: LaunchAction,
    session_id: Option<String>,
    environment_binding_id: Option<String>,
) -> Result<ActionStatus, String> {
    let engine = Arc::clone(&state.launch_engine);
    let mut action = action;
    resolve_working_dir(&mut action, &workspace);
    // Процессы, запущенные из профиля, привязываются к текущей сессии —
    // иначе таймер сессии никогда не увидит их завершения.
    let session_id = session_id.or_else(|| workspace.session.get_session().map(|s| s.started_at));
    let session_id_for_link = session_id.clone();

    // Resolve environment overlay from binding if provided
    let overlay = if let Some(ref binding_id) = environment_binding_id {
        if let Some(ref svc) = state.binding_service {
            match svc.get(binding_id) {
                Ok(binding) => {
                    let (ov, _diagnostics) =
                        crate::modules::project_environment::resolver::resolve_with_diagnostics(
                            &binding,
                        );
                    Some(ov)
                }
                Err(e) => {
                    // Binding not found — log warning but continue with host env
                    eprintln!(
                        "Warning: environment binding '{}' not found: {}. Using host environment.",
                        binding_id, e
                    );
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Clone overlay for the blocking task (EnvironmentOverlay is Clone)
    let overlay_for_task = overlay.clone();

    let (status, proc_id) = tauri::async_runtime::spawn_blocking(move || {
        engine.execute_action(&action, session_id, overlay_for_task.as_ref())
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
    settings: State<'_, SettingsState>,
    path: String,
) -> Result<LaunchProfile, String> {
    let analyzer = Arc::clone(&state.analyzer);
    let vscode_path = settings.0.get_settings().ok().map(|s| s.vscode_path);
    tauri::async_runtime::spawn_blocking(move || {
        analyzer.analyze(&path, vscode_path.as_deref())
    })
    .await
    .map_err(|e| format!("Analyze task failed: {e}"))?
}

/// Launch the preferred IDE for a project. Returns true if the IDE was
/// launched, false if no IDE is available.
#[tauri::command]
pub async fn launch_ide(
    state: State<'_, DevLauncherState>,
    settings: State<'_, SettingsState>,
    ide: PreferredIde,
    project_path: Option<String>,
) -> Result<bool, String> {
    let engine = Arc::clone(&state.launch_engine);
    let path = project_path;
    let vscode_path = settings.0.get_settings().ok().map(|s| s.vscode_path);
    tauri::async_runtime::spawn_blocking(move || {
        engine.launch_ide(&ide, path.as_deref(), None, vscode_path.as_deref())
    })
    .await
    .map_err(|e| format!("Launch IDE task failed: {e}"))?
}

/// Launch the preferred IDE for the current workspace project, then execute
/// all enabled actions in the profile. This is the "Run Project" entry point.
#[tauri::command]
pub async fn run_profile(
    state: State<'_, DevLauncherState>,
    workspace: State<'_, WorkspaceState>,
    settings: State<'_, SettingsState>,
    profile: LaunchProfile,
    session_id: Option<String>,
    environment_binding_id: Option<String>,
) -> Result<Vec<(String, ActionStatus)>, String> {
    let engine = Arc::clone(&state.launch_engine);
    let mut results: Vec<(String, ActionStatus)> = Vec::new();

    // 1. Launch the preferred IDE first so the user sees it open immediately.
    let vscode_path = settings.0.get_settings().ok().map(|s| s.vscode_path);
    if let Some(ide) = &profile.preferred_ide {
        let project_path = profile
            .project_path
            .clone()
            .or_else(|| workspace.project.get_current().and_then(|c| c.project_path));
        let launched = engine
            .launch_ide(
                ide,
                project_path.as_deref(),
                None,
                vscode_path.as_deref(),
            )
            .unwrap_or(false);
        results.push((
            format!("ide:{}", ide.cli_name()),
            if launched {
                ActionStatus::Success {
                    message: format!("{} launched", ide.label()),
                }
            } else {
                ActionStatus::Skipped {
                    reason: format!("{} not found", ide.label()),
                }
            },
        ));
    }

    // 2. Execute all enabled actions sequentially.
    let session_id_for_link = session_id
        .clone()
        .or_else(|| workspace.session.get_session().map(|s| s.started_at));

    // Resolve environment overlay once for all actions.
    let overlay = if let Some(ref binding_id) = environment_binding_id {
        if let Some(ref svc) = state.binding_service {
            match svc.get(binding_id) {
                Ok(binding) => {
                    let (ov, _diag) =
                        crate::modules::project_environment::resolver::resolve_with_diagnostics(
                            &binding,
                        );
                    Some(ov)
                }
                Err(e) => {
                    eprintln!(
                        "Warning: environment binding '{}' not found: {}. Using host environment.",
                        binding_id, e
                    );
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    for action in &profile.actions {
        if !action.enabled {
            results.push((
                action.id.clone(),
                ActionStatus::Skipped {
                    reason: format!("Action '{}' disabled", action.label),
                },
            ));
            continue;
        }

        let mut action = action.clone();
        let action_id = action.id.clone();
        let action_label = action.label.clone();
        resolve_working_dir(&mut action, &workspace);
        let ov = overlay.clone();
        let session = session_id_for_link.clone();
        let engine = Arc::clone(&engine);
        // A single failing action must not abort the rest of the profile:
        // record its error as a Failed status and continue.
        let (status, proc_id) =
            match tauri::async_runtime::spawn_blocking(move || {
                engine.execute_action(&action, session, ov.as_ref())
            })
            .await
            {
                Ok(Ok(res)) => res,
                Ok(Err(err)) => (
                    ActionStatus::Failed {
                        error: format!("Action '{}' failed: {}", action_label, err),
                    },
                    None,
                ),
                Err(err) => (
                    ActionStatus::Failed {
                        error: format!("Action '{}' task crashed: {}", action_label, err),
                    },
                    None,
                ),
            };

        if let Some(proc_id) = proc_id {
            if session_id_for_link.is_some() {
                workspace.session.link_process(&proc_id);
            }
        }
        results.push((action_id, status));
    }

    Ok(results)
}

/// Builds a LaunchProfile from WizardContext and saves it. This is the
/// integration entry point: Project Creator calls this after execution
/// completes so the profile is ready in DevLauncher without filesystem
/// analysis. If a profile with the same project_path already exists,
/// it is updated in place (preserving the name) instead of creating
/// a duplicate.
#[tauri::command]
pub fn build_profile_from_context(
    state: State<'_, DevLauncherState>,
    context: WizardContext,
) -> Result<LaunchProfile, String> {
    let mut profile = super::profile_builder::build_profile_from_context(&context);

    // Check if a profile already exists for this project path
    if let Some(ref path) = profile.project_path {
        if let Some(existing) = state.profile_manager.find_by_project_path(path) {
            // Update existing profile: keep its name, overwrite the rest
            profile.name = existing.name;
            state.profile_manager.save_profile(&profile)?;
            return Ok(profile);
        }
    }

    state.profile_manager.save_profile(&profile)?;
    Ok(profile)
}

/// Start watching a project directory for source file changes.
/// Emits `devlauncher:file_changed` events when source files are modified.
#[tauri::command]
pub fn start_file_watcher(
    state: State<'_, DevLauncherState>,
    app: tauri::AppHandle,
    path: String,
) -> Result<(), String> {
    state.file_watcher.start(std::path::PathBuf::from(&path), app)
}

/// Stop watching the current project directory.
#[tauri::command]
pub fn stop_file_watcher(state: State<'_, DevLauncherState>) -> Result<(), String> {
    state.file_watcher.stop();
    Ok(())
}

/// Check if the file watcher is currently active.
#[tauri::command]
pub fn is_file_watching(state: State<'_, DevLauncherState>) -> bool {
    state.file_watcher.is_watching()
}
