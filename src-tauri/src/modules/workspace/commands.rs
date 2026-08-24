use crate::modules::workspace::file_explorer;
use crate::modules::workspace::file_explorer::FileExplorerService;
use crate::modules::workspace::models::*;
use crate::modules::workspace::overview::{OverviewData, OverviewService};
use crate::modules::workspace::problems::{Problem, ProblemsService};
use crate::modules::workspace::project::ProjectService;
use crate::modules::workspace::session::SessionService;
use crate::modules::workspace::WorkspaceState;
use tauri::State;

// ===== Process commands =====

#[tauri::command]
pub fn spawn_process(
    state: State<'_, WorkspaceState>,
    command: String,
    args: Vec<String>,
    working_dir: Option<String>,
    label: String,
) -> Result<TrackedProcess, String> {
    let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let session_id = state.session.get_session().map(|s| s.started_at.clone());
    let proc = state.process_manager.spawn_and_track(
        &command,
        &args_refs,
        working_dir.as_deref(),
        &label,
        session_id,
    )?;
    state.session.link_process(&proc.id);
    Ok(proc)
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

/// Spawn a process in a new native terminal window (visible to the user).
#[tauri::command]
pub fn spawn_process_visible(
    state: State<'_, WorkspaceState>,
    command: String,
    args: Vec<String>,
    working_dir: Option<String>,
    label: String,
) -> Result<TrackedProcess, String> {
    let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let session_id = state.session.get_session().map(|s| s.started_at.clone());
    let proc = state.process_manager.spawn_visible(
        &command,
        &args_refs,
        working_dir.as_deref(),
        &label,
        session_id,
        None,
    )?;
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

/// Сессия автоматически завершается, когда все привязанные к ней процессы
/// завершились (в т.ч. с ошибкой) — таймер не должен тикать вечно.
/// Сессия без процессов живёт, пока проект открыт.
fn auto_end_session_if_idle(state: &WorkspaceState) {
    let linked = state.session.get_linked_processes();
    if linked.is_empty() {
        return;
    }
    let procs = state.process_manager.list();
    let all_done = linked.iter().all(|id| {
        procs
            .iter()
            .find(|p| &p.id == id)
            .map(|p| p.status != ProcessStatus::Running)
            .unwrap_or(true)
    });
    if all_done {
        state.session.end_session();
    }
}

#[tauri::command]
pub fn get_session_info(state: State<'_, WorkspaceState>) -> Option<SessionInfo> {
    auto_end_session_if_idle(&state);
    state.session.get_session()
}

// ===== File explorer commands =====

#[tauri::command]
pub fn list_directory(
    state: State<'_, WorkspaceState>,
    path: String,
) -> Result<Vec<file_explorer::FileEntry>, String> {
    state.file_explorer.list_directory(&path)
}

#[tauri::command]
pub fn read_file(
    state: State<'_, WorkspaceState>,
    path: String,
) -> Result<file_explorer::FileContent, String> {
    state.file_explorer.read_file(&path)
}

#[tauri::command]
pub fn write_file(
    state: State<'_, WorkspaceState>,
    path: String,
    content: String,
) -> Result<(), String> {
    state.file_explorer.write_file(&path, &content)
}

#[tauri::command]
pub fn open_in_vscode(state: State<'_, WorkspaceState>, path: String) -> Result<(), String> {
    state.file_explorer.open_in_vscode(&path)
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
    auto_end_session_if_idle(&state);
    let processes = state.process_manager.list();
    let session_started = state
        .session
        .get_session()
        .and_then(|s| s.started_at.parse::<u64>().ok());
    state.overview.compute_overview(&processes, session_started)
}
