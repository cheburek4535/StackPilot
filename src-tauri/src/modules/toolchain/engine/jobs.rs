// ============================================================
// Job engine (engine/jobs.rs)
// ============================================================
//
// Unified registry for all long-running toolchain operations
// (install / update / repair-path / health-check). Replaces the
// ad-hoc single install session slot:
//
//   - every job has a stable id, kind, persisted record and typed
//     terminal state (succeeded/partial/failed/cancelled/interrupted);
//   - the record is written BEFORE execution starts and updated on
//     every task transition (safe progress only — secrets never enter
//     job state);
//   - a restart mid-job yields Interrupted on the next load — never a
//     stuck "running" flag;
//   - events carry full identity (job/task/tool/seq/timestamp) and go
//     out through pluggable sinks;
//   - one active mutating job at a time (legacy single-slot semantics
//     preserved for Project Creator compatibility).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::modules::toolchain::core as tc_core;

use super::events::{JobEvent, JobEventPayload, TcxEventSink};
use super::plan::{CanonicalPlan, EngineTaskStatus, JobStatus, PathChangeRecord};
use super::request::OperationKind;

const JOBS_DIR: &str = "jobs";
const MAX_PERSISTED_JOBS: usize = 20;

/// Persisted job record. This exact shape is what survives restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedJob {
    pub job_id: String,
    pub plan_id: String,
    pub operation: OperationKind,
    /// Tool ids as requested by the caller (for retry/recovery).
    pub requested_tool_ids: Vec<String>,
    /// Explicit source choices (tool -> source id), safe metadata.
    #[serde(default)]
    pub source_choices: HashMap<String, String>,
    pub created_at: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    pub status: JobStatus,
    pub plan: CanonicalPlan,
    #[serde(default)]
    pub errors: Vec<String>,
    /// Audited PATH changes reported in the final result.
    #[serde(default)]
    pub path_changes: Vec<PathChangeRecord>,
    /// True when this record was recovered after an app restart.
    #[serde(default)]
    pub recovered: bool,
}

impl PersistedJob {
    fn new(plan: CanonicalPlan) -> Self {
        let now = tc_core::console::timestamp();
        Self {
            job_id: plan.plan_id.replacen("tcxp-", "tcxj-", 1),
            plan_id: plan.plan_id.clone(),
            operation: plan.operation,
            requested_tool_ids: plan.requested_tool_ids(),
            source_choices: plan
                .tasks
                .iter()
                .filter_map(|t| t.source.as_ref().map(|s| (t.tool_id.clone(), s.id.clone())))
                .collect(),
            created_at: now.clone(),
            started_at: None,
            updated_at: Some(now),
            finished_at: None,
            status: JobStatus::Queued,
            plan,
            errors: Vec::new(),
            path_changes: Vec::new(),
            recovered: false,
        }
    }

    /// Secrets must never appear in job state; the type has no field for
    /// them and this guard keeps it that way for future edits.
    #[allow(dead_code)]
    pub fn assert_secret_free(&self) -> bool {
        !serde_json::to_string(self)
            .map(|raw| raw.contains("\"secrets\""))
            .unwrap_or(true)
    }
}

// ------------------------------------------------------------
// Live handle
// ------------------------------------------------------------

pub struct JobHandle {
    pub record: Mutex<PersistedJob>,
    pub cancel: Arc<AtomicBool>,
    status_tx: watch::Sender<JobStatus>,
    /// Receiver держится живым намеренно: без него поведение
    /// Sender::borrow/send при отсутствии подписчиков не гарантировано,
    /// а status() должен всегда видеть актуальный статус.
    _status_rx: watch::Receiver<JobStatus>,
}

impl JobHandle {
    pub fn status(&self) -> JobStatus {
        *self.status_tx.borrow()
    }

    pub fn subscribe(&self) -> watch::Receiver<JobStatus> {
        self.status_tx.subscribe()
    }

    pub fn snapshot(&self) -> PersistedJob {
        self.record.lock().expect("job record poisoned").clone()
    }

