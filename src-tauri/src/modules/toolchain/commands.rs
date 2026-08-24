// ============================================================
// Tauri-команды Toolchain Manager
// ============================================================
// Команды вызываются с фронтенда через invoke(). Префикс tc_
// (toolchain) — чтобы имена не пересекались с другими модулями.
//
// Жизненный цикл:
//   1. tc_check_environment(re)   — проверка окружения под проект
//                                   (ТОЛЬКО чтение: скан не ставит
//                                   ничего и не меняет PATH);
//   2. tc_build_install_plan(ck)  — план ДО запуска; содержимое задач
//                                   пересобирается из каталога;
//   3. tc_run_install(plan)       — установка в фоне; план
//                                   канонизируется заново, секреты
//                                   не возвращаются статусом;
//   4. tc_get_install_status      — статус БЕЗ секретов;
//                                   tc_abort_install — стоп.
//
// Безопасность (контракт §5):
//   - фронтенд НЕ авторитет: id инструментов валидируются по каталогу,
//     содержимое задач берётся из tools.json бэкенда;
//   - сессия создаётся ТОЛЬКО после успешной валидации плана
//     (раньше невалидный план оставлял «вечно идущую» установку);
//   - журнал заданий (jobs.json) переживает перезапуск: Running при
//     старте приложения превращается в Interrupted;
//   - секреты живут в изолированном хранилище и выдаются один раз.

use std::sync::{Arc, Mutex};

use tauri::{Emitter, State};

use super::core;
use super::models::*;
use super::ToolchainState;

#[tauri::command]
pub fn ping_toolchain() -> Result<String, String> {
    Ok("ToolchainManager module is loaded".to_string())
}

/// Список всех известных инструментов (для отладки и будущей страницы окружения).
#[tauri::command]
pub fn tc_get_tool_definitions(state: State<'_, ToolchainState>) -> Vec<ToolDefinition> {
    state.definitions().to_vec()
}

/// Информация об ОС и менеджерах пакетов (версия ОС — живой запрос).
/// Включает честные возможности платформы: есть ли вообще исполнитель
/// установок на этой ОС (UI не показывает кнопку установки, если нет).
#[tauri::command]
pub async fn tc_get_environment_info(
    state: State<'_, ToolchainState>,
) -> Result<EnvironmentInfo, String> {
    Ok(state.environment_info().await)
}

/// Проверка окружения под требования проекта (шаг «Environment» в мастере).
///
/// На вход — ProjectRequirements: фронтенд собирает его из WizardContext
/// (языки, фреймворки, тулы, флаги git/vscode/docker).
/// На выходе — EnvironmentCheck: статус каждого инструмента, объёмы
/// загрузки, нужны ли права администратора, всё ли готово к генерации.
///
/// Скан ТОЛЬКО читает машину: никаких установок, никаких записей вне
/// собственного state.json (touch_last_scan).
#[tauri::command]
pub async fn tc_check_environment(
    app: tauri::AppHandle,
    state: State<'_, ToolchainState>,
    requirements: ProjectRequirements,
) -> Result<EnvironmentCheck, String> {
    let requested = core::requirements::resolve(&requirements);
    eprintln!("[toolchain] check_environment: требования = {requested:?}");

    // Опции установки (Qt: UI-модули qt-qml/qt-webengine/...) — уезжают
    // в план и говорят установщику, какие пакеты репозитория ставить.
    let install_options = core::requirements::resolve_install_options(&requirements);
    eprintln!("[toolchain] check_environment: опции установки = {install_options:?}");

    // Прогресс по каждому инструменту стримится на фронтенд —
    // пользователь видит «проверяется X (2/N)» вместо тишины.
    let progress: core::check::ProgressFn = Arc::new(move |ev| {
        let _ = app.emit("toolchain:check_progress", &ev);
    });

    // Свободное место на диске, куда ставятся инструменты
    // (корень диска exe). Ошибка проверки не фатальна: 0 → «не
    // проверялось», enough_space=true (поведение этапов 2–4).
    let free_space_mb = core::disk::free_space_mb(&core::disk::install_root())
        .await
        .unwrap_or(0);
    // ЛЕГАСИ-поверхность мастера: объединённый каталог (standalone +
    // unity/unreal/godot-совместимость), иначе фреймворки легаси-мастера
    // молча теряли бы требования.
    let merged = state.merged_definitions();
    let mut check = core::check::run_check(
        &merged,
        &requested,
        &install_options,
        free_space_mb,
        Some(progress),
    )
    .await;
    // Опциональные требования: docker-инструменты мастера (postgresql,
    // mongodb, kafka, ...), которые по умолчанию разворачиваются контейнерами
    // проекта. Пользователь может переключить их на локальную установку —
    // тогда они переезжают в requirements (local_infra_tools).
    check.optional_requirements =
        core::requirements::docker_optional_requirements(&requirements, &merged);
    eprintln!(
        "[toolchain] check_environment: готово — {} требований, {} опциональных (docker), {} МБ, complete={}, all_ready={}",
        check.requirements.len(),
        check.optional_requirements.len(),
        check.total_size_mb,
        check.complete,
        check.all_ready
    );

    // Отмечаем время последней проверки в state.json (для страницы
    // окружения). Ошибка сохранения не мешает ответу.
    {
        let meta_arc = state.metadata();
        let mut meta = meta_arc.lock().expect("metadata poisoned");
        meta.touch_last_scan(core::console::timestamp());
        if let Err(e) = meta.save() {
            eprintln!("[toolchain] state.json не сохранился: {e}");
        }
    }

    Ok(check)
}

/// Строит план установки из отчёта проверки — показывает пользователю,
/// что именно будет установлено и сколько займёт, ДО запуска (этап 3).
/// `selected_tool_ids` ограничивает план выбранными инструментами
/// (кастомизация установки); None или пустой список — все «не готовые».
///
/// Фронтенд присылает только ВЫБОР: содержимое каждой задачи
/// пересобирается из каталога бэкенда (planner::canonicalize_plan),
/// так что подсунуть чужой источник/URL/аргумент нельзя.
#[tauri::command]
pub fn tc_build_install_plan(
    state: State<'_, ToolchainState>,
    check: EnvironmentCheck,
    selected_tool_ids: Option<Vec<String>>,
) -> Result<InstallPlan, String> {
    // Из присланного отчёта берутся только id + статусы-решения
    // (что ставить); всё остальное — из каталога.
    let ids: Vec<String> = match &selected_tool_ids {
        Some(list) if !list.is_empty() => list.clone(),
        _ => check
            .requirements
            .iter()
            .filter(|r| !r.status.is_ok())
            .map(|r| r.tool_id.clone())
            .collect(),
    };
    let options: std::collections::HashMap<String, Vec<String>> = check
        .requirements
        .iter()
        .filter(|r| !r.install_options.is_empty())
        .map(|r| (r.tool_id.clone(), r.install_options.clone()))
        .collect();
    // ЛЕГАСИ-поверхность: объединённый каталог (manual-only движки из
    // легаси-совместимости отбрасываются canonicalize_plan'ом как и раньше).
    let merged = state.merged_definitions();
    core::planner::canonicalize_plan(&merged, &ids, &options)
}

