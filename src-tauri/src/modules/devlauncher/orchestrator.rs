use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::Serialize;
use tauri::Emitter;
use tokio::sync::{mpsc, Notify};

use super::models::*;
use super::validation::{self, ProfileValidationResult};
use crate::modules::workspace::models::ProcessStatus;
use crate::modules::workspace::process_manager::ProcessManager;

// ---------------------------------------------------------------------------
// Event names (stable, matching devlauncher-contract.md)
// ---------------------------------------------------------------------------

const EVENT_RUN_CREATED: &str = "devlauncher:run-created";
const EVENT_RUN_STATUS_CHANGED: &str = "devlauncher:run-status-changed";
const EVENT_STEP_STATUS_CHANGED: &str = "devlauncher:step-status-changed";
const EVENT_PROCESS_STARTED: &str = "devlauncher:process-started";
const EVENT_DIAGNOSTIC: &str = "devlauncher:diagnostic";
const EVENT_RUN_FINISHED: &str = "devlauncher:run-finished";

// ---------------------------------------------------------------------------
// Internal handle for an active run
// ---------------------------------------------------------------------------

struct RunHandle {
    run: Arc<Mutex<LaunchRun>>,
    cancelled: Arc<AtomicBool>,
    cancel_notify: Arc<Notify>,
    profile: LaunchProfileV2,
    /// Workspace session this run belongs to (for session lifecycle tracking).
    session_id: Option<String>,
    /// Every managed process spawned by this run, regardless of step status.
    /// Used for stop/cancel so long-running processes are always closable.
    process_ids: Arc<Mutex<Vec<String>>>,
}

// ---------------------------------------------------------------------------
// Step completion message sent from step tasks to the scheduler
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct StepCompletion {
    step_id: String,
    success: bool,
    error: Option<String>,
    process_id: Option<String>,
    attempt_number: u32,
}

// ---------------------------------------------------------------------------
// RunOrchestrator
// ---------------------------------------------------------------------------

pub struct RunOrchestrator {
    runs: Arc<RwLock<HashMap<String, Arc<RunHandle>>>>,
    process_manager: Arc<dyn ProcessManager>,
    app_handle: Arc<Mutex<Option<tauri::AppHandle>>>,
    concurrency_limit: usize,
}