    pub fn cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

// ------------------------------------------------------------
// Engine
// ------------------------------------------------------------

pub struct JobEngine {
    dir: PathBuf,
    jobs: Mutex<HashMap<String, Arc<JobHandle>>>,
    sinks: Mutex<Vec<Arc<dyn TcxEventSink>>>,
    seq_counters: Mutex<HashMap<String, AtomicU64>>,
}

impl JobEngine {
    /// Loads the journal directory and recovers interrupted jobs.
    pub fn load(dir: &Path) -> Self {
        let jobs_dir = dir.join(JOBS_DIR);
        let _ = std::fs::create_dir_all(&jobs_dir);
        let engine = Self {
            dir: jobs_dir,
            jobs: Mutex::new(HashMap::new()),
            sinks: Mutex::new(Vec::new()),
            seq_counters: Mutex::new(HashMap::new()),
        };
        engine.recover_on_startup();
        engine.prune_history();
        engine
    }

    // ---- persistence -------------------------------------------------

    fn job_path(&self, job_id: &str) -> PathBuf {
        self.dir.join(format!("{job_id}.json"))
    }

    fn write_record(&self, record: &PersistedJob) {
        let path = self.job_path(&record.job_id);
        if let Err(e) = std::fs::create_dir_all(&self.dir) {
            log::error!("[toolchainx] каталог заданий не создан: {e}");
            return;
        }
        match serde_json::to_string_pretty(record) {
            Ok(raw) => {
                let tmp = path.with_extension("json.tmp");
                if let Err(e) = std::fs::write(&tmp, &raw) {
                    log::error!("[toolchainx] запись задания не удалась: {e}");
                    return;
                }
                if let Err(e) = std::fs::rename(&tmp, &path) {
                    log::error!("[toolchainx] сохранение задания не удалось: {e}");
                }
            }
            Err(e) => log::error!("[toolchainx] сериализация задания не удалась: {e}"),
        }
    }

    fn read_record(path: &Path) -> Option<PersistedJob> {
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// Startup recovery: Running/Queued records become Interrupted with
    /// unfinished tasks marked Interrupted. Terminal states survive as-is.
    /// Returns the recovered records.
    pub fn recover_on_startup(&self) -> Vec<PersistedJob> {
        let mut recovered = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(_) => return recovered,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(mut record) = Self::read_record(&path) else {
                continue;
            };
            if matches!(record.status, JobStatus::Running | JobStatus::Queued) {
                for task in &mut record.plan.tasks {
                    if !task.status.terminal() {
                        task.status = EngineTaskStatus::Interrupted;
                    }
                }
                record.status = JobStatus::Interrupted;
                record.finished_at = Some(tc_core::console::timestamp());
                record.updated_at = record.finished_at.clone();
                record.recovered = true;
                self.write_record(&record);
                recovered.push(record);
            }
        }
        recovered
    }