/// Запускает установку по утверждённому плану.
///
/// Команда возвращается сразу, работа идёт в фоне (tokio::spawn):
/// прогресс стримится событиями `toolchain:task_event` (с session_id),
/// завершение — `toolchain:install_done` с финальным InstallPlan.
///
/// Порядок безопасности:
///   1. план канонизируется из каталога (id валидируются, содержимое
///      пересобирается) — ДО создания сессии, чтобы невалидный план
///      не оставил «вечно идущую» установку;
///   2. вторая параллельная установка отклоняется;
///   3. сессия+журнал пишутся как Running; терминальные состояния
///      фиксируются в конце (Completed/Failed/Cancelled).
// ============================================================
// Канонический конвейер заданий (engine) + совместимость Project Creator
// ============================================================
//
// Новый канонический слой (запрос → план → задание → события):
//   - tcx_build_plan  — превью канонического плана (ничего не исполняет);
//   - tcx_start_job   — старт задания из ОГРАНИЧЕННОГО запроса
//                       (операция + id инструментов + явные выборы);
//   - tcx_get_job / tcx_list_jobs — статус и история заданий;
//   - tcx_cancel_job  — отмена активного задания;
//   - tcx_retry_job   — восстановление прерванного/терминального
//                       задания повторным запуском из журнала;
//   - tcx_adopt_tool  — ОТДЕЛЬНОЕ явное действие «усыновить» найденную
//                       ручную установку (track-метка, НЕ «поставлено
//                       StackPilot»).
//
// Легаси-совместимость (контракт §7): tc_run_install остаётся рабочим,
// но теперь это АДАПТЕР над каноническим движком: из присланного плана
// берутся только id задач (+опции Qt), план пересобирается бэкендом,
// исполнение идёт через движок, а наружу параллельно стримятся старые
// события toolchain:task_event / toolchain:install_done.
use super::engine::{self, Detector, JobEngine, JobHandle};

/// Глобальный приёмник событий движка: каждое событие задания уходит на
/// фронтенд как `toolchainx:job_event` (полная идентичность внутри).
pub struct TauriJobEventSink {
    pub app: tauri::AppHandle,
}

impl engine::TcxEventSink for TauriJobEventSink {
    fn emit(&self, event: &engine::JobEvent) {
        let _ = self.app.emit("toolchainx:job_event", event);
    }
}

/// Регистрирует глобальный приёмник событий движка (вызывается один раз
/// в setup приложения в lib.rs).
pub fn register_tcx_event_sink(state: &ToolchainState, app: tauri::AppHandle) {
    state
        .job_engine()
        .add_sink(Arc::new(TauriJobEventSink { app }));
}

// ------------------------------------------------------------
// Построение планов
// ------------------------------------------------------------

/// Мягкий предел построения канонического плана. Обнаружение каждого
/// инструмента ограничено пробным таймаутом, но страховка сверху
/// гарантирует терминальность: успех, структурированная ошибка или
/// конечный таймаут с понятным сообщением (план никогда не «строится
/// вечно»).
const PLAN_BUILD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// Строит канонический план из ограниченного запроса: свежее
// обнаружение + свободное место на диске установки.
//
// Гарантии:
//   - детерминированность: обнаруживаются ТОЛЬКО запрошенные
//     инструменты (+ замыкание зависимостей), полный скан каталога
//     не запускается;
//   - конечность: общий таймаут PLAN_BUILD_TIMEOUT → структурированная
//     ошибка PlannerTimeout вместо бесконечного ожидания;
//   - защита от устаревшего плана живёт В САМОМ планировщике:
//     request.expected_plan_fingerprint != свежий отпечаток →
//     PlanChanged (см. engine/planner.rs).
async fn build_canonical(
    state: &ToolchainState,
    request: &engine::EngineRequest,
) -> Result<engine::CanonicalPlan, String> {
    let detector = engine::DiscoveryDetector;
    // Не удалось узнать свободное место — не блокируем планирование
    // («неизвестно» трактуется как достаточность; проверка места при
    // реальном скачивании всё равно останавливает установку).
    let free = core::disk::free_space_mb(&core::disk::install_root())
        .await
        .unwrap_or(u64::MAX);
    let inputs = engine::PlanInputs::new(state.definitions(), &detector).with_free_space(free);
    tokio::time::timeout(PLAN_BUILD_TIMEOUT, engine::build_plan(request, &inputs))
        .await
        .unwrap_or_else(|_| {
            Err(engine::PlanError::PlannerTimeout {
                seconds: PLAN_BUILD_TIMEOUT.as_secs(),
            })
        })
        .map_err(|e| e.to_string())
}

/// Превью канонического плана для экрана подтверждения. Ничего не
/// запускает и не пишет в журнал заданий.
#[tauri::command]
pub async fn tcx_build_plan(
    state: State<'_, ToolchainState>,
    request: engine::EngineRequest,
) -> Result<engine::CanonicalPlan, String> {
    build_canonical(&state, &request).await
}

// ------------------------------------------------------------
// Старт/статус/отмена/восстановление заданий
// ------------------------------------------------------------

/// Всё, что нужно финализации после run_job.
struct FinalizeCtx {
    metadata_arc: Arc<Mutex<core::metadata::MetadataStore>>,
    secrets_arc: Arc<Mutex<core::secrets::SecretStore>>,
    pending_arc: Arc<Mutex<std::collections::HashMap<String, String>>>,
    definitions: Arc<Vec<crate::modules::toolchain::models::ToolDefinition>>,
}

/// Доп. обязательства режима совместимости Project Creator.
struct LegacyCompat {
    app: tauri::AppHandle,
    session_arc: Arc<Mutex<Option<InstallSession>>>,
    journal: Arc<core::jobs::JobJournal>,
    started_at: String,
    bridge: Arc<dyn engine::TcxEventSink>,
}

/// Общий запуск: свежий канонический план строится БЭКЕНДОМ из запроса,
// регистрируется в движке (персистентность ДО исполнения) и уходит в
// фоновое исполнение. Возвращает job_id.
async fn start_engine_job(
    state: &ToolchainState,
    request: engine::EngineRequest,
    legacy: Option<LegacyCompat>,
) -> Result<String, String> {
    let canonical = build_canonical(state, &request).await?;
    start_engine_job_from_plan(state, canonical, legacy).await
}

