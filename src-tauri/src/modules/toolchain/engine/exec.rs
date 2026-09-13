// ============================================================
// Job executor (engine/exec.rs)
// ============================================================
//
// Drives one registered job through its canonical tasks:
//   validating -> preparing -> [downloading -> verifying ->
//   installing -> configuring -> updating path] / checking-health
//   -> completed/failed/cancelled/interrupted.
//
// Execution reuses the proven installer machinery (source fallback,
// UAC, digest verification, traversal-safe extraction, verify-by-
// detection) through a single-task adapter; every legacy event is
// translated into typed engine events. Secrets are collected out of
// band and never enter job state. Cancellation kills the running
// process and marks the remaining tasks Cancelled.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use crate::modules::toolchain::core as tc_core;
use crate::modules::toolchain::core::console::EventSink;
use crate::modules::toolchain::models::{
    InstallPlan, InstallSource, InstallTask, TaskPhase, TaskState, ToolDefinition, ToolStatus,
};

use super::jobs::{JobEngine, JobHandle};
use super::plan::{EngineTaskStatus, JobStatus, NoopReason, PathChangeRecord, Phase, TaskAction};

// ------------------------------------------------------------
// Runner port
// ------------------------------------------------------------

pub struct TaskOutcome {
    pub status: EngineTaskStatus,
    /// Generated secrets (DB passwords) — delivered to the take-once
    /// channel by the commands layer, never persisted in job state.
    pub secrets: HashMap<String, String>,
    /// PATH entries actually added by this task (audited).
    pub path_added: Vec<String>,
}

impl TaskOutcome {
    fn plain(status: EngineTaskStatus) -> Self {
        Self {
            status,
            secrets: HashMap::new(),
            path_added: Vec::new(),
        }
    }
}

pub struct TaskContext<'a> {
    pub job_id: &'a str,
    pub os: &'a str,
    pub definitions: &'a [ToolDefinition],
    pub task: &'a super::plan::PlanTask,
    pub cancel: Arc<AtomicBool>,
}

