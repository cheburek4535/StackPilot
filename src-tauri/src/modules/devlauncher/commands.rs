use crate::core::settings::SettingsState;
use crate::modules::devlauncher::models::*;
use crate::modules::devlauncher::profile_manager::{ProfileManager, ProfileManagerV2};
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
        schema_version: None,
        id: None,
        steps: None,
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
    // иначе таймер сессии никогда не увидит их завершения. If no session is
    // active (the workspace was idle and the session auto-ended), restart it
    // so the launched process is tracked by the running timer.
    let session_id = session_id
        .or_else(|| crate::modules::workspace::session::ensure_session(&workspace));
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
    tauri::async_runtime::spawn_blocking(move || analyzer.analyze(&path, vscode_path.as_deref()))
        .await
        .map_err(|e| format!("Analyze task failed: {e}"))?
}

/// Analyze a project and produce a *draft* profile: the step graph plus
/// per-inference confidence diagnostics. The draft is a proposal, not a
/// final execution plan — the user reviews it before running.
#[tauri::command]
pub async fn analyze_project_v2(
    state: State<'_, DevLauncherState>,
    settings: State<'_, SettingsState>,
    path: String,
) -> Result<super::analyzer::DraftProfile, String> {
    let analyzer = Arc::clone(&state.analyzer_v2);
    let vscode_path = settings.0.get_settings().ok().map(|s| s.vscode_path);
    tauri::async_runtime::spawn_blocking(move || {
        analyzer.analyze_draft(
            &path,
            &super::analyzer::AnalyzeOptions {
                vscode_path,
                ..super::analyzer::AnalyzeOptions::default()
            },
        )
    })
    .await
    .map_err(|e| format!("Analyze task failed: {e}"))?
}

// ---------------------------------------------------------------------------
// Versioned profile CRUD
// ---------------------------------------------------------------------------

/// List all profiles in V2 format. Tolerant: malformed profile files are
/// skipped with per-file diagnostics instead of aborting the listing.
#[tauri::command]
pub fn list_profiles_v2(
    state: State<'_, DevLauncherState>,
) -> Result<Vec<LaunchProfileV2>, String> {
    state.profile_manager.list_profiles_v2()
}

/// List profiles together with per-file parse diagnostics.
#[tauri::command]
pub fn profile_load_diagnostics(
    state: State<'_, DevLauncherState>,
) -> Result<super::profile_manager::ProfileLoadResult, String> {
    state.profile_manager.list_profiles_with_diagnostics()
}

#[tauri::command]
pub fn get_profile_v2(
    state: State<'_, DevLauncherState>,
    name: String,
) -> Result<LaunchProfileV2, String> {
    state.profile_manager.get_profile_v2(&name)
}

/// Persist a V2 profile. The stable profile ID is preserved (or adopted
/// from an existing profile with the same name / project path) and the
/// file is written atomically.
#[tauri::command]
pub fn save_profile_v2(
    state: State<'_, DevLauncherState>,
    profile: LaunchProfileV2,
) -> Result<(), String> {
    state.profile_manager.save_profile_v2(&profile)
}

#[tauri::command]
pub fn delete_profile_v2(state: State<'_, DevLauncherState>, name: String) -> Result<(), String> {
    state.profile_manager.delete_profile_v2(&name)
}

#[tauri::command]
pub fn delete_profile_by_id(state: State<'_, DevLauncherState>, id: String) -> Result<(), String> {
    state.profile_manager.delete_profile_by_id(&id)
}

