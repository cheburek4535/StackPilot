use crate::core::settings::SettingsState;
use crate::modules::workspace::file_explorer;
use crate::modules::workspace::file_explorer::FileExplorerService;
use crate::modules::workspace::models::*;
use crate::modules::workspace::overview::{OverviewData, OverviewService};
use crate::modules::workspace::problems::{Problem, ProblemsService};
use crate::modules::workspace::project::ProjectService;
use crate::modules::workspace::session;
use crate::modules::workspace::session::SessionService;
use crate::modules::workspace::WorkspaceState;
use std::sync::Arc;
use tauri::State;

// ===== Process commands =====

// Спавн процессов — блокирующая работа (резолв команд, создание pipe):
// async + spawn_blocking, чтобы не занимать главный поток Tauri.

#[tauri::command]
pub async fn spawn_process(
    state: State<'_, WorkspaceState>,
    command: String,
    args: Vec<String>,
    working_dir: Option<String>,
    label: String,
) -> Result<TrackedProcess, String> {
    let session_id = session::ensure_session(&state);
    let manager = Arc::clone(&state.process_manager);
    let proc = tauri::async_runtime::spawn_blocking(move || {
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        manager.spawn_and_track(
            &command,
            &args_refs,
            working_dir.as_deref(),
            &label,
            session_id,
        )
    })
    .await
    .map_err(|e| format!("Spawn task failed: {e}"))??;
    state.session.link_process(&proc.id);
    Ok(proc)
}

#[tauri::command]
pub fn list_processes(state: State<'_, WorkspaceState>) -> Vec<TrackedProcess> {
    state.process_manager.list()
}

#[tauri::command]
pub async fn kill_process(state: State<'_, WorkspaceState>, id: String) -> Result<(), String> {
    // Tree-kill может блокировать секунды (Unix: SIGTERM + 2s grace + SIGKILL):
    // вне главного потока, чтобы UI не замирал.
    let manager = Arc::clone(&state.process_manager);
    tauri::async_runtime::spawn_blocking(move || manager.kill(&id))
        .await
        .map_err(|e| format!("Kill task failed: {e}"))??;
    Ok(())
}

#[tauri::command]
pub async fn refresh_process(
    state: State<'_, WorkspaceState>,
    id: String,
) -> Result<ProcessStatus, String> {
    let manager = Arc::clone(&state.process_manager);
    tauri::async_runtime::spawn_blocking(move || manager.refresh_status(&id))
        .await
        .map_err(|e| format!("Refresh task failed: {e}"))?
}

#[tauri::command]
pub fn get_process_logs(
    state: State<'_, WorkspaceState>,
    id: String,
) -> Result<ProcessLogs, String> {
    state.process_manager.get_logs(&id)
}

/// Spawn a process in a new native terminal window (visible to the user).
#[tauri::command]
pub async fn spawn_process_visible(
    state: State<'_, WorkspaceState>,
    command: String,
    args: Vec<String>,
    working_dir: Option<String>,
    label: String,
) -> Result<TrackedProcess, String> {
    let session_id = session::ensure_session(&state);
    let manager = Arc::clone(&state.process_manager);
    let proc = tauri::async_runtime::spawn_blocking(move || {
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        manager.spawn_visible(
            &command,
            &args_refs,
            working_dir.as_deref(),
            &label,
            session_id,
            None,
        )
    })
    .await
    .map_err(|e| format!("Spawn task failed: {e}"))??;
    state.session.link_process(&proc.id);
    Ok(proc)
}

/// Launch a GUI application detached (IDE, browser, Docker Desktop).
#[tauri::command]
pub fn launch_application_detached(
    state: State<'_, WorkspaceState>,
    command: String,
    args: Vec<String>,
    working_dir: Option<String>,
) -> Result<(), String> {
    let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    state
        .process_manager
        .launch_detached(&command, &args_refs, working_dir.as_deref())
}

// ===== Project context commands =====