    /// Bounded ring: keep at most MAX_PERSISTED_JOBS newest terminal jobs
    /// on disk AND evict their stale live handles.
    fn prune_history(&self) {
        let mut records: Vec<(PathBuf, String, String)> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Some(rec) = Self::read_record(&path) {
                        records.push((path, rec.created_at, rec.job_id));
                    }
                }
            }
        }
        if records.len() <= MAX_PERSISTED_JOBS {
            return;
        }
        records.sort_by(|a, b| b.1.cmp(&a.1));
        let mut evicted: Vec<String> = Vec::new();
        for (path, _, job_id) in records.into_iter().skip(MAX_PERSISTED_JOBS) {
            let _ = std::fs::remove_file(path);
            evicted.push(job_id);
        }
        if let Ok(mut jobs) = self.jobs.lock() {
            for job_id in evicted {
                if let Some(h) = jobs.get(&job_id) {
                    if h.status().terminal() {
                        jobs.remove(&job_id);
                    }
                }
            }
        }
    }

    // ---- sinks ---------------------------------------------------------

    pub fn add_sink(&self, sink: Arc<dyn TcxEventSink>) {
        if let Ok(mut sinks) = self.sinks.lock() {
            sinks.push(sink);
        }
    }

    /// Removes a previously registered sink (identity by Arc pointer).
    pub fn remove_sink(&self, sink: &Arc<dyn TcxEventSink>) {
        if let Ok(mut sinks) = self.sinks.lock() {
            sinks.retain(|s| !Arc::ptr_eq(s, sink));
        }
    }

    // ---- queries ---------------------------------------------------------

    pub fn get(&self, job_id: &str) -> Option<Arc<JobHandle>> {
        self.jobs.lock().ok().and_then(|m| m.get(job_id).cloned())
    }

    /// Live handles (running + recently finished in this process).
    pub fn live_jobs(&self) -> Vec<PersistedJob> {
        self.jobs
            .lock()
            .map(|m| m.values().map(|h| h.snapshot()).collect())
            .unwrap_or_default()
    }

    /// Full history: live handles merged over persisted records.
    pub fn list(&self) -> Vec<PersistedJob> {
        let mut out: Vec<PersistedJob> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Some(rec) = Self::read_record(&path) {
                        out.push(rec);
                    }
                }
            }
        }
        for live in self.live_jobs() {
            if let Some(slot) = out.iter_mut().find(|r| r.job_id == live.job_id) {
                *slot = live;
            } else {
                out.push(live);
            }
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        out
    }

    /// The active (non-terminal) job, if any.
    pub fn active_job(&self) -> Option<Arc<JobHandle>> {
        self.jobs
            .lock()
            .ok()
            .and_then(|m| m.values().find(|h| !h.status().terminal()).cloned())
    }

    // ---- lifecycle -----------------------------------------------------------

    /// Registers a freshly built canonical plan as a queued job.
    /// Persists the record BEFORE returning — execution may begin only
    /// after the plan is durable. Rejects when another job is active.
    pub fn register(&self, plan: CanonicalPlan) -> Result<Arc<JobHandle>, String> {
        if self.active_job().is_some() {
            return Err(
                "Уже идёт другое задание — дождитесь завершения или отмените его".to_string(),
            );
        }
        let record = PersistedJob::new(plan);
        let job_id = record.job_id.clone();
        let (status_tx, status_rx) = watch::channel(record.status);
        let handle = Arc::new(JobHandle {
            record: Mutex::new(record),
            cancel: Arc::new(AtomicBool::new(false)),
            status_tx,
            _status_rx: status_rx,
        });
        {
            let snapshot = handle.snapshot();
            self.write_record(&snapshot);
        }
        self.jobs
            .lock()
            .expect("job map poisoned")
            .insert(job_id.clone(), Arc::clone(&handle));
        self.next_seq(&job_id);
        Ok(handle)
    }

    /// Marks the job Running and emits JobStarted.
    pub fn mark_started(&self, handle: &JobHandle) {
        {
            let mut rec = handle.record.lock().expect("job record poisoned");
            rec.started_at = Some(tc_core::console::timestamp());
            rec.status = JobStatus::Running;
            rec.updated_at = rec.started_at.clone();
        }
        self.set_status(handle, JobStatus::Running);
        self.write_record(&handle.snapshot());
        let (operation, total_tasks) = {
            let rec = handle.record.lock().expect("job record poisoned");
            (rec.operation, rec.plan.tasks.len())
        };
        self.emit(
            handle,
            "",
            "",
            JobEventPayload::JobStarted {
                operation,
                total_tasks,
            },
        );
    }

    /// Updates one task's status inside the record + persists + emits.
    pub fn update_task(&self, handle: &JobHandle, task_index: usize, status: EngineTaskStatus) {
        let ids = {
            let mut rec = handle.record.lock().expect("job record poisoned");
            match rec.plan.tasks.get_mut(task_index) {
                Some(task) => {
                    task.status = status.clone();
                    let ids = (task.task_id.clone(), task.tool_id.clone());
                    rec.updated_at = Some(tc_core::console::timestamp());
                    ids
                }
                None => return,
            }
        };
        let (task_id, tool_id) = ids;
        self.write_record(&handle.snapshot());
        self.emit(
            handle,
            &task_id,
            &tool_id,
            JobEventPayload::TaskCompleted { status },
        );
    }

    /// Records an audited PATH change for the final result.
    pub fn record_path_change(&self, handle: &JobHandle, record: PathChangeRecord) {
        {
            let mut rec = handle.record.lock().expect("job record poisoned");
            rec.path_changes.push(record.clone());
            rec.updated_at = Some(tc_core::console::timestamp());
        }
        self.write_record(&handle.snapshot());
        self.emit(handle, "", "", JobEventPayload::PathUpdated { record });
    }

    pub fn push_error(&self, handle: &JobHandle, error: String) {
        let mut rec = handle.record.lock().expect("job record poisoned");
        rec.errors.push(error);
        rec.updated_at = Some(tc_core::console::timestamp());
        drop(rec);
        self.write_record(&handle.snapshot());
    }

    /// Индекс задачи в плане задания по её task_id (для мостов событий:
    /// фазы реальной задачи обязаны быть подписаны ЕЁ идентичностью,
    /// а не нулевой задачи).
    pub fn task_index(&self, handle: &JobHandle, task_id: &str) -> Option<usize> {
        let rec = handle.record.lock().expect("job record poisoned");
        rec.plan
            .tasks
            .iter()
            .position(|t| t.task_id == task_id)
    }

    /// Emits a progress line event for a running task.
    pub fn emit_progress(&self, handle: &JobHandle, task_index: usize, line: String) {
        let (task_id, tool_id) = {
            let rec = handle.record.lock().expect("job record poisoned");
            match rec.plan.tasks.get(task_index) {
                Some(t) => (t.task_id.clone(), t.tool_id.clone()),
                None => return,
            }
        };
        self.emit(
            handle,
            &task_id,
            &tool_id,
            JobEventPayload::Progress { line },
        );
    }

    /// Emits a phase transition for a running task.
    pub fn emit_phase(&self, handle: &JobHandle, task_index: usize, phase: super::plan::Phase) {
        let (task_id, tool_id) = {
            let rec = handle.record.lock().expect("job record poisoned");
            match rec.plan.tasks.get(task_index) {
                Some(t) => (t.task_id.clone(), t.tool_id.clone()),
                None => return,
            }
        };
        self.emit(
            handle,
            &task_id,
            &tool_id,
            JobEventPayload::TaskPhase { phase },
        );
    }

    /// Finalizes the job with a terminal status.
    pub fn finish(&self, handle: &JobHandle, status: JobStatus) {
        let errors = {
            let mut rec = handle.record.lock().expect("job record poisoned");
            rec.status = status;
            rec.finished_at = Some(tc_core::console::timestamp());
            rec.updated_at = rec.finished_at.clone();
            rec.errors.clone()
        };
        debug_assert!(status.terminal());
        self.set_status(handle, status);
        self.write_record(&handle.snapshot());
        // Кольцо истории ограничивается на каждом терминальном завершении.
        self.prune_history();
        self.emit(
            handle,
            "",
            "",
            JobEventPayload::JobFinished { status, errors },
        );
    }

    fn set_status(&self, handle: &JobHandle, status: JobStatus) {
        let _ = handle.status_tx.send(status);
    }

    fn next_seq(&self, job_id: &str) -> u64 {
        let mut counters = self.seq_counters.lock().expect("seq counters poisoned");
        counters
            .entry(job_id.to_string())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::SeqCst)
            + 1
    }

    pub(crate) fn emit(
        &self,
        handle: &JobHandle,
        task_id: &str,
        tool_id: &str,
        payload: JobEventPayload,
    ) {
        let job_id = handle.snapshot().job_id;
        let seq = self.next_seq(&job_id);
        if let Some(event) = JobEvent::try_new(&job_id, task_id, tool_id, seq, payload) {
            if let Ok(sinks) = self.sinks.lock() {
                for sink in sinks.iter() {
                    sink.emit(&event);
                }
            }
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
    use crate::modules::toolchain::engine::plan::{ExecutionMode, PlanTask, TaskAction};
    use crate::modules::toolchain::models::PlatformCapabilities;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tcx-jobs-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_plan(operation: OperationKind, tool_id: &str) -> CanonicalPlan {
        CanonicalPlan {
            plan_id: format!("tcxp-test{tool_id}"),
            operation,
            os: "windows".into(),
            created_at: tc_core::console::timestamp(),
            fingerprint: "fp".into(),
            tasks: vec![PlanTask {
                task_id: format!("tcxp-test{tool_id}:{tool_id}"),
                tool_id: tool_id.to_string(),
                display: tool_id.to_string(),
                icon: None,
                action: TaskAction::InstallNew {
                    target_version: None,
                },
                source: None,
                size_mb: 1,
                needs_admin: false,
                depends_on: vec![],
                path_entries: vec![],
                install_options: vec![],
                execution_mode: ExecutionMode::Host,
                status: EngineTaskStatus::Pending,
            }],
            total_size_mb: 1,
            free_space_mb: 1000,
            enough_space: true,
            needs_admin_any: false,
            capabilities: PlatformCapabilities::default(),
            warnings: vec![],
        }
    }

    #[test]
    fn register_persists_before_execution() {
        let dir = temp_dir("persist");
        let engine = JobEngine::load(&dir);
        let handle = engine
            .register(sample_plan(OperationKind::Install, "git"))
            .unwrap();

        let file = dir
            .join(JOBS_DIR)
            .join(format!("{}.json", handle.snapshot().job_id));
        assert!(file.exists(), "record must be durable before execution");

        let raw = std::fs::read_to_string(file).unwrap();
        let parsed: PersistedJob = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.status, JobStatus::Queued);
        assert_eq!(parsed.requested_tool_ids, vec!["git"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn second_active_job_is_rejected() {
        let dir = temp_dir("single");
        let engine = JobEngine::load(&dir);
        let h1 = engine
            .register(sample_plan(OperationKind::Install, "git"))
            .unwrap();
        assert_eq!(h1.status(), JobStatus::Queued);
        let err = engine.register(sample_plan(OperationKind::Install, "node"));
        assert!(err.is_err(), "one active job at a time");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn running_job_recovers_as_interrupted_after_restart() {
        let dir = temp_dir("recover");

        {
            let engine = JobEngine::load(&dir);
            let handle = engine
                .register(sample_plan(OperationKind::Install, "node"))
                .unwrap();
            engine.mark_started(&handle);
            engine.update_task(
                &handle,
                0,
                EngineTaskStatus::Running {
                    phase: super::super::plan::Phase::Downloading,
                },
            );
        }

        // Simulate app restart: fresh engine over the same directory.
        let engine2 = JobEngine::load(&dir);
        let recovered = engine2.list();
        let job = recovered.first().expect("job survived restart");
        assert_eq!(job.status, JobStatus::Interrupted);
        assert!(job.recovered);
        assert!(
            job.plan
                .tasks
                .iter()
                .all(|t| t.status == EngineTaskStatus::Interrupted),
            "unfinished tasks are honestly interrupted"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn terminal_states_survive_restart_unchanged() {
        let dir = temp_dir("terminal");
        {
            let engine = JobEngine::load(&dir);
            let handle = engine
                .register(sample_plan(OperationKind::Install, "git"))
                .unwrap();
            engine.update_task(
                &handle,
                0,
                EngineTaskStatus::Succeeded {
                    version: "2.48".into(),
                },
            );
            engine.finish(&handle, JobStatus::Succeeded);
        }
        let engine2 = JobEngine::load(&dir);
        let job = engine2.list().first().unwrap().clone();
        assert_eq!(job.status, JobStatus::Succeeded);
        assert!(!job.recovered);
        assert!(matches!(
            job.plan.tasks[0].status,
            EngineTaskStatus::Succeeded { .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn events_carry_identity_and_monotonic_seq() {
        let dir = temp_dir("events");
        let engine = JobEngine::load(&dir);
        let sink = Arc::new(MemorySink::default());
        engine.add_sink(sink.clone());

        let handle = engine
            .register(sample_plan(OperationKind::Install, "git"))
            .unwrap();
        engine.mark_started(&handle);
        engine.emit_phase(&handle, 0, super::super::plan::Phase::Preparing);
        engine.emit_progress(&handle, 0, "line".into());
        engine.update_task(
            &handle,
            0,
            EngineTaskStatus::Succeeded {
                version: "1".into(),
            },
        );
        engine.finish(&handle, JobStatus::Succeeded);

        let events = sink.snapshot();
        assert!(events.len() >= 5);
        let seqs = sink.seqs_of(&handle.snapshot().job_id);
        let mut sorted = seqs.clone();
        sorted.sort();
        assert_eq!(seqs, sorted, "per-job sequence numbers are monotonic");
        assert!(seqs.windows(2).all(|w| w[1] > w[0]));
        for ev in &events {
            assert_eq!(ev.job_id, handle.snapshot().job_id);
            assert!(!ev.timestamp.is_empty());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_events_from_other_job_are_filterable() {
        let dir = temp_dir("stale");
        let engine = JobEngine::load(&dir);
        let sink = Arc::new(MemorySink::default());
        engine.add_sink(sink.clone());

        let h1 = engine
            .register(sample_plan(OperationKind::Install, "git"))
            .unwrap();
        engine.finish(&h1, JobStatus::Cancelled);

        // second job after first finished
        let h2 = engine
            .register(sample_plan(OperationKind::HealthCheck, "node"))
            .unwrap();
        engine.mark_started(&h2);
        engine.finish(&h2, JobStatus::Succeeded);

        let current_job = h2.snapshot().job_id;
        let all_events = sink.snapshot();
        let foreign: Vec<&JobEvent> = all_events
            .iter()
            .filter(|e| !e.belongs_to(&current_job))
            .collect();
        assert!(
            !foreign.is_empty(),
            "first job's events exist but are droppable"
        );
        assert!(all_events
            .iter()
            .all(|e| e.belongs_to(&current_job) || foreign.contains(&e)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn path_changes_are_audited_in_record() {
        let dir = temp_dir("pathaudit");
        let engine = JobEngine::load(&dir);
        let handle = engine
            .register(sample_plan(OperationKind::RepairPath, "postgres"))
            .unwrap();
        engine.mark_started(&handle);
        engine.record_path_change(
            &handle,
            PathChangeRecord {
                tool_id: "postgres".into(),
                added: vec!["C:\\Program Files\\PostgreSQL\\17\\bin".into()],
            },
        );
        let snap = handle.snapshot();
        assert_eq!(snap.path_changes.len(), 1);
        assert_eq!(snap.path_changes[0].added.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn job_state_never_contains_secrets_section() {
        let dir = temp_dir("nosecrets");
        let engine = JobEngine::load(&dir);
        let handle = engine
            .register(sample_plan(OperationKind::Install, "postgresql"))
            .unwrap();
        engine.mark_started(&handle);
        let raw = serde_json::to_string(&handle.snapshot()).unwrap();
        assert!(
            !raw.contains("password"),
            "no secret-looking fields in job state"
        );
        assert!(!raw.contains("\"secrets\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_is_bounded_ring() {
        let dir = temp_dir("ring");
        let engine = JobEngine::load(&dir);
        for i in 0..(MAX_PERSISTED_JOBS + 5) {
            let handle = engine
                .register(sample_plan(OperationKind::HealthCheck, &format!("t{i}")))
                .unwrap();
            engine.finish(&handle, JobStatus::Succeeded);
        }
        let listed = engine.list();
        assert!(
            listed.len() <= MAX_PERSISTED_JOBS,
            "history bounded to {MAX_PERSISTED_JOBS}, got {}",
            listed.len()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_flag_is_visible_on_handle() {
        let dir = temp_dir("cancel");
        let engine = JobEngine::load(&dir);
        let handle = engine
            .register(sample_plan(OperationKind::Install, "git"))
            .unwrap();
        assert!(!handle.cancel_requested());
        handle.cancel.store(true, Ordering::SeqCst);
        assert!(handle.cancel_requested());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
