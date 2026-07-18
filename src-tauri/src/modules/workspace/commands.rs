use tauri::State;
use crate::modules::workspace::models::*;
use crate::modules::workspace::WorkspaceState;

#[tauri::command]
pub fn spawn_process(
    state: State<'_, WorkspaceState>,
    command: String,
    args: Vec<String>,
    working_dir: Option<String>,
    label: String,
) -> Result<TrackedProcess, String> {
    let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    state.process_manager.spawn_and_track(
        &command,
        &args_refs,
        working_dir.as_deref(),
        &label,
    )
}

#[tauri::command]
pub fn list_processes(state: State<'_, WorkspaceState>) -> Vec<TrackedProcess> {
    state.process_manager.list()
}

#[tauri::command]
pub fn kill_process(state: State<'_, WorkspaceState>, id: String) -> Result<(), String> {
    state.process_manager.kill(&id)
}

#[tauri::command]
pub fn refresh_process(
    state: State<'_, WorkspaceState>,
    id: String,
) -> Result<ProcessStatus, String> {
    state.process_manager.refresh_status(&id)
}

#[tauri::command]
pub fn get_process_logs(
    state: State<'_, WorkspaceState>,
    id: String,
) -> Result<ProcessLogs, String> {
    state.process_manager.get_logs(&id)
}

// --- Project context commands ---

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
    state.project.set(ctx.clone());
    ctx
}

#[tauri::command]
pub fn get_current_project(
    state: State<'_, WorkspaceState>,
) -> Option<ProjectContext> {
    state.project.get()
}

#[tauri::command]
pub fn clear_current_project(state: State<'_, WorkspaceState>) {
    state.project.clear();
}

// --- Session info (derived) ---

#[tauri::command]
pub fn get_session_info(
    state: State<'_, WorkspaceState>,
) -> Option<SessionInfo> {
    let project = state.project.get()?;
    let started = project.opened_at.parse::<u64>().unwrap_or(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let processes = state.process_manager.list();
    let error_count = processes.iter().filter(|p| {
        matches!(p.status, ProcessStatus::Crashed | ProcessStatus::Killed)
    }).count();

    Some(SessionInfo {
        started_at: project.opened_at.clone(),
        duration_secs: now.saturating_sub(started),
        process_count: processes.len(),
        error_count,
    })
}