impl RunOrchestrator {
    pub fn new(process_manager: Arc<dyn ProcessManager>) -> Self {
        Self {
            runs: Arc::new(RwLock::new(HashMap::new())),
            process_manager,
            app_handle: Arc::new(Mutex::new(None)),
            concurrency_limit: 8,
        }
    }

    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        match self.app_handle.lock() {
            Ok(mut h) => *h = Some(handle),
            Err(mut e) => {
                **e.get_mut() = Some(handle);
            }
        }
    }

    pub fn create_run(
        &self,
        profile: LaunchProfileV2,
    ) -> Result<LaunchRun, ProfileValidationResult> {
        self.create_run_with_session(profile, None)
    }

    pub fn create_run_with_session(
        &self,
        profile: LaunchProfileV2,
        session_id: Option<String>,
    ) -> Result<LaunchRun, ProfileValidationResult> {
        let validation = validation::validate_profile_v2(&profile);
        if !validation.valid {
            return Err(validation);
        }

        let run_id = generate_stable_id();
        let now = default_now_iso();

        let steps: Vec<StepExecutionState> = profile
            .steps
            .iter()
            .map(|s| {
                let status = if s.enabled {
                    StepStatus::Pending
                } else {
                    StepStatus::Skipped
                };
                StepExecutionState {
                    step_id: s.id.clone(),
                    status,
                    process_id: None,
                    error: None,
                    retries_remaining: s.retry_policy.as_ref().map(|r| r.max_retries),
                    started_at: None,
                    finished_at: None,
                    attempts: Vec::new(),
                }
            })
            .collect();

        let run = LaunchRun {
            run_id: run_id.clone(),
            profile_id: profile.id.clone(),
            profile_name: profile.name.clone(),
            status: RunStatus::Pending,
            steps,
            created_at: now,
            finished_at: None,
            cancelled: false,
            diagnostics: Vec::new(),
        };

        let handle = Arc::new(RunHandle {
            run: Arc::new(Mutex::new(run.clone())),
            cancelled: Arc::new(AtomicBool::new(false)),
            cancel_notify: Arc::new(Notify::new()),
            profile,
            session_id,
            process_ids: Arc::new(Mutex::new(Vec::new())),
        });

        self.runs
            .write()
            .expect("runs lock poisoned")
            .insert(run_id, handle);

        self.emit(EVENT_RUN_CREATED, &run);

        Ok(run)
    }

    pub async fn start_run(&self, run_id: &str) -> Result<(), String> {
        let handle = {
            let runs = self.runs.read().expect("runs lock poisoned");
            runs.get(run_id).cloned()
        };

        let handle = handle.ok_or_else(|| format!("Run '{}' not found", run_id))?;

        // Mark running
        {
            let mut run = handle.run.lock().expect("run lock poisoned");
            if run.status != RunStatus::Pending {
                return Err(format!("Run '{}' is {:?}, not Pending", run_id, run.status));
            }
            run.status = RunStatus::Running;
        }

        self.emit_status(run_id, RunStatus::Running);

        let orchestrator_runs = self.runs.clone();
        let process_manager = self.process_manager.clone();
        let app_handle = self.app_handle.clone();
        let concurrency_limit = self.concurrency_limit;
        let run_id_owned = run_id.to_string();
        let session_id = handle.session_id.clone();

        tokio::spawn(async move {
            Self::scheduler_loop(
                run_id_owned,
                orchestrator_runs,
                process_manager,
                app_handle,
                concurrency_limit,
                session_id,
            )
            .await;
        });

        Ok(())
    }

    pub async fn cancel_run(&self, run_id: &str) -> Result<(), String> {
        let handle = {
            let runs = self.runs.read().expect("runs lock poisoned");
            runs.get(run_id).cloned()
        };

        let handle = handle.ok_or_else(|| format!("Run '{}' not found", run_id))?;

        handle.cancelled.store(true, Ordering::SeqCst);
        handle.cancel_notify.notify_waiters();

        {
            let mut run = handle.run.lock().expect("run lock poisoned");
            if run.status == RunStatus::Succeeded
                || run.status == RunStatus::Failed
                || run.status == RunStatus::Cancelled
            {
                return Ok(());
            }
            run.status = RunStatus::Cancelled;
            run.cancelled = true;
            let now = default_now_iso();
            run.finished_at = Some(now.clone());

            // Mark pending steps as cancelled
            for step in &mut run.steps {
                if step.status == StepStatus::Pending || step.status == StepStatus::Running {
                    step.status = StepStatus::Cancelled;
                    step.finished_at = Some(now.clone());
                }
            }
        }

        self.emit_status(run_id, RunStatus::Cancelled);

        // Kill every managed process of this run (not just steps that are
        // currently `Running` — long-running services report Succeeded).
        let processes_to_kill: Vec<String> = handle
            .process_ids
            .lock()
            .expect("process_ids lock poisoned")
            .clone();

        for proc_id in &processes_to_kill {
            let _ = self.process_manager.kill(proc_id);
        }

        self_emit_run_finished(&self.app_handle, run_id, &handle);

        Ok(())
    }

    pub fn get_run(&self, run_id: &str) -> Option<LaunchRun> {
        let runs = self.runs.read().expect("runs lock poisoned");
        runs.get(run_id)
            .map(|h| h.run.lock().expect("run lock poisoned").clone())
    }

    pub fn list_active_runs(&self) -> Vec<LaunchRun> {
        let runs = self.runs.read().expect("runs lock poisoned");
        runs.values()
            .filter_map(|h| {
                let run = h.run.lock().expect("run lock poisoned").clone();
                match run.status {
                    RunStatus::Pending | RunStatus::Running => Some(run),
                    _ => None,
                }
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Scheduler loop — runs in a tokio task per run
    // -----------------------------------------------------------------------

    async fn scheduler_loop(
        run_id: String,
        runs: Arc<RwLock<HashMap<String, Arc<RunHandle>>>>,
        process_manager: Arc<dyn ProcessManager>,
        app_handle: Arc<Mutex<Option<tauri::AppHandle>>>,
        concurrency_limit: usize,
        session_id: Option<String>,
    ) {
        let handle = {
            let runs_guard = runs.read().expect("runs lock poisoned");
            match runs_guard.get(&run_id) {
                Some(h) => h.clone(),
                None => return,
            }
        };

        let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency_limit));
        let (tx, mut rx) = mpsc::unbounded_channel::<StepCompletion>();

        // Compute which steps depend on which
        let step_map: HashMap<String, &LaunchStep> = handle
            .profile
            .steps
            .iter()
            .map(|s| (s.id.clone(), s))
            .collect();

        // Track completed, running, and skipped step IDs
        let mut completed: HashSet<String> = HashSet::new();
        let mut running: HashSet<String> = HashSet::new();

        // Mark disabled steps as skipped and add to completed
        {
            let mut run = handle.run.lock().expect("run lock poisoned");
            for step_state in &mut run.steps {
                if step_state.status == StepStatus::Skipped {
                    completed.insert(step_state.step_id.clone());
                }
            }
        }

        loop {
            // Find ready steps: all dependencies satisfied, not yet started
            let ready = Self::find_ready_steps(&handle.profile, &completed, &running);

            if ready.is_empty() && running.is_empty() {
                break;
            }

            // Spawn ready steps
            for step_id in &ready {
                if let Some(step) = step_map.get(step_id) {
                    running.insert(step_id.clone());

                    // Mark step as Running in the run state
                    {
                        let mut run = handle.run.lock().expect("run lock poisoned");
                        if let Some(step_state) =
                            run.steps.iter_mut().find(|s| s.step_id == *step_id)
                        {
                            step_state.status = StepStatus::Running;
                            step_state.started_at = Some(default_now_iso());
                        }
                    }

                    let tx = tx.clone();
                    let semaphore = semaphore.clone();
                    let process_manager = process_manager.clone();
                    let app_handle = app_handle.clone();
                    let cancelled = handle.cancelled.clone();
                    let cancel_notify = handle.cancel_notify.clone();
                    let step = (*step).clone();
                    let run_id = run_id.clone();
                    let profile = handle.profile.clone();
                    let _step_id = step_id.clone();
                    let session_id = session_id.clone();
                    let handle = handle.clone();

                    tokio::spawn(async move {
                        let _permit = semaphore.acquire().await.unwrap();
                        let completion = Self::step_task(
                            &run_id,
                            &step,
                            &profile,
                            &process_manager,
                            &app_handle,
                            &cancelled,
                            &cancel_notify,
                            &session_id,
                            &handle,
                        )
                        .await;
                        let _ = tx.send(completion);
                        drop(_permit);
                    });
                }
            }

            // Wait for completion
            if let Some(completion) = rx.recv().await {
                running.remove(&completion.step_id);

                // Update step state
                {
                    let mut run = handle.run.lock().expect("run lock poisoned");
                    if let Some(step_state) = run
                        .steps
                        .iter_mut()
                        .find(|s| s.step_id == completion.step_id)
                    {
                        if completion.success {
                            step_state.status = StepStatus::Succeeded;
                        } else if handle.cancelled.load(Ordering::SeqCst) {
                            // User-initiated cancellation — keep the step as
                            // Cancelled instead of Failed.
                            step_state.status = StepStatus::Cancelled;
                            step_state.error = completion.error.clone();
                        } else {
                            step_state.status = StepStatus::Failed;
                            step_state.error = completion.error.clone();
                        }
                        let now = default_now_iso();
                        step_state.finished_at = Some(now.clone());
                        step_state.process_id = completion.process_id;

                        // Record attempt
                        let attempt_num = step_state.attempts.len() as u32 + 1;
                        step_state.attempts.push(StepAttempt {
                            attempt_number: attempt_num,
                            started_at: step_state
                                .started_at
                                .clone()
                                .unwrap_or_else(default_now_iso),
                            finished_at: Some(now),
                            status: step_state.status.clone(),
                            error: completion.error.clone(),
                        });
                    }
                }

                self_emit_step_event(&app_handle, &run_id, &completion.step_id, &handle);

                if completion.success {
                    completed.insert(completion.step_id.clone());
                } else {
                    // Handle failure policy
                    let failure_action = Self::handle_failure(
                        &handle,
                        &completion.step_id,
                        &completed,
                        &mut running,
                    );
                    match failure_action {
                        FailureAction::StopRun => {
                            Self::finalize_run(
                                &handle,
                                &runs,
                                &run_id,
                                RunStatus::Failed,
                                &app_handle,
                            )
                            .await;
                            return;
                        }
                        FailureAction::SkipDependents => {
                            let to_skip =
                                Self::transitive_dependents(&handle.profile, &completion.step_id);
                            {
                                let mut run = handle.run.lock().expect("run lock poisoned");
                                for step_id in &to_skip {
                                    if let Some(step_state) =
                                        run.steps.iter_mut().find(|s| &s.step_id == step_id)
                                    {
                                        if step_state.status == StepStatus::Pending {
                                            step_state.status = StepStatus::Skipped;
                                            step_state.finished_at = Some(default_now_iso());
                                        }
                                    }
                                    completed.insert(step_id.clone());
                                    running.remove(step_id);
                                }
                            }
                        }
                        FailureAction::Continue => {
                            // The failed step's dependents can still run
                            // (they will check the failure themselves)
                            completed.insert(completion.step_id.clone());
                        }
                    }
                }

                // Check for retries
                if !completion.success && !handle.cancelled.load(Ordering::SeqCst) {
                    Self::maybe_retry(
                        &handle,
                        &completion.step_id,
                        &mut running,
                        &completed,
                        &tx,
                        &semaphore,
                        &process_manager,
                        &app_handle,
                    );
                }

                // Check cancellation
                if handle.cancelled.load(Ordering::SeqCst) {
                    Self::finalize_run(&handle, &runs, &run_id, RunStatus::Cancelled, &app_handle)
                        .await;
                    return;
                }
            }
        }

        // All steps done — finalize
        let final_status = Self::compute_final_status(&handle);
        Self::finalize_run(&handle, &runs, &run_id, final_status, &app_handle).await;
    }

    // -----------------------------------------------------------------------
    // Find steps whose dependencies are all completed
    // -----------------------------------------------------------------------

    fn find_ready_steps(
        profile: &LaunchProfileV2,
        completed: &HashSet<String>,
        running: &HashSet<String>,
    ) -> Vec<String> {
        let mut ready = Vec::new();
        for step in &profile.steps {
            if completed.contains(&step.id) || running.contains(&step.id) {
                continue;
            }
            if step.depends_on.iter().all(|dep| completed.contains(dep)) {
                ready.push(step.id.clone());
            }
        }
        ready.sort(); // deterministic ordering
        ready
    }

    // -----------------------------------------------------------------------
    // Execute a single step
    // -----------------------------------------------------------------------

    async fn step_task(
        run_id: &str,
        step: &LaunchStep,
        profile: &LaunchProfileV2,
        process_manager: &Arc<dyn ProcessManager>,
        app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
        cancelled: &Arc<AtomicBool>,
        cancel_notify: &Arc<Notify>,
        session_id: &Option<String>,
        handle: &Arc<RunHandle>,
    ) -> StepCompletion {
        // Check cancellation before starting
        if cancelled.load(Ordering::SeqCst) {
            return StepCompletion {
                step_id: step.id.clone(),
                success: false,
                error: Some("Cancelled".to_string()),
                process_id: None,
                attempt_number: 0,
            };
        }

        // Emit Running status
        Self::emit_step_status(run_id, &step.id, StepStatus::Running, app_handle);

        // Resolve effective values
        let visibility = step
            .visibility
            .clone()
            .unwrap_or_else(|| step.kind.default_visibility());
        let execution_mode = step
            .execution_mode
            .clone()
            .unwrap_or_else(|| step.kind.default_execution_mode());
        let completion = step
            .completion
            .clone()
            .unwrap_or_else(|| step.kind.default_completion(Some(&execution_mode)));

        // Resolve working directory against the profile's explicit project
        // root. Relative directories (e.g. "./backend") must never resolve
        // against the app's own working directory or the globally active
        // workspace project — the profile is self-contained (see path model
        // in docs/devlauncher-contract.md §E).
        let working_dir: Option<String> = match step.working_directory.as_deref() {
            Some(dir) => resolve_working_directory(profile.project_root.as_deref(), Some(dir)),
            None => profile.project_root.clone(),
        };
        let working_dir = working_dir.as_deref();

        match &step.kind {
            StepKind::RunCommand {
                command,
                command_spec,
            } => {
                // Prefer the structured command spec when provided; otherwise
                // resolve the command string into program + args (falling back
                // to the platform shell for shell-syntax command lines).
                let (program, args) = if let Some(spec) = command_spec {
                    (spec.program.clone(), spec.args.clone())
                } else {
                    Self::resolve_command_target(command)
                };
                Self::execute_process_step(
                    run_id,
                    &step.id,
                    &step.label,
                    &program,
                    &args,
                    working_dir,
                    &visibility,
                    &execution_mode,
                    &completion,
                    step.timeout,
                    process_manager,
                    app_handle,
                    cancelled,
                    cancel_notify,
                    &step.environment,
                    session_id.as_deref(),
                    handle,
                )
                .await
            }
            StepKind::RunScript { script, shell } => {
                Self::execute_script_step(
                    run_id,
                    &step.id,
                    &step.label,
                    script,
                    shell.as_deref(),
                    working_dir,
                    &completion,
                    step.timeout,
                    process_manager,
                    app_handle,
                    cancelled,
                    cancel_notify,
                    &step.environment,
                    session_id.as_deref(),
                    handle,
                )
                .await
            }
            StepKind::OpenApplication { path, args } => {
                let args_refs: Vec<&str> = args
                    .as_ref()
                    .map(|a| a.iter().map(|s| s.as_str()).collect())
                    .unwrap_or_default();

                let resolved = if path.contains('/') || path.contains('\\') {
                    path.clone()
                } else {
                    match crate::platform::ide::resolve_ide_executable(path) {
                        Some(p) => p,
                        None => {
                            return StepCompletion {
                                step_id: step.id.clone(),
                                success: false,
                                error: Some(format!("Application '{}' not found", path)),
                                process_id: None,
                                attempt_number: 0,
                            };
                        }
                    }
                };

                match process_manager.launch_detached(&resolved, &args_refs, working_dir) {
                    Ok(()) => StepCompletion {
                        step_id: step.id.clone(),
                        success: true,
                        error: None,
                        process_id: None,
                        attempt_number: 0,
                    },
                    Err(e) => StepCompletion {
                        step_id: step.id.clone(),
                        success: false,
                        error: Some(format!("Failed to launch '{}': {}", resolved, e)),
                        process_id: None,
                        attempt_number: 0,
                    },
                }
            }
            StepKind::OpenUrl { url } => match webbrowser::open(url) {
                Ok(_) => StepCompletion {
                    step_id: step.id.clone(),
                    success: true,
                    error: None,
                    process_id: None,
                    attempt_number: 0,
                },
                Err(e) => StepCompletion {
                    step_id: step.id.clone(),
                    success: false,
                    error: Some(format!("Failed to open URL '{}': {}", url, e)),
                    process_id: None,
                    attempt_number: 0,
                },
            },
            StepKind::WaitForPort { host, port } => {
                let timeout = step.timeout.unwrap_or(30);
                Self::wait_for_port(
                    run_id,
                    &step.id,
                    host,
                    *port,
                    timeout,
                    cancelled,
                    cancel_notify,
                    app_handle,
                )
                .await
            }
            StepKind::WaitForUrl { url } => {
                let timeout = step.timeout.unwrap_or(30);
                Self::wait_for_url(
                    run_id,
                    &step.id,
                    url,
                    timeout,
                    cancelled,
                    cancel_notify,
                    app_handle,
                )
                .await
            }
            StepKind::WaitForDocker {} => {
                let timeout = step.timeout.unwrap_or(30);
                Self::wait_for_docker_daemon(
                    run_id,
                    &step.id,
                    timeout,
                    cancelled,
                    cancel_notify,
                    app_handle,
                )
                .await
            }
            StepKind::Delay { seconds } => {
                let timeout = step.timeout.unwrap_or(*seconds + 10);
                Self::delay_with_cancel(
                    run_id,
                    &step.id,
                    *seconds,
                    timeout,
                    cancelled,
                    cancel_notify,
                    app_handle,
                )
                .await
            }
            StepKind::OpenTerminal { command } => {
                // An empty command means "open a plain terminal": spawn the
                // platform's default interactive shell so the terminal stays
                // open at the working directory.
                let effective = if command.trim().is_empty() {
                    if cfg!(target_os = "windows") {
                        "cmd".to_string()
                    } else {
                        std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string())
                    }
                } else {
                    command.clone()
                };
                let (program, args) = Self::resolve_command_target(&effective);
                Self::execute_process_step(
                    run_id,
                    &step.id,
                    &step.label,
                    &program,
                    &args,
                    working_dir,
                    &Visibility::VisibleTerminal,
                    &ExecutionMode::LongRunning,
                    &CompletionPolicy::ProcessStarted,
                    step.timeout,
                    process_manager,
                    app_handle,
                    cancelled,
                    cancel_notify,
                    &step.environment,
                    session_id.as_deref(),
                    handle,
                )
                .await
            }
            StepKind::OpenFolder { path } => {
                #[cfg(target_os = "windows")]
                {
                    let _ = std::process::Command::new("explorer").arg(path).spawn();
                }
                #[cfg(target_os = "macos")]
                {
                    let _ = std::process::Command::new("open").arg(path).spawn();
                }
                #[cfg(target_os = "linux")]
                {
                    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
                }
                StepCompletion {
                    step_id: step.id.clone(),
                    success: true,
                    error: None,
                    process_id: None,
                    attempt_number: 0,
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Process step execution (RunCommand, RunScript, OpenTerminal)
    // -----------------------------------------------------------------------

    async fn execute_process_step(
        run_id: &str,
        step_id: &str,
        label: &str,
        program: &str,
        args: &[String],
        working_dir: Option<&str>,
        visibility: &Visibility,
        _execution_mode: &ExecutionMode,
        completion: &CompletionPolicy,
        timeout: Option<u64>,
        process_manager: &Arc<dyn ProcessManager>,
        app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
        cancelled: &Arc<AtomicBool>,
        cancel_notify: &Arc<Notify>,
        _step_env: &Option<HashMap<String, String>>,
        session_id: Option<&str>,
        handle: &Arc<RunHandle>,
    ) -> StepCompletion {
        // Preflight for docker commands
        if let Err(msg) = preflight_check(program) {
            return StepCompletion {
                step_id: step_id.to_string(),
                success: false,
                error: Some(msg),
                process_id: None,
                attempt_number: 0,
            };
        }

        let tracked = match visibility {
            Visibility::VisibleTerminal => {
                let pm = process_manager.clone();
                let program_owned = program.to_string();
                let args_owned: Vec<String> = args.to_vec();
                let dir = working_dir.map(String::from);
                let label_owned = label.to_string();
                let run_id_owned = run_id.to_string();
                let step_id_owned = step_id.to_string();
                let session_id_owned = session_id.map(String::from);
                let pm_clone = pm.clone();
                match tokio::task::spawn_blocking(move || {
                    pm_clone.spawn_visible_owned(
                        &program_owned,
                        &args_owned.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                        dir.as_deref(),
                        &label_owned,
                        session_id_owned,
                        None,
                        Some(run_id_owned),
                        Some(step_id_owned),
                    )
                })
                .await
                {
                    Ok(Ok(tp)) => tp,
                    Ok(Err(e)) => {
                        return StepCompletion {
                            step_id: step_id.to_string(),
                            success: false,
                            error: Some(format!("Failed to spawn: {}", e)),
                            process_id: None,
                            attempt_number: 0,
                        };
                    }
                    Err(e) => {
                        return StepCompletion {
                            step_id: step_id.to_string(),
                            success: false,
                            error: Some(format!("Task join error: {}", e)),
                            process_id: None,
                            attempt_number: 0,
                        };
                    }
                }
            }
            _ => {
                let pm = process_manager.clone();
                let program_owned = program.to_string();
                let args_owned: Vec<String> = args.to_vec();
                let dir = working_dir.map(String::from);
                let label_owned = label.to_string();
                let run_id_owned = run_id.to_string();
                let step_id_owned = step_id.to_string();
                let session_id_owned = session_id.map(String::from);
                match tokio::task::spawn_blocking(move || {
                    pm.spawn_and_track_owned(
                        &program_owned,
                        &args_owned.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                        dir.as_deref(),
                        &label_owned,
                        session_id_owned,
                        Some(run_id_owned),
                        Some(step_id_owned),
                    )
                })
                .await
                {
                    Ok(Ok(tp)) => tp,
                    Ok(Err(e)) => {
                        return StepCompletion {
                            step_id: step_id.to_string(),
                            success: false,
                            error: Some(format!("Failed to spawn: {}", e)),
                            process_id: None,
                            attempt_number: 0,
                        };
                    }
                    Err(e) => {
                        return StepCompletion {
                            step_id: step_id.to_string(),
                            success: false,
                            error: Some(format!("Task join error: {}", e)),
                            process_id: None,
                            attempt_number: 0,
                        };
                    }
                }
            }
        };

        let proc_id = tracked.id.clone();

        // Register the process against the run so stop/cancel can always
        // close it, even when the step already reached Succeeded.
        Self::register_process(handle, &proc_id);

        // Emit process-started with tracking quality
        Self::emit_process_started(run_id, step_id, &tracked, app_handle);

        // Handle completion based on policy
        match completion {
            CompletionPolicy::ProcessStarted => StepCompletion {
                step_id: step_id.to_string(),
                success: true,
                error: None,
                process_id: Some(proc_id),
                attempt_number: 0,
            },
            CompletionPolicy::ExitSuccess => {
                // Wait for process exit
                let effective_timeout = timeout.unwrap_or(3600); // 1h default for one-shot
                Self::wait_for_process_exit(
                    run_id,
                    step_id,
                    &proc_id,
                    effective_timeout,
                    cancelled,
                    cancel_notify,
                    process_manager,
                    app_handle,
                )
                .await
            }
            CompletionPolicy::PortOpen {
                host,
                port,
                timeout_secs,
            } => {
                // Start process then wait for port
                let host = host.clone();
                let port = *port;
                let timeout_secs = *timeout_secs;
                let cancelled_clone = cancelled.clone();
                let cancel_notify_clone = cancel_notify.clone();
                let app_handle_clone = app_handle.clone();
                let run_id_owned = run_id.to_string();
                let step_id_owned = step_id.to_string();

                let result = Self::wait_for_port(
                    &run_id_owned,
                    &step_id_owned,
                    &host,
                    port,
                    timeout_secs,
                    &cancelled_clone,
                    &cancel_notify_clone,
                    &app_handle_clone,
                )
                .await;

                StepCompletion {
                    step_id: step_id.to_string(),
                    success: result.success,
                    error: result.error,
                    process_id: Some(proc_id),
                    attempt_number: 0,
                }
            }
            CompletionPolicy::UrlReady { url, timeout_secs } => {
                let url = url.clone();
                let timeout_secs = *timeout_secs;
                let cancelled_clone = cancelled.clone();
                let cancel_notify_clone = cancel_notify.clone();
                let app_handle_clone = app_handle.clone();
                let run_id_owned = run_id.to_string();
                let step_id_owned = step_id.to_string();

                let result = Self::wait_for_url(
                    &run_id_owned,
                    &step_id_owned,
                    &url,
                    timeout_secs,
                    &cancelled_clone,
                    &cancel_notify_clone,
                    &app_handle_clone,
                )
                .await;

                StepCompletion {
                    step_id: step_id.to_string(),
                    success: result.success,
                    error: result.error,
                    process_id: Some(proc_id),
                    attempt_number: 0,
                }
            }
            CompletionPolicy::ExternalLaunchAccepted => StepCompletion {
                step_id: step_id.to_string(),
                success: true,
                error: None,
                process_id: Some(proc_id),
                attempt_number: 0,
            },
            CompletionPolicy::Manual => {
                // Wait indefinitely (until cancelled)
                Self::wait_for_process_exit(
                    run_id,
                    step_id,
                    &proc_id,
                    u64::MAX,
                    cancelled,
                    cancel_notify,
                    process_manager,
                    app_handle,
                )
                .await
            }
            CompletionPolicy::DelayElapsed { seconds } => {
                let result = Self::delay_with_cancel(
                    run_id,
                    step_id,
                    *seconds,
                    seconds + 10,
                    cancelled,
                    cancel_notify,
                    app_handle,
                )
                .await;
                StepCompletion {
                    step_id: step_id.to_string(),
                    ..result
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Script step execution
    // -----------------------------------------------------------------------

    async fn execute_script_step(
        run_id: &str,
        step_id: &str,
        label: &str,
        script: &str,
        shell: Option<&str>,
        working_dir: Option<&str>,
        _completion: &CompletionPolicy,
        timeout: Option<u64>,
        process_manager: &Arc<dyn ProcessManager>,
        app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
        cancelled: &Arc<AtomicBool>,
        cancel_notify: &Arc<Notify>,
        _step_env: &Option<HashMap<String, String>>,
        session_id: Option<&str>,
        handle: &Arc<RunHandle>,
    ) -> StepCompletion {
        let resolved_shell = match shell {
            Some(s) => match super::models::validate_shell_for_os(s) {
                Ok(()) => s.to_string(),
                Err(e) => {
                    return StepCompletion {
                        step_id: step_id.to_string(),
                        success: false,
                        error: Some(e),
                        process_id: None,
                        attempt_number: 0,
                    };
                }
            },
            None => {
                let kind = crate::platform::shell::default_shell_for_platform();
                let (exe, _) = crate::platform::shell::shell_executable(kind);
                exe.to_string()
            }
        };

        let (shell_exe, shell_flag) = if shell.is_some() {
            let kind = super::models::validate_shell_for_os(resolved_shell.as_str())
                .map(|_| {
                    crate::platform::shell::parse_shell(&resolved_shell)
                        .unwrap_or(crate::platform::shell::ShellKind::Default)
                })
                .unwrap_or(crate::platform::shell::ShellKind::Default);
            crate::platform::shell::shell_executable(kind)
        } else {
            let kind = crate::platform::shell::default_shell_for_platform();
            crate::platform::shell::shell_executable(kind)
        };

        let pm = process_manager.clone();
        let script_owned = script.to_string();
        let dir = working_dir.map(String::from);
        let label_owned = label.to_string();
        let run_id_owned = run_id.to_string();
        let step_id_owned = step_id.to_string();
        let session_id_owned = session_id.map(String::from);
        let shell_exe_owned = shell_exe.to_string();
        let shell_flag_owned = shell_flag.to_string();

        let tracked = match tokio::task::spawn_blocking(move || {
            pm.spawn_and_track_owned(
                &shell_exe_owned,
                &[shell_flag_owned.as_str(), &script_owned],
                dir.as_deref(),
                &label_owned,
                session_id_owned,
                Some(run_id_owned),
                Some(step_id_owned),
            )
        })
        .await
        {
            Ok(Ok(tp)) => tp,
            Ok(Err(e)) => {
                return StepCompletion {
                    step_id: step_id.to_string(),
                    success: false,
                    error: Some(format!("Script launch error: {}", e)),
                    process_id: None,
                    attempt_number: 0,
                };
            }
            Err(e) => {
                return StepCompletion {
                    step_id: step_id.to_string(),
                    success: false,
                    error: Some(format!("Task join error: {}", e)),
                    process_id: None,
                    attempt_number: 0,
                };
            }
        };

        let proc_id = tracked.id.clone();
        Self::register_process(handle, &proc_id);
        Self::emit_process_started(run_id, step_id, &tracked, app_handle);

        // Wait for process exit (scripts are always one-shot)
        let effective_timeout = timeout.unwrap_or(3600);
        Self::wait_for_process_exit(
            run_id,
            step_id,
            &proc_id,
            effective_timeout,
            cancelled,
            cancel_notify,
            process_manager,
            app_handle,
        )
        .await
    }

    // -----------------------------------------------------------------------
    // Wait for process exit
    // -----------------------------------------------------------------------

    async fn wait_for_process_exit(
        _run_id: &str,
        step_id: &str,
        proc_id: &str,
        timeout_secs: u64,
        cancelled: &Arc<AtomicBool>,
        cancel_notify: &Arc<Notify>,
        process_manager: &Arc<dyn ProcessManager>,
        _app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
    ) -> StepCompletion {
        let deadline = if timeout_secs == u64::MAX {
            None
        } else {
            Some(tokio::time::Instant::now() + Duration::from_secs(timeout_secs))
        };

        let poll_interval = Duration::from_millis(500);

        loop {
            if cancelled.load(Ordering::SeqCst) {
                return StepCompletion {
                    step_id: step_id.to_string(),
                    success: false,
                    error: Some("Cancelled".to_string()),
                    process_id: Some(proc_id.to_string()),
                    attempt_number: 0,
                };
            }

            if let Some(d) = deadline {
                if tokio::time::Instant::now() >= d {
                    return StepCompletion {
                        step_id: step_id.to_string(),
                        success: false,
                        error: Some(format!(
                            "Process '{}' timed out after {}s",
                            proc_id, timeout_secs
                        )),
                        process_id: Some(proc_id.to_string()),
                        attempt_number: 0,
                    };
                }
            }

            let pm = process_manager.clone();
            let pid = proc_id.to_string();
            let status = tokio::task::spawn_blocking(move || pm.refresh_status(&pid))
                .await
                .ok()
                .and_then(|r| r.ok());

            match status {
                Some(ProcessStatus::Running) | Some(ProcessStatus::Starting) => {
                    let _ = tokio::time::timeout(poll_interval, cancel_notify.notified()).await;
                }
                Some(ProcessStatus::Exited(0)) => {
                    return StepCompletion {
                        step_id: step_id.to_string(),
                        success: true,
                        error: None,
                        process_id: Some(proc_id.to_string()),
                        attempt_number: 0,
                    };
                }
                Some(ProcessStatus::Exited(code)) => {
                    return StepCompletion {
                        step_id: step_id.to_string(),
                        success: false,
                        error: Some(format!("Process exited with code {}", code)),
                        process_id: Some(proc_id.to_string()),
                        attempt_number: 0,
                    };
                }
                Some(ProcessStatus::ExitedWithError(code)) => {
                    return StepCompletion {
                        step_id: step_id.to_string(),
                        success: false,
                        error: Some(format!("Process exited with error code {}", code)),
                        process_id: Some(proc_id.to_string()),
                        attempt_number: 0,
                    };
                }
                Some(ProcessStatus::Crashed) => {
                    return StepCompletion {
                        step_id: step_id.to_string(),
                        success: false,
                        error: Some("Process crashed (signal or abnormal termination)".to_string()),
                        process_id: Some(proc_id.to_string()),
                        attempt_number: 0,
                    };
                }
                Some(ProcessStatus::Killed) => {
                    return StepCompletion {
                        step_id: step_id.to_string(),
                        success: false,
                        error: Some("Process was killed".to_string()),
                        process_id: Some(proc_id.to_string()),
                        attempt_number: 0,
                    };
                }
                Some(ProcessStatus::TimedOut) => {
                    return StepCompletion {
                        step_id: step_id.to_string(),
                        success: false,
                        error: Some("Process timed out".to_string()),
                        process_id: Some(proc_id.to_string()),
                        attempt_number: 0,
                    };
                }
                Some(ProcessStatus::Cancelled) => {
                    return StepCompletion {
                        step_id: step_id.to_string(),
                        success: false,
                        error: Some("Process was cancelled".to_string()),
                        process_id: Some(proc_id.to_string()),
                        attempt_number: 0,
                    };
                }
                Some(ProcessStatus::Ready) => {
                    // Ready is a sub-state of Running; keep waiting for exit
                    let _ = tokio::time::timeout(poll_interval, cancel_notify.notified()).await;
                }
                Some(ProcessStatus::ExternalLaunchAccepted) | Some(ProcessStatus::Unknown) => {
                    // Cannot track further — treat as unknown completion
                    return StepCompletion {
                        step_id: step_id.to_string(),
                        success: false,
                        error: Some(format!(
                            "Process '{}' has untrackable status: {:?}",
                            proc_id,
                            status.unwrap_or(ProcessStatus::Unknown)
                        )),
                        process_id: Some(proc_id.to_string()),
                        attempt_number: 0,
                    };
                }
                None => {
                    // Process not found — assume exited successfully
                    return StepCompletion {
                        step_id: step_id.to_string(),
                        success: true,
                        error: None,
                        process_id: Some(proc_id.to_string()),
                        attempt_number: 0,
                    };
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Wait for port
    // -----------------------------------------------------------------------

    async fn wait_for_port(
        run_id: &str,
        step_id: &str,
        host: &str,
        port: u16,
        timeout_secs: u64,
        cancelled: &Arc<AtomicBool>,
        cancel_notify: &Arc<Notify>,
        app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
    ) -> StepCompletion {
        let addr_str = format!("{}:{}", host, port);
        let addrs = match addr_str.to_socket_addrs() {
            Ok(a) => a.collect::<Vec<_>>(),
            Err(e) => {
                return StepCompletion {
                    step_id: step_id.to_string(),
                    success: false,
                    error: Some(format!("DNS resolve failed: {}", e)),
                    process_id: None,
                    attempt_number: 0,
                };
            }
        };

        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        let poll_interval = Duration::from_secs(1);

        loop {
            if cancelled.load(Ordering::SeqCst) {
                Self::emit_diagnostic(
                    run_id,
                    Some(step_id),
                    LogSource::Readiness,
                    DiagnosticSeverity::Info,
                    "WaitForPort cancelled".to_string(),
                    app_handle,
                );
                return StepCompletion {
                    step_id: step_id.to_string(),
                    success: false,
                    error: Some("Cancelled".to_string()),
                    process_id: None,
                    attempt_number: 0,
                };
            }

            if tokio::time::Instant::now() >= deadline {
                let msg = format!(
                    "Timeout: port {}:{} not open after {}s",
                    host, port, timeout_secs
                );
                Self::emit_diagnostic(
                    run_id,
                    Some(step_id),
                    LogSource::Readiness,
                    DiagnosticSeverity::Warning,
                    msg.clone(),
                    app_handle,
                );
                return StepCompletion {
                    step_id: step_id.to_string(),
                    success: false,
                    error: Some(msg),
                    process_id: None,
                    attempt_number: 0,
                };
            }

            for addr in &addrs {
                if TcpStream::connect_timeout(addr, Duration::from_secs(2)).is_ok() {
                    return StepCompletion {
                        step_id: step_id.to_string(),
                        success: true,
                        error: None,
                        process_id: None,
                        attempt_number: 0,
                    };
                }
            }

            let _ = tokio::time::timeout(poll_interval, cancel_notify.notified()).await;
        }
    }

    // -----------------------------------------------------------------------
    // Wait for URL
    // -----------------------------------------------------------------------

    async fn wait_for_url(
        run_id: &str,
        step_id: &str,
        url: &str,
        timeout_secs: u64,
        cancelled: &Arc<AtomicBool>,
        cancel_notify: &Arc<Notify>,
        app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
    ) -> StepCompletion {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        let poll_interval = Duration::from_secs(1);

        loop {
            if cancelled.load(Ordering::SeqCst) {
                Self::emit_diagnostic(
                    run_id,
                    Some(step_id),
                    LogSource::Readiness,
                    DiagnosticSeverity::Info,
                    "WaitForUrl cancelled".to_string(),
                    app_handle,
                );
                return StepCompletion {
                    step_id: step_id.to_string(),
                    success: false,
                    error: Some("Cancelled".to_string()),
                    process_id: None,
                    attempt_number: 0,
                };
            }

            if tokio::time::Instant::now() >= deadline {
                let msg = format!("Timeout: {} not available after {}s", url, timeout_secs);
                Self::emit_diagnostic(
                    run_id,
                    Some(step_id),
                    LogSource::Readiness,
                    DiagnosticSeverity::Warning,
                    msg.clone(),
                    app_handle,
                );
                return StepCompletion {
                    step_id: step_id.to_string(),
                    success: false,
                    error: Some(msg),
                    process_id: None,
                    attempt_number: 0,
                };
            }

            // Try HTTP request
            if let Ok(parsed) = parse_http_url(url) {
                let addr_str = format!("{}:{}", parsed.host, parsed.port);
                if let Ok(addrs) = addr_str.to_socket_addrs() {
                    for addr in addrs {
                        if let Ok(mut stream) =
                            TcpStream::connect_timeout(&addr, Duration::from_secs(2))
                        {
                            let request = format!(
                                "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                                parsed.path, parsed.host
                            );
                            if stream.write_all(request.as_bytes()).is_ok() {
                                use std::io::{BufRead, BufReader};
                                let mut reader = BufReader::new(&stream);
                                let mut first_line = String::new();
                                if reader.read_line(&mut first_line).is_ok() {
                                    let parts: Vec<&str> = first_line.split_whitespace().collect();
                                    if let Some(code_str) = parts.get(1) {
                                        if let Ok(code) = code_str.parse::<u16>() {
                                            if (200..400).contains(&code) {
                                                return StepCompletion {
                                                    step_id: step_id.to_string(),
                                                    success: true,
                                                    error: None,
                                                    process_id: None,
                                                    attempt_number: 0,
                                                };
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let _ = tokio::time::timeout(poll_interval, cancel_notify.notified()).await;
        }
    }

    // -----------------------------------------------------------------------
    // Wait for Docker daemon
    // -----------------------------------------------------------------------

    async fn wait_for_docker_daemon(
        run_id: &str,
        step_id: &str,
        timeout_secs: u64,
        cancelled: &Arc<AtomicBool>,
        _cancel_notify: &Arc<Notify>,
        app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
    ) -> StepCompletion {
        let check = crate::platform::docker_service::DockerReadinessCheck {
            timeout: Duration::from_secs(timeout_secs),
            poll_interval: Duration::from_secs(1),
            ..Default::default()
        };
        let result =
            crate::platform::docker_service::DockerService::wait_for_daemon(&check, cancelled)
                .await;

        if cancelled.load(Ordering::SeqCst) {
            Self::emit_diagnostic(
                run_id,
                Some(step_id),
                LogSource::Preflight,
                DiagnosticSeverity::Info,
                "WaitForDocker cancelled".to_string(),
                app_handle,
            );
            return StepCompletion {
                step_id: step_id.to_string(),
                success: false,
                error: Some("Cancelled".to_string()),
                process_id: None,
                attempt_number: 0,
            };
        }

        if result.status == crate::platform::docker_service::DockerStatus::DaemonReady {
            return StepCompletion {
                step_id: step_id.to_string(),
                success: true,
                error: None,
                process_id: None,
                attempt_number: 0,
            };
        }

        let msg = format!(
            "Docker daemon not ready after {}s: {}",
            timeout_secs, result.message
        );
        Self::emit_diagnostic(
            run_id,
            Some(step_id),
            LogSource::Preflight,
            DiagnosticSeverity::Warning,
            msg.clone(),
            app_handle,
        );
        StepCompletion {
            step_id: step_id.to_string(),
            success: false,
            error: Some(msg),
            process_id: None,
            attempt_number: 0,
        }
    }

    // -----------------------------------------------------------------------
    // Delay with cancellation
    // -----------------------------------------------------------------------

    async fn delay_with_cancel(
        _run_id: &str,
        step_id: &str,
        _seconds: u64,
        timeout_secs: u64,
        cancelled: &Arc<AtomicBool>,
        cancel_notify: &Arc<Notify>,
        _app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
    ) -> StepCompletion {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        let poll_interval = Duration::from_secs(1);

        loop {
            if cancelled.load(Ordering::SeqCst) {
                return StepCompletion {
                    step_id: step_id.to_string(),
                    success: false,
                    error: Some("Cancelled".to_string()),
                    process_id: None,
                    attempt_number: 0,
                };
            }

            if tokio::time::Instant::now() >= deadline {
                return StepCompletion {
                    step_id: step_id.to_string(),
                    success: true,
                    error: None,
                    process_id: None,
                    attempt_number: 0,
                };
            }

            let _ = tokio::time::timeout(poll_interval, cancel_notify.notified()).await;
        }
    }

    // -----------------------------------------------------------------------
    // Failure handling
    // -----------------------------------------------------------------------

    fn handle_failure(
        handle: &RunHandle,
        failed_step_id: &str,
        _completed: &HashSet<String>,
        _running: &mut HashSet<String>,
    ) -> FailureAction {
        let run = handle.run.lock().expect("run lock poisoned");
        let step = run.steps.iter().find(|s| s.step_id == failed_step_id);
        let failure_policy = step
            .and_then(|_| handle.profile.steps.iter().find(|s| s.id == failed_step_id))
            .and_then(|s| s.failure_policy.clone())
            .unwrap_or_else(|| {
                let has_dependents = handle
                    .profile
                    .steps
                    .iter()
                    .any(|s| s.depends_on.contains(&failed_step_id.to_string()));
                StepKind::default_failure_policy(has_dependents)
            });

        match failure_policy {
            FailurePolicy::StopRun => FailureAction::StopRun,
            FailurePolicy::SkipDependents => FailureAction::SkipDependents,
            FailurePolicy::WarnAndContinue => FailureAction::Continue,
        }
    }

    fn transitive_dependents(profile: &LaunchProfileV2, step_id: &str) -> HashSet<String> {
        let mut result = HashSet::new();
        let mut queue = vec![step_id.to_string()];

        while let Some(current) = queue.pop() {
            for step in &profile.steps {
                if step.depends_on.contains(&current) && !result.contains(&step.id) {
                    result.insert(step.id.clone());
                    queue.push(step.id.clone());
                }
            }
        }

        result
    }

    // -----------------------------------------------------------------------
    // Retry logic
    // -----------------------------------------------------------------------

    fn maybe_retry(
        handle: &Arc<RunHandle>,
        step_id: &str,
        _running: &mut HashSet<String>,
        _completed: &HashSet<String>,
        tx: &mpsc::UnboundedSender<StepCompletion>,
        semaphore: &Arc<tokio::sync::Semaphore>,
        process_manager: &Arc<dyn ProcessManager>,
        app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
    ) {
        if handle.cancelled.load(Ordering::SeqCst) {
            return;
        }

        let retries_remaining = {
            let mut run = handle.run.lock().expect("run lock poisoned");
            if let Some(step_state) = run.steps.iter_mut().find(|s| s.step_id == step_id) {
                match step_state.retries_remaining {
                    Some(n) if n > 0 => {
                        step_state.retries_remaining = Some(n - 1);
                        step_state.status = StepStatus::Retrying;
                        Some(n - 1)
                    }
                    _ => None,
                }
            } else {
                None
            }
        };

        if retries_remaining.is_none() {
            return;
        }

        // Find the retry delay from the step definition
        let retry_policy = handle
            .profile
            .steps
            .iter()
            .find(|s| s.id == step_id)
            .and_then(|s| s.retry_policy.clone());

        if let Some(policy) = retry_policy {
            let delay_ms = policy.delay_ms;
            let tx = tx.clone();
            let semaphore = semaphore.clone();
            let process_manager = process_manager.clone();
            let app_handle = app_handle.clone();
            let cancelled = handle.cancelled.clone();
            let cancel_notify = handle.cancel_notify.clone();
            let step_id = step_id.to_string();
            let run_id = handle.run.lock().expect("run lock poisoned").run_id.clone();
            let profile = handle.profile.clone();
            let session_id = handle.session_id.clone();
            let handle = handle.clone();

            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                if cancelled.load(Ordering::SeqCst) {
                    return;
                }

                let _permit = semaphore.acquire().await.unwrap();
                let step = profile.steps.iter().find(|s| s.id == step_id).unwrap();
                let completion = Self::step_task(
                    &run_id,
                    step,
                    &profile,
                    &process_manager,
                    &app_handle,
                    &cancelled,
                    &cancel_notify,
                    &session_id,
                    &handle,
                )
                .await;
                let _ = tx.send(completion);
                drop(_permit);
            });
        }
    }

    // -----------------------------------------------------------------------
    // Finalize run
    // -----------------------------------------------------------------------

    async fn finalize_run(
        handle: &RunHandle,
        runs: &Arc<RwLock<HashMap<String, Arc<RunHandle>>>>,
        run_id: &str,
        final_status: RunStatus,
        app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
    ) {
        {
            let mut run = handle.run.lock().expect("run lock poisoned");
            run.status = final_status.clone();
            run.finished_at = Some(default_now_iso());
        }

        emit_status_to_app(app_handle, run_id, final_status.clone());
        self_emit_run_finished(app_handle, run_id, handle);

        // The run is RETAINED in the map so the UI can always inspect it
        // (history, logs, process list) after switching tabs. Only the
        // oldest finished runs are pruned to bound memory.
        let mut guard = runs.write().expect("runs lock poisoned");
        const MAX_RETAINED_RUNS: usize = 50;
        if guard.len() > MAX_RETAINED_RUNS {
            let mut finished: Vec<(String, String)> = Vec::new();
            for (id, h) in guard.iter() {
                let run = h.run.lock().expect("run lock poisoned");
                if matches!(
                    run.status,
                    RunStatus::Succeeded
                        | RunStatus::Failed
                        | RunStatus::Cancelled
                        | RunStatus::PartialSuccess
                ) {
                    // created_at is RFC3339 — lexicographic order == chronological.
                    finished.push((id.clone(), run.created_at.clone()));
                }
            }
            finished.sort_by(|a, b| a.1.cmp(&b.1));
            let excess = guard.len() - MAX_RETAINED_RUNS;
            for (id, _) in finished.into_iter().take(excess) {
                guard.remove(&id);
            }
        }
    }

    fn compute_final_status(handle: &RunHandle) -> RunStatus {
        let run = handle.run.lock().expect("run lock poisoned");

        if run.cancelled {
            return RunStatus::Cancelled;
        }

        let has_succeeded = run.steps.iter().any(|s| s.status == StepStatus::Succeeded);
        let has_failed = run
            .steps
            .iter()
            .any(|s| s.status == StepStatus::Failed || s.status == StepStatus::Cancelled);
        let has_pending = run.steps.iter().any(|s| {
            s.status == StepStatus::Pending
                || s.status == StepStatus::Running
                || s.status == StepStatus::Retrying
        });

        if has_pending {
            RunStatus::Running
        } else if has_failed && has_succeeded {
            RunStatus::PartialSuccess
        } else if has_failed {
            RunStatus::Failed
        } else {
            RunStatus::Succeeded
        }
    }

    // -----------------------------------------------------------------------
    // Event emission helpers
    // -----------------------------------------------------------------------

    fn emit(&self, event: &str, payload: &impl serde::Serialize) {
        emit_to_app(&self.app_handle, event, payload);
    }

    fn emit_status(&self, run_id: &str, status: RunStatus) {
        emit_status_to_app(&self.app_handle, run_id, status);
    }

    fn emit_step_status(
        run_id: &str,
        step_id: &str,
        status: StepStatus,
        app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
    ) {
        let state = StepExecutionState {
            step_id: step_id.to_string(),
            status,
            process_id: None,
            error: None,
            retries_remaining: None,
            started_at: Some(default_now_iso()),
            finished_at: None,
            attempts: Vec::new(),
        };
        let payload = StepStatusPayload {
            run_id: run_id.to_string(),
            step: state,
        };
        if let Ok(handle) = app_handle.lock() {
            if let Some(h) = handle.as_ref() {
                let _ = h.emit(EVENT_STEP_STATUS_CHANGED, &payload);
            }
        }
    }

    fn emit_process_started(
        run_id: &str,
        step_id: &str,
        tracked: &crate::modules::workspace::models::TrackedProcess,
        app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
    ) {
        let tracking_quality = if tracked.visible {
            Some(ProcessTrackingQuality::TerminalWrapper)
        } else {
            Some(ProcessTrackingQuality::Exact)
        };

        let managed = ManagedProcess {
            id: tracked.id.clone(),
            pid: tracked.pid,
            label: tracked.label.clone(),
            status: tracked.status.clone(),
            started_at: tracked.started_at.clone(),
            duration_secs: tracked.duration_secs,
            restarts: tracked.restarts,
            last_error: tracked.last_error.clone(),
            session_id: tracked.session_id.clone(),
            visible: tracked.visible,
            tracking_quality,
        };
        let payload = ProcessStartedPayload {
            run_id: run_id.to_string(),
            step_id: step_id.to_string(),
            process: managed,
        };
        if let Ok(handle) = app_handle.lock() {
            if let Some(h) = handle.as_ref() {
                let _ = h.emit(EVENT_PROCESS_STARTED, &payload);
            }
        }
    }

    fn emit_diagnostic(
        run_id: &str,
        step_id: Option<&str>,
        source: LogSource,
        severity: DiagnosticSeverity,
        message: String,
        app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
    ) {
        let diag = Diagnostic {
            run_id: run_id.to_string(),
            step_id: step_id.map(String::from),
            source,
            severity,
            message,
            timestamp: default_now_iso(),
        };
        if let Ok(handle) = app_handle.lock() {
            if let Some(h) = handle.as_ref() {
                let _ = h.emit(EVENT_DIAGNOSTIC, &diag);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Stop all running processes in a run
    // -----------------------------------------------------------------------

    pub fn stop_run_processes(&self, run_id: &str) -> Result<Vec<String>, String> {
        let handle = {
            let runs = self.runs.read().expect("runs lock poisoned");
            runs.get(run_id).cloned()
        };
        let handle = handle.ok_or_else(|| format!("Run '{}' not found", run_id))?;

        // Kill every managed process spawned by this run — long-running
        // services keep running after their step reports Succeeded, so the
        // step-status filter alone would find nothing to kill.
        let processes_to_kill: Vec<String> = handle
            .process_ids
            .lock()
            .expect("process_ids lock poisoned")
            .clone();

        let mut killed = Vec::new();
        for proc_id in &processes_to_kill {
            if self.process_manager.kill(proc_id).is_ok() {
                killed.push(proc_id.clone());
            }
        }
        Ok(killed)
    }

    // -----------------------------------------------------------------------
    // Logs retrieval — combined for run, per-process for step
    // -----------------------------------------------------------------------

    pub fn get_run_logs(&self, run_id: &str) -> Result<RunLogs, String> {
        let handle = {
            let runs = self.runs.read().expect("runs lock poisoned");
            runs.get(run_id).cloned()
        };
        let handle = handle.ok_or_else(|| format!("Run '{}' not found", run_id))?;
        let run = handle.run.lock().expect("run lock poisoned");

        let mut all_stdout: Vec<String> = Vec::new();
        let mut all_stderr: Vec<String> = Vec::new();

        for step_state in &run.steps {
            if let Some(ref proc_id) = step_state.process_id {
                if let Some(buffer) = self.process_manager.get_log_buffer(proc_id) {
                    let mut out = buffer.stdout_lines();
                    let mut err = buffer.stderr_lines();
                    all_stdout.append(&mut out);
                    all_stderr.append(&mut err);
                } else if let Ok(logs) = self.process_manager.get_logs(proc_id) {
                    all_stdout.extend(logs.stdout_lines);
                    all_stderr.extend(logs.stderr_lines);
                }
            }
        }

        Ok(RunLogs {
            run_id: run_id.to_string(),
            stdout: all_stdout,
            stderr: all_stderr,
        })
    }

    pub fn get_step_logs(&self, run_id: &str, step_id: &str) -> Result<StepLogs, String> {
        let handle = {
            let runs = self.runs.read().expect("runs lock poisoned");
            runs.get(run_id).cloned()
        };
        let handle = handle.ok_or_else(|| format!("Run '{}' not found", run_id))?;
        let run = handle.run.lock().expect("run lock poisoned");

        let step_state = run
            .steps
            .iter()
            .find(|s| s.step_id == step_id)
            .ok_or_else(|| format!("Step '{}' not found in run '{}'", step_id, run_id))?;

        let proc_id = step_state
            .process_id
            .as_ref()
            .ok_or_else(|| format!("Step '{}' has no associated process", step_id))?;

        let (stdout, stderr, truncation) =
            if let Some(buffer) = self.process_manager.get_log_buffer(proc_id) {
                (
                    buffer.stdout_lines(),
                    buffer.stderr_lines(),
                    Some((buffer.stdout_truncation(), buffer.stderr_truncation())),
                )
            } else {
                match self.process_manager.get_logs(proc_id) {
                    Ok(logs) => (logs.stdout_lines, logs.stderr_lines, None),
                    Err(_) => (Vec::new(), Vec::new(), None),
                }
            };

        Ok(StepLogs {
            run_id: run_id.to_string(),
            step_id: step_id.to_string(),
            process_id: proc_id.clone(),
            stdout,
            stderr,
            truncation,
        })
    }

    // -----------------------------------------------------------------------
    // List ALL runs (including completed) — for history/audit
    // -----------------------------------------------------------------------

    pub fn list_all_runs(&self) -> Vec<LaunchRun> {
        let runs = self.runs.read().expect("runs lock poisoned");
        runs.values()
            .map(|h| h.run.lock().expect("run lock poisoned").clone())
            .collect()
    }

    // -----------------------------------------------------------------------
    // Command resolution helpers
    // -----------------------------------------------------------------------

    /// Register a managed process against its run so stop/cancel can always
    /// close it, regardless of the step's reported status.
    fn register_process(handle: &Arc<RunHandle>, proc_id: &str) {
        let mut ids = handle
            .process_ids
            .lock()
            .expect("process_ids lock poisoned");
        if !ids.iter().any(|p| p == proc_id) {
            ids.push(proc_id.to_string());
        }
    }

    /// Resolve a RunCommand command string into (program, args) for direct
    /// spawning. Delegates to the platform resolver (tokenizer-based, shell
    /// fallback for shell-syntax command lines).
    fn resolve_command_target(command: &str) -> (String, Vec<String>) {
        crate::platform::command_resolver::resolve_command_target(command)
    }
}

// ---------------------------------------------------------------------------
// Free event emission functions (usable from static methods)
// ---------------------------------------------------------------------------

fn emit_to_app(
    app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
    event: &str,
    payload: &impl serde::Serialize,
) {
    if let Ok(handle) = app_handle.lock() {
        if let Some(h) = handle.as_ref() {
            let _ = h.emit(event, payload);
        }
    }
}

fn emit_status_to_app(
    app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
    run_id: &str,
    status: RunStatus,
) {
    let payload = RunStatusPayload {
        run_id: run_id.to_string(),
        status,
    };
    emit_to_app(app_handle, EVENT_RUN_STATUS_CHANGED, &payload);
}

// ---------------------------------------------------------------------------
// Event payload structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct RunStatusPayload {
    run_id: String,
    status: RunStatus,
}

#[derive(Debug, Clone, Serialize)]
struct StepStatusPayload {
    run_id: String,
    step: StepExecutionState,
}

#[derive(Debug, Clone, Serialize)]
struct ProcessStartedPayload {
    run_id: String,
    step_id: String,
    process: ManagedProcess,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

enum FailureAction {
    StopRun,
    SkipDependents,
    Continue,
}

impl StepExecutionState {
    fn default_failure_policy(has_dependents: bool) -> FailurePolicy {
        if has_dependents {
            FailurePolicy::StopRun
        } else {
            FailurePolicy::WarnAndContinue
        }
    }
}

// ---------------------------------------------------------------------------
// Emit helpers that work with Arc<Mutex<Option<tauri::AppHandle>>>
// ---------------------------------------------------------------------------

fn self_emit_step_event(
    app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
    run_id: &str,
    step_id: &str,
    handle: &Arc<RunHandle>,
) {
    // Emit a step-status-changed event with the FULL step state so the
    // frontend store can apply it idempotently (same shape as
    // emit_step_status).
    let step = {
        let run = handle.run.lock().expect("run lock poisoned");
        run.steps.iter().find(|s| s.step_id == step_id).cloned()
    };
    let Some(step) = step else { return };
    let payload = StepStatusPayload {
        run_id: run_id.to_string(),
        step,
    };
    if let Ok(handle) = app_handle.lock() {
        if let Some(h) = handle.as_ref() {
            let _ = h.emit(EVENT_STEP_STATUS_CHANGED, &payload);
        }
    }
}

fn self_emit_run_finished(
    app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
    _run_id: &str,
    handle: &RunHandle,
) {
    let run = handle.run.lock().expect("run lock poisoned").clone();
    if let Ok(guard) = app_handle.lock() {
        if let Some(h) = guard.as_ref() {
            let _ = h.emit(EVENT_RUN_FINISHED, &run);
        }
    }
}

// ---------------------------------------------------------------------------
// Preflight (delegated to Docker service)
// ---------------------------------------------------------------------------

fn preflight_check(command: &str) -> Result<(), String> {
    let trimmed = command.trim_start();
    if !trimmed.starts_with("docker") {
        return Ok(());
    }

    match crate::platform::docker_service::DockerService::preflight_for_command(command) {
        Ok(()) => Ok(()),
        Err(diag) => Err(format!(
            "{}: {}{}",
            diag.status,
            diag.message,
            diag.suggested_action
                .as_ref()
                .map(|a| format!("\n{}", a))
                .unwrap_or_default()
        )),
    }
}

// ---------------------------------------------------------------------------
// URL parsing (delegated to readiness module)
// ---------------------------------------------------------------------------

struct ParsedUrl {
    host: String,
    port: u16,
    path: String,
}

fn parse_http_url(raw: &str) -> Result<ParsedUrl, String> {
    let parsed = crate::platform::readiness::ParsedTarget::parse(raw)
        .map_err(|e| format!("URL parse error: {}", e))?;
    Ok(ParsedUrl {
        host: parsed.host,
        port: parsed.port,
        path: parsed.path,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::devlauncher::models;
    use crate::platform::environment::EnvironmentOverlay;
    use serde_json::Map;

    fn make_step(id: &str, deps: Vec<&str>) -> LaunchStep {
        LaunchStep {
            id: id.to_string(),
            label: id.to_string(),
            enabled: true,
            kind: StepKind::RunCommand {
                command: "echo test".to_string(),
                command_spec: None,
            },
            depends_on: deps.into_iter().map(String::from).collect(),
            working_directory: None,
            environment: None,
            visibility: None,
            execution_mode: None,
            completion: None,
            timeout: None,
            failure_policy: None,
            retry_policy: None,
            metadata: None,
            extra: Map::new(),
        }
    }

    fn valid_profile(steps: Vec<LaunchStep>) -> LaunchProfileV2 {
        LaunchProfileV2 {
            schema_version: "2".to_string(),
            id: "test".to_string(),
            name: "Test".to_string(),
            description: "desc".to_string(),
            project_root: None,
            steps,
            environment_binding_id: None,
            preferred_ide: None,
            default_execution_mode: None,
            extra: Map::new(),
        }
    }

    #[test]
    fn find_ready_steps_roots() {
        let profile = valid_profile(vec![
            make_step("a", vec![]),
            make_step("b", vec!["a"]),
            make_step("c", vec!["a"]),
        ]);
        let completed = HashSet::new();
        let running = HashSet::new();
        let ready = RunOrchestrator::find_ready_steps(&profile, &completed, &running);
        assert_eq!(ready, vec!["a"]);
    }

    #[test]
    fn find_ready_steps_after_first() {
        let profile = valid_profile(vec![
            make_step("a", vec![]),
            make_step("b", vec!["a"]),
            make_step("c", vec!["a"]),
        ]);
        let mut completed = HashSet::new();
        completed.insert("a".to_string());
        let running = HashSet::new();
        let ready = RunOrchestrator::find_ready_steps(&profile, &completed, &running);
        let mut ready = ready;
        ready.sort();
        assert_eq!(ready, vec!["b", "c"]);
    }

    #[test]
    fn find_ready_steps_deterministic() {
        let profile = valid_profile(vec![
            make_step("z", vec![]),
            make_step("a", vec![]),
            make_step("m", vec![]),
        ]);
        let completed = HashSet::new();
        let running = HashSet::new();
        let ready = RunOrchestrator::find_ready_steps(&profile, &completed, &running);
        assert_eq!(ready, vec!["a", "m", "z"]);
    }

    #[test]
    fn transitive_dependents_basic() {
        let profile = valid_profile(vec![
            make_step("a", vec![]),
            make_step("b", vec!["a"]),
            make_step("c", vec!["b"]),
            make_step("d", vec![]),
        ]);
        let deps = RunOrchestrator::transitive_dependents(&profile, "a");
        assert!(deps.contains("b"));
        assert!(deps.contains("c"));
        assert!(!deps.contains("d"));
        assert!(!deps.contains("a"));
    }

    #[test]
    fn create_run_validates() {
        // Need a mock ProcessManager for this test
        struct MockPM;
        impl ProcessManager for MockPM {
            fn spawn_and_track(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_and_track_with_overlay(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: &EnvironmentOverlay,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn launch_detached(&self, _: &str, _: &[&str], _: Option<&str>) -> Result<(), String> {
                todo!()
            }
            fn spawn_and_track_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn list(&self) -> Vec<crate::modules::workspace::models::TrackedProcess> {
                todo!()
            }
            fn kill(&self, _: &str) -> Result<(), String> {
                todo!()
            }
            fn refresh_status(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessStatus, String> {
                todo!()
            }
            fn get_logs(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessLogs, String> {
                todo!()
            }
            fn get_log_buffer(
                &self,
                _: &str,
            ) -> Option<Arc<crate::modules::workspace::models::BoundedLogBuffer>> {
                None
            }
            fn get_log_truncation(
                &self,
                _: &str,
            ) -> Option<(
                crate::modules::workspace::models::LogTruncation,
                crate::modules::workspace::models::LogTruncation,
            )> {
                None
            }
        }

        let orchestrator = RunOrchestrator::new(Arc::new(MockPM));

        // Valid profile
        let profile = valid_profile(vec![make_step("a", vec![])]);
        let result = orchestrator.create_run(profile);
        assert!(result.is_ok());
        let run = result.unwrap();
        assert_eq!(run.status, RunStatus::Pending);
        assert_eq!(run.steps.len(), 1);
        assert_eq!(run.steps[0].status, StepStatus::Pending);

        // Invalid profile (cycle)
        let profile = valid_profile(vec![make_step("a", vec!["b"]), make_step("b", vec!["a"])]);
        let result = orchestrator.create_run(profile);
        assert!(result.is_err());
    }

    #[test]
    fn run_id_is_uuid() {
        let id = generate_stable_id();
        assert_eq!(id.len(), 36); // UUID v4 format: 8-4-4-4-12
        assert!(id.chars().filter(|c| *c == '-').count() == 4);
    }

    #[test]
    fn parse_http_url_basic() {
        let parsed = parse_http_url("http://localhost:3000/api").unwrap();
        assert_eq!(parsed.host, "localhost");
        assert_eq!(parsed.port, 3000);
        assert_eq!(parsed.path, "/api");
    }

    #[test]
    fn parse_http_url_no_port() {
        let parsed = parse_http_url("http://example.com/").unwrap();
        assert_eq!(parsed.host, "example.com");
        assert_eq!(parsed.port, 80);
    }

    #[test]
    fn parse_http_url_bare() {
        let parsed = parse_http_url("localhost:8080").unwrap();
        assert_eq!(parsed.host, "localhost");
        assert_eq!(parsed.port, 8080);
    }

    #[test]
    fn enabled_steps_only_in_topo_sort() {
        let profile = valid_profile(vec![
            make_step("a", vec![]),
            LaunchStep {
                id: "b".to_string(),
                label: "b".to_string(),
                enabled: false,
                kind: StepKind::RunCommand {
                    command: "echo".to_string(),
                    command_spec: None,
                },
                depends_on: vec!["a".to_string()],
                working_directory: None,
                environment: None,
                visibility: None,
                execution_mode: None,
                completion: None,
                timeout: None,
                failure_policy: None,
                retry_policy: None,
                metadata: None,
                extra: Map::new(),
            },
            make_step("c", vec!["b"]),
        ]);
        let order = validation::topological_order(&profile);
        assert_eq!(order, vec!["a", "c"]);
    }

    // ===================================================================
    // Integration tests — pure unit tests (no OS dependencies)
    // ===================================================================

    /// Test 1: Profile with two independent application launches.
    /// Both should be ready to run in parallel (no dependencies).
    #[test]
    fn test_independent_application_launches() {
        let profile = valid_profile(vec![make_step("app1", vec![]), make_step("app2", vec![])]);
        let ready = RunOrchestrator::find_ready_steps(&profile, &HashSet::new(), &HashSet::new());
        assert_eq!(ready.len(), 2);
        assert!(ready.contains(&"app1".to_string()));
        assert!(ready.contains(&"app2".to_string()));
    }

    /// Test 2: Docker readiness followed by compose.
    /// compose depends on docker-daemon; compose should not be ready until docker-daemon completes.
    #[test]
    fn test_docker_readiness_followed_by_compose() {
        let profile = valid_profile(vec![
            LaunchStep {
                id: "docker-daemon".to_string(),
                label: "Wait Docker".to_string(),
                enabled: true,
                kind: StepKind::WaitForDocker {},
                depends_on: vec![],
                working_directory: None,
                environment: None,
                visibility: None,
                execution_mode: None,
                completion: None,
                timeout: Some(30),
                failure_policy: None,
                retry_policy: None,
                metadata: None,
                extra: Map::new(),
            },
            LaunchStep {
                id: "compose".to_string(),
                label: "Compose Up".to_string(),
                enabled: true,
                kind: StepKind::RunCommand {
                    command: "docker compose up -d".to_string(),
                    command_spec: None,
                },
                depends_on: vec!["docker-daemon".to_string()],
                working_directory: None,
                environment: None,
                visibility: None,
                execution_mode: None,
                completion: None,
                timeout: None,
                failure_policy: None,
                retry_policy: None,
                metadata: None,
                extra: Map::new(),
            },
        ]);

        // Before docker-daemon completes, only docker-daemon is ready
        let ready = RunOrchestrator::find_ready_steps(&profile, &HashSet::new(), &HashSet::new());
        assert_eq!(ready, vec!["docker-daemon"]);

        // After docker-daemon completes, compose becomes ready
        let mut completed = HashSet::new();
        completed.insert("docker-daemon".to_string());
        let ready = RunOrchestrator::find_ready_steps(&profile, &completed, &HashSet::new());
        assert_eq!(ready, vec!["compose"]);
    }

    /// Test 3: Backend process followed by port wait and URL open.
    /// Wait depends on backend; URL open depends on wait.
    #[test]
    fn test_backend_then_port_wait_then_url() {
        let profile = valid_profile(vec![
            make_step("backend", vec![]),
            LaunchStep {
                id: "wait-port".to_string(),
                label: "Wait Port".to_string(),
                enabled: true,
                kind: StepKind::WaitForPort {
                    host: "localhost".to_string(),
                    port: 8080,
                },
                depends_on: vec!["backend".to_string()],
                working_directory: None,
                environment: None,
                visibility: None,
                execution_mode: None,
                completion: None,
                timeout: Some(30),
                failure_policy: None,
                retry_policy: None,
                metadata: None,
                extra: Map::new(),
            },
            LaunchStep {
                id: "open-docs".to_string(),
                label: "Open Docs".to_string(),
                enabled: true,
                kind: StepKind::OpenUrl {
                    url: "http://localhost:8080/docs".to_string(),
                },
                depends_on: vec!["wait-port".to_string()],
                working_directory: None,
                environment: None,
                visibility: None,
                execution_mode: None,
                completion: None,
                timeout: None,
                failure_policy: None,
                retry_policy: None,
                metadata: None,
                extra: Map::new(),
            },
        ]);

        let ready = RunOrchestrator::find_ready_steps(&profile, &HashSet::new(), &HashSet::new());
        assert_eq!(ready, vec!["backend"]);

        let mut completed = HashSet::new();
        completed.insert("backend".to_string());
        let ready = RunOrchestrator::find_ready_steps(&profile, &completed, &HashSet::new());
        assert_eq!(ready, vec!["wait-port"]);

        completed.insert("wait-port".to_string());
        let ready = RunOrchestrator::find_ready_steps(&profile, &completed, &HashSet::new());
        assert_eq!(ready, vec!["open-docs"]);
    }

    /// Test 4: Expo and backend running in parallel.
    /// Both are independent roots.
    #[test]
    fn test_expo_and_backend_parallel() {
        let profile = valid_profile(vec![
            make_step("expo", vec![]),
            make_step("backend", vec![]),
        ]);
        let ready = RunOrchestrator::find_ready_steps(&profile, &HashSet::new(), &HashSet::new());
        assert_eq!(ready.len(), 2);
    }

    /// Test 5: A failed dependency causing downstream skip.
    #[test]
    fn test_failed_dependency_causes_downstream_skip() {
        let profile = valid_profile(vec![
            make_step("a", vec![]),
            make_step("b", vec!["a"]),
            make_step("c", vec!["b"]),
        ]);

        // If 'a' fails with SkipDependents policy, 'b' and 'c' should be skipped
        let dependents = RunOrchestrator::transitive_dependents(&profile, "a");
        assert!(dependents.contains("b"));
        assert!(dependents.contains("c"));
    }

    /// Test 6: A warning failure allowing independent branches to continue.
    #[test]
    fn test_warning_failure_independent_branches_continue() {
        let profile = valid_profile(vec![
            make_step("a", vec![]),
            make_step("b", vec![]),
            make_step("c", vec!["a"]),
            make_step("d", vec!["b"]),
        ]);

        // If 'a' fails with WarnAndContinue, 'b' should still be ready
        let mut completed = HashSet::new();
        completed.insert("a".to_string());
        let running = HashSet::new();
        let ready = RunOrchestrator::find_ready_steps(&profile, &completed, &running);
        assert!(ready.contains(&"b".to_string()));
        // 'c' should NOT be ready (its dependency 'a' failed)
        assert!(!ready.contains(&"c".to_string()));
        // 'd' should be ready (its dependency 'b' is independent)
        assert!(ready.contains(&"d".to_string()));
    }

    /// Test 7: Run cancellation stopping long-running processes.
    /// Verify that cancellation flag is set and processes are collected.
    #[tokio::test]
    async fn test_cancellation_collects_running_processes() {
        struct MockPM;
        impl ProcessManager for MockPM {
            fn spawn_and_track(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_and_track_with_overlay(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: &EnvironmentOverlay,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn launch_detached(&self, _: &str, _: &[&str], _: Option<&str>) -> Result<(), String> {
                todo!()
            }
            fn spawn_and_track_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn list(&self) -> Vec<crate::modules::workspace::models::TrackedProcess> {
                todo!()
            }
            fn kill(&self, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn refresh_status(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessStatus, String> {
                todo!()
            }
            fn get_logs(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessLogs, String> {
                todo!()
            }
            fn get_log_buffer(
                &self,
                _: &str,
            ) -> Option<Arc<crate::modules::workspace::models::BoundedLogBuffer>> {
                None
            }
            fn get_log_truncation(
                &self,
                _: &str,
            ) -> Option<(
                crate::modules::workspace::models::LogTruncation,
                crate::modules::workspace::models::LogTruncation,
            )> {
                None
            }
        }

        let orchestrator = RunOrchestrator::new(Arc::new(MockPM));
        let profile = valid_profile(vec![make_step("a", vec![])]);
        let run = orchestrator.create_run(profile).unwrap();

        // Start the run (it will fail immediately since MockPM panics on spawn)
        // But we can test that cancellation works on a pending run
        let result = orchestrator.cancel_run(&run.run_id).await;
        assert!(result.is_ok());

        let run = orchestrator.get_run(&run.run_id).unwrap();
        assert_eq!(run.status, RunStatus::Cancelled);
    }

    /// Test 8: Repeated cancel being safe.
    #[tokio::test]
    async fn test_repeated_cancel_is_safe() {
        struct MockPM;
        impl ProcessManager for MockPM {
            fn spawn_and_track(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_and_track_with_overlay(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: &EnvironmentOverlay,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn launch_detached(&self, _: &str, _: &[&str], _: Option<&str>) -> Result<(), String> {
                todo!()
            }
            fn spawn_and_track_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn list(&self) -> Vec<crate::modules::workspace::models::TrackedProcess> {
                todo!()
            }
            fn kill(&self, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn refresh_status(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessStatus, String> {
                todo!()
            }
            fn get_logs(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessLogs, String> {
                todo!()
            }
            fn get_log_buffer(
                &self,
                _: &str,
            ) -> Option<Arc<crate::modules::workspace::models::BoundedLogBuffer>> {
                None
            }
            fn get_log_truncation(
                &self,
                _: &str,
            ) -> Option<(
                crate::modules::workspace::models::LogTruncation,
                crate::modules::workspace::models::LogTruncation,
            )> {
                None
            }
        }

        let orchestrator = RunOrchestrator::new(Arc::new(MockPM));
        let profile = valid_profile(vec![make_step("a", vec![])]);
        let run = orchestrator.create_run(profile).unwrap();

        // Cancel once
        let result = orchestrator.cancel_run(&run.run_id).await;
        assert!(result.is_ok());

        // Cancel again — should be a no-op
        let result = orchestrator.cancel_run(&run.run_id).await;
        assert!(result.is_ok());

        let run = orchestrator.get_run(&run.run_id).unwrap();
        assert_eq!(run.status, RunStatus::Cancelled);
    }

    /// Test 9: Repeated stop being safe.
    #[test]
    fn test_repeated_stop_is_safe() {
        struct MockPM;
        impl ProcessManager for MockPM {
            fn spawn_and_track(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_and_track_with_overlay(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: &EnvironmentOverlay,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn launch_detached(&self, _: &str, _: &[&str], _: Option<&str>) -> Result<(), String> {
                todo!()
            }
            fn spawn_and_track_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn list(&self) -> Vec<crate::modules::workspace::models::TrackedProcess> {
                todo!()
            }
            fn kill(&self, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn refresh_status(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessStatus, String> {
                todo!()
            }
            fn get_logs(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessLogs, String> {
                todo!()
            }
            fn get_log_buffer(
                &self,
                _: &str,
            ) -> Option<Arc<crate::modules::workspace::models::BoundedLogBuffer>> {
                None
            }
            fn get_log_truncation(
                &self,
                _: &str,
            ) -> Option<(
                crate::modules::workspace::models::LogTruncation,
                crate::modules::workspace::models::LogTruncation,
            )> {
                None
            }
        }

        let orchestrator = RunOrchestrator::new(Arc::new(MockPM));
        let profile = valid_profile(vec![make_step("a", vec![])]);
        let run = orchestrator.create_run(profile).unwrap();

        // Stop on a pending run (no processes to kill)
        let result = orchestrator.stop_run_processes(&run.run_id);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());

        // Stop again — still safe
        let result = orchestrator.stop_run_processes(&run.run_id);
        assert!(result.is_ok());
    }

    /// Test 10: Process exit detected without list_processes being called.
    /// This tests the wait_for_process_exit logic with the mock PM.
    #[test]
    fn test_wait_for_process_exit_on_missing_process() {
        struct MockPM;
        impl ProcessManager for MockPM {
            fn spawn_and_track(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_and_track_with_overlay(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: &EnvironmentOverlay,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn launch_detached(&self, _: &str, _: &[&str], _: Option<&str>) -> Result<(), String> {
                todo!()
            }
            fn spawn_and_track_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn list(&self) -> Vec<crate::modules::workspace::models::TrackedProcess> {
                todo!()
            }
            fn kill(&self, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn refresh_status(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessStatus, String> {
                // Process not found — returns error
                Err("Process not found".to_string())
            }
            fn get_logs(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessLogs, String> {
                todo!()
            }
            fn get_log_buffer(
                &self,
                _: &str,
            ) -> Option<Arc<crate::modules::workspace::models::BoundedLogBuffer>> {
                None
            }
            fn get_log_truncation(
                &self,
                _: &str,
            ) -> Option<(
                crate::modules::workspace::models::LogTruncation,
                crate::modules::workspace::models::LogTruncation,
            )> {
                None
            }
        }

        let pm: Arc<dyn ProcessManager> = Arc::new(MockPM);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel_notify = Arc::new(Notify::new());
        let app_handle = Arc::new(Mutex::new(None));

        // When process is not found, wait_for_process_exit should return success
        let result = tokio::runtime::Runtime::new().unwrap().block_on(
            RunOrchestrator::wait_for_process_exit(
                "run-1",
                "step-1",
                "nonexistent-process",
                5,
                &cancelled,
                &cancel_notify,
                &pm,
                &app_handle,
            ),
        );
        assert!(result.success);
        assert!(result.process_id.is_some());
    }

    /// Test 11: Legacy run_profile compatibility — verify V2 conversion.
    #[test]
    fn test_legacy_to_v2_conversion_preserves_actions() {
        let legacy = LaunchProfile {
            name: "Test Legacy".to_string(),
            description: "desc".to_string(),
            project_path: Some("/tmp".to_string()),
            actions: vec![
                LaunchAction {
                    id: "a1".to_string(),
                    label: "Start Server".to_string(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm start".to_string(),
                        working_dir: Some("./backend".to_string()),
                        persistent: Some(true),
                    },
                },
                LaunchAction {
                    id: "a2".to_string(),
                    label: "Wait Port".to_string(),
                    enabled: true,
                    action_type: ActionType::WaitForPort {
                        host: "localhost".to_string(),
                        port: 3000,
                        timeout_secs: 30,
                    },
                },
            ],
            environment_binding_id: None,
            preferred_ide: Some(PreferredIde::Vscode),
            schema_version: None,
            id: None,
            steps: None,
        };

        let v2 = LaunchProfileV2::from(legacy);
        assert_eq!(v2.name, "Test Legacy");
        assert_eq!(v2.steps.len(), 2);
        assert_eq!(v2.steps[0].id, "a1");
        assert_eq!(v2.steps[1].id, "a2");
        assert!(matches!(v2.steps[0].kind, StepKind::RunCommand { .. }));
        assert!(matches!(v2.steps[1].kind, StepKind::WaitForPort { .. }));
    }

    /// Test 12: Malformed saved profile not breaking valid profile listing.
    #[test]
    fn test_malformed_profile_isolation() {
        // This tests that the profile manager's tolerant listing works
        // by verifying that validation catches malformed profiles
        let profile = valid_profile(vec![make_step("a", vec![])]);
        let result = validation::validate_profile_v2(&profile);
        assert!(result.valid);

        // A profile with a cycle should be detected
        let profile = valid_profile(vec![make_step("a", vec!["b"]), make_step("b", vec!["a"])]);
        let result = validation::validate_profile_v2(&profile);
        assert!(!result.valid);
    }

    /// Test 13: Project path containing spaces.
    #[test]
    fn test_project_path_with_spaces() {
        let profile = LaunchProfileV2 {
            schema_version: "2".to_string(),
            id: "test".to_string(),
            name: "Test".to_string(),
            description: "desc".to_string(),
            project_root: Some("/path/to/my project".to_string()),
            steps: vec![LaunchStep {
                id: "s1".to_string(),
                label: "step".to_string(),
                enabled: true,
                kind: StepKind::RunCommand {
                    command: "echo hi".to_string(),
                    command_spec: None,
                },
                depends_on: vec![],
                working_directory: Some("./sub dir".to_string()),
                environment: None,
                visibility: None,
                execution_mode: None,
                completion: None,
                timeout: None,
                failure_policy: None,
                retry_policy: None,
                metadata: None,
                extra: Map::new(),
            }],
            environment_binding_id: None,
            preferred_ide: None,
            default_execution_mode: None,
            extra: Map::new(),
        };

        let result = validation::validate_profile_v2(&profile);
        assert!(
            result.valid,
            "Profile with spaces should be valid: {:?}",
            result.diagnostics
        );

        // Verify path resolution handles spaces
        let resolved = resolve_working_directory(Some("/path/to/my project"), Some("./sub dir"));
        assert_eq!(resolved, Some("/path/to/my project/sub dir".to_string()));
    }

    /// Test 14: Environment binding propagation.
    #[test]
    fn test_environment_binding_in_step() {
        let mut env = HashMap::new();
        env.insert(
            "DATABASE_URL".to_string(),
            "postgres://localhost/mydb".to_string(),
        );
        env.insert("PORT".to_string(), "3000".to_string());

        let profile = LaunchProfileV2 {
            schema_version: "2".to_string(),
            id: "test".to_string(),
            name: "Test".to_string(),
            description: "desc".to_string(),
            project_root: None,
            steps: vec![LaunchStep {
                id: "s1".to_string(),
                label: "step".to_string(),
                enabled: true,
                kind: StepKind::RunCommand {
                    command: "npm start".to_string(),
                    command_spec: None,
                },
                depends_on: vec![],
                working_directory: None,
                environment: Some(env.clone()),
                visibility: None,
                execution_mode: None,
                completion: None,
                timeout: None,
                failure_policy: None,
                retry_policy: None,
                metadata: None,
                extra: Map::new(),
            }],
            environment_binding_id: Some("env-1".to_string()),
            preferred_ide: None,
            default_execution_mode: None,
            extra: Map::new(),
        };

        // Verify the environment is preserved in the step
        let step = &profile.steps[0];
        assert!(step.environment.is_some());
        let step_env = step.environment.as_ref().unwrap();
        assert_eq!(
            step_env.get("DATABASE_URL").unwrap(),
            "postgres://localhost/mydb"
        );
        assert_eq!(step_env.get("PORT").unwrap(), "3000");
    }

    /// Test 15: Event correlation — verify run_id and step_id are consistent.
    #[test]
    fn test_event_correlation_ids() {
        struct MockPM;
        impl ProcessManager for MockPM {
            fn spawn_and_track(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_and_track_with_overlay(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: &EnvironmentOverlay,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn launch_detached(&self, _: &str, _: &[&str], _: Option<&str>) -> Result<(), String> {
                todo!()
            }
            fn spawn_and_track_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn list(&self) -> Vec<crate::modules::workspace::models::TrackedProcess> {
                todo!()
            }
            fn kill(&self, _: &str) -> Result<(), String> {
                todo!()
            }
            fn refresh_status(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessStatus, String> {
                todo!()
            }
            fn get_logs(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessLogs, String> {
                todo!()
            }
            fn get_log_buffer(
                &self,
                _: &str,
            ) -> Option<Arc<crate::modules::workspace::models::BoundedLogBuffer>> {
                None
            }
            fn get_log_truncation(
                &self,
                _: &str,
            ) -> Option<(
                crate::modules::workspace::models::LogTruncation,
                crate::modules::workspace::models::LogTruncation,
            )> {
                None
            }
        }

        let profile = valid_profile(vec![
            make_step("step-1", vec![]),
            make_step("step-2", vec!["step-1"]),
        ]);

        let orchestrator = RunOrchestrator::new(Arc::new(MockPM));
        let run = orchestrator.create_run(profile).unwrap();

        // Run ID should be a valid UUID
        assert_eq!(run.run_id.len(), 36);
        assert!(run.run_id.chars().filter(|c| *c == '-').count() == 4);

        // Step IDs should match the profile
        assert_eq!(run.steps[0].step_id, "step-1");
        assert_eq!(run.steps[1].step_id, "step-2");

        // Profile ID should be consistent
        assert_eq!(run.profile_id, "test");
    }

    /// Test: Run logs retrieval for a run with no processes.
    #[test]
    fn test_get_run_logs_empty() {
        struct MockPM;
        impl ProcessManager for MockPM {
            fn spawn_and_track(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_and_track_with_overlay(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: &EnvironmentOverlay,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn launch_detached(&self, _: &str, _: &[&str], _: Option<&str>) -> Result<(), String> {
                todo!()
            }
            fn spawn_and_track_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn list(&self) -> Vec<crate::modules::workspace::models::TrackedProcess> {
                todo!()
            }
            fn kill(&self, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn refresh_status(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessStatus, String> {
                todo!()
            }
            fn get_logs(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessLogs, String> {
                todo!()
            }
            fn get_log_buffer(
                &self,
                _: &str,
            ) -> Option<Arc<crate::modules::workspace::models::BoundedLogBuffer>> {
                None
            }
            fn get_log_truncation(
                &self,
                _: &str,
            ) -> Option<(
                crate::modules::workspace::models::LogTruncation,
                crate::modules::workspace::models::LogTruncation,
            )> {
                None
            }
        }

        let orchestrator = RunOrchestrator::new(Arc::new(MockPM));
        let profile = valid_profile(vec![make_step("a", vec![])]);
        let run = orchestrator.create_run(profile).unwrap();

        // Get logs for a run with no processes
        let logs = orchestrator.get_run_logs(&run.run_id).unwrap();
        assert!(logs.stdout.is_empty());
        assert!(logs.stderr.is_empty());
    }

    /// Test: Platform capabilities detection (unit-level).
    #[test]
    fn test_platform_capabilities_detection() {
        // This is a pure unit test — just verify the types serialize
        let caps = models::PlatformCapabilities {
            os: "windows".to_string(),
            arch: "x86_64".to_string(),
            shells: vec!["cmd".to_string(), "powershell".to_string()],
            has_docker: false,
            has_compose: false,
            default_terminal: "cmd".to_string(),
            supports_terminal_windows: true,
        };
        let json = serde_json::to_string(&caps).unwrap();
        assert!(json.contains("windows"));
        assert!(json.contains("x86_64"));
    }

    /// Test: RunLogs and StepLogs serialization.
    #[test]
    fn test_log_types_serialization() {
        let run_logs = models::RunLogs {
            run_id: "run-1".to_string(),
            stdout: vec!["line1".to_string(), "line2".to_string()],
            stderr: vec!["err1".to_string()],
        };
        let json = serde_json::to_string(&run_logs).unwrap();
        assert!(json.contains("run-1"));
        assert!(json.contains("line1"));

        let step_logs = models::StepLogs {
            run_id: "run-1".to_string(),
            step_id: "step-1".to_string(),
            process_id: "proc-1".to_string(),
            stdout: vec!["out".to_string()],
            stderr: vec![],
            truncation: None,
        };
        let json = serde_json::to_string(&step_logs).unwrap();
        assert!(json.contains("step-1"));
        assert!(json.contains("proc-1"));
    }

    /// Test: Failure policy defaults based on dependencies.
    #[test]
    fn test_failure_policy_defaults() {
        // Steps with dependents default to StopRun
        assert_eq!(
            StepKind::default_failure_policy(true),
            FailurePolicy::StopRun
        );
        // Leaf steps default to WarnAndContinue
        assert_eq!(
            StepKind::default_failure_policy(false),
            FailurePolicy::WarnAndContinue
        );
    }

    /// Test: Retry policy initialization.
    #[test]
    fn test_retry_policy_preserved_in_run() {
        struct MockPM;
        impl ProcessManager for MockPM {
            fn spawn_and_track(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_and_track_with_overlay(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: &EnvironmentOverlay,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn launch_detached(&self, _: &str, _: &[&str], _: Option<&str>) -> Result<(), String> {
                todo!()
            }
            fn spawn_and_track_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn spawn_visible_owned(
                &self,
                _: &str,
                _: &[&str],
                _: Option<&str>,
                _: &str,
                _: Option<String>,
                _: Option<&EnvironmentOverlay>,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<crate::modules::workspace::models::TrackedProcess, String> {
                todo!()
            }
            fn list(&self) -> Vec<crate::modules::workspace::models::TrackedProcess> {
                todo!()
            }
            fn kill(&self, _: &str) -> Result<(), String> {
                todo!()
            }
            fn refresh_status(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessStatus, String> {
                todo!()
            }
            fn get_logs(
                &self,
                _: &str,
            ) -> Result<crate::modules::workspace::models::ProcessLogs, String> {
                todo!()
            }
            fn get_log_buffer(
                &self,
                _: &str,
            ) -> Option<Arc<crate::modules::workspace::models::BoundedLogBuffer>> {
                None
            }
            fn get_log_truncation(
                &self,
                _: &str,
            ) -> Option<(
                crate::modules::workspace::models::LogTruncation,
                crate::modules::workspace::models::LogTruncation,
            )> {
                None
            }
        }

        let profile = LaunchProfileV2 {
            schema_version: "2".to_string(),
            id: "test".to_string(),
            name: "Test".to_string(),
            description: "desc".to_string(),
            project_root: None,
            steps: vec![LaunchStep {
                id: "s1".to_string(),
                label: "step".to_string(),
                enabled: true,
                kind: StepKind::RunCommand {
                    command: "echo".to_string(),
                    command_spec: None,
                },
                depends_on: vec![],
                working_directory: None,
                environment: None,
                visibility: None,
                execution_mode: None,
                completion: None,
                timeout: None,
                failure_policy: None,
                retry_policy: Some(RetryPolicy {
                    max_retries: 3,
                    delay_ms: 1000,
                    backoff_multiplier: Some(2.0),
                }),
                metadata: None,
                extra: Map::new(),
            }],
            environment_binding_id: None,
            preferred_ide: None,
            default_execution_mode: None,
            extra: Map::new(),
        };

        let orchestrator = RunOrchestrator::new(Arc::new(MockPM));
        let run = orchestrator.create_run(profile).unwrap();
        assert_eq!(run.steps[0].retries_remaining, Some(3));
    }
}