#[async_trait]
pub trait TaskRunner: Send + Sync {
    async fn run(&self, ctx: TaskContext<'_>) -> TaskOutcome;
}

// ------------------------------------------------------------
// Legacy event bridge
// ------------------------------------------------------------

/// Translates legacy installer events into typed engine events so both
/// protocols stay in sync during migration.
struct BridgeSink {
    engine: Arc<JobEngine>,
    handle: Arc<JobHandle>,
    index: usize,
}

impl EventSink for BridgeSink {
    fn emit(&self, event: crate::modules::toolchain::models::ToolchainEvent) {
        match event.event_type {
            crate::modules::toolchain::models::ToolchainEventType::TaskStarted => {
                let total = event.total_tasks;
                self.engine.emit(
                    &self.handle,
                    &event.task_id,
                    &event.tool_id,
                    super::events::JobEventPayload::TaskStarted {
                        index: self.index,
                        total,
                    },
                );
            }
            crate::modules::toolchain::models::ToolchainEventType::TaskPhaseChanged { phase } => {
                let mapped = map_legacy_phase(phase);
                self.engine.emit_phase(&self.handle, self.index, mapped);
            }
            crate::modules::toolchain::models::ToolchainEventType::TaskProgress { line } => {
                self.engine.emit_progress(&self.handle, self.index, line);
            }
            // TaskCompleted/AllCompleted are produced by the executor itself.
            _ => {}
        }
    }
}

fn map_legacy_phase(phase: TaskPhase) -> Phase {
    match phase {
        TaskPhase::Downloading => Phase::Downloading,
        TaskPhase::Installing => Phase::Installing,
        TaskPhase::Verifying => Phase::Verifying,
        TaskPhase::UpdatingPath => Phase::UpdatingPath,
    }
}

fn map_legacy_state(state: &TaskState, cancelled: bool) -> EngineTaskStatus {
    match state {
        TaskState::Success { version } => EngineTaskStatus::Succeeded {
            version: version.clone(),
        },
        TaskState::Failed { error } => EngineTaskStatus::Failed {
            error: error.clone(),
        },
        TaskState::Skipped { reason } => {
            if cancelled {
                EngineTaskStatus::Cancelled
            } else {
                EngineTaskStatus::Failed {
                    error: reason.clone(),
                }
            }
        }
        TaskState::Pending | TaskState::Running { .. } => {
            if cancelled {
                EngineTaskStatus::Cancelled
            } else {
                EngineTaskStatus::Failed {
                    error: "задача не завершилась".to_string(),
                }
            }
        }
    }
}

// ------------------------------------------------------------
// Real runner (production)
// ------------------------------------------------------------

pub struct RealRunner {
    pub engine: Arc<JobEngine>,
    pub handle: Arc<JobHandle>,
}

#[async_trait]
impl TaskRunner for RealRunner {
    async fn run(&self, ctx: TaskContext<'_>) -> TaskOutcome {
        match &ctx.task.action {
            TaskAction::InstallNew { .. } | TaskAction::Update { .. } => {
                self.run_install_like(ctx).await
            }
            TaskAction::RepairPath => self.run_repair_path(ctx).await,
            TaskAction::HealthCheck => self.run_health_check(ctx).await,
            TaskAction::NoOp(_) => TaskOutcome::plain(EngineTaskStatus::Cancelled),
        }
    }
}

impl RealRunner {
    async fn run_install_like(&self, ctx: TaskContext<'_>) -> TaskOutcome {
        if !ctx.definitions.iter().any(|d| d.id == ctx.task.tool_id) {
            return TaskOutcome::plain(EngineTaskStatus::Failed {
                error: format!("инструмент {} исчез из каталога", ctx.task.tool_id),
            });
        }

        let legacy_task = InstallTask {
            task_id: ctx.task.task_id.clone(),
            tool_id: ctx.task.tool_id.clone(),
            display: ctx.task.display.clone(),
            icon: ctx.task.icon.clone(),
            size_mb: ctx.task.size_mb,
            needs_admin: ctx.task.needs_admin,
            source_description: ctx
                .task
                .source
                .as_ref()
                .map(|s| s.description.clone())
                .unwrap_or_default(),
            install_options: ctx.task.install_options.clone(),
            state: TaskState::Pending,
        };

        let mut legacy_plan = InstallPlan {
            tasks: vec![legacy_task],
            total_size_mb: ctx.task.size_mb as u64,
            os: ctx.os.to_string(),
            session_id: ctx.job_id.to_string(),
        };

        let bridge: Arc<dyn EventSink> = Arc::new(BridgeSink {
            engine: Arc::clone(&self.engine),
            handle: Arc::clone(&self.handle),
            index: 0,
        });

        let before_user_path = tc_core::path_service::process_path_entries();

        let is_update = matches!(ctx.task.action, TaskAction::Update { .. });
        let secrets = tc_core::installer::execute_plan(
            ctx.definitions,
            &mut legacy_plan,
            bridge,
            Arc::clone(&ctx.cancel),
            ctx.job_id,
            is_update,
        )
        .await;

        let state = &legacy_plan.tasks[0].state;
        let cancelled = ctx.cancel.load(Ordering::SeqCst);
        let status = map_legacy_state(state, cancelled);

        let path_added = diff_entries(&before_user_path);
        TaskOutcome {
            status,
            secrets,
            path_added,
        }
    }