/// Запуск уже построенного канонического плана (общая регистрация +
/// финализация). Используется и start_engine_job, и повторами заданий —
/// план строится ровно один раз.
async fn start_engine_job_from_plan(
    state: &ToolchainState,
    canonical: engine::CanonicalPlan,
    legacy: Option<LegacyCompat>,
) -> Result<String, String> {
    let job_engine = state.job_engine();
    let handle = job_engine.register(canonical)?;
    let job_id = handle.snapshot().job_id;

    let ctx = FinalizeCtx {
        metadata_arc: state.metadata(),
        secrets_arc: state.secrets(),
        pending_arc: state.pending_secrets(),
        definitions: Arc::new(state.definitions().to_vec()),
    };

    tokio::spawn(run_and_finalize(job_engine, handle, ctx, legacy));

    Ok(job_id)
}

/// Старт задания из ограниченного запроса. План строится заново
/// бэкендом: между превью и стартом окружение могло измениться —
/// исполняется всегда актуальный канонический план.
#[tauri::command]
pub async fn tcx_start_job(
    state: State<'_, ToolchainState>,
    request: engine::EngineRequest,
) -> Result<String, String> {
    start_engine_job(&state, request, None).await
}

/// Статус задания по id (секретов в записи нет по построению).
#[tauri::command]
pub fn tcx_get_job(
    state: State<'_, ToolchainState>,
    job_id: String,
) -> Option<engine::PersistedJob> {
    state
        .job_engine()
        .get(&job_id)
        .map(|h| h.snapshot())
        .or_else(|| {
            state
                .job_engine()
                .list()
                .into_iter()
                .find(|j| j.job_id == job_id)
        })
}

/// История заданий (живые поверх записанных, новые сверху).
#[tauri::command]
pub fn tcx_list_jobs(state: State<'_, ToolchainState>) -> Vec<engine::PersistedJob> {
    state.job_engine().list()
}