#[tauri::command]
pub fn set_current_project(
    state: State<'_, WorkspaceState>,
    profile_name: String,
    project_path: Option<String>,
    description: String,
    stack: Vec<String>,
) -> ProjectContext {
    let ctx = ProjectContext {
        profile_name,
        project_path,
        description,
        stack,
        opened_at: crate::modules::workspace::project::timestamp_now(),
    };
    state.project.set_current(ctx.clone());
    state.session.start_session(&ctx);
    ctx
}

#[tauri::command]
pub fn get_current_project(state: State<'_, WorkspaceState>) -> Option<ProjectContext> {
    state.project.get_current()
}

#[tauri::command]
pub fn clear_current_project(state: State<'_, WorkspaceState>) {
    state.project.clear_current();
    state.session.end_session();
}

#[tauri::command]
pub fn open_project_from_path(state: State<'_, WorkspaceState>, path: String) -> ProjectContext {
    let name = std::path::Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Untitled")
        .to_string();
    let ctx = state.project.open_project(&path, &name, "", &[]);
    state.session.start_session(&ctx);
    ctx
}

// ===== Session commands =====

/// The session timer runs for as long as a project is open in the workspace —
/// it is NOT tied to running processes. Switching tabs or leaving the launch
/// dashboard never stops it. The session ends only when the project is closed
/// (`clear_current_project`), at which point the elapsed time is folded into
/// the project's persisted total (see `session.rs`).
#[tauri::command]
pub fn get_session_info(state: State<'_, WorkspaceState>) -> Option<SessionInfo> {
    state.session.get_session()
}

// ===== File explorer commands =====

// Файловый IO (read_dir, чтение/запись файлов) — блокирующая работа:
// async + spawn_blocking, чтобы навигация по файлам не фризила UI.

#[tauri::command]
pub async fn list_directory(
    state: State<'_, WorkspaceState>,
    path: String,
) -> Result<Vec<file_explorer::FileEntry>, String> {
    let explorer = Arc::clone(&state.file_explorer);
    tauri::async_runtime::spawn_blocking(move || explorer.list_directory(&path))
        .await
        .map_err(|e| format!("List directory task failed: {e}"))?
}

#[tauri::command]
pub async fn read_file(
    state: State<'_, WorkspaceState>,
    path: String,
) -> Result<file_explorer::FileContent, String> {
    let explorer = Arc::clone(&state.file_explorer);
    tauri::async_runtime::spawn_blocking(move || explorer.read_file(&path))
        .await
        .map_err(|e| format!("Read file task failed: {e}"))?
}

#[tauri::command]
pub async fn write_file(
    state: State<'_, WorkspaceState>,
    path: String,
    content: String,
) -> Result<(), String> {
    let explorer = Arc::clone(&state.file_explorer);
    tauri::async_runtime::spawn_blocking(move || explorer.write_file(&path, &content))
        .await
        .map_err(|e| format!("Write file task failed: {e}"))?
}

#[tauri::command]
pub async fn open_in_vscode(
    state: State<'_, WorkspaceState>,
    settings: State<'_, SettingsState>,
    path: String,
) -> Result<(), String> {
    let vscode_path = settings.0.get_settings().ok().map(|s| s.vscode_path);
    let explorer = Arc::clone(&state.file_explorer);
    tauri::async_runtime::spawn_blocking(move || {
        explorer.open_in_vscode(&path, vscode_path.as_deref())
    })
    .await
    .map_err(|e| format!("Open in VS Code task failed: {e}"))?
}

// ===== Problems commands =====

#[tauri::command]
pub fn get_problems(state: State<'_, WorkspaceState>) -> Vec<Problem> {
    let processes = state.process_manager.list();
    state.problems.collect_from_processes(&processes);
    state.problems.get_all()
}

#[tauri::command]
pub fn clear_problems(state: State<'_, WorkspaceState>) {
    state.problems.clear()
}

// other
#[tauri::command]
pub fn get_workspace_overview(state: State<'_, WorkspaceState>) -> OverviewData {
    let processes = state.process_manager.list();
    let session_started = state
        .session
        .get_session()
        .and_then(|s| s.started_at.parse::<u64>().ok());
    state.overview.compute_overview(&processes, session_started)
}