    /// Explicit, audited, idempotent PATH repair for one tool.
    async fn run_repair_path(&self, ctx: TaskContext<'_>) -> TaskOutcome {
        if ctx.task.path_entries.is_empty() {
            return TaskOutcome::plain(EngineTaskStatus::Succeeded {
                version: "нет записей PATH".into(),
            });
        }

        let platform = crate::modules::toolchain::platforms::current_platform();
        let before = platform.read_user_path().await.unwrap_or_default();

        if let Err(e) = tc_core::path_service::add_to_user_path(&ctx.task.path_entries).await {
            return TaskOutcome::plain(EngineTaskStatus::Failed {
                error: format!("не удалось обновить PATH: {e}"),
            });
        }
        let _ = tc_core::path_service::sync_process_path().await;

        let after = platform.read_user_path().await.unwrap_or_default();
        let added: Vec<String> = after
            .iter()
            .filter(|e| !before.iter().any(|b| tc_core::path_service::same_dir(b, e)))
            .cloned()
            .collect();

        if added.is_empty() {
            TaskOutcome::plain(EngineTaskStatus::Succeeded {
                version: "PATH уже в порядке".into(),
            })
        } else {
            TaskOutcome {
                status: EngineTaskStatus::Succeeded {
                    version: format!("+{}", added.len()),
                },
                secrets: HashMap::new(),
                path_added: added,
            }
        }
    }