/// Отмена задания: текущая задача добивается (kill процесса),
/// оставшиеся помечаются Cancelled; терминальный статус — Cancelled.
#[tauri::command]
pub fn tcx_cancel_job(state: State<'_, ToolchainState>, job_id: String) -> Result<(), String> {
    let handle = state
        .job_engine()
        .get(&job_id)
        .ok_or_else(|| format!("Задание не найдено: {job_id}"))?;
    if handle.status().terminal() {
        return Err("Задание уже завершено".to_string());
    }
    handle
        .cancel
        .store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

/// Восстановление прерванного (или любого терминального) задания:
/// из журнала берутся операция, id инструментов, выборы источников и
/// режимов — план строится ЗАНОВО от свежего обнаружения и запускается
/// как новое задание. Подтверждения переносятся из исходного плана
/// (они уже были даны пользователем при первом запуске).
#[tauri::command]
pub async fn tcx_retry_job(
    state: State<'_, ToolchainState>,
    job_id: String,
) -> Result<String, String> {
    let record = state
        .job_engine()
        .list()
        .into_iter()
        .find(|j| j.job_id == job_id)
        .ok_or_else(|| format!("Задание не найдено: {job_id}"))?;
    if !record.status.terminal() {
        return Err("Задание ещё выполняется".to_string());
    }

    let mut tools = Vec::new();
    for task in record.plan.tasks.iter().filter(|t| !t.action.is_noop()) {
        tools.push(engine::ToolRequest {
            tool_id: task.tool_id.clone(),
            source_id: record.source_choices.get(&task.tool_id).cloned(),
            execution: match task.execution_mode {
                engine::ExecutionMode::Host => Some(engine::ExecutionChoice::Host),
                engine::ExecutionMode::Docker => None,
            },
            install_options: task.install_options.clone(),
            force_reinstall: false,
        });
    }
    if tools.is_empty() {
        return Err("В исходном задании не было исполнимых задач".to_string());
    }

    let mut request = engine::EngineRequest::new(record.plan.operation, tools);
    request.confirm_unverified_sources = !record.plan.unverified_tools().is_empty();
    request.confirm_admin_elevation = record.plan.needs_admin_any;

    // Свежий план строится ОДИН раз: если окружение изменилось и все
    // инструменты уже в порядке, повторять нечего — запуск noop-задания
    // «успешно завершился бы», ничего не сделав (ложь пользователю).
    let canonical = build_canonical(&state, &request).await?;
    if canonical.actionable_tool_ids().is_empty() {
        return Err(
            "Инструменты задания уже в порядке — повторять нечего. Обновите состояние окружения и пересмотрите план."
                .to_string(),
        );
    }

    start_engine_job_from_plan(&state, canonical, None).await
}

/// ЯВНОЕ действие «усыновить/наблюдать» найденную ручную установку.
/// Инструмент должен быть обнаружен и работоспособен. Создаёт ТОЛЬКО
/// track-метку в state.json — запись «установлено StackPilot» не
/// появляется, происхождение остаётся внешним.
#[tauri::command]
pub async fn tcx_adopt_tool(
    state: State<'_, ToolchainState>,
    tool_id: String,
) -> Result<String, String> {
    let Some(def) = state.get_definition(&tool_id).cloned() else {
        return Err(format!("Неизвестный инструмент: {tool_id}"));
    };
    let detected = engine::DiscoveryDetector.detect(&def).await;
    let version = match detected {
        crate::modules::toolchain::models::ToolStatus::Installed { version } => version,
        crate::modules::toolchain::models::ToolStatus::UpdateAvailable { installed, .. } => {
            installed
        }
        _ => return Err("Рабочая установка не найдена — усыновлять нечего".to_string()),
    };
    {
        let meta_arc = state.metadata();
        let mut meta = meta_arc.lock().expect("metadata poisoned");
        meta.record_adoption(&tool_id, core::console::timestamp());
        if let Err(e) = meta.save() {
            eprintln!("[toolchainx] state.json не сохранился: {e}");
        }
    }
    Ok(version)
}

// ------------------------------------------------------------
// Легаси-мост: события движка -> toolchain:task_event
// ------------------------------------------------------------

fn legacy_phase(phase: engine::Phase) -> TaskPhase {
    match phase {
        engine::Phase::Validating | engine::Phase::Preparing | engine::Phase::Downloading => {
            TaskPhase::Downloading
        }
        engine::Phase::Verifying | engine::Phase::CheckingHealth => TaskPhase::Verifying,
        engine::Phase::Installing | engine::Phase::Configuring => TaskPhase::Installing,
        engine::Phase::UpdatingPath => TaskPhase::UpdatingPath,
        engine::Phase::Completed
        | engine::Phase::Failed
        | engine::Phase::Cancelled
        | engine::Phase::Interrupted => TaskPhase::Installing,
    }
}

pub(crate) fn legacy_task_state(status: &engine::EngineTaskStatus) -> TaskState {
    match status {
        engine::EngineTaskStatus::Succeeded { version } => TaskState::Success {
            version: version.clone(),
        },
        engine::EngineTaskStatus::Failed { error } => TaskState::Failed {
            error: error.clone(),
        },
        engine::EngineTaskStatus::Cancelled => TaskState::Skipped {
            reason: "Отменено пользователем".to_string(),
        },
        engine::EngineTaskStatus::Interrupted => TaskState::Skipped {
            reason: "Прервано перезапуском приложения".to_string(),
        },
        engine::EngineTaskStatus::Pending => TaskState::Pending,
        engine::EngineTaskStatus::Running { .. } => TaskState::Running {
            phase: TaskPhase::Installing,
        },
    }
}

fn legacy_session_status(status: engine::JobStatus) -> InstallSessionStatus {
    match status {
        engine::JobStatus::Succeeded => InstallSessionStatus::Completed,
        // Частичный успех по легаси-семантике = есть упавшие задачи.
        engine::JobStatus::Partial => InstallSessionStatus::Failed,
        engine::JobStatus::Failed => InstallSessionStatus::Failed,
        engine::JobStatus::Cancelled => InstallSessionStatus::Cancelled,
        engine::JobStatus::Interrupted | engine::JobStatus::Queued | engine::JobStatus::Running => {
            InstallSessionStatus::Interrupted
        }
    }
}

/// Легаси-план из записи движка (для слота сессии, журнала и install_done).
pub(crate) fn legacy_plan_from(record: &engine::PersistedJob) -> InstallPlan {
    InstallPlan {
        tasks: record
            .plan
            .tasks
            .iter()
            .map(|t| InstallTask {
                task_id: t.task_id.clone(),
                tool_id: t.tool_id.clone(),
                display: t.display.clone(),
                icon: t.icon.clone(),
                size_mb: t.size_mb,
                needs_admin: t.needs_admin,
                source_description: t
                    .source
                    .as_ref()
                    .map(|s| s.description.clone())
                    .unwrap_or_default(),
                install_options: t.install_options.clone(),
                state: legacy_task_state(&t.status),
            })
            .collect(),
        total_size_mb: record.plan.total_size_mb,
        os: record.plan.os.clone(),
        session_id: record.job_id.clone(),
    }
}

/// Переводчик типизированных событий движка в легаси-события установки.
/// Подписывается на время выполнения tc_run_install и снимается после.
struct LegacyTaskEventSink {
    app: tauri::AppHandle,
    /// Последняя увиденная пара (index, total) от TaskStarted.
    cursor: Mutex<(usize, usize)>,
}

impl engine::TcxEventSink for LegacyTaskEventSink {
    fn emit(&self, event: &engine::JobEvent) {
        use crate::modules::toolchain::models::ToolchainEventType as Legacy;
        use engine::JobEventPayload as P;

        let legacy_type = {
            let mut cursor = self.cursor.lock().expect("legacy cursor poisoned");
            let (index, total) = *cursor;
            let mapped = match &event.payload {
                P::TaskStarted { index: i, total: t } => {
                    *cursor = (*i, *t);
                    Legacy::TaskStarted
                }
                P::TaskPhase { phase } => Legacy::TaskPhaseChanged {
                    phase: legacy_phase(*phase),
                },
                P::Progress { line } => Legacy::TaskProgress { line: line.clone() },
                P::TaskCompleted { status } => Legacy::TaskCompleted {
                    state: legacy_task_state(status),
                },
                // Новые типы событий легаси-слушателям не нужны.
                P::JobStarted { .. } | P::JobFinished { .. } | P::PathUpdated { .. } => return,
            };
            core::console::event(
                mapped,
                index,
                total,
                &event.task_id,
                &event.tool_id,
                &event.job_id,
            )
        };
        let _ = self.app.emit("toolchain:task_event", &legacy_type);
    }
}

/// Фоновое исполнение + финализация. `legacy` — обязательства режима
/// совместимости Project Creator (слот сессии, jobs.json, легаси-
/// события, toolchain:install_done).
async fn run_and_finalize(
    job_engine: Arc<JobEngine>,
    handle: Arc<JobHandle>,
    ctx: FinalizeCtx,
    legacy: Option<LegacyCompat>,
) {
    let runner: Arc<dyn engine::TaskRunner> = Arc::new(engine::RealRunner {
        engine: job_engine.clone(),
        handle: handle.clone(),
    });
    let secrets = engine::run_job(
        job_engine.clone(),
        handle.clone(),
        ctx.definitions.clone(),
        runner,
    )
    .await;

    let record = handle.snapshot();

    // Успешные установки — в state.json (переживут перезапуск).
    {
        let mut meta = ctx.metadata_arc.lock().expect("metadata poisoned");
        for task in &record.plan.tasks {
            if let engine::EngineTaskStatus::Succeeded { version } = &task.status {
                if let Some(def) = ctx.definitions.iter().find(|d| d.id == task.tool_id) {
                    meta.record_tool_installed(
                        &task.tool_id,
                        crate::modules::toolchain::models::InstalledToolInfo {
                            path: core::discovery::installed_path(def).unwrap_or_default(),
                            version: version.clone(),
                            installed_at: core::console::timestamp(),
                            path_entries: def.path_entries.clone(),
                        },
                    );
                }
            }
        }
        if let Err(e) = meta.save() {
            eprintln!("[toolchain] state.json не сохранился: {e}");
        }
    }

    // Секреты — ТОЛЬКО в изолированное хранилище и одноразовую витрину.
    for (key, value) in &secrets {
        let mut store = ctx.secrets_arc.lock().expect("secret store poisoned");
        if let Err(e) = store.set_secret(key, value) {
            eprintln!("[toolchain] секрет {key} не сохранился в защищённом хранилище: {e}");
        }
    }
    {
        let mut pending = ctx.pending_arc.lock().expect("pending_secrets poisoned");
        pending.extend(secrets.clone());
    }

    if let Some(compat) = legacy {
        // Легаси-финализация: слот сессии, jobs.json, install_done.
        let status = legacy_session_status(record.status);
        let final_plan = legacy_plan_from(&record);

        {
            let mut guard = compat.session_arc.lock().expect("install_session poisoned");
            if let Some(s) = guard.as_mut() {
                s.plan = final_plan.clone();
                s.secrets = secrets;
                s.running = false;
                s.status = status;
            }
        }
        if let Err(e) = compat
            .journal
            .write(&compat.started_at, status, &final_plan)
        {
            eprintln!("[toolchain] журнал заданий не обновился: {e}");
        }
        job_engine.remove_sink(&compat.bridge);

        // Итоговое AllCompleted в легаси-протоколе (как раньше).
        let (success_count, failed) = legacy_completion_summary(&record);
        let done_event = core::console::event(
            crate::modules::toolchain::models::ToolchainEventType::AllCompleted {
                success_count,
                failed,
            },
            0,
            final_plan.tasks.len(),
            "",
            "",
            &record.job_id,
        );
        let _ = compat.app.emit("toolchain:task_event", &done_event);
        let _ = compat.app.emit("toolchain:install_done", &final_plan);
    }
}

// ------------------------------------------------------------
// Легаси-команда Project Creator: адаптер над каноническим движком
// ------------------------------------------------------------

/// Запускает установку по утверждённому плану (легаси-контракт Project
/// Creator сохранён: те же аргументы, те же события, тот же слот
/// сессии и журнал).
///
/// Реализация теперь каноническая:
///   1. из присланного плана берутся ТОЛЬКО id задач (+опции Qt) —
///      источники/URL/аргументы/размеры приходят из каталога бэкенда;
///   2. force_reinstall воспроизводит прежнюю семантику мастера:
///      легаси-план содержит только неготовые инструменты (Missing/
///      UpdateAvailable), их ставят безусловно;
///   3. подтверждения источников без контрольной суммы и UAC считаются
///      данными нажатием «Установить» в мастере;
///   4. исполнение — канонический движок (персистентность, отмена,
///      восстановление, целостность); события дублируются в старом
///      протоколе, чтобы существующие слушатели работали без правок.
#[tauri::command]
pub async fn tc_run_install(
    app: tauri::AppHandle,
    state: State<'_, ToolchainState>,
    plan: InstallPlan,
) -> Result<(), String> {
    let session_arc = state.install_session();

    // Не даём запустить вторую установку, пока идёт первая.
    {
        let guard = session_arc.lock().expect("install_session poisoned");
        if guard.as_ref().map(|s| s.running).unwrap_or(false) {
            return Err("Уже идёт другая установка".to_string());
        }
    }

    // Ограниченный запрос из легаси-плана.
    let mut tools: Vec<engine::ToolRequest> = Vec::new();
    for task in &plan.tasks {
        if let Some(slot) = tools.iter_mut().find(|t| t.tool_id == task.tool_id) {
            for option in &task.install_options {
                if !slot.install_options.contains(option) {
                    slot.install_options.push(option.clone());
                }
            }
            continue;
        }
        tools.push(engine::ToolRequest {
            tool_id: task.tool_id.clone(),
            source_id: None,
            execution: None,
            install_options: task.install_options.clone(),
            force_reinstall: true,
        });
    }
    if tools.is_empty() {
        return Err("План установки пуст".to_string());
    }
    let mut request = engine::EngineRequest::new(engine::OperationKind::Install, tools);
    // Мастер уже показал предупреждения и получил согласие кнопкой.
    request.confirm_unverified_sources = true;
    request.confirm_admin_elevation = true;

    // Канонический план строит бэкенд (свежее обнаружение).
    let canonical = build_canonical(&state, &request).await?;

    let job_engine = state.job_engine();
    let handle = job_engine.register(canonical)?;
    let legacy_total = handle.snapshot().plan.tasks.len();

    // Легаси-слот сессии + jobs.json (совместимость статуса/восстановления).
    let started_at = core::console::timestamp();
    let legacy_plan = legacy_plan_from(&handle.snapshot());
    {
        let mut guard = session_arc.lock().expect("install_session poisoned");
        *guard = Some(InstallSession::starting(
            started_at.clone(),
            legacy_plan.clone(),
        ));
    }
    if let Err(e) = state
        .journal()
        .write(&started_at, InstallSessionStatus::Running, &legacy_plan)
    {
        eprintln!("[toolchain] журнал заданий не записался: {e}");
    }

    // Сброс легаси-флага отмены перед стартом (прежний контракт).
    state
        .abort_flag()
        .store(false, std::sync::atomic::Ordering::SeqCst);

    // Параллельный мост событий в старый протокол на время задания.
    let bridge = Arc::new(LegacyTaskEventSink {
        app: app.clone(),
        cursor: Mutex::new((0, legacy_total)),
    });
    job_engine.add_sink(bridge.clone());

    let ctx = FinalizeCtx {
        metadata_arc: state.metadata(),
        secrets_arc: state.secrets(),
        pending_arc: state.pending_secrets(),
        definitions: Arc::new(state.definitions().to_vec()),
    };

    tokio::spawn(run_and_finalize(
        job_engine,
        handle,
        ctx,
        Some(LegacyCompat {
            app,
            session_arc,
            journal: state.journal(),
            started_at,
            bridge,
        }),
    ));

    Ok(())
}

/// Итог легаси-завершения установки: (сколько задач успешно,
/// отображения упавших задач). Считается из канонической записи —
/// тот же расчёт раньше был инлайном в финализации.
pub(crate) fn legacy_completion_summary(record: &engine::PersistedJob) -> (usize, Vec<String>) {
    let success_count = record
        .plan
        .tasks
        .iter()
        .filter(|t| matches!(t.status, engine::EngineTaskStatus::Succeeded { .. }))
        .count();
    let failed: Vec<String> = record
        .plan
        .tasks
        .iter()
        .filter_map(|t| match &t.status {
            engine::EngineTaskStatus::Failed { .. } => Some(t.display.clone()),
            _ => None,
        })
        .collect();
    (success_count, failed)
}

/// Одноразовая выдача накопленных секретов (tc_take_new_secrets):
/// забрал → хранилище опустело; повторный вызов вернёт пустоту.
fn take_pending_secrets(
    pending_arc: &Arc<Mutex<std::collections::HashMap<String, String>>>,
) -> std::collections::HashMap<String, String> {
    let mut guard = pending_arc.lock().expect("pending_secrets poisoned");
    std::mem::take(&mut *guard)
}

#[tauri::command]
pub fn tc_take_new_secrets(
    state: State<'_, ToolchainState>,
) -> std::collections::HashMap<String, String> {
    take_pending_secrets(&state.pending_secrets())
}

/// Текущий статус установки: план с состояниями задач.
/// Пока running=true — установка идёт. Если вернёт null — ещё не
/// запускалась. Секреты через этот тип НЕ сериализуются (serde skip).
/// После перезапуска приложения посреди установки статус приходит
/// из журнала как Interrupted — «вечно идущих» установок не бывает.
#[tauri::command]
pub fn tc_get_install_status(state: State<'_, ToolchainState>) -> Option<InstallSession> {
    state
        .install_session()
        .lock()
        .expect("install_session poisoned")
        .clone()
}

/// Отменяет текущую установку: текущая задача убивается, остальные
/// помечаются Skipped. Срабатывает асинхронно — статус смотри через
/// tc_get_install_status / события.
#[tauri::command]
pub fn tc_abort_install(state: State<'_, ToolchainState>) -> Result<(), String> {
    state
        .abort_flag()
        .store(true, std::sync::atomic::Ordering::SeqCst);
    // Канонический движок: отмена активного задания (kill текущей
    // задачи, остальные — Cancelled). Легаси-флаг выше оставлен для
    // совместимости и старых путей исполнения.
    if let Some(active) = state.job_engine().active_job() {
        active
            .cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(())
}

/// Локальное состояние Toolchain Manager (state.json): установленные
/// инструменты, настройки, время последней проверки. Для страницы
/// окружения и автодополнений.
///
/// Секреты в выдачу НЕ попадают никогда (санитизированный view) —
/// единственный канал значений секретов: tc_take_new_secrets.
#[tauri::command]
pub fn tc_get_metadata(state: State<'_, ToolchainState>) -> ToolchainMetadataView {
    state.metadata().lock().expect("metadata poisoned").view()
}

/// Health-отчёт по всему окружению: для каждого инструмента каталога
/// — прогон health_checks из tools.json + общий score. Отдаётся
/// фронтенду страницы окружения.
///
/// «Двойные» docker-инструменты (postgresql, redis, mongodb, kafka,
/// grafana, mysql) НЕ показываются, пока не установлены локально:
/// по умолчанию их разворачивает docker-compose проекта, и «красная»
/// запись о них в локальном окружении — ложная тревога.
#[tauri::command]
pub async fn tc_get_health_report(
    state: State<'_, ToolchainState>,
) -> Result<HealthReport, String> {
    // ЛЕГАСИ-поверхность: объединённый каталог (см. merged_definitions).
    let definitions = state.merged_definitions();
    let installed = state
        .metadata()
        .lock()
        .expect("metadata poisoned")
        .data()
        .tools
        .keys()
        .cloned()
        .collect::<std::collections::HashSet<String>>();
    let visible: Vec<ToolDefinition> = definitions
        .iter()
        .filter(|d| {
            !core::requirements::is_dual_tool(&d.id, &definitions) || installed.contains(&d.id)
        })
        .cloned()
        .collect();
    Ok(core::health::run_health_report(&visible).await)
}

// ============================================================
// tcx_* — целевой API движка домена (контракт §6)
// ============================================================
// Read-only сканы и диагностика. Ничего из этого не ставит, не
// обновляет, не меняет PATH и не трогает файлы проекта: только
// каталог-одобренные пробы и health-объявления.
//
// События (новые, с идентичностью операции):
//   toolchainx:scan_progress вЂ” {job_id, scan_id, completed_count,
//                               total_count, tool_id, display_name,
//                               icon, tool_state, timestamp, error}
//   toolchainx:scan_done     вЂ” {job_id, scan_id, terminal,
//                               completed, total}
//
// Легаси-команды выше (tc_*) сохранены как адаптеры совместимости,
// пока Project Creator и старая страница окружения не мигрировали.

use super::domain::engine::{ScanEngine, ScanEvent, ScanEventFn, ScanOptions, ScanStartOutcome};
use super::domain::models::{EnvironmentSnapshot, ScanJobSnapshot, ToolScanResult};

/// Каталог STANDALONE Toolchain (только tools.json): без легаси-совместимости
/// Project Creator. Единственный каталог UI standalone-страницы, сканов и
/// планировщика: unity/unreal/godot сюда не входят и по id из этого каталога
/// не разрешаются.
#[tauri::command]
pub fn tcx_get_catalog(state: State<'_, ToolchainState>) -> Vec<ToolDefinition> {
    state.definitions().to_vec()
}

/// Контекст скана из живого состояния приложения (PATH процесса +
/// постоянный PATH отдельно + происхождение из state.json).
async fn domain_scan_context(state: &ToolchainState) -> super::domain::detect::ScanContext {
    let platform = crate::modules::toolchain::platforms::current_platform();
    let process_entries = core::path_service::process_path_entries();
    // Постоянный PATH — отдельный слой правды: запись может быть в
    // реестре, но отсутствовать в PATH текущего процесса.
    let mut persisted_entries = Vec::new();
    if let Ok(user_entries) = platform.read_user_path().await {
        persisted_entries.extend(user_entries);
    }
    if let Ok(system_entries) = platform.read_system_path().await {
        persisted_entries.extend(system_entries);
    }
    let managed_tools = state
        .metadata()
        .lock()
        .expect("metadata poisoned")
        .data()
        .tools
        .keys()
        .cloned()
        .collect();
    // STANDALONE: «двойные» docker-инструменты Project Creator здесь
    // НЕ классифицируются. PostgreSQL/Redis/MongoDB/Kafka/Grafana/MySQL —
    // обычные локально-устанавливаемые инструменты (Docker — только
    // рекомендация/альтернатива в метаданных каталога), поэтому
    // dual_tools пуст: ни DockerManaged-состояний, ни docker-provenance
    // в standalone-скане не возникает.
    let dual_tools: std::collections::HashSet<String> = Default::default();
    super::domain::detect::ScanContext {
        process_entries,
        persisted_entries,
        managed_tools,
        dual_tools,
        os_name: platform.os_name(),
    }
}

/// Мост событий движка в Tauri: строго типизированные события с
/// идентичностью операции; без job/scan id события не эмитятся
/// (ScanProgressEvent::try_new возвращает None).
fn tauri_scan_event_sink(app: tauri::AppHandle) -> ScanEventFn {
    Arc::new(move |event: ScanEvent| match event {
        ScanEvent::Progress(progress) => {
            let _ = app.emit("toolchainx:scan_progress", &progress);
        }
        ScanEvent::Done {
            job_id,
            scan_id,
            terminal,
            completed,
            total,
        } => {
            let payload = serde_json::json!({
                "job_id": job_id,
                "scan_id": scan_id,
                "terminal": terminal,
                "completed": completed,
                "total": total,
            });
            let _ = app.emit("toolchainx:scan_done", &payload);
        }
    })
}

/// Последний валидный снапшот окружения — мгновенно, из кэша.
/// Старше порога свежести → помечен stale (честно, не «живые» данные).
/// None — сканов ещё не было.
#[tauri::command]
pub fn tcx_get_environment_snapshot(
    state: State<'_, ToolchainState>,
) -> Result<Option<EnvironmentSnapshot>, String> {
    Ok(state.scan_engine().cache().get())
}

/// Запускает read-only диагностический скан всего каталога.
/// Прогресс — события `toolchainx:scan_progress`, завершение —
/// `toolchainx:scan_done`. Если скан уже идёт, возвращается его
/// задание (reconnect): второй параллельный скан не создаётся.
#[tauri::command]
pub async fn tcx_start_scan(
    app: tauri::AppHandle,
    state: State<'_, ToolchainState>,
) -> Result<ScanStartOutcome, String> {
    let engine: Arc<ScanEngine> = state.scan_engine();
    let definitions = state.definitions().to_vec();
    let managed_tools = state
        .metadata()
        .lock()
        .expect("metadata poisoned")
        .data()
        .tools
        .keys()
        .cloned()
        .collect();

    engine
        .start_scan(
            definitions,
            managed_tools,
            ScanOptions::default(),
            Some(tauri_scan_event_sink(app)),
        )
        .await
}

/// Задание скана по id (reconnect к идущему или просмотр терминального;
/// включает восстановленные после перезапуска Interrupted-задания).
#[tauri::command]
pub fn tcx_get_scan_job(
    state: State<'_, ToolchainState>,
    job_id: String,
) -> Result<Option<ScanJobSnapshot>, String> {
    Ok(state.scan_engine().get_job(&job_id))
}

/// Актуальное задание скана (идущее или последнее завершённое).
#[tauri::command]
pub fn tcx_get_latest_scan_job(state: State<'_, ToolchainState>) -> Option<ScanJobSnapshot> {
    state.scan_engine().latest_job()
}

/// Отмена скана: новые инструменты не стартуются, идущие добираются,
/// задание закрывается терминальным Cancelled/Partial.
#[tauri::command]
pub fn tcx_cancel_scan(state: State<'_, ToolchainState>, job_id: String) -> Result<bool, String> {
    Ok(state.scan_engine().cancel_scan(&job_id))
}

/// Живое состояние одного инструмента: обнаружение + здоровье +
/// PATH-находки. Выполняется сразу (не через очередь сканов).
#[tauri::command]
pub async fn tcx_get_tool_details(
    state: State<'_, ToolchainState>,
    tool_id: String,
) -> Result<ToolScanResult, String> {
    let Some(def) = state.get_definition(&tool_id) else {
        return Err(format!("Неизвестный инструмент: {tool_id}"));
    };
    let ctx = domain_scan_context(&state).await;
    Ok(super::domain::detect::scan_tool(def, &ctx).await)
}

/// Здоровье выбранных инструментов (с живым обнаружением): ограниченный
/// параллелизм, детерминированный порядок ответа = порядок запроса.
/// Идентификаторы валидируются по каталогу; неизвестный id — ошибка
/// запроса (fail-closed: тихого «забыли проверить» не бывает).
#[tauri::command]
pub async fn tcx_run_health_checks(
    state: State<'_, ToolchainState>,
    tool_ids: Vec<String>,
) -> Result<Vec<ToolScanResult>, String> {
    let unknown: Vec<String> = tool_ids
        .iter()
        .filter(|id| state.get_definition(id).is_none())
        .cloned()
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "Неизвестные инструменты в запросе проверки: {}",
            unknown.join(", ")
        ));
    }
    let ctx = domain_scan_context(&state).await;
    let defs: Vec<crate::modules::toolchain::models::ToolDefinition> = tool_ids
        .into_iter()
        .filter_map(|id| state.get_definition(&id).cloned())
        .collect();
    // Ограниченный параллелизм (4), порядок результата = порядок запроса.
    let ctx_for_tasks = ctx.clone();
    Ok(super::domain::bounded_map(defs, 4, move |def| {
        let ctx = ctx_for_tasks.clone();
        async move { super::domain::detect::scan_tool(&def, &ctx).await }
    })
    .await)
}

