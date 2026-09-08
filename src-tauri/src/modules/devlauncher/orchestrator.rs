use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::path::Path;
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
use crate::platform::environment::EnvironmentOverlay;

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
    /// Environment overlay (from the profile's environment binding) applied
    /// to every process-spawning step of this run.
    overlay: Option<EnvironmentOverlay>,
    /// User-configured browser executable (from settings) used to open URL
    /// steps; None = OS default browser.
    browser_path: Option<String>,
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

    #[allow(dead_code)]
    pub fn create_run(
        &self,
        profile: LaunchProfileV2,
    ) -> Result<LaunchRun, ProfileValidationResult> {
        self.create_run_with_session(profile, None)
    }

    #[allow(dead_code)]
    pub fn create_run_with_session(
        &self,
        profile: LaunchProfileV2,
        session_id: Option<String>,
    ) -> Result<LaunchRun, ProfileValidationResult> {
        self.create_run_with_session_and_overlay(profile, session_id, None)
    }

    pub fn create_run_with_session_and_overlay(
        &self,
        profile: LaunchProfileV2,
        session_id: Option<String>,
        overlay: Option<EnvironmentOverlay>,
    ) -> Result<LaunchRun, ProfileValidationResult> {
        self.create_run_inner(profile, session_id, overlay, None)
    }

    /// Create a run with a user-configured browser for URL steps.
    pub fn create_run_with_browser(
        &self,
        profile: LaunchProfileV2,
        browser_path: Option<String>,
    ) -> Result<LaunchRun, ProfileValidationResult> {
        self.create_run_inner(profile, None, None, browser_path)
    }

    /// Create a run with a user-configured browser for URL steps, a workspace
    /// session and an environment overlay.
    pub fn create_run_with_browser_and_overlay(
        &self,
        profile: LaunchProfileV2,
        session_id: Option<String>,
        overlay: Option<EnvironmentOverlay>,
        browser_path: Option<String>,
    ) -> Result<LaunchRun, ProfileValidationResult> {
        self.create_run_inner(profile, session_id, overlay, browser_path)
    }

    fn create_run_inner(
        &self,
        profile: LaunchProfileV2,
        session_id: Option<String>,
        overlay: Option<EnvironmentOverlay>,
        browser_path: Option<String>,
    ) -> Result<LaunchRun, ProfileValidationResult> {
        let mut validation = validation::validate_profile_v2(&profile);
        if !validation.valid {
            return Err(validation);
        }

        // Run-time normalization (never persisted): old profiles saved with
        // tight wait timeouts (e.g. 30s from pre-refactor versions) routinely
        // fail on cold starts. Readiness waits get a sensible floor and a
        // default retry policy so a stale profile still launches reliably.
        let profile = normalize_profile_for_run(profile, &mut validation.diagnostics);

        // Docker compose bootstrap preflight: a compose step whose config
        // file does not exist can never succeed — docker dies immediately
        // with the cryptic "no configuration file provided: not found" and
        // the StopRun policy aborts the whole run. Skip such steps up front
        // (with a clear diagnostic) so the rest of the run proceeds.
        let missing_compose: HashSet<String> = profile
            .steps
            .iter()
            .filter(|s| s.enabled && is_compose_bootstrap_step(s))
            .filter(|s| {
                let working_dir = match s.working_directory.as_deref() {
                    Some(dir) => {
                        resolve_working_directory(profile.project_root.as_deref(), Some(dir))
                    }
                    None => profile.project_root.clone(),
                };
                working_dir
                    .as_deref()
                    .and_then(|dir| {
                        crate::platform::docker_service::DockerService::find_compose_file(
                            Path::new(dir),
                        )
                    })
                    .is_none()
            })
            .map(|s| s.id.clone())
            .collect();
        for step in &profile.steps {
            if !missing_compose.contains(&step.id) {
                continue;
            }
            validation
                .diagnostics
                .push(validation::ProfileValidationDiagnostic {
                    severity: DiagnosticSeverity::Warning,
                    code: "COMPOSE_FILE_MISSING".to_string(),
                    message: format!(
                        "Step '{}' skipped: no docker compose configuration file found in its \
                     working directory (expected compose.yaml, compose.yml, \
                     docker-compose.yaml or docker-compose.yml)",
                        step.label
                    ),
                    step_id: Some(step.id.clone()),
                    field: Some("working_directory".to_string()),
                });
        }

        let run_id = generate_stable_id();
        let now = default_now_iso();

        let steps: Vec<StepExecutionState> = profile
            .steps
            .iter()
            .map(|s| {
                let status = if !s.enabled || missing_compose.contains(&s.id) {
                    StepStatus::Skipped
                } else {
                    StepStatus::Pending
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
            diagnostics: validation
                .diagnostics
                .into_iter()
                .map(|d| Diagnostic {
                    run_id: run_id.clone(),
                    step_id: d.step_id,
                    source: LogSource::Preflight,
                    severity: d.severity,
                    message: d.message,
                    timestamp: default_now_iso(),
                })
                .collect(),
        };

        let handle = Arc::new(RunHandle {
            run: Arc::new(Mutex::new(run.clone())),
            cancelled: Arc::new(AtomicBool::new(false)),
            cancel_notify: Arc::new(Notify::new()),
            profile,
            session_id,
            process_ids: Arc::new(Mutex::new(Vec::new())),
            overlay,
            browser_path,
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
        let browser_path = handle.browser_path.clone();

        tokio::spawn(async move {
            Self::scheduler_loop(
                run_id_owned,
                orchestrator_runs,
                process_manager,
                app_handle,
                concurrency_limit,
                session_id,
                browser_path,
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
        browser_path: Option<String>,
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
        // Steps whose retry backoff is pending: (step_id, ready_at).
        let mut retry_schedule: Vec<(String, tokio::time::Instant)> = Vec::new();
        // Steps currently inside `retry_schedule` — excluded from the
        // ready set so the scheduler never double-spawns a retried step.
        let mut scheduled_retries: HashSet<String> = HashSet::new();

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
            // 1. Promote due retries into the running set.
            let now = tokio::time::Instant::now();
            let mut due: Vec<String> = Vec::new();
            retry_schedule.retain(|(id, at)| {
                if *at <= now {
                    due.push(id.clone());
                    false
                } else {
                    true
                }
            });
            for step_id in &due {
                scheduled_retries.remove(step_id);
            }

            // 2. Collect everything to spawn this iteration: due retries
            //    first, then newly-ready steps.
            let ready = Self::find_ready_steps(&handle.profile, &completed, &running);
            let mut to_spawn: Vec<String> = due;
            for step_id in ready {
                if scheduled_retries.contains(&step_id) {
                    continue;
                }
                to_spawn.push(step_id);
            }

            for step_id in &to_spawn {
                // A user-initiated cancellation must stop the launch from
                // starting additional processes that were still waiting to be
                // launched. Steps already in flight are allowed to report back
                // (their completion drives the Cancelled finalization), but no
                // NEW step is spawned once the cancel flag is set.
                if handle.cancelled.load(Ordering::SeqCst) {
                    break;
                }
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
                    let session_id = session_id.clone();
                    let overlay = handle.overlay.clone();
                    let browser_path = browser_path.clone();
                    let handle = handle.clone();

                    tokio::spawn(async move {
                        let _permit = semaphore.acquire().await.unwrap();

                        // Run the step in a child task and await its JoinHandle.
                        // If `step_task` panics (an unwrap/expect in the process
                        // or Docker path), the JoinHandle returns a `JoinError`
                        // instead of a completion. Converting that into a Failed
                        // completion guarantees the scheduler can never hang
                        // waiting for a step that will never report back.
                        let run_id_inner = run_id.clone();
                        let step_inner = step.clone();
                        let profile_inner = profile.clone();
                        let pm_inner = process_manager.clone();
                        let app_inner = app_handle.clone();
                        let cancelled_inner = cancelled.clone();
                        let notify_inner = cancel_notify.clone();
                        let session_inner = session_id.clone();
                        let overlay_inner = overlay.clone();
                        let browser_inner = browser_path.clone();
                        let handle_inner = handle.clone();
                        let step_id_owned = step.id.clone();
                        let inner = tokio::spawn(async move {
                            Self::step_task(
                                &run_id_inner,
                                &step_inner,
                                &profile_inner,
                                &pm_inner,
                                &app_inner,
                                &cancelled_inner,
                                &notify_inner,
                                &session_inner,
                                &overlay_inner,
                                &browser_inner,
                                &handle_inner,
                            )
                            .await
                        });

                        let completion = match inner.await {
                            Ok(c) => c,
                            Err(join_err) => {
                                let msg = if join_err.is_panic() {
                                    let payload = join_err.into_panic();
                                    if let Some(s) = payload.downcast_ref::<&str>() {
                                        format!("Step panicked: {}", s)
                                    } else if let Some(s) = payload.downcast_ref::<String>() {
                                        format!("Step panicked: {}", s)
                                    } else {
                                        "Step panicked (no message)".to_string()
                                    }
                                } else {
                                    format!("Step task failed: {}", join_err)
                                };
                                StepCompletion {
                                    step_id: step_id_owned,
                                    success: false,
                                    error: Some(msg),
                                    process_id: None,
                                    attempt_number: 0,
                                }
                            }
                        };
                        let _ = tx.send(completion);
                        drop(_permit);
                    });
                }
            }

            // 3. Termination: nothing running, nothing scheduled.
            if running.is_empty() && retry_schedule.is_empty() {
                break;
            }

            // 4. Wait for the next event.
            if !running.is_empty() {
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
                    } else if handle.cancelled.load(Ordering::SeqCst) {
                        // User-initiated cancellation: finalize immediately.
                        Self::finalize_run(
                            &handle,
                            &runs,
                            &run_id,
                            RunStatus::Cancelled,
                            &app_handle,
                            &process_manager,
                        )
                        .await;
                        return;
                    } else {
                        // A retry takes precedence over the failure policy:
                        // dependents wait for the retried step to settle.
                        if let Some(delay) = Self::consume_retry(&handle, &completion.step_id) {
                            let ready_at = tokio::time::Instant::now() + delay;
                            retry_schedule.push((completion.step_id.clone(), ready_at));
                            scheduled_retries.insert(completion.step_id.clone());
                            Self::emit_step_status(
                                &run_id,
                                &completion.step_id,
                                StepStatus::Retrying,
                                &app_handle,
                            );
                            continue;
                        }

                        // Retries exhausted (or none configured): apply the
                        // step's failure policy.
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
                                    &process_manager,
                                )
                                .await;
                                return;
                            }
                            FailureAction::SkipDependents => {
                                // The failed step itself is now terminal:
                                // mark it completed so the scheduler never
                                // respawns it, then skip its dependents.
                                completed.insert(completion.step_id.clone());
                                let to_skip = Self::transitive_dependents(
                                    &handle.profile,
                                    &completion.step_id,
                                );
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

                    // Check cancellation
                    if handle.cancelled.load(Ordering::SeqCst) {
                        Self::finalize_run(
                            &handle,
                            &runs,
                            &run_id,
                            RunStatus::Cancelled,
                            &app_handle,
                            &process_manager,
                        )
                        .await;
                        return;
                    }
                }
            } else {
                // Only scheduled retries remain: sleep until the earliest
                // one becomes due (or until cancellation).
                let next = retry_schedule
                    .iter()
                    .map(|(_, at)| *at)
                    .min()
                    .expect("retry_schedule non-empty in this branch");
                let sleep = next.saturating_duration_since(tokio::time::Instant::now());
                let _ = tokio::time::timeout(sleep, handle.cancel_notify.notified()).await;
                if handle.cancelled.load(Ordering::SeqCst) {
                    Self::finalize_run(
                        &handle,
                        &runs,
                        &run_id,
                        RunStatus::Cancelled,
                        &app_handle,
                        &process_manager,
                    )
                    .await;
                    return;
                }
            }
        }

        // All steps done — finalize
        let final_status = Self::compute_final_status(&handle);
        Self::finalize_run(
            &handle,
            &runs,
            &run_id,
            final_status,
            &app_handle,
            &process_manager,
        )
        .await;
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
        overlay: &Option<EnvironmentOverlay>,
        browser_path: &Option<String>,
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

        // A relative working directory needs the profile's project root to
        // resolve against. Without it the spawn silently runs in the APP's
        // own working directory, so every relative command (`.venv\Scripts\
        // python.exe`, npm scripts, ...) dies with the cryptic "The system
        // cannot find the path specified" (cmd exit code 3) — or, worse,
        // starts in the wrong place and the following port waits time out.
        // Surface the real cause up front.
        let root_missing = profile
            .project_root
            .as_deref()
            .map(str::trim)
            .map_or(true, |r| r.is_empty());
        if root_missing
            && step
                .working_directory
                .as_deref()
                .is_some_and(|d| !Path::new(d).is_absolute())
        {
            return StepCompletion {
                step_id: step.id.clone(),
                success: false,
                error: Some(format!(
                    "Step '{}' has a relative working directory '{}' but the profile has no \
                     project path to resolve it against. Re-analyze the project or fix the \
                     profile's project path, otherwise commands run in the wrong folder.",
                    step.label,
                    step.working_directory.as_deref().unwrap_or("")
                )),
                process_id: None,
                attempt_number: 0,
            };
        }

        match &step.kind {
            StepKind::RunCommand {
                command,
                command_spec,
            } => {
                // Prefer the structured command spec when provided; otherwise
                // resolve the command string into program + args (falling back
                // to the platform shell for shell-syntax command lines).
                // Visible-terminal steps resolve WITHOUT the batch-shim
                // `cmd /C` wrapper — the terminal's own shell runs batch
                // files natively (nested cmd breaks the command line).
                //
                // Docker preflight runs on the RAW command string (never the
                // resolved program path): the resolver turns `docker` into
                // `C:\Program Files\...\docker.exe`, which no longer starts
                // with "docker", so a program-based check would silently skip
                // the daemon preflight for every Docker step.
                if let Err(msg) = preflight_check(command) {
                    Self::emit_diagnostic(
                        run_id,
                        Some(&step.id),
                        LogSource::Preflight,
                        DiagnosticSeverity::Warning,
                        msg.clone(),
                        app_handle,
                    );
                    return StepCompletion {
                        step_id: step.id.clone(),
                        success: false,
                        error: Some(msg),
                        process_id: None,
                        attempt_number: 0,
                    };
                }
                let (program, args) = if let Some(spec) = command_spec {
                    (spec.program.clone(), spec.args.clone())
                } else if matches!(visibility, Visibility::VisibleTerminal) {
                    Self::resolve_terminal_command(command)
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
                    overlay,
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
                    overlay,
                    session_id.as_deref(),
                    handle,
                )
                .await
            }
            StepKind::OpenApplication { path, args } => {
                // Application resolution is structured: Store/MSIX launchers
                // (and other wrappers) may need their own arguments.  The old
                // path-only lookup discarded those arguments, so Docker
                // Desktop could appear in the plan but fail to launch.
                let mut resolved_args: Vec<String> = args.clone().unwrap_or_default();
                let resolved = if path.contains('/') || path.contains('\\') {
                    path.clone()
                } else if let Some(p) = crate::platform::ide::resolve_ide_executable(path) {
                    p
                } else {
                    let launcher =
                        crate::platform::app_launcher::resolve_application(path, None, None);
                    if !launcher.found {
                        return StepCompletion {
                            step_id: step.id.clone(),
                            success: false,
                            error: Some(format!("Application '{}' not found", path)),
                            process_id: None,
                            attempt_number: 0,
                        };
                    }
                    // Launcher arguments must precede user-supplied args
                    // (e.g. `explorer shell:AppsFolder\\...`).
                    if !launcher.args.is_empty() {
                        let mut launcher_args = launcher.args;
                        launcher_args.append(&mut resolved_args);
                        resolved_args = launcher_args;
                    }
                    launcher.program
                };
                let args_refs: Vec<&str> = resolved_args.iter().map(|s| s.as_str()).collect();

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
            StepKind::OpenUrl { url } => {
                // Doc URLs (/docs, /swagger-ui/...) often 404 when the docs
                // package is not installed (NestJS without @nestjs/swagger,
                // Spring Boot without springdoc). Resolve what to open BEFORE
                // launching the browser: the path is probed and the origin
                // root opens instead when the path answers with an error.
                let requested = url.clone();
                let resolved = tokio::task::spawn_blocking({
                    let requested = requested.clone();
                    move || crate::platform::readiness::resolve_browser_url(&requested)
                })
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or_else(|| requested.clone());
                if resolved != requested {
                    Self::emit_diagnostic(
                        run_id,
                        Some(step.id.as_str()),
                        LogSource::Readiness,
                        DiagnosticSeverity::Info,
                        format!(
                            "'{}' answered with an error; opening '{}' instead",
                            requested, resolved
                        ),
                        app_handle,
                    );
                }
                match crate::platform::app_launcher::open_url_in_browser(
                    &resolved,
                    browser_path.as_deref(),
                ) {
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
                        error: Some(format!("Failed to open URL '{}': {}", resolved, e)),
                        process_id: None,
                        attempt_number: 0,
                    },
                }
            }
            StepKind::WaitForPort {
                host,
                port,
                candidate_ports,
            } => {
                // Generous default: dev servers (Metro, Go, Django) need
                // headroom for cold starts.
                let timeout = step.timeout.unwrap_or(120);
                Self::wait_for_port(
                    run_id,
                    &step.id,
                    host,
                    *port,
                    candidate_ports,
                    timeout,
                    cancelled,
                    cancel_notify,
                    app_handle,
                )
                .await
            }
            StepKind::WaitForUrl { url } => {
                // Generous default: HTTP services take longer to respond.
                let timeout = step.timeout.unwrap_or(120);
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
                // Generous default: cold Docker Desktop boot (WSL2 backend
                // included) routinely exceeds 2 minutes.
                let timeout = step.timeout.unwrap_or(180);
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
                let (program, args) = Self::resolve_terminal_command(&effective);
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
                    overlay,
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
        step_env: &Option<HashMap<String, String>>,
        run_overlay: &Option<EnvironmentOverlay>,
        session_id: Option<&str>,
        handle: &Arc<RunHandle>,
    ) -> StepCompletion {
        // The working directory must exist BEFORE the spawn attempt. A stale
        // profile pointing at a removed/renamed directory would otherwise
        // surface as a confusing "Process exited with error code 1" (the
        // spawned shell cannot chdir, so a batch shim like npm.cmd starts in
        // the wrong place and dies instantly). Fail loudly with the real path
        // so the user can fix the profile instead of the whole run dying.
        if let Some(dir) = working_dir {
            if !std::path::Path::new(dir).is_dir() {
                return StepCompletion {
                    step_id: step_id.to_string(),
                    success: false,
                    error: Some(format!("Working directory '{}' does not exist", dir)),
                    process_id: None,
                    attempt_number: 0,
                };
            }
        }

        // Resolve a RELATIVE program path (e.g. `.venv\Scripts\python.exe`)
        // against the step's working directory. The command resolver checks
        // relative paths against the APP's own cwd, so without this the spawn
        // would run in the project root while cmd fails with the cryptic
        // "The system cannot find the path specified" (exit code 3) — exactly
        // what the startup probe reports for a missing venv interpreter.
        // When the executable is genuinely absent (install step never ran or
        // was disabled), fail with the real reason instead.
        let (program, args) = match resolve_relative_program(program, args, working_dir) {
            Ok(pinned) => pinned,
            Err(msg) => {
                return StepCompletion {
                    step_id: step_id.to_string(),
                    success: false,
                    error: Some(msg),
                    process_id: None,
                    attempt_number: 0,
                };
            }
        };

        // docker compose resolves its configuration file from the process
        // working directory. Pin the file with `-f` so the step never depends
        // on the cwd, and fail with an actionable message instead of docker's
        // cryptic "no configuration file provided: not found" when the file
        // disappeared between profile creation and execution.
        let (program, args) = match pin_docker_compose_config(&program, &args, working_dir) {
            Ok(pinned) => pinned,
            Err(msg) => {
                return StepCompletion {
                    step_id: step_id.to_string(),
                    success: false,
                    error: Some(msg),
                    process_id: None,
                    attempt_number: 0,
                };
            }
        };

        // Effective environment: run-level overlay (environment binding)
        // merged with the step's own `environment` map.
        let effective_overlay = build_effective_overlay(run_overlay, step_env);
        let overlay_arg = if effective_overlay.is_empty() {
            None
        } else {
            Some(&effective_overlay)
        };

        // Re-check cancellation right before the spawn: a step task may have
        // passed the initial `cancelled` check and then lost the race against
        // `cancel_run`. No process is spawned for a canceled run.
        if cancelled.load(Ordering::SeqCst) {
            return StepCompletion {
                step_id: step_id.to_string(),
                success: false,
                error: Some("Cancelled".to_string()),
                process_id: None,
                attempt_number: 0,
            };
        }

        // For visible-terminal service steps we must not blindly trust that
        // "a terminal window opened" means "the command started". The inner
        // command writes its exit code to a startup-probe marker after it
        // exits; the ProcessStarted probe then verifies the marker (see
        // `probe_startup_marker`). Plain interactive terminals (`cmd` with no
        // args) get no marker — there is no command to verify.
        let startup_marker: Option<std::path::PathBuf> =
            if matches!(visibility, Visibility::VisibleTerminal)
                && matches!(completion, CompletionPolicy::ProcessStarted)
                && !args.is_empty()
            {
                Some(create_startup_marker_path())
            } else {
                None
            };

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
                let overlay_owned = overlay_arg.cloned();
                let marker_owned = startup_marker.clone();
                match tokio::task::spawn_blocking(move || {
                    pm.spawn_visible_owned_with_startup_marker(
                        &program_owned,
                        &args_owned.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                        dir.as_deref(),
                        &label_owned,
                        session_id_owned,
                        overlay_owned.as_ref(),
                        Some(run_id_owned),
                        Some(step_id_owned),
                        marker_owned.as_deref().and_then(|p| p.to_str()),
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
                let overlay_owned = overlay_arg.cloned();
                match tokio::task::spawn_blocking(move || {
                    if let Some(ov) = overlay_owned.as_ref() {
                        pm.spawn_and_track_owned_with_overlay(
                            &program_owned,
                            &args_owned.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                            dir.as_deref(),
                            &label_owned,
                            session_id_owned,
                            ov,
                            Some(run_id_owned),
                            Some(step_id_owned),
                        )
                    } else {
                        pm.spawn_and_track_owned(
                            &program_owned,
                            &args_owned.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                            dir.as_deref(),
                            &label_owned,
                            session_id_owned,
                            Some(run_id_owned),
                            Some(step_id_owned),
                        )
                    }
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
            CompletionPolicy::ProcessStarted => {
                // Probe: the spawned program must still be alive shortly
                // after start. This catches instant-exit spawns (wrong
                // working directory, missing executable, broken shell
                // quoting) that would otherwise report "started" while
                // nothing is actually running. Terminal wrappers are
                // probed leniently (the launcher detaches by design).
                //
                // When a startup marker was requested, the probe is
                // authoritative: the inner command writes its real exit code
                // to the marker, so a command that fails instantly (e.g. a
                // broken `.venv` activation) fails the step instead of
                // reporting "success". The probe result is captured so the
                // marker file is always cleaned up afterwards.
                let marker = startup_marker.clone();
                let probe_result = match &marker {
                    Some(path) => {
                        probe_startup_marker(path, &proc_id, process_manager, cancelled).await
                    }
                    None => {
                        probe_process_alive(
                            &proc_id,
                            process_manager,
                            cancelled,
                            tracked.tracking_quality.as_ref(),
                        )
                        .await
                    }
                };
                if let Some(path) = &marker {
                    let _ = std::fs::remove_file(path);
                }
                match probe_result {
                    Ok(true) => StepCompletion {
                        step_id: step_id.to_string(),
                        success: true,
                        error: None,
                        process_id: Some(proc_id),
                        attempt_number: 0,
                    },
                    Ok(false) => {
                        let stderr = process_manager
                            .get_logs(&proc_id)
                            .ok()
                            .map(|l| l.stderr_lines.join("\n"))
                            .unwrap_or_default();
                        let hint = if stderr.is_empty() {
                            String::new()
                        } else {
                            format!("; output: {}", truncate_lines(&stderr, 5))
                        };
                        StepCompletion {
                            step_id: step_id.to_string(),
                            success: false,
                            error: Some(format!(
                                "Process '{}' exited immediately after start{}",
                                proc_id, hint
                            )),
                            process_id: Some(proc_id),
                            attempt_number: 0,
                        }
                    }
                    Err(msg) => StepCompletion {
                        step_id: step_id.to_string(),
                        success: false,
                        error: Some(msg),
                        process_id: Some(proc_id),
                        attempt_number: 0,
                    },
                }
            }
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
                    tracked.tracking_quality.as_ref(),
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
                    &[],
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
                    tracked.tracking_quality.as_ref(),
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
        step_env: &Option<HashMap<String, String>>,
        run_overlay: &Option<EnvironmentOverlay>,
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
        // `cmd /C` parses its argument from the raw command line: the
        // script must be appended RAW, otherwise backslash-escaped quotes
        // break scripts containing quoted paths. Other shells accept a
        // regular argument.
        let (spawn_args, raw_tail): (Vec<String>, Option<String>) =
            if shell_flag.eq_ignore_ascii_case("/C") {
                (vec![shell_flag_owned.clone()], Some(script_owned.clone()))
            } else {
                (vec![shell_flag_owned.clone(), script_owned.clone()], None)
            };
        let effective_overlay = build_effective_overlay(run_overlay, step_env);
        let overlay_owned = if effective_overlay.is_empty() {
            None
        } else {
            Some(effective_overlay)
        };

        let tracked = match tokio::task::spawn_blocking(move || {
            let spawn_arg_refs: Vec<&str> = spawn_args.iter().map(|s| s.as_str()).collect();
            if let Some(ov) = overlay_owned.as_ref() {
                pm.spawn_and_track_owned_with_overlay(
                    &shell_exe_owned,
                    &spawn_arg_refs,
                    dir.as_deref(),
                    &label_owned,
                    session_id_owned,
                    ov,
                    Some(run_id_owned),
                    Some(step_id_owned),
                )
            } else {
                pm.spawn_and_track_owned_with_raw_tail(
                    &shell_exe_owned,
                    &spawn_arg_refs,
                    raw_tail.as_deref(),
                    dir.as_deref(),
                    &label_owned,
                    session_id_owned,
                    Some(run_id_owned),
                    Some(step_id_owned),
                )
            }
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
            tracked.tracking_quality.as_ref(),
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
        tracking_quality: Option<&ProcessTrackingQuality>,
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
                        error: Some(format!(
                            "Process exited with code {}{}",
                            code,
                            output_tail_hint(process_manager, proc_id)
                        )),
                        process_id: Some(proc_id.to_string()),
                        attempt_number: 0,
                    };
                }
                Some(ProcessStatus::ExitedWithError(code)) => {
                    return StepCompletion {
                        step_id: step_id.to_string(),
                        success: false,
                        error: Some(format!(
                            "Process exited with error code {}{}",
                            code,
                            output_tail_hint(process_manager, proc_id)
                        )),
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
                    let untrackable = status.clone().unwrap_or(ProcessStatus::Unknown);
                    return StepCompletion {
                        step_id: step_id.to_string(),
                        success: false,
                        error: Some(format!(
                            "Process '{}' has untrackable status: {:?}",
                            proc_id, untrackable
                        )),
                        process_id: Some(proc_id.to_string()),
                        attempt_number: 0,
                    };
                }
                None => {
                    // Process is no longer tracked. The process manager never
                    // prunes entries, so a freshly-spawned id becoming "not
                    // found" is anomalous and we cannot confirm the exit code.
                    // For terminal-wrapper steps a clean detach is the NORMAL
                    // case (the terminal owns the real process from here on).
                    // For directly-tracked one-shots, report an error instead
                    // of silently claiming success — that would mask a failed
                    // `docker compose up` as a successful step.
                    let is_wrapper = matches!(
                        tracking_quality,
                        Some(ProcessTrackingQuality::TerminalWrapper)
                    );
                    if is_wrapper {
                        return StepCompletion {
                            step_id: step_id.to_string(),
                            success: true,
                            error: None,
                            process_id: Some(proc_id.to_string()),
                            attempt_number: 0,
                        };
                    }
                    return StepCompletion {
                        step_id: step_id.to_string(),
                        success: false,
                        error: Some(format!(
                            "Process '{}' is no longer tracked; cannot confirm its exit status",
                            proc_id
                        )),
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
        candidate_ports: &[u16],
        timeout_secs: u64,
        cancelled: &Arc<AtomicBool>,
        cancel_notify: &Arc<Notify>,
        app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
    ) -> StepCompletion {
        // Resolve the host plus its loopback aliases: on dual-stack hosts
        // `localhost` may resolve to ::1 only while the server binds
        // 127.0.0.1 (and vice versa). Trying both makes the wait robust
        // across platform/stack configurations.
        //
        // Candidate ports: dev servers (Vite, Next.js, Expo) auto-increment
        // their port when the configured one is already taken. The wait
        // succeeds when ANY of `[port] + candidate_ports` opens, so a busy
        // port no longer fails the readiness wait.
        let mut ports: Vec<u16> = vec![port];
        for p in candidate_ports {
            if *p != port && !ports.contains(p) {
                ports.push(*p);
            }
        }
        let hosts = loopback_host_aliases(host);
        let mut addrs: Vec<std::net::SocketAddr> = Vec::new();
        for h in &hosts {
            for p in &ports {
                let addr_str = format!("{}:{}", h, p);
                match addr_str.to_socket_addrs() {
                    Ok(a) => addrs.extend(a),
                    Err(e) => {
                        return StepCompletion {
                            step_id: step_id.to_string(),
                            success: false,
                            error: Some(format!("DNS resolve failed: {}", e)),
                            process_id: None,
                            attempt_number: 0,
                        };
                    }
                }
            }
        }
        addrs.dedup();

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
                    "Timeout: none of ports [{}] on {} open after {}s",
                    ports
                        .iter()
                        .map(|p| p.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    host,
                    timeout_secs
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
                for host in loopback_host_aliases(&parsed.host) {
                    let addr_str = format!("{}:{}", host, parsed.port);
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
                                        let parts: Vec<&str> =
                                            first_line.split_whitespace().collect();
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
        // When the daemon is down, `ensure_daemon` starts Docker Desktop /
        // the Docker daemon before the wait loop. This is the core fix for
        // "Docker daemon not ready": the wait now *starts* Docker instead
        // of passively polling a dead daemon.
        let daemon_diag = crate::platform::docker_service::DockerService::check_daemon();
        if daemon_diag.status != crate::platform::docker_service::DockerStatus::DaemonReady {
            Self::emit_diagnostic(
                run_id,
                Some(step_id),
                LogSource::Preflight,
                DiagnosticSeverity::Info,
                "Docker daemon is not running — attempting to start \
                 Docker Desktop / the Docker daemon"
                    .to_string(),
                app_handle,
            );
        }

        let check = crate::platform::docker_service::DockerReadinessCheck {
            timeout: Duration::from_secs(timeout_secs),
            poll_interval: Duration::from_secs(1),
            auto_launch: true,
            ..Default::default()
        };
        let result =
            crate::platform::docker_service::DockerService::ensure_daemon(&check, cancelled).await;

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
            Self::emit_diagnostic(
                run_id,
                Some(step_id),
                LogSource::Preflight,
                DiagnosticSeverity::Info,
                format!("Docker daemon ready (waited {}s)", result.elapsed.as_secs()),
                app_handle,
            );
            return StepCompletion {
                step_id: step_id.to_string(),
                success: true,
                error: None,
                process_id: None,
                attempt_number: 0,
            };
        }

        let msg = format!(
            "Docker daemon not ready after {}s: {}{}",
            timeout_secs,
            result.message,
            result
                .suggested_action
                .as_ref()
                .map(|a| format!("\n{}", a))
                .unwrap_or_default()
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
        let step_state = run.steps.iter().find(|s| s.step_id == failed_step_id);
        let profile_step = handle.profile.steps.iter().find(|s| s.id == failed_step_id);
        let has_dependents = handle
            .profile
            .steps
            .iter()
            .any(|d| d.depends_on.contains(&failed_step_id.to_string()));
        let failure_policy = match (step_state, profile_step) {
            (Some(_), Some(ps)) => ps
                .failure_policy
                .clone()
                .unwrap_or_else(|| ps.kind.default_failure_policy(has_dependents)),
            _ => {
                if has_dependents {
                    FailurePolicy::StopRun
                } else {
                    FailurePolicy::WarnAndContinue
                }
            }
        };

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

    /// Consume one retry for a failed step. Returns the backoff delay when a
    /// retry was scheduled, or `None` when no retries remain.
    ///
    /// The step state is flipped to `Retrying`; the scheduler moves it back
    /// to `Running` when the delay elapses and the step is re-spawned.
    fn consume_retry(handle: &Arc<RunHandle>, step_id: &str) -> Option<tokio::time::Duration> {
        if handle.cancelled.load(Ordering::SeqCst) {
            return None;
        }

        let delay_ms: u64 = {
            let mut run = handle.run.lock().expect("run lock poisoned");
            let step_state = run.steps.iter_mut().find(|s| s.step_id == step_id)?;
            let remaining = match step_state.retries_remaining {
                Some(n) if n > 0 => {
                    step_state.retries_remaining = Some(n - 1);
                    step_state.status = StepStatus::Retrying;
                    n - 1
                }
                _ => return None,
            };
            let policy = handle
                .profile
                .steps
                .iter()
                .find(|s| s.id == step_id)
                .and_then(|s| s.retry_policy.clone());
            let policy = policy?;
            // Backoff: each consecutive retry multiplies the base delay by
            // the configured multiplier (1.5 -> 2s, 3s, 4.5s, ...).
            let consumed = policy.max_retries.saturating_sub(remaining + 1);
            let multiplier = policy.backoff_multiplier.unwrap_or(1.0).max(1.0);
            (policy.delay_ms as f64 * multiplier.powi(consumed as i32)) as u64
        };

        Some(tokio::time::Duration::from_millis(delay_ms))
    }

    // -----------------------------------------------------------------------
    // Finalize run
    // -----------------------------------------------------------------------

    /// Finalize a run: settle in-flight step states, terminate leftover
    /// processes on failure/cancellation, set the terminal status and
    /// prune the oldest finished runs.
    async fn finalize_run(
        handle: &RunHandle,
        runs: &Arc<RwLock<HashMap<String, Arc<RunHandle>>>>,
        run_id: &str,
        final_status: RunStatus,
        app_handle: &Arc<Mutex<Option<tauri::AppHandle>>>,
        process_manager: &Arc<dyn ProcessManager>,
    ) {
        // A Failed (abort) or Cancelled run must not leave the processes of
        // the failed/in-flight steps running — but it must also NOT tear down
        // services that already came up. Killing every process on a Failed
        // run is what destroyed an already-starting `docker compose up` when
        // an unrelated leaf step (e.g. `npm install`) failed: the containers
        // never got to start. So:
        //   - Cancelled  (user asked to stop) → kill everything.
        //   - Failed     → kill only processes NOT owned by a Succeeded step;
        //                 keep the infra/services that are already running.
        if matches!(final_status, RunStatus::Failed | RunStatus::Cancelled) {
            let keep_alive: HashSet<String> = {
                let run = handle.run.lock().expect("run lock poisoned");
                if final_status == RunStatus::Cancelled {
                    HashSet::new()
                } else {
                    run.steps
                        .iter()
                        .filter(|s| s.status == StepStatus::Succeeded)
                        .filter_map(|s| s.process_id.clone())
                        .collect()
                }
            };
            let ids: Vec<String> = handle
                .process_ids
                .lock()
                .expect("process_ids lock poisoned")
                .clone();
            for proc_id in &ids {
                if !keep_alive.contains(proc_id) {
                    let _ = process_manager.kill(proc_id);
                }
            }
        }

        {
            let mut run = handle.run.lock().expect("run lock poisoned");
            run.status = final_status.clone();
            run.finished_at = Some(default_now_iso());
            // Settle steps that are still in flight so the step list never
            // shows Running/Pending under a terminal run status.
            let now = default_now_iso();
            for step in &mut run.steps {
                match (&step.status, &final_status) {
                    (StepStatus::Pending, RunStatus::Failed) => {
                        step.status = StepStatus::Skipped;
                        step.finished_at = Some(now.clone());
                    }
                    (StepStatus::Running, RunStatus::Failed) => {
                        step.status = StepStatus::Failed;
                        step.error = step.error.clone().or_else(|| {
                            Some("Run aborted by a failing step; process terminated".to_string())
                        });
                        step.finished_at = Some(now.clone());
                    }
                    (StepStatus::Pending | StepStatus::Running, RunStatus::Cancelled) => {
                        step.status = StepStatus::Cancelled;
                        step.finished_at = Some(now.clone());
                    }
                    _ => {}
                }
            }
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

        // Stopping is an explicit user cancellation.  Set the shared flag so
        // the scheduler cannot launch another pending step while we tear down
        // already-running processes.
        handle.cancelled.store(true, Ordering::SeqCst);
        handle.cancel_notify.notify_waiters();

        let mut killed = Vec::new();
        for proc_id in &processes_to_kill {
            if self.process_manager.kill(proc_id).is_ok() {
                killed.push(proc_id.clone());
            }
        }

        // Docker Compose containers are daemon-owned and therefore are not
        // children of the terminal process tracked above.  Always tear down
        // the profile's compose project as part of the same Stop action.
        if let Some(root) = handle.profile.project_root.as_deref() {
            let docker = crate::platform::docker_service::DockerService::resolve_cli()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "docker".to_string());
            let _ = std::process::Command::new(&docker)
                .args(["compose", "down", "--remove-orphans"])
                .current_dir(root)
                .status();
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

        // Steps that never spawned a process (URL opens, port waits, delay,
        // failed spawns) have nothing to show — an empty log sheet, not an
        // error. The frontend hides the button anyway, but a stale click
        // (or a step whose process was already pruned) must never surface
        // a confusing "no associated process" failure.
        let Some(proc_id) = step_state.process_id.as_ref() else {
            return Ok(StepLogs {
                run_id: run_id.to_string(),
                step_id: step_id.to_string(),
                process_id: String::new(),
                stdout: Vec::new(),
                stderr: Vec::new(),
                truncation: None,
            });
        };

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
        let (program, args) = crate::platform::command_resolver::resolve_command_target(command);
        // The docker CLI is often missing from PATH in GUI-launched apps
        // (shell profiles are not inherited). Fall back to the Docker
        // Desktop bundled CLI so compose/docker steps keep working.
        if program == "docker" {
            if let Some(cli) = crate::platform::docker_service::DockerService::resolve_cli() {
                return (cli.to_string_lossy().into_owned(), args);
            }
        }
        (program, args)
    }

    /// Resolve a command for execution INSIDE a visible terminal window.
    ///
    /// Unlike [`Self::resolve_command_target`], batch shims are NOT wrapped
    /// in a nested `cmd /C`: the terminal's own shell executes batch files
    /// natively, and nesting cmd inside cmd breaks the command line (quote
    /// mangling) — the terminal would open empty with nothing running.
    fn resolve_terminal_command(command: &str) -> (String, Vec<String>) {
        let resolved =
            crate::platform::command_resolver::resolve_command_string(command, None, None);
        let program = resolved.command.program;
        let args = resolved.command.args;
        if program == "docker" {
            if let Some(cli) = crate::platform::docker_service::DockerService::resolve_cli() {
                return (cli.to_string_lossy().into_owned(), args);
            }
        }
        (program, args)
    }
}

// ---------------------------------------------------------------------------
// Run-time profile normalization
// ---------------------------------------------------------------------------

/// Minimum timeout for generated readiness waits. Dev servers (Metro, Go,
/// Django, Docker Desktop cold boots) routinely exceed 30s, and old profiles
/// saved before the timeout rework carry 30s waits.
const WAIT_TIMEOUT_FLOOR_SECS: u64 = 60;

/// Default retry policy attached to readiness waits that have none.
fn default_wait_retry_policy() -> RetryPolicy {
    RetryPolicy {
        max_retries: 2,
        delay_ms: 2000,
        backoff_multiplier: Some(1.5),
    }
}

/// Whether a command is a docker compose bootstrap invocation.
fn is_compose_bootstrap_command(command: &str) -> bool {
    let trimmed = command.trim_start();
    trimmed.starts_with("docker compose") || trimmed.starts_with("docker-compose")
}

/// Whether a step is a docker compose bootstrap (its command invokes
/// `docker compose` / `docker-compose`).
fn is_compose_bootstrap_step(step: &LaunchStep) -> bool {
    match &step.kind {
        StepKind::RunCommand { command, .. } => is_compose_bootstrap_command(command),
        _ => false,
    }
}

/// Resolve a RELATIVE program path against the step's working directory,
/// returning an absolute executable when it exists there. Bare command names
/// (npm, python, docker) stay PATH-resolved; absolute paths and shell
/// wrappers (`cmd`, `sh`) pass through unchanged.
///
/// Without this, a relative program (`.venv\Scripts\python.exe`) is checked
/// against the APP's own cwd by the command resolver and the spawn runs in
/// the project root, where cmd fails with the cryptic "The system cannot
/// find the path specified" (exit code 3) and the startup probe reports
/// "Command failed with exit code 3".
fn resolve_relative_program(
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
) -> Result<(String, Vec<String>), String> {
    let has_separator = program.contains('/') || program.contains('\\');
    if !has_separator || Path::new(program).is_absolute() {
        return Ok((program.to_string(), args.to_vec()));
    }
    let Some(dir) = working_dir else {
        return Err(format!(
            "Executable '{}' is a relative path, but this step has no working directory to \
             resolve it against (the profile's project path is missing or empty). Re-analyze \
             the project or fix the profile.",
            program
        ));
    };
    let candidate = Path::new(dir).join(program);
    if !candidate.is_file() {
        return Err(format!(
            "Executable '{}' not found in '{}'. The project's virtual environment may not \
             have been created: check that the install step ran (or is enabled), or \
             re-analyze the project.",
            program, dir
        ));
    }
    Ok((candidate.to_string_lossy().into_owned(), args.to_vec()))
}

/// Rewrite a docker compose invocation to pin its configuration file with
/// `-f`, so the step never depends on the process working directory to
/// find its config. Commands that are not docker compose, or that already
/// carry an explicit file flag, are returned unchanged.
fn pin_docker_compose_config(
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
) -> Result<(String, Vec<String>), String> {
    let exe = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    // Recognize both the bare `docker` and Windows batch shims
    // (`docker.cmd`/`docker.bat`): the resolver may return a `.cmd`/`.bat`
    // shim on Windows, and failing to recognize it silently drops the `-f`
    // pin, leaving the step dependent on the process working directory.
    let is_docker_cli =
        exe == "docker" || exe == "docker.exe" || exe == "docker.cmd" || exe == "docker.bat";
    let is_compose = exe.starts_with("docker-compose")
        || (is_docker_cli && args.first().map(|a| a == "compose").unwrap_or(false));
    if !is_compose
        || args
            .iter()
            .any(|a| a == "-f" || a == "--file" || a.starts_with("--file="))
    {
        return Ok((program.to_string(), args.to_vec()));
    }
    let dir = working_dir.ok_or_else(|| {
        "docker compose step has no working directory to locate its configuration file".to_string()
    })?;
    match crate::platform::docker_service::DockerService::find_compose_file(Path::new(dir)) {
        Some(file) => {
            let file = file.to_string_lossy().into_owned();
            let mut pinned = Vec::with_capacity(args.len() + 3);
            if exe.starts_with("docker-compose") {
                pinned.push("-f".to_string());
                pinned.push(file);
                pinned.extend(args.iter().cloned());
            } else {
                pinned.push("compose".to_string());
                pinned.push("-f".to_string());
                pinned.push(file);
                pinned.extend(args.iter().skip(1).cloned());
            }
            Ok((program.to_string(), pinned))
        }
        None => Err(format!(
            "docker compose could not find a configuration file in '{}' (expected \
             compose.yaml, compose.yml, docker-compose.yaml or docker-compose.yml)",
            dir
        )),
    }
}

/// Normalize a profile for execution (never persisted back):
///
/// - `WaitForPort` / `WaitForUrl` steps with a timeout below the floor are
///   raised to the floor, with an informational diagnostic.
/// - Readiness waits without a retry policy get a default backoff retry so
///   a stale profile still survives cold starts.
/// - A stored `StopRun` failure policy is downgraded to `SkipDependents`
///   for every step EXCEPT the docker compose bootstrap. Profiles generated
///   before the resilience rework baked `StopRun` into every step that had
///   dependents, so a failing `npm install` hard-aborted the whole run and
///   killed a still-starting `docker compose up` — the exact symptom where
///   "Start Docker Compose" dies with the generic "Run aborted by a failing
///   step; process terminated" while docker itself was never at fault.
fn normalize_profile_for_run(
    mut profile: LaunchProfileV2,
    diagnostics: &mut Vec<super::validation::ProfileValidationDiagnostic>,
) -> LaunchProfileV2 {
    for step in &mut profile.steps {
        // Legacy StopRun downgrade (see doc comment above).
        if step.failure_policy == Some(FailurePolicy::StopRun) {
            let is_compose_bootstrap = matches!(
                &step.kind,
                StepKind::RunCommand { command, .. } if is_compose_bootstrap_command(command)
            );
            if !is_compose_bootstrap {
                step.failure_policy = Some(FailurePolicy::SkipDependents);
                diagnostics.push(super::validation::ProfileValidationDiagnostic {
                    severity: DiagnosticSeverity::Info,
                    code: "FAILURE_POLICY_DOWNGRADED".to_string(),
                    message: "Failure policy downgraded from StopRun to SkipDependents: a failing \
                         step must not abort the run and kill already-started infrastructure"
                        .to_string(),
                    step_id: Some(step.id.clone()),
                    field: Some("failure_policy".to_string()),
                });
            }
        }

        let is_wait = matches!(
            step.kind,
            StepKind::WaitForPort { .. } | StepKind::WaitForUrl { .. }
        );
        if !is_wait {
            continue;
        }

        let effective_timeout: u64 = match step.timeout {
            Some(t) if t >= WAIT_TIMEOUT_FLOOR_SECS => t,
            Some(t) => {
                diagnostics.push(super::validation::ProfileValidationDiagnostic {
                    severity: DiagnosticSeverity::Info,
                    code: "WAIT_TIMEOUT_RAISED".to_string(),
                    message: format!(
                        "Wait timeout raised from {}s to {}s for cold-start reliability",
                        t, WAIT_TIMEOUT_FLOOR_SECS
                    ),
                    step_id: Some(step.id.clone()),
                    field: Some("timeout".to_string()),
                });
                WAIT_TIMEOUT_FLOOR_SECS
            }
            None => WAIT_TIMEOUT_FLOOR_SECS,
        };
        step.timeout = Some(effective_timeout);

        // Keep the completion policy in sync with the normalized timeout so
        // RunCommand steps with an embedded PortOpen/UrlReady wait honor it.
        if let Some(completion) = &mut step.completion {
            match completion {
                CompletionPolicy::PortOpen { timeout_secs, .. } => {
                    *timeout_secs = effective_timeout;
                }
                CompletionPolicy::UrlReady { timeout_secs, .. } => {
                    *timeout_secs = effective_timeout;
                }
                _ => {}
            }
        }

        if step.retry_policy.is_none() {
            step.retry_policy = Some(default_wait_retry_policy());
        }
    }
    profile
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

/// Preflight a raw command string for Docker steps.
///
/// Operates on the RAW command (as the user wrote it), not the resolved
/// program path: the resolver turns `docker` into `C:\Program Files\...\
/// docker.exe`, which no longer starts with "docker" — a program-based check
/// would silently skip the daemon preflight for every Docker step.
///
/// Fast-fails only on definitively-broken states (CLI missing, daemon down),
/// with an actionable message instead of docker's cryptic pipe errors. A
/// daemon that is still starting is deliberately allowed through: the
/// `WaitForDocker` step (or docker's own retry) handles the transition, and
/// failing here would abort the whole run (the compose bootstrap is StopRun)
/// on a transient that resolves on its own.
fn preflight_check(command: &str) -> Result<(), String> {
    let trimmed = command.trim_start();
    if !trimmed.starts_with("docker") {
        return Ok(());
    }

    match crate::platform::docker_service::DockerService::preflight_for_command(command) {
        Ok(()) => Ok(()),
        Err(diag) => match diag.status {
            crate::platform::docker_service::DockerStatus::DaemonStarting => Ok(()),
            _ => Err(format!(
                "{}: {}{}",
                diag.status,
                diag.message,
                diag.suggested_action
                    .as_ref()
                    .map(|a| format!("\n{}", a))
                    .unwrap_or_default()
            )),
        },
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
// Environment overlay helpers
// ---------------------------------------------------------------------------

/// Merge the run-level overlay (environment binding) with a step's own
/// `environment` map. Step-level variables win over the run overlay.
fn build_effective_overlay(
    run_overlay: &Option<EnvironmentOverlay>,
    step_env: &Option<HashMap<String, String>>,
) -> EnvironmentOverlay {
    let mut overlay = run_overlay.clone().unwrap_or_default();
    if let Some(env) = step_env {
        for (key, value) in env {
            overlay = overlay.set_var(key.clone(), value.clone());
        }
    }
    overlay
}

// ---------------------------------------------------------------------------
// Process liveness probe
// ---------------------------------------------------------------------------

/// Short grace period after a `ProcessStarted`-style spawn: the child must
/// still be alive for the step to count as started.
const START_PROBE_GRACE_MS: u64 = 4000;

/// Maximum time the startup-marker probe waits for the inner command to
/// either exit (writing its exit code to the marker) or keep running. A
/// long-running dev server never exits, so the probe treats "still running
/// after this window with no marker" as a successful start.
const MARKER_PROBE_WINDOW_MS: u64 = 6000;

/// Create a fresh startup-probe marker path in the system temp directory.
/// The file itself is created by the command run inside the terminal; the
/// orchestrator only polls for it.
fn create_startup_marker_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "devlauncher_startup_probe_{}.txt",
        uuid::Uuid::new_v4()
    ))
}

/// Verify that a visible-terminal command actually started, using the
/// exit-code marker the terminal writes after the inner command exits.
///
/// The startup marker is AUTHORITATIVE: the tracked terminal-launcher
/// process (wt.exe and similar wrappers) detaches by design once the window
/// is up, so its exit status says nothing about the inner command. Treating
/// a detached wrapper as "started" is what let instantly-failing commands
/// (a broken `cd`, a missing interpreter) report success — the launcher
/// exits 0 before the marker is ever written, and the probe short-circuited.
///
/// Returns:
/// - `Ok(true)` — the marker reports exit code 0 (command completed
///   successfully), or no marker appeared within the probe window (a
///   long-running dev server never exits, so it never writes a marker).
/// - `Err(msg)` — the marker reports a non-zero exit code (the command ran
///   and FAILED, e.g. a broken `.venv` invocation), or the terminal process
///   itself failed to open (non-zero launcher exit).
async fn probe_startup_marker(
    marker: &std::path::Path,
    proc_id: &str,
    process_manager: &Arc<dyn ProcessManager>,
    cancelled: &Arc<AtomicBool>,
) -> Result<bool, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(MARKER_PROBE_WINDOW_MS);
    loop {
        if cancelled.load(Ordering::SeqCst) {
            return Err("Cancelled".to_string());
        }

        if let Ok(meta) = std::fs::metadata(marker) {
            if meta.is_file() {
                let content = std::fs::read_to_string(marker).unwrap_or_default();
                match content.trim().parse::<i32>() {
                    Ok(0) => return Ok(true),
                    Ok(code) => {
                        let mut msg = format!(
                            "Command failed with exit code {} (reported by the startup probe); \
                             check the terminal output",
                            code
                        );
                        // The wrapper's captured output (cmd backend) usually
                        // carries the real failure line ("The system cannot
                        // find the path specified" and similar) — surface it.
                        let tail = output_tail_hint(process_manager, proc_id);
                        if !tail.is_empty() {
                            msg.push_str(&tail);
                        }
                        return Err(msg);
                    }
                    Err(_) => {
                        // Marker exists but holds no number yet (e.g. a
                        // PowerShell wrapper writing asynchronously) — keep
                        // waiting for a valid value.
                    }
                }
            }
        }

        // The launcher's clean exit (or disappearance from the tracker) is
        // NOT a verdict: terminal wrappers detach on purpose. Only a
        // non-zero exit means the terminal itself failed to open.
        let pm = process_manager.clone();
        let pid = proc_id.to_string();
        let status = tokio::task::spawn_blocking(move || pm.refresh_status(&pid))
            .await
            .ok()
            .and_then(|r| r.ok());

        match status {
            Some(ProcessStatus::Running)
            | Some(ProcessStatus::Starting)
            | Some(ProcessStatus::Ready)
            | Some(ProcessStatus::Exited(0))
            | None => {}
            Some(_) => {
                return Err(
                    "Terminal process failed before the startup probe could confirm the command"
                        .to_string(),
                );
            }
        }

        if tokio::time::Instant::now() >= deadline {
            // No marker within the window: the command is still running (a
            // long-running dev server never exits, so the marker is never
            // written). Treat as started.
            return Ok(true);
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Check that a freshly-spawned process is still alive after a short grace
/// period. Returns:
/// - `Ok(true)` — process is running (or already finished naturally);
/// - `Ok(false)` — process exited before the probe (instant-exit spawn);
/// - `Err(msg)` — the manager reported an unexpected state.
///
/// Terminal-wrapper processes (`tracking == TerminalWrapper`) are probed
/// leniently: the launcher process (wt.exe, osascript, gnome-terminal, …)
/// detaches as soon as the terminal window is up, and the real command
/// keeps running inside it. A wrapper exiting with code 0 is the NORMAL
/// case; only a non-zero exit (real spawn failure) fails the probe. The
/// actual service readiness is checked by the follow-up WaitForPort steps.
async fn probe_process_alive(
    proc_id: &str,
    process_manager: &Arc<dyn ProcessManager>,
    cancelled: &Arc<AtomicBool>,
    tracking_quality: Option<&ProcessTrackingQuality>,
) -> Result<bool, String> {
    let is_wrapper = matches!(
        tracking_quality,
        Some(ProcessTrackingQuality::TerminalWrapper)
    );
    let grace = Duration::from_millis(START_PROBE_GRACE_MS);
    let start = tokio::time::Instant::now();
    loop {
        let pm = process_manager.clone();
        let pid = proc_id.to_string();
        let status = tokio::task::spawn_blocking(move || pm.refresh_status(&pid))
            .await
            .ok()
            .and_then(|r| r.ok());

        match status {
            Some(ProcessStatus::Running)
            | Some(ProcessStatus::Starting)
            | Some(ProcessStatus::Ready) => {
                if tokio::time::Instant::now() - start >= grace {
                    return Ok(true);
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Some(ProcessStatus::Exited(code)) => {
                // A wrapper that detached cleanly is a successful launch.
                if is_wrapper && code == 0 {
                    return Ok(true);
                }
                return Ok(false);
            }
            Some(ProcessStatus::ExitedWithError(_))
            | Some(ProcessStatus::Crashed)
            | Some(ProcessStatus::Killed) => return Ok(false),
            None => {
                // Process no longer tracked: it exited (or the manager was
                // restarted). For a wrapper this is still a clean launch —
                // the terminal window owns the real process from here on.
                if is_wrapper {
                    return Ok(true);
                }
                return Ok(false);
            }
            Some(ProcessStatus::TimedOut) | Some(ProcessStatus::Cancelled) => {
                return Ok(false);
            }
            Some(ProcessStatus::ExternalLaunchAccepted) | Some(ProcessStatus::Unknown) => {
                return Ok(true);
            }
        }

        if cancelled.load(Ordering::SeqCst) {
            return Err("Cancelled".to_string());
        }
    }
}

/// Collapse multi-line output into a single short hint line.
fn truncate_lines(text: &str, max_lines: usize) -> String {
    let mut lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut out = String::new();
    for (i, line) in lines.drain(..max_lines.min(lines.len())).enumerate() {
        if i > 0 {
            out.push_str(" | ");
        }
        let trimmed = line.trim();
        out.push_str(&trimmed[..trimmed.len().min(200)]);
    }
    out
}

/// Tail of a failed process's captured output, appended to failure
/// messages so the user sees WHY a command failed (e.g. a Docker build
/// error) instead of a bare exit code. Never returns more than ~2KB.
fn output_tail_hint(process_manager: &Arc<dyn ProcessManager>, proc_id: &str) -> String {
    let logs = process_manager.get_logs(proc_id).ok();
    let mut lines: Vec<String> = Vec::new();
    if let Some(l) = logs {
        for line in l.stderr_lines.into_iter().chain(l.stdout_lines) {
            if !line.trim().is_empty() {
                lines.push(line);
            }
        }
    }
    if lines.is_empty() {
        return String::new();
    }
    let mut tail: Vec<&str> = lines.iter().map(|s| s.as_str()).rev().take(12).collect();
    tail.reverse();
    let text = tail.join("\n");
    let text = if text.len() > 2048 {
        format!("…{}", &text[text.len() - 2048..])
    } else {
        text
    };
    format!("\n--- last output ---\n{}", text)
}

/// Loopback aliases for a host name: `localhost` ↔ `127.0.0.1` (plus
/// `::1`), so a port wait works regardless of which interface the server
/// bound to. Other hosts pass through unchanged.
fn loopback_host_aliases(host: &str) -> Vec<String> {
    let lower = host.trim().to_ascii_lowercase();
    match lower.as_str() {
        "localhost" | "127.0.0.1" | "::1" | "0.0.0.0" => vec![
            "127.0.0.1".to_string(),
            "localhost".to_string(),
            "::1".to_string(),
        ],
        _ => vec![host.to_string()],
    }
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
    fn create_run_skips_compose_step_without_config_file() {
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

        let dir = std::env::temp_dir().join(format!(
            "stackpilot_dl_orch_nocompose_{}",
            std::process::id()
        ));
        let empty_dir =
            std::env::temp_dir().join(format!("stackpilot_dl_orch_empty_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&empty_dir).unwrap();
        std::fs::write(
            dir.join("docker-compose.yaml"),
            "services:\n  db:\n    image: postgres:16\n",
        )
        .unwrap();

        let mut with_file = make_step("compose_ok", vec![]);
        with_file.kind = StepKind::RunCommand {
            command: "docker compose up -d".to_string(),
            command_spec: None,
        };
        with_file.working_directory = Some(dir.to_string_lossy().into_owned());
        let mut without_file = make_step("compose_missing", vec![]);
        without_file.kind = StepKind::RunCommand {
            command: "docker compose up -d".to_string(),
            command_spec: None,
        };
        without_file.working_directory = Some(empty_dir.to_string_lossy().into_owned());

        let mut profile = valid_profile(vec![with_file, without_file]);
        profile.project_root = Some(dir.to_string_lossy().into_owned());

        let orchestrator = RunOrchestrator::new(Arc::new(MockPM));
        let run = orchestrator.create_run(profile).unwrap();
        let by_id = |id: &str| run.steps.iter().find(|s| s.step_id == id).unwrap();
        assert_eq!(by_id("compose_ok").status, StepStatus::Pending);
        assert_eq!(by_id("compose_missing").status, StepStatus::Skipped);
        assert!(run
            .diagnostics
            .iter()
            .any(|d| d.message.contains("no docker compose configuration file")));

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&empty_dir);
    }

    #[test]
    fn pin_compose_config_pins_existing_file() {
        let dir =
            std::env::temp_dir().join(format!("stackpilot_dl_orch_pin_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("compose.yaml"),
            "services:\n  db:\n    image: postgres:16\n",
        )
        .unwrap();

        let (program, args) = pin_docker_compose_config(
            "docker",
            &["compose".to_string(), "up".to_string(), "-d".to_string()],
            Some(dir.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(program, "docker");
        assert_eq!(args[0], "compose");
        assert_eq!(args[1], "-f");
        assert!(Path::new(&args[2]).is_file());
        assert_eq!(&args[3..], &["up", "-d"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pin_compose_config_preserves_explicit_file_flag() {
        let (program, args) = pin_docker_compose_config(
            "docker",
            &[
                "compose".to_string(),
                "-f".to_string(),
                "my-compose.yml".to_string(),
                "up".to_string(),
                "-d".to_string(),
            ],
            Some("C:\\nonexistent"),
        )
        .unwrap();
        assert_eq!(program, "docker");
        assert_eq!(args[0], "compose");
        assert_eq!(args[1], "-f");
        assert_eq!(args[2], "my-compose.yml");
    }

    #[test]
    fn pin_compose_config_errors_when_file_missing() {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_dl_orch_pin_empty_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let err = pin_docker_compose_config(
            "docker",
            &["compose".to_string(), "up".to_string(), "-d".to_string()],
            Some(dir.to_str().unwrap()),
        )
        .unwrap_err();
        assert!(
            err.contains("could not find a configuration file"),
            "{}",
            err
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pin_compose_config_ignores_non_compose_commands() {
        let (program, args) =
            pin_docker_compose_config("docker", &["ps".to_string()], None).unwrap();
        assert_eq!(program, "docker");
        assert_eq!(args, vec!["ps".to_string()]);
    }

    #[test]
    fn pin_compose_config_pins_windows_cmd_shim() {
        // Windows may resolve `docker` to a `.cmd`/`.bat` batch shim. The
        // `-f` pin must still be applied, otherwise the step silently depends
        // on the process working directory.
        let dir =
            std::env::temp_dir().join(format!("stackpilot_dl_orch_pin_cmd_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("compose.yaml"),
            "services:\n  db:\n    image: postgres:16\n",
        )
        .unwrap();

        for shim in ["docker.cmd", "docker.bat"] {
            let program = format!("C:\\Tools\\Docker\\{}", shim);
            let (program_out, args) = pin_docker_compose_config(
                &program,
                &["compose".to_string(), "up".to_string(), "-d".to_string()],
                Some(dir.to_str().unwrap()),
            )
            .unwrap();
            assert_eq!(program_out, program);
            assert_eq!(args[0], "compose");
            assert_eq!(args[1], "-f");
            assert!(Path::new(&args[2]).is_file());
            assert_eq!(&args[3..], &["up", "-d"]);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pin_compose_config_pins_bare_bat_shim() {
        let dir =
            std::env::temp_dir().join(format!("stackpilot_dl_orch_pin_bat_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("docker-compose.yaml"),
            "services:\n  db:\n    image: postgres:16\n",
        )
        .unwrap();

        let (program, args) = pin_docker_compose_config(
            "docker.bat",
            &["compose".to_string(), "up".to_string(), "-d".to_string()],
            Some(dir.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(program, "docker.bat");
        assert_eq!(args[0], "compose");
        assert_eq!(args[1], "-f");
        assert!(Path::new(&args[2]).is_file());

        let _ = std::fs::remove_dir_all(&dir);
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

    // -------------------------------------------------------------------
    // Run-time profile normalization
    // -------------------------------------------------------------------

    fn make_wait_step(id: &str, timeout: Option<u64>, retry: Option<RetryPolicy>) -> LaunchStep {
        LaunchStep {
            id: id.to_string(),
            label: id.to_string(),
            enabled: true,
            kind: StepKind::WaitForPort {
                host: "127.0.0.1".to_string(),
                port: 8080,
                candidate_ports: vec![],
            },
            depends_on: vec![],
            working_directory: None,
            environment: None,
            visibility: None,
            execution_mode: None,
            completion: Some(CompletionPolicy::PortOpen {
                host: "127.0.0.1".to_string(),
                port: 8080,
                timeout_secs: timeout.unwrap_or(30),
            }),
            timeout,
            failure_policy: None,
            retry_policy: retry,
            metadata: None,
            extra: Map::new(),
        }
    }

    #[test]
    fn normalize_profile_raises_low_wait_timeouts() {
        let profile = valid_profile(vec![make_wait_step("w1", Some(30), None)]);
        let mut diagnostics = Vec::new();
        let normalized = normalize_profile_for_run(profile, &mut diagnostics);
        assert_eq!(normalized.steps[0].timeout, Some(60));
        // The embedded completion policy stays in sync.
        match normalized.steps[0].completion.as_ref().unwrap() {
            CompletionPolicy::PortOpen { timeout_secs, .. } => assert_eq!(*timeout_secs, 60),
            other => panic!("expected PortOpen, got {:?}", other),
        }
        assert!(diagnostics.iter().any(|d| d.code == "WAIT_TIMEOUT_RAISED"));
    }

    #[test]
    fn normalize_profile_adds_default_retry_policy_to_waits() {
        let profile = valid_profile(vec![
            make_wait_step("w1", Some(90), None),
            make_step("other", vec![]),
        ]);
        let mut diagnostics = Vec::new();
        let normalized = normalize_profile_for_run(profile, &mut diagnostics);
        let wait = normalized.steps.iter().find(|s| s.id == "w1").unwrap();
        assert!(
            wait.retry_policy.is_some(),
            "wait steps get a default retry"
        );
        let other = normalized.steps.iter().find(|s| s.id == "other").unwrap();
        assert!(other.retry_policy.is_none(), "non-wait steps untouched");
    }

    #[test]
    fn normalize_profile_keeps_long_timeouts() {
        let profile = valid_profile(vec![make_wait_step("w1", Some(300), None)]);
        let mut diagnostics = Vec::new();
        let normalized = normalize_profile_for_run(profile, &mut diagnostics);
        assert_eq!(normalized.steps[0].timeout, Some(300));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn normalize_profile_downgrades_legacy_stop_run_for_install_steps() {
        // A legacy profile that baked `StopRun` into an `npm install` step
        // must not hard-abort the run (and kill a still-starting docker
        // compose) when the install fails. Run-time normalization rewrites
        // it to SkipDependents; the docker compose bootstrap keeps StopRun.
        let mut install = make_step("install", vec!["compose"]);
        install.failure_policy = Some(FailurePolicy::StopRun);
        install.kind = StepKind::RunCommand {
            command: "npm install".to_string(),
            command_spec: None,
        };
        let mut compose = make_step("compose", vec![]);
        compose.failure_policy = Some(FailurePolicy::StopRun);
        compose.kind = StepKind::RunCommand {
            command: "docker compose up -d".to_string(),
            command_spec: None,
        };

        let profile = valid_profile(vec![install, compose]);
        let mut diagnostics = Vec::new();
        let normalized = normalize_profile_for_run(profile, &mut diagnostics);

        let normalized_install = normalized.steps.iter().find(|s| s.id == "install").unwrap();
        assert_eq!(
            normalized_install.failure_policy,
            Some(FailurePolicy::SkipDependents),
            "install steps are downgraded so a failure never aborts the run"
        );
        let normalized_compose = normalized.steps.iter().find(|s| s.id == "compose").unwrap();
        assert_eq!(
            normalized_compose.failure_policy,
            Some(FailurePolicy::StopRun),
            "the docker compose bootstrap keeps its hard-abort policy"
        );
        assert!(diagnostics
            .iter()
            .any(|d| d.code == "FAILURE_POLICY_DOWNGRADED"));
    }

    // -------------------------------------------------------------------
    // Loopback alias resolution
    // -------------------------------------------------------------------

    #[test]
    fn loopback_aliases_cover_localhost_variants() {
        let aliases = loopback_host_aliases("localhost");
        assert!(aliases.contains(&"127.0.0.1".to_string()));
        assert!(aliases.contains(&"::1".to_string()));
        let aliases = loopback_host_aliases("127.0.0.1");
        assert!(aliases.contains(&"localhost".to_string()));
        // Foreign hosts pass through untouched.
        assert_eq!(loopback_host_aliases("192.168.1.10"), vec!["192.168.1.10"]);
        assert_eq!(
            loopback_host_aliases("api.example.com"),
            vec!["api.example.com"]
        );
    }

    // -------------------------------------------------------------------
    // Overlay merge
    // -------------------------------------------------------------------

    #[test]
    fn effective_overlay_merges_step_env_over_run_overlay() {
        let mut run_vars = HashMap::new();
        run_vars.insert("PORT".to_string(), "3000".to_string());
        run_vars.insert("SHARED".to_string(), "run".to_string());
        let run_overlay = Some(
            EnvironmentOverlay::new()
                .set_var("PORT", "3000")
                .set_var("SHARED", "run"),
        );

        let mut step_env = HashMap::new();
        step_env.insert("SHARED".to_string(), "step".to_string());
        step_env.insert("EXTRA".to_string(), "1".to_string());

        let merged = build_effective_overlay(&run_overlay, &Some(step_env));
        assert_eq!(merged.vars_set.get("PORT").unwrap(), "3000");
        // Step-level variable wins over the run overlay.
        assert_eq!(merged.vars_set.get("SHARED").unwrap(), "step");
        assert_eq!(merged.vars_set.get("EXTRA").unwrap(), "1");
        let _ = run_vars;
    }

    // -------------------------------------------------------------------
    // Output truncation
    // -------------------------------------------------------------------

    #[test]
    fn truncate_lines_collapses_and_limits() {
        assert_eq!(truncate_lines("a\nb\nc\n", 2), "a | b");
        assert_eq!(truncate_lines("only", 5), "only");
        assert!(truncate_lines("", 3).is_empty());
    }

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
                    candidate_ports: vec![],
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

        // Under `WarnAndContinue`, a failed step is recorded as completed:
        // its own dependents may still run (they make their own checks),
        // while independent branches are unaffected.
        let mut completed = HashSet::new();
        completed.insert("a".to_string());
        let running = HashSet::new();
        let ready = RunOrchestrator::find_ready_steps(&profile, &completed, &running);
        assert!(ready.contains(&"b".to_string()));
        // 'c' depends on 'a' which completed (failed-but-continued):
        // per the Continue policy its dependents are NOT blocked.
        assert!(ready.contains(&"c".to_string()));
        // 'd' depends on 'b' which has not run yet.
        assert!(!ready.contains(&"d".to_string()));
        // Once 'b' completes, 'd' becomes ready.
        let mut completed2 = completed;
        completed2.insert("b".to_string());
        let ready2 = RunOrchestrator::find_ready_steps(&profile, &completed2, &running);
        assert!(ready2.contains(&"d".to_string()));
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

        // A directly-tracked (non-wrapper) process that vanishes without an
        // exit status cannot be confirmed as successful — the step must fail
        // (this is what would otherwise mask a failed `docker compose up`).
        let result = tokio::runtime::Runtime::new().unwrap().block_on(
            RunOrchestrator::wait_for_process_exit(
                "run-1",
                "step-1",
                "nonexistent-process",
                5,
                &cancelled,
                &cancel_notify,
                None, // Exact tracking (no wrapper)
                &pm,
                &app_handle,
            ),
        );
        assert!(!result.success, "non-wrapper lost process must fail");
        assert!(result.process_id.is_some());
        assert!(result.error.unwrap().contains("no longer tracked"));

        // A terminal-wrapper process detaching cleanly is the NORMAL case
        // (the terminal owns the real process from here on) — success.
        let result = tokio::runtime::Runtime::new().unwrap().block_on(
            RunOrchestrator::wait_for_process_exit(
                "run-1",
                "step-1",
                "nonexistent-process",
                5,
                &cancelled,
                &cancel_notify,
                Some(&ProcessTrackingQuality::TerminalWrapper),
                &pm,
                &app_handle,
            ),
        );
        assert!(
            result.success,
            "terminal wrapper detach is a successful launch"
        );
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
        // The project root must actually exist (the validator fails fast
        // on deleted projects), so use a real temp directory with spaces.
        let dir = std::env::temp_dir().join("stackpilot test dir with spaces");
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.to_string_lossy().into_owned();

        let profile = LaunchProfileV2 {
            schema_version: "2".to_string(),
            id: "test".to_string(),
            name: "Test".to_string(),
            description: "desc".to_string(),
            project_root: Some(root.clone()),
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
        let resolved = resolve_working_directory(Some(&root), Some("./sub dir"));
        let expected = std::path::Path::new(&root)
            .join("sub dir")
            .to_string_lossy()
            .into_owned();
        assert_eq!(resolved, Some(expected));

        let _ = std::fs::remove_dir_all(&dir);
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

    /// Test: Failure policy defaults based on step kind and dependencies.
    #[test]
    fn test_failure_policy_defaults() {
        // Non-wait steps with dependents default to SkipDependents: a failing
        // install/build/service must skip its own chain, never abort the run
        // and kill already-started infrastructure (e.g. docker compose).
        assert_eq!(
            StepKind::RunCommand {
                command: "x".to_string(),
                command_spec: None,
            }
            .default_failure_policy(true),
            FailurePolicy::SkipDependents
        );
        // Readiness waits with dependents default to SkipDependents so a
        // failed infra wait does not take down independent branches
        assert_eq!(
            StepKind::WaitForDocker {}.default_failure_policy(true),
            FailurePolicy::SkipDependents
        );
        assert_eq!(
            StepKind::WaitForPort {
                host: "127.0.0.1".to_string(),
                port: 3000,
                candidate_ports: vec![],
            }
            .default_failure_policy(true),
            FailurePolicy::SkipDependents
        );
        // Leaf steps default to WarnAndContinue
        assert_eq!(
            StepKind::RunCommand {
                command: "x".to_string(),
                command_spec: None,
            }
            .default_failure_policy(false),
            FailurePolicy::WarnAndContinue
        );
    }

    /// A failed infra wait (SkipDependents) must not collapse the whole
    /// run: succeeded branches + failed waits yield PartialSuccess, only
    /// failures yield Failed, all good yields Succeeded.
    #[test]
    fn compute_final_status_handles_partial_success() {
        fn make_run(statuses: &[StepStatus]) -> LaunchRun {
            LaunchRun {
                run_id: "r1".to_string(),
                profile_id: "p1".to_string(),
                profile_name: "P".to_string(),
                status: RunStatus::Running,
                steps: statuses
                    .iter()
                    .enumerate()
                    .map(|(i, s)| StepExecutionState {
                        step_id: format!("step_{}", i),
                        status: s.clone(),
                        process_id: None,
                        error: None,
                        retries_remaining: None,
                        started_at: None,
                        finished_at: None,
                        attempts: Vec::new(),
                    })
                    .collect(),
                created_at: "t".to_string(),
                finished_at: None,
                cancelled: false,
                diagnostics: Vec::new(),
            }
        }

        fn make_handle(run: LaunchRun) -> Arc<RunHandle> {
            Arc::new(RunHandle {
                run: Arc::new(Mutex::new(run)),
                cancelled: Arc::new(AtomicBool::new(false)),
                cancel_notify: Arc::new(tokio::sync::Notify::new()),
                profile: valid_profile(Vec::new()),
                session_id: None,
                process_ids: Arc::new(Mutex::new(Vec::new())),
                overlay: None,
                browser_path: None,
            })
        }

        // Failed wait + succeeded branches + skipped dependents.
        let run = make_run(&[
            StepStatus::Succeeded,
            StepStatus::Failed,
            StepStatus::Skipped,
        ]);
        assert_eq!(
            RunOrchestrator::compute_final_status(&make_handle(run)),
            RunStatus::PartialSuccess
        );

        // Only failures -> Failed.
        let run = make_run(&[StepStatus::Failed, StepStatus::Skipped]);
        assert_eq!(
            RunOrchestrator::compute_final_status(&make_handle(run)),
            RunStatus::Failed
        );

        // All good -> Succeeded.
        let run = make_run(&[StepStatus::Succeeded, StepStatus::Skipped]);
        assert_eq!(
            RunOrchestrator::compute_final_status(&make_handle(run)),
            RunStatus::Succeeded
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

    // -----------------------------------------------------------------------
    // Startup-marker probe
    // -----------------------------------------------------------------------

    struct RunningPM;

    impl ProcessManager for RunningPM {
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
            Ok(crate::modules::workspace::models::ProcessStatus::Running)
        }
        fn get_logs(
            &self,
            _: &str,
        ) -> Result<crate::modules::workspace::models::ProcessLogs, String> {
            Ok(crate::modules::workspace::models::ProcessLogs {
                stdout_lines: Vec::new(),
                stderr_lines: Vec::new(),
            })
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

    #[tokio::test]
    async fn startup_marker_probe_success_on_exit_code_zero() {
        let marker =
            std::env::temp_dir().join(format!("stackpilot_dl_probe_ok_{}.txt", std::process::id()));
        std::fs::write(&marker, "0").unwrap();
        let pm: Arc<dyn ProcessManager> = Arc::new(RunningPM);
        let cancelled = Arc::new(AtomicBool::new(false));
        let result = probe_startup_marker(&marker, "proc_x", &pm, &cancelled).await;
        let _ = std::fs::remove_file(&marker);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[tokio::test]
    async fn startup_marker_probe_reports_nonzero_exit() {
        let marker = std::env::temp_dir().join(format!(
            "stackpilot_dl_probe_fail_{}.txt",
            std::process::id()
        ));
        std::fs::write(&marker, "9009").unwrap();
        let pm: Arc<dyn ProcessManager> = Arc::new(RunningPM);
        let cancelled = Arc::new(AtomicBool::new(false));
        let result = probe_startup_marker(&marker, "proc_x", &pm, &cancelled).await;
        let _ = std::fs::remove_file(&marker);
        assert!(result.is_err(), "{result:?}");
        assert!(result.unwrap_err().contains("9009"));
    }

    #[test]
    fn resolve_relative_program_resolves_against_working_dir() {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_dl_rrp_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let exe = dir.join("bin").join("tool.py");
        std::fs::write(&exe, "#!/usr/bin/env python\n").unwrap();
        let dir_str = dir.to_string_lossy().into_owned();
        let args = vec!["manage.py".to_string(), "runserver".to_string()];

        // Bare names stay PATH-resolved.
        let (p, a) = resolve_relative_program("python", &args, Some(&dir_str)).unwrap();
        assert_eq!(p, "python");
        assert_eq!(a, args);

        // Absolute paths pass through.
        let abs = exe.to_string_lossy().into_owned();
        let (p, _) = resolve_relative_program(&abs, &args, Some(&dir_str)).unwrap();
        assert_eq!(p, abs);

        // Relative path with separator resolves to an absolute executable.
        let (p, _) = resolve_relative_program(
            if cfg!(windows) {
                r"bin\tool.py"
            } else {
                "bin/tool.py"
            },
            &args,
            Some(&dir_str),
        )
        .unwrap();
        assert_eq!(p, exe.to_string_lossy());

        // Missing executable → actionable error (this is the exit-code-3
        // scenario: `.venv\Scripts\python.exe` not found in the cwd).
        let err = resolve_relative_program(".venv\\Scripts\\python.exe", &args, Some(&dir_str))
            .unwrap_err();
        assert!(err.contains("not found"), "{err}");

        // Relative program with NO working directory → clear error instead
        // of silently spawning in the app's own cwd.
        let err = resolve_relative_program(".venv\\Scripts\\python.exe", &args, None).unwrap_err();
        assert!(err.contains("no working directory"), "{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn startup_marker_probe_assumes_running_when_no_marker() {
        // No marker appears and the terminal keeps running: a long-running
        // dev server. The probe must NOT claim success on a failed command
        // — it must keep waiting and finally assume the process is running.
        let marker = std::env::temp_dir().join(format!(
            "stackpilot_dl_probe_running_{}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let pm: Arc<dyn ProcessManager> = Arc::new(RunningPM);
        let cancelled = Arc::new(AtomicBool::new(false));
        let result = probe_startup_marker(&marker, "proc_x", &pm, &cancelled).await;
        let _ = std::fs::remove_file(&marker);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