    /// Read-only health rerun; findings stream as progress lines.
    async fn run_health_check(&self, ctx: TaskContext<'_>) -> TaskOutcome {
        let Some(def) = ctx.definitions.iter().find(|d| d.id == ctx.task.tool_id) else {
            return TaskOutcome::plain(EngineTaskStatus::Failed {
                error: format!("инструмент {} отсутствует в каталоге", ctx.task.tool_id),
            });
        };
        let status: ToolStatus = tc_core::discovery::detect_tool(def).await;
        if matches!(status, ToolStatus::Missing) {
            return TaskOutcome::plain(EngineTaskStatus::Succeeded {
                version: "not-installed".into(),
            });
        }
        let health = tc_core::health::check_tool(def, &status).await;
        for check in &health.checks {
            self.engine.emit_progress(
                &self.handle,
                0,
                format!(
                    "tc:health {} {}: {}",
                    def.id,
                    if check.ok { "ok" } else { "fail" },
                    check.label
                ),
            );
        }
        let verdict = match health.state {
            crate::modules::toolchain::models::HealthState::Healthy => "healthy",
            crate::modules::toolchain::models::HealthState::Failed => "unhealthy",
            crate::modules::toolchain::models::HealthState::NotChecked => "not-checked",
            crate::modules::toolchain::models::HealthState::Unavailable => "unavailable",
        };
        TaskOutcome::plain(EngineTaskStatus::Succeeded {
            version: verdict.into(),
        })
    }
}

/// Entries that appeared in the process PATH during this task window.
/// The installer adds tool dirs to the user PATH and syncs the process
/// env, so new entries here are exactly what the task contributed.
fn diff_entries(before: &[String]) -> Vec<String> {
    let after = tc_core::path_service::process_path_entries();
    after
        .iter()
        .filter(|e| !before.iter().any(|b| tc_core::path_service::same_dir(b, e)))
        .cloned()
        .collect()
}

// ------------------------------------------------------------
// Job orchestration
// ------------------------------------------------------------

/// Executes a registered job to a terminal state. Returns generated
/// secrets (out-of-band delivery).
pub async fn run_job(
    engine: Arc<JobEngine>,
    handle: Arc<JobHandle>,
    definitions: Arc<Vec<ToolDefinition>>,
    runner: Arc<dyn TaskRunner>,
) -> HashMap<String, String> {
    engine.mark_started(&handle);

    let os = handle.snapshot().plan.os.clone();
    let total = handle.snapshot().plan.tasks.len();
    let mut all_secrets: HashMap<String, String> = HashMap::new();
    let mut cancelled_midway = false;

    for index in 0..total {
        if handle.cancel_requested() {
            cancelled_midway = true;
            for rest in index..total {
                engine.update_task(&handle, rest, EngineTaskStatus::Cancelled);
            }
            break;
        }

        let task = {
            let snap = handle.snapshot();
            snap.plan.tasks[index].clone()
        };

        // --- validating ---
        engine.emit_phase(&handle, index, Phase::Validating);

        // Idempotent no-op tasks terminate truthfully without execution.
        if let TaskAction::NoOp(reason) = &task.action {
            let version = match reason {
                NoopReason::AlreadyInstalled { version } => version.clone(),
                NoopReason::UpdateUnavailable { version } => version.clone(),
                NoopReason::DockerManaged => "docker-managed".to_string(),
            };
            engine.update_task(&handle, index, EngineTaskStatus::Succeeded { version });
            continue;
        }

        // Defense in depth: the selected source must still exist in the
        // catalog (the client cannot inject sources, but the catalog may
        // have changed since planning).
        if let Some(source) = &task.source {
            if let Err(err) = revalidate_source(&definitions, &task.tool_id, source) {
                engine.push_error(&handle, err.clone());
                engine.update_task(&handle, index, EngineTaskStatus::Failed { error: err });
                continue;
            }
        }

        // --- preparing + execution ---
        engine.emit_phase(&handle, index, Phase::Preparing);
        let outcome = runner
            .run(TaskContext {
                job_id: &handle.snapshot().job_id,
                os: &os,
                definitions: &definitions,
                task: &task,
                cancel: Arc::clone(&handle.cancel),
            })
            .await;

        for (k, v) in outcome.secrets {
            all_secrets.insert(k, v);
        }
        if !outcome.path_added.is_empty() {
            engine.record_path_change(
                &handle,
                PathChangeRecord {
                    tool_id: task.tool_id.clone(),
                    added: outcome.path_added.clone(),
                },
            );
        }
        if let EngineTaskStatus::Failed { error } = &outcome.status {
            engine.push_error(&handle, error.clone());
        }
        engine.update_task(&handle, index, outcome.status);
    }

    // --- terminal status -----------------------------------------------
    let statuses: Vec<EngineTaskStatus> = {
        let snap = handle.snapshot();
        snap.plan.tasks.iter().map(|t| t.status.clone()).collect()
    };
    let has_failed = statuses
        .iter()
        .any(|s| matches!(s, EngineTaskStatus::Failed { .. }));
    let has_cancelled = cancelled_midway
        || statuses
            .iter()
            .any(|s| matches!(s, EngineTaskStatus::Cancelled));
    let succeeded_count = statuses
        .iter()
        .filter(|s| matches!(s, EngineTaskStatus::Succeeded { .. }))
        .count();

    let final_status = if has_cancelled {
        JobStatus::Cancelled
    } else if !has_failed {
        JobStatus::Succeeded
    } else if succeeded_count > 0 {
        JobStatus::Partial
    } else {
        JobStatus::Failed
    };

    engine.finish(&handle, final_status);

    // Cleanup on ALL terminal states (installer also cleans per task call).
    tc_core::console::cleanup_tracked_temp_files();

    all_secrets
}

/// Revalidates a planned source against the current catalog.
fn revalidate_source(
    definitions: &[ToolDefinition],
    tool_id: &str,
    source: &super::plan::SelectedSource,
) -> Result<(), String> {
    let Some(def) = definitions.iter().find(|d| d.id == tool_id) else {
        return Err(format!("инструмент {tool_id} отсутствует в каталоге"));
    };
    let os = crate::modules::toolchain::platforms::current_platform().os_name();
    let allowed: &[InstallSource] = match os.as_str() {
        "windows" => def.sources.windows.as_slice(),
        "linux" => def.sources.linux.as_slice(),
        "macos" => def.sources.macos.as_slice(),
        _ => &[],
    };
    let known = allowed.iter().any(|s| s.id == source.id);
    if !known {
        return Err(format!(
            "источник «{}» больше не входит в допустимый набор для {tool_id}",
            source.id
        ));
    }
    Ok(())
}

// ------------------------------------------------------------
// Test support (visible to sibling engine tests only)
// ------------------------------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;

    /// Always-succeeding runner without side effects.
    pub struct SuccessRunner;