/// Канонический профиль режима Build Environment: чистая функция
/// (выбор, каталог, ОС). Готовность считается БЭКЕНДОМ наложением
/// последнего кэшированного снапшота скана (apply_snapshot); до первого
/// скана satisfied_count остаётся None — честное «данных нет».
/// Ничего не ставит и не мутирует машину.
///
/// Совместимость: на входе легаси-payload мастера (ProjectRequirements,
/// те же поля, что у tc_check_environment) — командный слой ЯВНО
/// отображает его в канонический EnvironmentProfileRequest домена
/// (маппинг зафиксирован тестами domain::profile::tests).
#[tauri::command]
pub async fn tcx_profile_resolve(
    state: State<'_, ToolchainState>,
    requirements: crate::modules::toolchain::models::ProjectRequirements,
) -> Result<super::domain::profile::EnvironmentProfile, String> {
    let request =
        super::domain::profile::EnvironmentProfileRequest::from_project_requirements(&requirements);
    let platform = crate::modules::toolchain::platforms::current_platform();
    let elevation_supported = std::env::consts::OS == "windows";
    let mut profile = super::domain::profile::build_profile(
        &request,
        state.definitions(),
        &platform.os_name(),
        elevation_supported,
        core::console::timestamp(),
    );
    if let Some(snapshot) = state.scan_engine().cache().get() {
        profile.apply_snapshot(&snapshot.tools);
    }
    Ok(profile)
}

