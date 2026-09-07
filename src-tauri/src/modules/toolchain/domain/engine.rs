// ============================================================
// Движок диагностического скана (domain/engine.rs)
// ============================================================
// Жизненный цикл задания скана (контракт §6.1 Scan / §4.6 Job):
//
//   start_scan → Queued → Environment → Tools → Finalizing → Done
//        │                                              │
//        └─ reconnect: get_job(job_id) в любой момент ──┘
//
// Гарантии:
//   - СКАН ТОЛЬКО ЧИТАЕТ машину (пробы/пути/реестр/health-объявления);
//   - ограниченный параллелизм (Semaphore), а не процесс на каждый тул;
//   - детерминированный порядок результатов = порядок каталога,
//     независимо от реального порядка завершения проб;
//   - отмена: флаг проверяется каждой задачей после получения слота;
//     незапущенные инструменты остаются ScanPending (честное «не проверено»);
//   - события только с идентичностью операции (job_id + scan_id);
//   - перезапуск посреди скана → Interrupted из журнала, не «вечный running»;
//   - последний валидный снапшот живёт в кэше и честно помечается stale.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::modules::toolchain::core::console;
use crate::modules::toolchain::core::crypto;
use crate::modules::toolchain::models::ToolDefinition;

use super::cache::{stamp_finished, SnapshotCache};
use super::detect::{self, ScanContext};
use super::models::{
    AdminCapability, DiskSpaceInfo, EnvironmentSnapshot, PathReport, ScanJobSnapshot, ScanPhase,
    ScanProgressEvent, ScanTerminal, ScoreSummary, SnapshotIssue, StatusCounts, ToolScanResult,
    ToolState,
};
use super::path_report;
use super::score;

/// Параметры прогона скана.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Максимум ОДНОВРЕМЕННЫХ опросов инструментов (ограниченный
    /// параллелизм; никакого «процесса на каждый тул»).
    pub max_parallel_probes: usize,
    /// Мягкий дедлайн всего скана: по истечении новые инструменты не
    /// стартуют, задание закрывается как Partial.
    pub overall_deadline: Duration,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            max_parallel_probes: 4,
            overall_deadline: Duration::from_secs(90),
        }
    }
}

/// Типизированные события скана. Прогресс — по каждому закрытому
/// инструменту; Done — терминальная сводка. Ни одно событие не существует
/// без идентичности операции.
#[derive(Debug, Clone)]
pub enum ScanEvent {
    Progress(ScanProgressEvent),
    Done {
        job_id: String,
        scan_id: String,
        terminal: ScanTerminal,
        completed: usize,
        total: usize,
    },
}

/// Callback событий (команды оборачивают в app.emit, тесты — в буфер).
pub type ScanEventFn = Arc<dyn Fn(ScanEvent) + Send + Sync>;

const SCAN_JOURNAL_FILE: &str = "scan-journal.json";

/// Результат запуска скана: новый или уже идущий (reconnect).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ScanStartOutcome {
    Started(ScanJobSnapshot),
    AlreadyRunning(ScanJobSnapshot),
}

struct ActiveScan {
    snapshot: Arc<Mutex<ScanJobSnapshot>>,
    cancel: Arc<AtomicBool>,
}

struct EngineInner {
    active: Option<ActiveScan>,
    last_job: Option<ScanJobSnapshot>,
}

/// Движок сканов. Один экземпляр на приложение (в ToolchainState).
pub struct ScanEngine {
    dir: PathBuf,
    /// Кэш последнего снапшота (память + диск), обновляется по завершении.
    cache: Arc<SnapshotCache>,
    inner: Mutex<EngineInner>,
}

impl ScanEngine {
    pub fn new(dir: &Path) -> Self {
        Self::with_cache(Arc::new(SnapshotCache::new(dir)), dir)
    }

