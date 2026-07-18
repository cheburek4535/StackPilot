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
    state.process_manager.spawn_and_track(
        &command,
        &args,
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