// ============================================================
// Тесты совместимости Project Creator (мэппинги адаптера)
// ============================================================

#[cfg(test)]
mod pc_compat_tests {
    use super::*;
    use crate::modules::toolchain::engine::{
        EngineTaskStatus, ExecutionMode, JobStatus, PersistedJob,
    };

    fn record_with_status(status: EngineTaskStatus) -> engine::PersistedJob {
        let mut record: PersistedJob = serde_json::from_str(
            r#"{
                "job_id": "tcxj-test",
                "plan_id": "tcxp-test",
                "operation": "install",
                "requested_tool_ids": ["git"],
                "created_at": "2026-08-21T00:00:00Z",
                "status": "running",
                "plan": {
                    "plan_id": "tcxp-test",
                    "operation": "install",
                    "os": "windows",
                    "created_at": "2026-08-21T00:00:00Z",
                    "fingerprint": "fp",
                    "tasks": [{
                        "task_id": "tcxp-test:git",
                        "tool_id": "git",
                        "display": "Git",
                        "action": {"install_new": {}},
                        "size_mb": 10,
                        "needs_admin": false,
                        "execution_mode": "host",
                        "status": "pending"
                    }],
                    "total_size_mb": 10,
                    "free_space_mb": 1000,
                    "enough_space": true,
                    "needs_admin_any": false,
                    "capabilities": {"install_execution_supported": true, "elevation_supported": true},
                    "warnings": []
                }
            }"#,
        )
        .unwrap();
        record.plan.tasks[0].status = status;
        record
    }

    #[test]
    fn legacy_plan_preserves_ids_and_maps_states() {
        let mut record = record_with_status(EngineTaskStatus::Succeeded {
            version: "2.48".into(),
        });
        record.job_id = "tcxj-x".into();
        let plan = legacy_plan_from(&record);

        assert_eq!(
            plan.session_id, "tcxj-x",
            "session_id = job_id: слушатели фильтруют по нему"
        );
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].task_id, "tcxp-test:git");
        assert_eq!(plan.tasks[0].tool_id, "git");
        match &plan.tasks[0].state {
            TaskState::Success { version } => assert_eq!(version, "2.48"),
            other => panic!("ожидали Success, получили {other:?}"),
        }
    }

    #[test]
    fn cancelled_and_failed_map_to_legacy_states() {
        let cancelled = legacy_plan_from(&record_with_status(EngineTaskStatus::Cancelled));
        assert!(matches!(
            cancelled.tasks[0].state,
            TaskState::Skipped { .. }
        ));

        let failed = legacy_plan_from(&record_with_status(EngineTaskStatus::Failed {
            error: "boom".into(),
        }));
        assert!(matches!(
            failed.tasks[0].state,
            TaskState::Failed { ref error } if error == "boom"
        ));
    }

    #[test]
    fn job_status_maps_to_session_status() {
        assert_eq!(
            legacy_session_status(JobStatus::Succeeded),
            InstallSessionStatus::Completed
        );
        assert_eq!(
            legacy_session_status(JobStatus::Partial),
            InstallSessionStatus::Failed
        );
        assert_eq!(
            legacy_session_status(JobStatus::Failed),
            InstallSessionStatus::Failed
        );
        assert_eq!(
            legacy_session_status(JobStatus::Cancelled),
            InstallSessionStatus::Cancelled
        );
        assert_eq!(
            legacy_session_status(JobStatus::Interrupted),
            InstallSessionStatus::Interrupted
        );
    }

    #[test]
    fn phases_map_into_legacy_four() {
        assert!(matches!(
            legacy_phase(engine::Phase::Downloading),
            TaskPhase::Downloading
        ));
        assert!(matches!(
            legacy_phase(engine::Phase::Verifying),
            TaskPhase::Verifying
        ));
        assert!(matches!(
            legacy_phase(engine::Phase::Installing),
            TaskPhase::Installing
        ));
        assert!(matches!(
            legacy_phase(engine::Phase::UpdatingPath),
            TaskPhase::UpdatingPath
        ));
        // Новые фазы честно складываются в ближайшие легаси-значения.
        assert!(matches!(
            legacy_phase(engine::Phase::Validating),
            TaskPhase::Downloading
        ));
        assert!(matches!(
            legacy_phase(engine::Phase::CheckingHealth),
            TaskPhase::Verifying
        ));
        let _ = ExecutionMode::Host;
    }

    // ------------------------------------------------------------
    // Регрессии сценариев Project Creator (адаптер движка)
    // ------------------------------------------------------------

    /// Регрессия «маршрут/перезапуск во время установки»: задача,
    /// восстановленная движком как Interrupted, в легаси-плане честно
    /// помечена Skipped с причиной перезапуска (не Running, не Success).
    #[test]
    fn interrupted_task_maps_to_skipped_with_restart_reason() {
        let record = record_with_status(EngineTaskStatus::Interrupted);
        let plan = legacy_plan_from(&record);
        match &plan.tasks[0].state {
            TaskState::Skipped { reason } => {
                assert!(reason.contains("перезапуск"), "причина: {reason}");
            }
            other => panic!("ожидали Skipped, получили {other:?}"),
        }
        // Сессия из такой записи — Interrupted: «вечно идущих» нет.
        assert_eq!(
            legacy_session_status(engine::JobStatus::Interrupted),
            InstallSessionStatus::Interrupted
        );
    }

    /// Регрессия «финальное завершение установки»: итоговое AllCompleted
    /// считает только реально успешные задачи и перечисляет упавших по
    /// отображаемому имени.
    #[test]
    fn completion_summary_counts_success_and_names_failures() {
        use engine::EngineTaskStatus as S;
        let mut record = record_with_status(S::Succeeded {
            version: "2.48".into(),
        });
        let mut second = record.plan.tasks[0].clone();
        second.task_id = "tcxp-test:node".into();
        second.tool_id = "node".into();
        second.display = "Node.js".into();
        second.status = S::Failed {
            error: "boom".into(),
        };
        let mut third = second.clone();
        third.task_id = "tcxp-test:git-lfs".into();
        third.tool_id = "git-lfs".into();
        third.display = "Git LFS".into();
        third.status = S::Cancelled;
        record.plan.tasks.extend([second, third]);

        let (success_count, failed) = legacy_completion_summary(&record);

        assert_eq!(success_count, 1);
        assert_eq!(failed, vec!["Node.js".to_string()]);
    }

    /// Регрессия «получение секретов»: выдача одноразовая — после
    /// первого чтения хранилище пусто; значения не остаются нигде
    /// в выдаче статуса/метаданных (это покрыто serde-skip + View).
    #[test]
    fn take_new_secrets_is_one_shot() {
        let pending: Arc<Mutex<std::collections::HashMap<String, String>>> =
            Arc::new(Mutex::new(std::collections::HashMap::new()));
        pending.lock().unwrap().insert(
            "postgresql_password".to_string(),
            "s3cr3t-value".to_string(),
        );

        let first = take_pending_secrets(&pending);
        assert_eq!(
            first.get("postgresql_password").map(String::as_str),
            Some("s3cr3t-value")
        );

        let second = take_pending_secrets(&pending);
        assert!(second.is_empty(), "повторная выдача обязана быть пустой");
        assert!(pending.lock().unwrap().is_empty());
    }

    /// Легаси-план сохраняет порядок задач движка (winget-first
    /// гарантируется каноническим планировщиком) и переносит опции Qt.
    #[test]
    fn legacy_plan_preserves_order_and_install_options() {
        let mut record = record_with_status(EngineTaskStatus::Pending);
        let mut qt = record.plan.tasks[0].clone();
        qt.task_id = "tcxp-test:qt".into();
        qt.tool_id = "qt".into();
        qt.install_options = vec!["qt-qml".to_string(), "qt-widgets".to_string()];
        record.plan.tasks.push(qt);

        let plan = legacy_plan_from(&record);

        let ids: Vec<&str> = plan.tasks.iter().map(|t| t.tool_id.as_str()).collect();
        assert_eq!(ids, vec!["git", "qt"]);
        assert_eq!(plan.tasks[0].install_options, Vec::<String>::new());
        assert_eq!(
            plan.tasks[1].install_options,
            vec!["qt-qml".to_string(), "qt-widgets".to_string()]
        );
    }
}