    pub fn with_cache(cache: Arc<SnapshotCache>, dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            cache,
            inner: Mutex::new(EngineInner {
                active: None,
                last_job: None,
            }),
        }
    }

    /// Кэш снапшотов (для чтения командами snapshot_get).
    pub fn cache(&self) -> &Arc<SnapshotCache> {
        &self.cache
    }

    /// id активных заданий скана (для поля active_jobs снапшота).
    /// Установки/обновления ведутся движком engine::jobs и в этот список
    /// добавляются командным слоем при сборке ответа.
    pub fn active_job_ids(&self) -> Vec<String> {
        let guard = self.inner.lock().expect("scan engine poisoned");
        match &guard.active {
            Some(scan) => {
                let snapshot = scan.snapshot.lock().expect("job snapshot poisoned");
                vec![snapshot.job_id.clone()]
            }
            None => Vec::new(),
        }
    }

    // ------------------------------------------------------------
    // Журнал сканов (перезапуск посреди скана)
    // ------------------------------------------------------------

    fn journal_path(&self) -> PathBuf {
        self.dir.join(SCAN_JOURNAL_FILE)
    }

    fn write_journal(&self, snapshot: &ScanJobSnapshot) {
        let path = self.journal_path();
        let Some(parent) = path.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let Ok(raw) = serde_json::to_string_pretty(snapshot) else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, raw).is_ok() {
            // Журнал — best-effort: его отсутствие после сбоя записи
            // безопасно (recovery просто не найдёт «зависший» скан),
            // но причина сбоя обязана быть видна в логе.
            if let Err(e) = std::fs::rename(&tmp, &path) {
                eprintln!("[toolchainx] журнал скана не сохранился: {e}");
            }
        }
    }

    fn read_journal(&self) -> Option<ScanJobSnapshot> {
        let raw = std::fs::read_to_string(self.journal_path()).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// Восстановление после перезапуска: Running в журнале переводится
    /// в Interrupted («вечных» сканов нет). Возвращает восстановленное
    /// задание, если оно было.
    pub fn recover_on_startup(&self) -> Option<ScanJobSnapshot> {
        let mut job = self.read_journal()?;
        if job.terminal != ScanTerminal::Running {
            self.inner.lock().expect("scan engine poisoned").last_job = Some(job.clone());
            return Some(job);
        }
        job.running = false;
        job.cancel_requested = false;
        job.current_tool = None;
        job.phase = ScanPhase::Done;
        job.finished_at = Some(console::timestamp());
        job.updated_at = console::timestamp();
        job.terminal = ScanTerminal::Interrupted;
        job.recovered = true;
        self.write_journal(&job);
        self.inner.lock().expect("scan engine poisoned").last_job = Some(job.clone());
        eprintln!(
            "[toolchain] скан {} прерван перезапуском приложения",
            job.job_id
        );
        Some(job)
    }

    // ------------------------------------------------------------
    // Запуск / reconnect / статус / отмена
    // ------------------------------------------------------------

    /// Запускает новый скан. Если скан уже идёт — возвращает его задание
    /// (reconnect): второй параллельный скан не создаётся.
    pub async fn start_scan(
        self: &Arc<Self>,
        definitions: Vec<ToolDefinition>,
        managed_tools: std::collections::HashSet<String>,
        options: ScanOptions,
        on_event: Option<ScanEventFn>,
    ) -> Result<ScanStartOutcome, String> {
        {
            let inner = self.inner.lock().expect("scan engine poisoned");
            if let Some(active) = &inner.active {
                let snap = active
                    .snapshot
                    .lock()
                    .expect("job snapshot poisoned")
                    .clone();
                return Ok(ScanStartOutcome::AlreadyRunning(snap));
            }
        }

        let job_id = format!("tcx-job-{}", random_id()?);
        let scan_id = format!("tcx-scan-{}", random_id()?);
        let now = console::timestamp();

        let initial = ScanJobSnapshot {
            job_id: job_id.clone(),
            scan_id: scan_id.clone(),
            started_at: now.clone(),
            updated_at: now,
            finished_at: None,
            phase: ScanPhase::Queued,
            total_tools: definitions.len(),
            completed_tools: 0,
            current_tool: None,
            running: true,
            cancel_requested: false,
            terminal: ScanTerminal::Running,
            recovered: false,
        };
        self.write_journal(&initial);

        let snapshot_arc = Arc::new(Mutex::new(initial.clone()));
        let cancel = Arc::new(AtomicBool::new(false));

        {
            let mut inner = self.inner.lock().expect("scan engine poisoned");
            inner.active = Some(ActiveScan {
                snapshot: Arc::clone(&snapshot_arc),
                cancel: Arc::clone(&cancel),
            });
        }

        let engine = Arc::clone(self);
        let handle_snapshot = Arc::clone(&snapshot_arc);
        let handle_cancel = Arc::clone(&cancel);

        tokio::spawn(async move {
            let finished = engine
                .run_scan_job(
                    &job_id,
                    &scan_id,
                    definitions,
                    managed_tools,
                    options,
                    handle_snapshot.clone(),
                    handle_cancel.clone(),
                    on_event,
                )
                .await;

            // Терминальное состояние: журнал + освобождение активного слота.
            {
                let mut guard = handle_snapshot.lock().expect("job snapshot poisoned");
                guard.running = false;
                guard.finished_at = Some(console::timestamp());
                guard.updated_at = guard.finished_at.clone().unwrap_or_default();
                guard.phase = ScanPhase::Done;
                guard.terminal = finished;
                guard.current_tool = None;
            }
            let final_snapshot = handle_snapshot
                .lock()
                .expect("job snapshot poisoned")
                .clone();
            engine.write_journal(&final_snapshot);
            engine.inner.lock().expect("scan engine poisoned").last_job = Some(final_snapshot);
            engine.inner.lock().expect("scan engine poisoned").active = None;
        });

        Ok(ScanStartOutcome::Started(initial))
    }

    /// Задание по id: активное → последнее → журнал (включая Interrupted).
    pub fn get_job(&self, job_id: &str) -> Option<ScanJobSnapshot> {
        let inner = self.inner.lock().expect("scan engine poisoned");
        if let Some(active) = &inner.active {
            let snap = active
                .snapshot
                .lock()
                .expect("job snapshot poisoned")
                .clone();
            if snap.job_id == job_id {
                return Some(snap);
            }
        }
        if let Some(last) = &inner.last_job {
            if last.job_id == job_id {
                return Some(last.clone());
            }
        }
        drop(inner);
        self.read_journal().filter(|j| j.job_id == job_id)
    }

    /// Актуальное задание (идущее или последнее завершённое).
    pub fn latest_job(&self) -> Option<ScanJobSnapshot> {
        let inner = self.inner.lock().expect("scan engine poisoned");
        if let Some(active) = &inner.active {
            return Some(
                active
                    .snapshot
                    .lock()
                    .expect("job snapshot poisoned")
                    .clone(),
            );
        }
        inner.last_job.clone()
    }

    /// Отмена: ставит флаг; задачи добивают текущие пробы и выходят.
    pub fn cancel_scan(&self, job_id: &str) -> bool {
        let inner = self.inner.lock().expect("scan engine poisoned");
        let Some(active) = &inner.active else {
            return false;
        };
        let mut snap = active.snapshot.lock().expect("job snapshot poisoned");
        if snap.job_id != job_id || !snap.running {
            return false;
        }
        snap.cancel_requested = true;
        drop(snap);
        active.cancel.store(true, Ordering::SeqCst);
        true
    }

    // ------------------------------------------------------------
    // Исполнение задания
    // ------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    async fn run_scan_job(
        &self,
        job_id: &str,
        scan_id: &str,
        definitions: Vec<ToolDefinition>,
        managed_tools: std::collections::HashSet<String>,
        options: ScanOptions,
        job_snapshot: Arc<Mutex<ScanJobSnapshot>>,
        cancel: Arc<AtomicBool>,
        on_event: Option<ScanEventFn>,
    ) -> ScanTerminal {
        let total = definitions.len();
        let started_at = console::timestamp();
        // Владельческие копии для задач ('static): ссылки не переживают spawn.
        let job_id = job_id.to_string();
        let scan_id = scan_id.to_string();

        // --- Фаза Environment: окружение и PATH (только чтение) ---
        self.set_phase(&job_snapshot, ScanPhase::Environment);
        let platform = crate::modules::toolchain::platforms::current_platform();
        let os_name = platform.os_name();
        let os_version = platform.os_version().await;
        let package_managers = platform.package_managers();

        let process_entries = crate::modules::toolchain::core::path_service::process_path_entries();
        // Постоянный PATH (пользователь + система) хранится ОТДЕЛЬНО:
        // смешивать его с PATH процесса нельзя — «есть в реестре, но не в
        // процессе» и «работает прямо сейчас» это разные честные классы.
        let mut persisted_entries = Vec::new();
        if let Ok(user_entries) = platform.read_user_path().await {
            persisted_entries.extend(user_entries);
        }
        if let Ok(system_entries) = platform.read_system_path().await {
            persisted_entries.extend(system_entries);
        }
        let path_report: PathReport = path_report::build_report(&process_entries);

        // STANDALONE-скан не знает «двойных» docker-инструментов мастера:
        // PostgreSQL/Redis/MongoDB/Kafka/Grafana/MySQL сканируются как
        // обычные локальные инструменты (Docker — рекомендация в
        // метаданных каталога, не классификация). dual_tools пуст —
        // DockerDefault/DockerManaged в standalone-снапшотах не возникает.
        let ctx = Arc::new(ScanContext {
            process_entries,
            persisted_entries,
            managed_tools,
            dual_tools: Default::default(),
            os_name: os_name.clone(),
        });

        // --- Фаза Tools: ограниченный параллелизм, детерминированный порядок ---
        self.set_phase(&job_snapshot, ScanPhase::Tools);
        let slots: Arc<Mutex<Vec<Option<ToolScanResult>>>> =
            Arc::new(Mutex::new((0..total).map(|_| None).collect()));
        let completed_counter = Arc::new(AtomicUsize::new(0));
        let semaphore = Arc::new(Semaphore::new(options.max_parallel_probes.max(1)));

        let mut set: JoinSet<()> = JoinSet::new();
        for (index, def) in definitions.iter().enumerate() {
            {
                let mut guard = job_snapshot.lock().expect("job snapshot poisoned");
                guard.current_tool = Some(def.id.clone());
                guard.updated_at = console::timestamp();
            }
            let permit = match semaphore.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => break, // семафор закрыт — скан останавливается
            };
            if cancel.load(Ordering::SeqCst) {
                drop(permit);
                break; // незапущенные остаются ScanPending
            }
            let def = def.clone();
            let ctx = Arc::clone(&ctx);
            let slots = Arc::clone(&slots);
            let counter = Arc::clone(&completed_counter);
            let job_snapshot = Arc::clone(&job_snapshot);
            let cancel = Arc::clone(&cancel);
            let job_id = job_id.clone();
            let scan_id = scan_id.clone();
            let event_cb = on_event.clone();
            set.spawn(async move {
                // Отмена проверяется ПОСЛЕ получения слота: уже начатые
                // инструменты добираются, новые не стартуют.
                if cancel.load(Ordering::SeqCst) {
                    return;
                }
                let result = detect::scan_tool(&def, &ctx).await;
                {
                    slots.lock().expect("slots poisoned")[index] = Some(result.clone());
                }
                let done = counter.fetch_add(1, Ordering::SeqCst) + 1;
                {
                    let mut guard = job_snapshot.lock().expect("job snapshot poisoned");
                    guard.completed_tools = done;
                    guard.updated_at = console::timestamp();
                }
                drop(permit);
                emit_progress(event_cb.as_ref(), &job_id, &scan_id, done, total, &result);
            });
        }

        // Сбор с дедлайном: зависшие/медленные пробы не замораживают задание.
        let collect = async { while set.join_next().await.is_some() {} };
        let deadline_hit = tokio::time::timeout(options.overall_deadline, collect)
            .await
            .is_err();
        if deadline_hit {
            cancel.store(true, Ordering::SeqCst);
            set.abort_all();
            eprintln!(
                "[toolchain] скан {job_id} превысил дедлайн {}с — частичный отчёт",
                options.overall_deadline.as_secs()
            );
        }

        let cancelled = cancel.load(Ordering::SeqCst);
        let results: Vec<ToolScanResult> = {
            let mut guard = slots.lock().expect("slots poisoned");
            guard
                .iter_mut()
                .enumerate()
                .map(|(index, slot)| {
                    slot.take().unwrap_or_else(|| {
                        // Частичный отчёт: честные размерности из каталога
                        // (применимость/возможности), вердикт — ScanPending.
                        ToolScanResult::pending(
                            &definitions[index],
                            &os_name,
                            false,
                            crate::modules::toolchain::core::installer::install_execution_supported(
                            ),
                        )
                    })
                })
                .collect()
        };

        // --- Фаза Finalizing: скоринг и сборка снапшота ---
        self.set_phase(&job_snapshot, ScanPhase::Finalizing);
        let complete = results.iter().all(|r| r.state != ToolState::ScanPending);
        let score_summary: ScoreSummary = score::compute_score(&results);
        let summary = StatusCounts::from_results(&results);

        // Диск и админ-возможности — только чтение; ошибка пробы диска
        // становится предупреждением, а не провалом скана.
        let mut warnings: Vec<SnapshotIssue> = Vec::new();
        let mut errors: Vec<SnapshotIssue> = Vec::new();
        if deadline_hit {
            errors.push(SnapshotIssue {
                code: "partial_scan".to_string(),
                message: format!(
                    "Скан превысил дедлайн {}с: часть инструментов осталась не проверенной",
                    options.overall_deadline.as_secs()
                ),
            });
        }
        if cancelled && !complete {
            warnings.push(SnapshotIssue {
                code: "scan_cancelled".to_string(),
                message: "Скан отменён пользователем: отчёт частичный".to_string(),
            });
        }

        let install_root = crate::modules::toolchain::core::disk::install_root();
        let disk = match crate::modules::toolchain::core::disk::free_space_mb(&install_root).await {
            Ok(free_mb) => vec![DiskSpaceInfo {
                root: install_root.to_string_lossy().into_owned(),
                free_mb,
            }],
            Err(e) => {
                warnings.push(SnapshotIssue {
                    code: "disk_probe_failed".to_string(),
                    message: format!("Не удалось определить свободное место: {e}"),
                });
                Vec::new()
            }
        };

        let admin = AdminCapability {
            elevation_supported: std::env::consts::OS == "windows",
            required_by_tools: definitions.iter().any(|d| d.needs_admin),
        };

        let mut snapshot = EnvironmentSnapshot {
            snapshot_id: format!("tcx-snap-{}", random_id().unwrap_or_default()),
            job_id: job_id.to_string(),
            scan_id: scan_id.to_string(),
            os: os_name,
            os_version,
            arch: std::env::consts::ARCH.to_string(),
            package_managers,
            disk,
            admin,
            started_at,
            finished_at: String::new(),
            complete,
            cancelled,
            tools: results,
            path_report,
            score: score_summary,
            summary,
            warnings,
            errors,
            active_jobs: self.active_job_ids(),
            age_seconds: 0,
            stale: false,
            from_cache: false,
        };
        stamp_finished(&mut snapshot);

        // Кэш обновляется даже для частичного отчёта: это последнее ЧЕСТНОЕ
        // состояние машины на момент скана (stale-пометка решает остальное).
        let _ = self.cache.store(&snapshot);

        let terminal = if cancelled && !complete {
            ScanTerminal::Cancelled
        } else if complete {
            ScanTerminal::Completed
        } else {
            ScanTerminal::Partial
        };

        if let Some(cb) = on_event.as_ref() {
            cb(ScanEvent::Done {
                job_id: job_id.to_string(),
                scan_id: scan_id.to_string(),
                terminal,
                completed: completed_counter.load(Ordering::SeqCst),
                total,
            });
        }

        terminal
    }

    fn set_phase(&self, job_snapshot: &Arc<Mutex<ScanJobSnapshot>>, phase: ScanPhase) {
        let mut guard = job_snapshot.lock().expect("job snapshot poisoned");
        guard.phase = phase;
        guard.updated_at = console::timestamp();
    }
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::toolchain::models::{
        DetectionRules, InstallSource, InstallSourceKind, InstallSources,
    };
    use std::sync::Mutex as StdMutex;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tcx-engine-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Быстрый инструмент: проба cmd /c echo отвечает мгновенно.
    fn echo_def(id: &str) -> ToolDefinition {
        ToolDefinition {
            id: id.to_string(),
            category: "utility".to_string(),
            display: format!("Echo {id}"),
            description: String::new(),
            icon: None,
            detection: DetectionRules {
                version_probes: vec![vec![
                    "cmd".to_string(),
                    "/c".to_string(),
                    "echo".to_string(),
                    "1.0.0".to_string(),
                ]],
                known_paths: vec![],
                registry_keys: vec![],
                ..Default::default()
            },
            versions: Default::default(),
            sources: InstallSources {
                windows: vec![InstallSource {
                    kind: InstallSourceKind::PkgManager,
                    id: "Fake.Id".to_string(),
                    url: None,
                    args: vec![],
                    extra_args: vec![],
                    dynamic_args: false,
                    install_dir: None,
                    needs_admin: None,
                    file_name: None,
                    execution: None,
                    bootstrap: None,
                    sha256: None,
                }],
                linux: vec![],
                macos: vec![],
            },
            size_mb: 1,
            needs_admin: false,
            path_entries: vec![],
            bundled_with: None,
            health_checks: vec![],
            notes: None,
            manual_install: None,
            extended: Default::default(),
        }
    }

    /// Медленный инструмент: ping ~2с (укладывается в пробный таймаут 10с).
    fn slow_def(id: &str) -> ToolDefinition {
        let mut def = echo_def(id);
        def.detection.version_probes = vec![vec![
            "ping".to_string(),
            #[cfg(target_os = "windows")]
            "-n".to_string(),
            #[cfg(target_os = "windows")]
            "3".to_string(),
            #[cfg(not(target_os = "windows"))]
            "-c".to_string(),
            #[cfg(not(target_os = "windows"))]
            "3".to_string(),
            "127.0.0.1".to_string(),
        ]];
        def
    }

    type EventLog = Arc<StdMutex<Vec<ScanEvent>>>;

    fn event_collector() -> (ScanEventFn, EventLog) {
        let log: EventLog = Arc::new(StdMutex::new(Vec::new()));
        let sink_log = Arc::clone(&log);
        let sink: ScanEventFn = Arc::new(move |event| {
            sink_log.lock().unwrap().push(event);
        });
        (sink, log)
    }

    /// Ждёт терминальное состояние задания (поллинг get_job).
    async fn wait_terminal(engine: &Arc<ScanEngine>, job_id: &str) -> ScanJobSnapshot {
        for _ in 0..600 {
            if let Some(job) = engine.get_job(job_id) {
                if !job.running {
                    return job;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("задание {job_id} не завершилось за отведённое время");
    }

    #[tokio::test]
    async fn scan_completes_in_catalog_order_with_identified_events() {
        let dir = temp_dir("order");
        let engine = Arc::new(ScanEngine::new(&dir));
        // Порядок каталога намеренно НЕ алфавитный и не по времени:
        // b-tool раньше a-tool, хотя завершиться могут в любом порядке.
        let defs = vec![echo_def("b-tool"), echo_def("a-tool")];
        let (sink, log) = event_collector();

        let outcome = engine
            .start_scan(
                defs,
                Default::default(),
                ScanOptions {
                    max_parallel_probes: 2,
                    overall_deadline: Duration::from_secs(30),
                },
                Some(sink),
            )
            .await
            .unwrap();
        let started_job = match outcome {
            ScanStartOutcome::Started(job) => job,
            other => panic!("ожидали Started, получили {other:?}"),
        };

        let finished = wait_terminal(&engine, &started_job.job_id).await;
        assert_eq!(finished.terminal, ScanTerminal::Completed);
        assert_eq!(finished.completed_tools, 2);

        // Детерминированный порядок снапшота = порядок каталога.
        let snapshot = engine.cache().get().expect("снапшот обязан быть в кэше");
        assert_eq!(snapshot.job_id, started_job.job_id);
        assert_eq!(snapshot.scan_id, started_job.scan_id);
        assert_eq!(snapshot.tools[0].tool_id, "b-tool");
        assert_eq!(snapshot.tools[1].tool_id, "a-tool");
        assert!(snapshot.complete);
        assert!(!snapshot.stale);

        // Все события несут идентичность ЭТОГО задания.
        let events = log.lock().unwrap();
        let mut progress_done = Vec::new();
        for event in events.iter() {
            match event {
                ScanEvent::Progress(p) => {
                    assert_eq!(p.job_id, started_job.job_id, "чужой job_id в событии");
                    assert_eq!(p.scan_id, started_job.scan_id, "чужой scan_id в событии");
                    progress_done.push(p.completed_count);
                }
                ScanEvent::Done {
                    job_id,
                    terminal,
                    completed,
                    ..
                } => {
                    assert_eq!(job_id, &started_job.job_id);
                    assert_eq!(terminal, &ScanTerminal::Completed);
                    assert_eq!(completed, &2);
                }
            }
        }
        progress_done.sort();
        assert_eq!(progress_done, vec![1, 2], "прогресс: по событию на тул");
    }

    #[tokio::test]
    async fn cancel_leaves_unstarted_tools_pending_and_terminal_cancelled() {
        let dir = temp_dir("cancel");
        let engine = Arc::new(ScanEngine::new(&dir));
        // Первый — медленный (~2с), остальные быстрые; параллелизм 1:
        // после отмены остальные не стартуют и остаются ScanPending.
        let defs = vec![slow_def("slow-1"), echo_def("fast-2"), echo_def("fast-3")];
        let (sink, _log) = event_collector();

        let outcome = engine
            .start_scan(
                defs,
                Default::default(),
                ScanOptions {
                    max_parallel_probes: 1,
                    overall_deadline: Duration::from_secs(60),
                },
                Some(sink),
            )
            .await
            .unwrap();
        let job = match outcome {
            ScanStartOutcome::Started(job) => job,
            other => panic!("ожидали Started, получили {other:?}"),
        };

        // Даём первому инструменту начаться, затем отменяем.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(engine.cancel_scan(&job.job_id), "отмена активного скана");

        let finished = wait_terminal(&engine, &job.job_id).await;
        assert!(
            matches!(
                finished.terminal,
                ScanTerminal::Cancelled | ScanTerminal::Partial
            ),
            "терминал после отмены: {:?}",
            finished.terminal
        );
        assert!(finished.cancel_requested);
        assert!(finished.completed_tools < 3, "не всё успело провериться");

        let snapshot = engine.cache().get().unwrap();
        assert!(snapshot.cancelled);
        assert!(!snapshot.complete);
        // Незапущенные инструменты честно помечены «не проверено».
        let pending: Vec<_> = snapshot
            .tools
            .iter()
            .filter(|t| t.state == ToolState::ScanPending)
            .map(|t| t.tool_id.clone())
            .collect();
        assert!(!pending.is_empty(), "незапущенные должны остаться pending");

        // Повторная отмена уже не нужна (задание завершено).
        assert!(!engine.cancel_scan(&job.job_id));
    }

    #[tokio::test]
    async fn second_start_reconnects_to_running_scan() {
        let dir = temp_dir("reconnect");
        let engine = Arc::new(ScanEngine::new(&dir));
        let defs = vec![slow_def("slow-re"), echo_def("fast-re")];

        let first = engine
            .start_scan(
                defs.clone(),
                Default::default(),
                ScanOptions {
                    max_parallel_probes: 1,
                    overall_deadline: Duration::from_secs(60),
                },
                None,
            )
            .await
            .unwrap();

        let second = engine
            .start_scan(
                defs,
                Default::default(),
                ScanOptions {
                    max_parallel_probes: 1,
                    overall_deadline: Duration::from_secs(60),
                },
                None,
            )
            .await
            .unwrap();

        let first_job = match first {
            ScanStartOutcome::Started(job) => job,
            other => panic!("первый запуск обязан стартовать: {other:?}"),
        };
        // Второй вызов подключается к ИДУЩЕМУ заданию, а не создаёт новое.
        match second {
            ScanStartOutcome::AlreadyRunning(job) => {
                assert_eq!(job.job_id, first_job.job_id);
            }
            other => panic!("ожидали AlreadyRunning (reconnect), получили {other:?}"),
        }

        engine.cancel_scan(&first_job.job_id);
        wait_terminal(&engine, &first_job.job_id).await;
    }

    #[tokio::test]
    async fn interrupted_journal_recovers_as_interrupted() {
        let dir = temp_dir("recover");
        // Готовим журнал «скан шёл и приложение умерло».
        let running = ScanJobSnapshot {
            job_id: "tcx-job-dead".to_string(),
            scan_id: "tcx-scan-dead".to_string(),
            started_at: console::timestamp(),
            updated_at: console::timestamp(),
            finished_at: None,
            phase: ScanPhase::Tools,
            total_tools: 5,
            completed_tools: 2,
            current_tool: Some("node".to_string()),
            running: true,
            cancel_requested: false,
            terminal: ScanTerminal::Running,
            recovered: false,
        };
        std::fs::write(
            dir.join(SCAN_JOURNAL_FILE),
            serde_json::to_string_pretty(&running).unwrap(),
        )
        .unwrap();

        // «Новый запуск приложения»: движок обязан перевести в Interrupted.
        let engine = ScanEngine::new(&dir);
        let recovered = engine.recover_on_startup().expect("журнал был");
        assert_eq!(recovered.job_id, "tcx-job-dead");
        assert_eq!(recovered.terminal, ScanTerminal::Interrupted);
        assert!(recovered.recovered);
        assert!(!recovered.running);
        assert!(recovered.finished_at.is_some());

        // Задание доступно по id (get_scan_job из команды).
        let by_id = engine.get_job("tcx-job-dead").unwrap();
        assert_eq!(by_id.terminal, ScanTerminal::Interrupted);

        // Повторное восстановление ничего не меняет (уже не Running).
        let again = engine.recover_on_startup().unwrap();
        assert_eq!(again.terminal, ScanTerminal::Interrupted);
    }

    /// Регрессионный страж правила «скан не мутирует машину»: скан
    /// отсутствующего инструмента не создаёт state.json/secrets.bin и
    /// не оставляет ничего, кроме собственных журнала/снапшота.
    #[tokio::test]
    async fn scan_never_writes_machine_state_files() {
        let dir = temp_dir("nomut");
        let engine = Arc::new(ScanEngine::new(&dir));
        let defs = vec![echo_def("ghost-tool")];

        let outcome = engine
            .start_scan(
                defs,
                Default::default(),
                ScanOptions {
                    max_parallel_probes: 1,
                    overall_deadline: Duration::from_secs(30),
                },
                None,
            )
            .await
            .unwrap();
        let job = match outcome {
            ScanStartOutcome::Started(job) => job,
            other => panic!("ожидали Started: {other:?}"),
        };
        wait_terminal(&engine, &job.job_id).await;

        let entries: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        for file in &entries {
            assert!(
                file == "scan-journal.json" || file == "scan-snapshot.json",
                "скан не имеет права писать «{file}» в хранилище состояния"
            );
        }
        assert!(
            !entries.contains(&"state.json".to_string()),
            "скан не трогает state.json"
        );

        // Результат — живое обнаружение пробой (echo отвечает); проверок
        // здоровья нет, но ответившая проба версии — положительный вердикт.
        // Никаких установок/записей.
        let snapshot = engine.cache().get().unwrap();
        assert!(matches!(
            snapshot.tools[0].state,
            ToolState::InstalledHealthy { .. }
        ));
    }

    /// Регрессия на уровне СНАПШОТА (не только scan_tool): результат,
    /// доехавший до кэша через движок скана, несёт честный вердикт
    /// обнаружения. Критический баг «NotDetected при найденной установке»
    /// проявлялся именно в снапшоте, который читает UI.
    #[tokio::test]
    async fn snapshot_preserves_truthful_detection_outcome() {
        let dir = temp_dir("truthful");
        let engine = Arc::new(ScanEngine::new(&dir));
        let defs = vec![echo_def("truthful-tool")];

        let outcome = engine
            .start_scan(
                defs,
                Default::default(),
                ScanOptions {
                    max_parallel_probes: 1,
                    overall_deadline: Duration::from_secs(30),
                },
                None,
            )
            .await
            .unwrap();
        let job = match outcome {
            ScanStartOutcome::Started(job) => job,
            other => panic!("ожидали Started: {other:?}"),
        };
        wait_terminal(&engine, &job.job_id).await;

        let snapshot = engine.cache().get().unwrap();
        assert_eq!(snapshot.tools.len(), 1);
        let tool = &snapshot.tools[0];
        assert_eq!(tool.tool_id, "truthful-tool");
        // Проба echo ответила → в снапшоте ОБЯЗАН быть Detected.
        assert!(
            matches!(
                tool.detection,
                crate::modules::toolchain::domain::models::DetectionOutcome::Detected
            ),
            "снапшот содержит ложный NotDetected: {:?}",
            tool.detection
        );
        assert!(!tool.installs.is_empty());
        assert_eq!(tool.installs[0].parsed_version.as_deref(), Some("1.0.0"));
    }

    #[tokio::test]
    async fn events_of_different_scans_are_isolatable_by_identity() {
        // Два последовательных скана: события второго обязаны отличаться
        // идентичностью от событий первого (фильтр «хвостов» на фронте).
        let dir = temp_dir("isolate");
        let engine = Arc::new(ScanEngine::new(&dir));

        let mut all_events: Vec<(String, String)> = Vec::new(); // (job_id, scan_id)

        for round in 0..2 {
            let (sink, log) = event_collector();
            let outcome = engine
                .start_scan(
                    vec![echo_def(&format!("tool-{round}"))],
                    Default::default(),
                    ScanOptions {
                        max_parallel_probes: 1,
                        overall_deadline: Duration::from_secs(30),
                    },
                    Some(sink),
                )
                .await
                .unwrap();
            let job = match outcome {
                ScanStartOutcome::Started(job) => job,
                other => panic!("ожидали Started: {other:?}"),
            };
            wait_terminal(&engine, &job.job_id).await;

            for event in log.lock().unwrap().iter() {
                match event {
                    ScanEvent::Progress(p) => {
                        all_events.push((p.job_id.clone(), p.scan_id.clone()));
                    }
                    ScanEvent::Done {
                        job_id, scan_id, ..
                    } => {
                        all_events.push((job_id.clone(), scan_id.clone()));
                    }
                }
            }
        }

        // Идентичности двух запусков различны; каждое событие несёт свою.
        assert_eq!(all_events.len(), 4); // 2 прогресса + 2 done
        let first = all_events[0].clone();
        let second = all_events[2].clone();
        assert_ne!(first, second, "разные запуски = разные идентичности");
        for (job_id, scan_id) in &all_events[..2] {
            assert_eq!((job_id, scan_id), (&first.0, &first.1));
        }
        for (job_id, scan_id) in &all_events[2..] {
            assert_eq!((job_id, scan_id), (&second.0, &second.1));
        }
    }

    #[test]
    fn progress_event_requires_operation_identity() {
        use crate::modules::toolchain::domain::models::ScanProgressEvent;
        let state = ToolState::Missing;

        // Без идентичности события НЕ существует.
        assert!(
            ScanProgressEvent::try_new("", "scan", 1, 2, "node", "Node", None, &state, None)
                .is_none()
        );
        assert!(
            ScanProgressEvent::try_new("job", "", 1, 2, "node", "Node", None, &state, None)
                .is_none()
        );
        assert!(
            ScanProgressEvent::try_new("job", "scan", 1, 2, "", "Node", None, &state, None)
                .is_none()
        );

        // С идентичностью — существует и заполнен.
        let event = ScanProgressEvent::try_new(
            "job-1",
            "scan-1",
            1,
            2,
            "node",
            "Node.js",
            Some("node.png"),
            &state,
            Some("бум"),
        )
        .unwrap();
        assert_eq!(event.job_id, "job-1");
        assert_eq!(event.scan_id, "scan-1");
        assert_eq!(event.tool_state, "missing");
        assert_eq!(event.error.as_deref(), Some("бум"));
        assert!(!event.timestamp.is_empty());
    }
}

/// Прогресс-событие: строго с идентичностью операции (конструктор
/// возвращает None без job/scan/tool id — такие события не эмитятся).
fn emit_progress(
    on_event: Option<&ScanEventFn>,
    job_id: &str,
    scan_id: &str,
    done: usize,
    total: usize,
    result: &ToolScanResult,
) {
    let Some(cb) = on_event else { return };
    if let Some(event) = ScanProgressEvent::try_new(
        job_id,
        scan_id,
        done,
        total,
        &result.tool_id,
        &result.display,
        result.icon.as_deref(),
        &result.state,
        result.error.as_deref(),
    ) {
        cb(ScanEvent::Progress(event));
    }
}

fn random_id() -> Result<String, String> {
    let bytes = crypto::random_bytes(8)?;
    Ok(crypto::sha256_hex(&bytes)[..16].to_string())
}