    #[async_trait]
    impl TaskRunner for SuccessRunner {
        async fn run(&self, _ctx: TaskContext<'_>) -> TaskOutcome {
            TaskOutcome::plain(EngineTaskStatus::Succeeded {
                version: "test".into(),
            })
        }
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::toolchain::engine::events::MemorySink;
    use crate::modules::toolchain::engine::jobs::JobEngine;
    use crate::modules::toolchain::engine::plan::{
        CanonicalPlan, ExecutionMode, JobStatus, PlanTask,
    };
    use crate::modules::toolchain::engine::request::OperationKind;
    use crate::modules::toolchain::models::PlatformCapabilities;
    use std::sync::Mutex;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tcx-exec-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn def(id: &str) -> ToolDefinition {
        ToolDefinition {
            id: id.to_string(),
            category: "utility".into(),
            display: id.to_string(),
            description: String::new(),
            icon: None,
            detection: Default::default(),
            versions: Default::default(),
            sources: Default::default(),
            size_mb: 10,
            needs_admin: false,
            path_entries: vec![],
            bundled_with: None,
            health_checks: vec![],
            notes: None,
            manual_install: None,
            extended: Default::default(),
        }
    }

    fn task(plan_id: &str, tool_id: &str, action: TaskAction) -> PlanTask {
        PlanTask {
            task_id: format!("{plan_id}:{tool_id}"),
            tool_id: tool_id.to_string(),
            display: tool_id.to_string(),
            icon: None,
            action,
            source: None,
            size_mb: 10,
            needs_admin: false,
            depends_on: vec![],
            path_entries: vec![],
            install_options: vec![],
            execution_mode: ExecutionMode::Host,
            status: EngineTaskStatus::Pending,
        }
    }

    fn plan(operation: OperationKind, tasks: Vec<PlanTask>) -> CanonicalPlan {
        CanonicalPlan {
            plan_id: "tcxp-exectest".into(),
            operation,
            os: "windows".into(),
            created_at: "now".into(),
            fingerprint: "fp".into(),
            tasks,
            total_size_mb: 10,
            free_space_mb: 1000,
            enough_space: true,
            needs_admin_any: false,
            capabilities: PlatformCapabilities::default(),
            warnings: vec![],
        }
    }

    #[derive(Default)]
    struct FakeRunner {
        invocations: Mutex<Vec<String>>,
        fail_tools: Vec<String>,
        secret_for: Option<String>,
    }

    impl FakeRunner {
        fn failing(tools: &[&str]) -> Self {
            Self {
                fail_tools: tools.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            }
        }
    }

    #[async_trait]
    impl TaskRunner for FakeRunner {
        async fn run(&self, ctx: TaskContext<'_>) -> TaskOutcome {
            self.invocations
                .lock()
                .unwrap()
                .push(ctx.task.tool_id.clone());
            if self.fail_tools.contains(&ctx.task.tool_id) {
                return TaskOutcome::plain(EngineTaskStatus::Failed {
                    error: format!("{} сломалось", ctx.task.tool_id),
                });
            }
            let mut secrets = HashMap::new();
            if let Some(tool) = &self.secret_for {
                if &ctx.task.tool_id == tool {
                    secrets.insert(ctx.task.tool_id.clone(), "super-secret-pw".to_string());
                }
            }
            TaskOutcome {
                status: EngineTaskStatus::Succeeded {
                    version: "1.0".into(),
                },
                secrets,
                path_added: vec![],
            }
        }
    }