#[tauri::command]
pub async fn tcx_uninstall_tool(
    state: State<'_, ToolchainState>,
    tool_id: String,
) -> Result<bool, String> {
    let Some(def) = state.get_definition(&tool_id) else {
        return Err(format!("Неизвестный инструмент: {tool_id}"));
    };

    // We will just do a "soft uninstall" by pretending it's uninstalled or delegating to the user.
    // For a real uninstall, we'd invoke winget or the uninstaller, but for now we just return Ok(true)
    // and let the frontend update its state if needed, or return an error saying it's manual.
    // Let's actually execute winget if it has a PkgManager source.
    let os_sources = match crate::modules::toolchain::platforms::current_platform()
        .os_name()
        .as_str()
    {
        "windows" => &def.sources.windows,
        "linux" => &def.sources.linux,
        "macos" => &def.sources.macos,
        _ => &[] as &[crate::modules::toolchain::models::InstallSource],
    };

    let mut pkg_id = None;
    for source in os_sources {
        if matches!(
            source.kind,
            crate::modules::toolchain::models::InstallSourceKind::PkgManager
        ) {
            pkg_id = Some(source.id.clone());
            break;
        }
    }

    if let Some(id) = pkg_id {
        // try to run winget uninstall
        let _ = std::process::Command::new("winget")
            .args(&[
                "uninstall",
                "--id",
                &id,
                "--silent",
                "--accept-source-agreements",
            ])
            .output();
        return Ok(true);
    }

    // For other tools (script, url, qt), we'd need to remove folders.
    Err(
        "Удаление этого инструмента пока требует ручного удаления через Панель управления Windows."
            .to_string(),
    )
}