/// Rewrite all legacy (v1) profile files on disk to the V2 schema.
#[tauri::command]
pub fn migrate_profiles(
    state: State<'_, DevLauncherState>,
) -> Result<super::profile_manager::MigrationReport, String> {
    state.profile_manager.migrate_legacy()
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
///
/// Internally delegates to the V2 orchestrator: the legacy profile is
/// converted to V2, a run is created and started, and the results are
/// derived from StepExecutionState entries for backward compatibility.
#[tauri::command]
pub async fn run_profile(
    state: State<'_, DevLauncherState>,
    workspace: State<'_, WorkspaceState>,
    settings: State<'_, SettingsState>,
    profile: LaunchProfile,
    _session_id: Option<String>,
    environment_binding_id: Option<String>,
) -> Result<Vec<(String, ActionStatus)>, String> {
    let engine = Arc::clone(&state.launch_engine);

    // 1. Launch the preferred IDE first so the user sees it open immediately.
    let vscode_path = settings.0.get_settings().ok().map(|s| s.vscode_path);
    if let Some(ide) = &profile.preferred_ide {
        let project_path = profile
            .project_path
            .clone()
            .or_else(|| workspace.project.get_current().and_then(|c| c.project_path));
        let _ = engine.launch_ide(ide, project_path.as_deref(), None, vscode_path.as_deref());
    }

    // 2. Convert legacy profile to V2 and run through the orchestrator.
    // Legacy profiles were strictly sequential; `migrate_legacy_profile`
    // chains the steps so a later WaitForPort does not race its server.
    let v2_profile = migrate_legacy_profile(profile.clone());

    // Resolve environment overlay once for the run
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

    // 3. Create and start the run
    let session_id = workspace
        .session
        .get_session()
        .map(|s| s.started_at.clone());
    let run = state
        .orchestrator
        .create_run_with_session_and_overlay(v2_profile, session_id, overlay)
        .map_err(|v| {
            let msgs: Vec<String> = v
                .diagnostics
                .iter()
                .map(|d| format!("[{}] {}", d.code, d.message))
                .collect();
            format!("Validation failed: {}", msgs.join("; "))
        })?;
    state.orchestrator.start_run(&run.run_id).await?;

    // 4. Wait for the run to complete (poll with timeout)
    let run_id = run.run_id.clone();
    let timeout = std::time::Duration::from_secs(3600); // 1h max
    let poll_interval = std::time::Duration::from_millis(250);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err("Run timed out after 1 hour".to_string());
        }

        if let Some(current_run) = state.orchestrator.get_run(&run_id) {
            match current_run.status {
                RunStatus::Succeeded
                | RunStatus::Failed
                | RunStatus::Cancelled
                | RunStatus::PartialSuccess => {
                    // 5. Convert StepExecutionState results to legacy ActionStatus
                    let results: Vec<(String, ActionStatus)> = current_run
                        .steps
                        .iter()
                        .map(|step| {
                            let status = match step.status {
                                StepStatus::Succeeded => ActionStatus::Success {
                                    message: format!("Step '{}' succeeded", step.step_id),
                                },
                                StepStatus::Failed => ActionStatus::Failed {
                                    error: step
                                        .error
                                        .clone()
                                        .unwrap_or_else(|| "Unknown error".to_string()),
                                },
                                StepStatus::Skipped => ActionStatus::Skipped {
                                    reason: "Step was skipped".to_string(),
                                },
                                StepStatus::Cancelled => ActionStatus::Skipped {
                                    reason: "Step was cancelled".to_string(),
                                },
                                _ => ActionStatus::Skipped {
                                    reason: format!("Step status: {:?}", step.status),
                                },
                            };
                            (step.step_id.clone(), status)
                        })
                        .collect();
                    return Ok(results);
                }
                RunStatus::Running | RunStatus::Pending => {
                    tokio::time::sleep(poll_interval).await;
                    continue;
                }
            }
        } else {
            return Err(format!("Run '{}' disappeared", run_id));
        }
    }
}

/// Builds a LaunchProfile from WizardContext and saves it. This is the
/// integration entry point: Project Creator calls this after execution
/// completes so the profile is ready in DevLauncher without filesystem
/// analysis. If a profile with the same project_path already exists,
/// it is updated in place (preserving the name and stable ID) instead of
/// creating a duplicate.
///
/// The profile is built as a coherent V2 step graph and persisted in V2
/// format; the legacy `LaunchProfile` response is derived from the graph
/// for backward compatibility with existing callers.
#[tauri::command]
pub fn build_profile_from_context(
    state: State<'_, DevLauncherState>,
    context: WizardContext,
) -> Result<LaunchProfile, String> {
    let mut diagnostics = Vec::new();
    let mut profile =
        super::profile_builder::build_profile_v2_from_context(&context, &mut diagnostics);

    // Check if a profile already exists for this project path
    if let Some(ref path) = profile.project_root {
        if let Some(existing) = state.profile_manager.find_by_project_path_v2(path) {
            // Update existing profile: keep its name and stable ID,
            // overwrite the rest.
            profile.name = existing.name;
            profile.id = existing.id;
        }
    }

    state.profile_manager.save_profile_v2(&profile)?;
    Ok(LaunchProfile::from(profile))
}