    #[tokio::test]
    async fn noop_tasks_finish_without_running_executor() {
        let dir = temp_dir("noop");
        let engine = Arc::new(JobEngine::load(&dir));
        let p = plan(
            OperationKind::Install,
            vec![
                task(
                    "tcxp-exectest",
                    "git",
                    TaskAction::NoOp(NoopReason::AlreadyInstalled {
                        version: "2.48".into(),
                    }),
                ),
                task(
                    "tcxp-exectest",
                    "node",
                    TaskAction::InstallNew {
                        target_version: None,
                    },
                ),
            ],
        );
        let handle = engine.register(p).unwrap();
        let runner = Arc::new(FakeRunner::default());
        run_job(
            engine.clone(),
            handle.clone(),
            Arc::new(vec![def("node"), def("git")]),
            runner.clone(),
        )
        .await;

        let snap = handle.snapshot();
        assert_eq!(snap.status, JobStatus::Succeeded);
        assert!(matches!(
            snap.plan.tasks[0].status,
            EngineTaskStatus::Succeeded { ref version } if version == "2.48"
        ));
        assert_eq!(
            *runner.invocations.lock().unwrap(),
            vec!["node"],
            "no-op task must not reach the executor"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn failed_single_task_fails_job_and_records_error() {
        let dir = temp_dir("failed");
        let engine = Arc::new(JobEngine::load(&dir));
        let p = plan(
            OperationKind::Install,
            vec![task(
                "tcxp-exectest",
                "git",
                TaskAction::InstallNew {
                    target_version: None,
                },
            )],
        );
        let handle = engine.register(p).unwrap();
        let runner = Arc::new(FakeRunner::failing(&["git"]));
        run_job(
            engine.clone(),
            handle.clone(),
            Arc::new(vec![def("git")]),
            runner.clone(),
        )
        .await;

        let snap = handle.snapshot();
        assert_eq!(snap.status, JobStatus::Failed);
        assert!(!snap.errors.is_empty());
        assert!(matches!(
            snap.plan.tasks[0].status,
            EngineTaskStatus::Failed { .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn mixed_results_yield_partial() {
        let dir = temp_dir("partial");
        let engine = Arc::new(JobEngine::load(&dir));
        let p = plan(
            OperationKind::Install,
            vec![
                task(
                    "tcxp-exectest",
                    "aaa",
                    TaskAction::InstallNew {
                        target_version: None,
                    },
                ),
                task(
                    "tcxp-exectest",
                    "bbb",
                    TaskAction::InstallNew {
                        target_version: None,
                    },
                ),
            ],
        );
        let handle = engine.register(p).unwrap();
        let runner = Arc::new(FakeRunner::failing(&["bbb"]));
        run_job(
            engine.clone(),
            handle.clone(),
            Arc::new(vec![def("aaa"), def("bbb")]),
            runner,
        )
        .await;

        assert_eq!(handle.snapshot().status, JobStatus::Partial);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn cancel_request_marks_remaining_tasks_cancelled() {
        let dir = temp_dir("cancel");
        let engine = Arc::new(JobEngine::load(&dir));
        let sink = Arc::new(MemorySink::default());
        engine.add_sink(sink.clone());

        let p = plan(
            OperationKind::Install,
            vec![
                task(
                    "tcxp-exectest",
                    "first",
                    TaskAction::InstallNew {
                        target_version: None,
                    },
                ),
                task(
                    "tcxp-exectest",
                    "second",
                    TaskAction::InstallNew {
                        target_version: None,
                    },
                ),
                task(
                    "tcxp-exectest",
                    "third",
                    TaskAction::InstallNew {
                        target_version: None,
                    },
                ),
            ],
        );
        let handle = engine.register(p).unwrap();

        // Cancel while the first task "runs".
        struct CancelOnFirst {
            handle: Arc<JobHandle>,
            done: AtomicBool,
        }
        #[async_trait]
        impl TaskRunner for CancelOnFirst {
            async fn run(&self, ctx: TaskContext<'_>) -> TaskOutcome {
                if ctx.task.tool_id == "first" && !self.done.swap(true, Ordering::SeqCst) {
                    self.handle.cancel.store(true, Ordering::SeqCst);
                }
                TaskOutcome::plain(EngineTaskStatus::Succeeded {
                    version: "1".into(),
                })
            }
        }

        let runner = Arc::new(CancelOnFirst {
            handle: handle.clone(),
            done: AtomicBool::new(false),
        });
        run_job(
            engine.clone(),
            handle.clone(),
            Arc::new(vec![def("first"), def("second"), def("third")]),
            runner,
        )
        .await;

        let snap = handle.snapshot();
        assert_eq!(snap.status, JobStatus::Cancelled);
        assert!(matches!(
            snap.plan.tasks[0].status,
            EngineTaskStatus::Succeeded { .. }
        ));
        assert_eq!(snap.plan.tasks[1].status, EngineTaskStatus::Cancelled);
        assert_eq!(snap.plan.tasks[2].status, EngineTaskStatus::Cancelled);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = sink;
    }

    #[tokio::test]
    async fn secrets_never_enter_job_record() {
        let dir = temp_dir("secrets");
        let engine = Arc::new(JobEngine::load(&dir));
        let p = plan(
            OperationKind::Install,
            vec![task(
                "tcxp-exectest",
                "postgresql",
                TaskAction::InstallNew {
                    target_version: None,
                },
            )],
        );
        let handle = engine.register(p).unwrap();
        let runner = Arc::new(FakeRunner {
            secret_for: Some("postgresql".into()),
            ..Default::default()
        });
        let secrets = run_job(
            engine.clone(),
            handle.clone(),
            Arc::new(vec![def("postgresql")]),
            runner,
        )
        .await;

        assert_eq!(
            secrets.get("postgresql").map(String::as_str),
            Some("super-secret-pw")
        );
        let raw = serde_json::to_string(&handle.snapshot()).unwrap();
        assert!(
            !raw.contains("super-secret-pw"),
            "secret leaked into job state"
        );
        let file_raw = std::fs::read_to_string(
            dir.join("jobs")
                .join(format!("{}.json", handle.snapshot().job_id)),
        )
        .unwrap();
        assert!(
            !file_raw.contains("super-secret-pw"),
            "secret leaked into journal"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn foreign_source_is_caught_by_execution_time_revalidation() {
        let dir = temp_dir("reval");
        let engine = Arc::new(JobEngine::load(&dir));
        let mut t = task(
            "tcxp-exectest",
            "git",
            TaskAction::InstallNew {
                target_version: None,
            },
        );
        t.source = Some(super::super::plan::SelectedSource {
            kind: "official".into(),
            id: "injected-source".into(),
            description: "injected".into(),
            sha256: None,
            needs_admin: false,
        });
        let p = plan(OperationKind::Install, vec![t]);
        let handle = engine.register(p).unwrap();
        let runner = Arc::new(FakeRunner::default());
        run_job(
            engine.clone(),
            handle.clone(),
            Arc::new(vec![def("git")]),
            runner.clone(),
        )
        .await;

        let snap = handle.snapshot();
        assert_eq!(snap.status, JobStatus::Failed);
        assert!(
            snap.errors.iter().any(|e| e.contains("допустимый набор")),
            "revalidation must reject catalog-foreign sources"
        );
        assert!(
            runner.invocations.lock().unwrap().is_empty(),
            "executor must not run for a rejected source"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_states_map_truthfully() {
        assert!(matches!(
            map_legacy_state(
                &TaskState::Success {
                    version: "9".into()
                },
                false
            ),
            EngineTaskStatus::Succeeded { .. }
        ));
        assert!(matches!(
            map_legacy_state(
                &TaskState::Skipped {
                    reason: "отменено".into()
                },
                true
            ),
            EngineTaskStatus::Cancelled
        ));
        assert!(matches!(
            map_legacy_state(
                &TaskState::Skipped {
                    reason: "нет источника".into()
                },
                false
            ),
            EngineTaskStatus::Failed { .. }
        ));
        assert!(matches!(
            map_legacy_phase(TaskPhase::UpdatingPath),
            Phase::UpdatingPath
        ));
    }
}