/// Build and persist a V2 profile from a wizard context. Returns the full
/// step graph (never the flattened legacy form).
#[tauri::command]
pub fn build_profile_v2_from_context(
    state: State<'_, DevLauncherState>,
    context: WizardContext,
) -> Result<LaunchProfileV2, String> {
    let mut diagnostics = Vec::new();
    let mut profile =
        super::profile_builder::build_profile_v2_from_context(&context, &mut diagnostics);

    if let Some(ref path) = profile.project_root {
        if let Some(existing) = state.profile_manager.find_by_project_path_v2(path) {
            profile.name = existing.name;
            profile.id = existing.id;
        }
    }

    state.profile_manager.save_profile_v2(&profile)?;
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
    state
        .file_watcher
        .start(std::path::PathBuf::from(&path), app)
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

// ---------------------------------------------------------------------------
// V2 Orchestrator Commands
// ---------------------------------------------------------------------------

/// Validate a V2 profile and return structured diagnostics.
#[tauri::command]
pub fn validate_profile_v2(profile: LaunchProfileV2) -> Result<serde_json::Value, String> {
    let result = super::validation::validate_profile_v2(&profile);
    serde_json::to_value(result).map_err(|e| format!("Serialization error: {}", e))
}

/// Create a new run from a V2 profile. The run is created in Pending status.
#[tauri::command]
pub fn create_run(
    state: State<'_, DevLauncherState>,
    profile: LaunchProfileV2,
) -> Result<LaunchRun, String> {
    state
        .orchestrator
        .create_run(profile)
        .map_err(|validation| {
            let msgs: Vec<String> = validation
                .diagnostics
                .iter()
                .map(|d| format!("[{}] {}", d.code, d.message))
                .collect();
            format!("Validation failed: {}", msgs.join("; "))
        })
}

/// Start a run asynchronously. The run must be in Pending status.
#[tauri::command]
pub async fn start_run(state: State<'_, DevLauncherState>, run_id: String) -> Result<(), String> {
    state.orchestrator.start_run(&run_id).await
}

/// Cancel a running or pending run.
#[tauri::command]
pub async fn cancel_run(state: State<'_, DevLauncherState>, run_id: String) -> Result<(), String> {
    state.orchestrator.cancel_run(&run_id).await
}

/// Get the current state of a run.
#[tauri::command]
pub fn get_run(state: State<'_, DevLauncherState>, run_id: String) -> Result<LaunchRun, String> {
    state
        .orchestrator
        .get_run(&run_id)
        .ok_or_else(|| format!("Run '{}' not found", run_id))
}

/// List all active (Pending or Running) runs.
#[tauri::command]
pub fn list_active_runs(state: State<'_, DevLauncherState>) -> Vec<LaunchRun> {
    state.orchestrator.list_active_runs()
}

/// Launch the preferred IDE for the current workspace project, then execute
/// all enabled actions in the profile using the V2 orchestrator.
/// This is the "Run Project" entry point for V2 profiles.
#[tauri::command]
pub async fn run_profile_v2(
    state: State<'_, DevLauncherState>,
    workspace: State<'_, WorkspaceState>,
    settings: State<'_, SettingsState>,
    profile: LaunchProfileV2,
    _session_id: Option<String>,
    environment_binding_id: Option<String>,
) -> Result<LaunchRun, String> {
    // 1. Launch the preferred IDE first
    let engine = Arc::clone(&state.launch_engine);
    let vscode_path = settings.0.get_settings().ok().map(|s| s.vscode_path);
    if let Some(ide) = &profile.preferred_ide {
        let project_path = profile
            .project_root
            .clone()
            .or_else(|| workspace.project.get_current().and_then(|c| c.project_path));
        let _ = engine.launch_ide(ide, project_path.as_deref(), None, vscode_path.as_deref());
    }

    // 2. Resolve the environment overlay from the profile's binding once
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

    // 3. Create and start the run via the orchestrator
    let session_id = workspace
        .session
        .get_session()
        .map(|s| s.started_at.clone());
    let run = state
        .orchestrator
        .create_run_with_session_and_overlay(profile, session_id, overlay)
        .map_err(|v| {
            let msgs: Vec<String> = v
                .diagnostics
                .iter()
                .map(|d| format!("[{}] {}", d.code, d.message))
                .collect();
            format!("Validation failed: {}", msgs.join("; "))
        })?;
    state.orchestrator.start_run(&run.run_id).await?;

    // Return the initial run state (the orchestrator will update it asynchronously)
    state
        .orchestrator
        .get_run(&run.run_id)
        .ok_or_else(|| "Run was created but immediately disappeared".to_string())
}

// ---------------------------------------------------------------------------
// Run process management commands
// ---------------------------------------------------------------------------

/// Kill all running processes associated with a run.
#[tauri::command]
pub fn stop_run_processes(
    state: State<'_, DevLauncherState>,
    run_id: String,
) -> Result<Vec<String>, String> {
    state.orchestrator.stop_run_processes(&run_id)
}

/// Get combined logs for all processes in a run.
#[tauri::command]
pub fn get_run_logs(
    state: State<'_, DevLauncherState>,
    run_id: String,
) -> Result<super::models::RunLogs, String> {
    state.orchestrator.get_run_logs(&run_id)
}

/// Get logs for a specific step's process within a run.
#[tauri::command]
pub fn get_step_logs(
    state: State<'_, DevLauncherState>,
    run_id: String,
    step_id: String,
) -> Result<super::models::StepLogs, String> {
    state.orchestrator.get_step_logs(&run_id, &step_id)
}

/// List all runs (including completed) for history/audit.
#[tauri::command]
pub fn list_all_runs(state: State<'_, DevLauncherState>) -> Vec<LaunchRun> {
    state.orchestrator.list_all_runs()
}

// ---------------------------------------------------------------------------
// Platform integration commands
// ---------------------------------------------------------------------------

/// Detect a project and produce a draft V2 profile with diagnostics.
/// This is a convenience wrapper that analyzes a project path and returns
/// both the detected profile and per-inference confidence diagnostics.
#[tauri::command]
pub async fn detect_project_profile(
    state: State<'_, DevLauncherState>,
    settings: State<'_, SettingsState>,
    path: String,
) -> Result<super::analyzer::DraftProfile, String> {
    let analyzer = Arc::clone(&state.analyzer_v2);
    let vscode_path = settings.0.get_settings().ok().map(|s| s.vscode_path);
    tauri::async_runtime::spawn_blocking(move || {
        analyzer.analyze_draft(
            &path,
            &super::analyzer::AnalyzeOptions {
                vscode_path,
                ..super::analyzer::AnalyzeOptions::default()
            },
        )
    })
    .await
    .map_err(|e| format!("Detect task failed: {e}"))?
}

/// Return platform capabilities: OS, arch, available shells, Docker status,
/// default terminal, and terminal window support.
#[tauri::command]
pub async fn get_platform_capabilities() -> Result<super::models::PlatformCapabilities, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let os = crate::platform::host::current_os();
        let arch = format!("{:?}", crate::platform::host::current_arch());

        let shells: Vec<String> = crate::platform::shell_service::available_shells_for_os(os)
            .into_iter()
            .map(|s| format!("{:?}", s))
            .collect();

        // Check Docker CLI and daemon status
        let cli_diag = crate::platform::docker_service::DockerService::check_cli();
        let has_docker =
            cli_diag.status != crate::platform::docker_service::DockerStatus::CliMissing;
        // Compose availability is a property of the CLI, not the daemon:
        // `docker compose version` answers even when the daemon is down.
        let has_compose = if has_docker {
            std::process::Command::new("docker")
                .args(["compose", "version"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        } else {
            false
        };

        let default_shell = crate::platform::shell::default_shell_for_platform();
        let (default_terminal, _) = crate::platform::shell::shell_executable(default_shell);

        let supports_terminal_windows = cfg!(not(target_os = "windows"))
            || std::env::var("WT_SESSION").is_ok()
            || std::env::var("TERM_PROGRAM").is_ok();

        Ok(super::models::PlatformCapabilities {
            os: format!("{}", os),
            arch,
            shells,
            has_docker,
            has_compose,
            default_terminal: default_terminal.to_string(),
            supports_terminal_windows,
        })
    })
    .await
    .map_err(|e| format!("Platform capabilities task failed: {e}"))?
}

/// Resolve an application name to its executable path on the current platform.
/// Checks PATH, App Paths registry, known install directories, and flatpak.
#[tauri::command]
pub fn resolve_application(name: String) -> Result<serde_json::Value, String> {
    // First try IDE resolver (broader coverage)
    if let Some(path) = crate::platform::ide::resolve_ide_executable(&name) {
        return Ok(serde_json::json!({
            "program": path,
            "args": [],
            "found": true,
            "source": "ide_resolver",
        }));
    }

    // Then try the application launcher (free function)
    let result = crate::platform::app_launcher::resolve_application(&name, None, None);
    Ok(serde_json::json!({
        "program": result.program,
        "args": result.args,
        "is_flatpak": result.is_flatpak,
        "found": result.found,
        "diagnostics": result.diagnostics,
    }))
}

/// Resolve the terminal backend configuration for the current platform.
/// Returns the backend name, configuration, and available shells.
#[tauri::command]
pub async fn resolve_terminal(
    command: Option<String>,
    working_dir: Option<String>,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let config = crate::platform::terminal::TerminalConfig {
            backend: crate::platform::terminal::TerminalBackend::Default,
            command: command.unwrap_or_default(),
            working_dir,
            ..Default::default()
        };
        let plan = crate::platform::terminal::resolve_terminal_plan(&config)?;

        serde_json::to_value(&plan).map_err(|e| format!("Serialization error: {}", e))
    })
    .await
    .map_err(|e| format!("Terminal resolve task failed: {e}"))?
}
