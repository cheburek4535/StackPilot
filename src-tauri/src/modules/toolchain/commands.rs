// ============================================================
// Tauri-команды Toolchain Manager
// ============================================================
// Команды вызываются с фронтенда через invoke(). Префикс tc_
// (toolchain) — чтобы имена не пересекались с другими модулями.
//
// Жизненный цикл:
//   1. tc_check_environment(re)   — проверка окружения под проект;
//   2. tc_build_install_plan(ck)  — показать план установки ДО запуска;
//   3. tc_run_install(plan)       — установка в фоне (события task_event);
//   4. tc_get_install_status      — статус/секреты; tc_abort_install — стоп.

use std::sync::Arc;

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
#[tauri::command]
pub async fn tc_check_environment(
    app: tauri::AppHandle,
    state: State<'_, ToolchainState>,
    requirements: ProjectRequirements,
) -> Result<EnvironmentCheck, String> {
    let requested = core::requirements::resolve(&requirements);
    eprintln!("[toolchain] check_environment: требования = {requested:?}");

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
    let check = core::check::run_check(state.definitions(), &requested, free_space_mb, Some(progress)).await;
    eprintln!(
        "[toolchain] check_environment: готово — {} требований, {} МБ, all_ready={}",
        check.requirements.len(),
        check.total_size_mb,
        check.all_ready
    );

    // Отмечаем время последней проверки в state.json (для страницы
    // окружения). Ошибка сохранения не мешает ответу.
    {
        let meta_arc = state.metadata();
        let mut meta = meta_arc.lock().expect("metadata poisoned");
        meta.touch_last_scan(core::console::timestamp());
        let _ = meta.save();
    }

    Ok(check)
}

/// Строит план установки из отчёта проверки — показывает пользователю,
/// что именно будет установлено и сколько займёт, ДО запуска (этап 3).
#[tauri::command]
pub fn tc_build_install_plan(check: EnvironmentCheck) -> InstallPlan {
    core::planner::build_plan(&check)
}

/// Запускает установку по утверждённому плану.
///
/// Команда возвращается сразу, работа идёт в фоне (tokio::spawn):
/// прогресс стримится событиями `toolchain:task_event`, завершение —
/// `toolchain:install_done` с финальным InstallPlan. Генерируемые
/// при установке секреты (пароль PostgreSQL) доступны через
/// tc_get_install_status.
#[tauri::command]
pub fn tc_run_install(
    app: tauri::AppHandle,
    state: State<'_, ToolchainState>,
    plan: InstallPlan,
) -> Result<(), String> {
    let session_arc = state.install_session();

    // Не даём запустить вторую установку, пока идёт первая.
    {
        let mut guard = session_arc.lock().expect("install_session poisoned");
        if guard.as_ref().map(|s| s.running).unwrap_or(false) {
            return Err("Уже идёт другая установка".to_string());
        }
        *guard = Some(InstallSession {
            started_at: core::console::timestamp(),
            running: true,
            plan: plan.clone(),
            secrets: std::collections::HashMap::new(),
        });
    }

    // План пришёл с фронтенда: валидируем, что все инструменты известны
    // каталогу, иначе не начинаем установку.
    for task in &plan.tasks {
        if state.get_definition(&task.tool_id).is_none() {
            return Err(format!("Неизвестный инструмент в плане: {}", task.tool_id));
        }
    }

    let definitions = state.definitions().to_vec();
    let metadata_arc = state.metadata();
    let pending_arc = state.pending_secrets();
    // Сбрасываем флаг отмены перед стартом (контракт: run = «сначала»).
    state.abort_flag().store(false, std::sync::atomic::Ordering::SeqCst);
    let abort = state.abort_flag();

    tokio::spawn(async move {
        let mut working_plan = plan.clone();
        let sink: Arc<dyn core::console::EventSink> = Arc::new(AppEventSink {
            app: app.clone(),
        });
        let secrets = core::installer::execute_plan(&definitions, &mut working_plan, sink, abort).await;

        // Успешно установленные инструменты и сгенерированные секреты
        // сохраняем в state.json — переживут перезапуск приложения.
        {
            let mut meta = metadata_arc.lock().expect("metadata poisoned");
            for task in &working_plan.tasks {
                if let TaskState::Success { version } = &task.state {
                    if let Some(def) = definitions.iter().find(|d| d.id == task.tool_id) {
                        meta.record_tool_installed(
                            &task.tool_id,
                            super::models::InstalledToolInfo {
                                path: core::discovery::installed_path(def).unwrap_or_default(),
                                version: version.clone(),
                                installed_at: core::console::timestamp(),
                                path_entries: def.path_entries.clone(),
                            },
                        );
                    }
                }
            }
            for (key, value) in &secrets {
                meta.set_secret(key, value);
            }
            if let Err(e) = meta.save() {
                eprintln!("[toolchain] state.json не сохранился: {e}");
            }
        }

        // Финальный снапшот в состоянии (для tc_get_install_status).
        {
            let mut guard = session_arc.lock().expect("install_session poisoned");
            if let Some(s) = guard.as_mut() {
                s.plan = working_plan.clone();
                s.secrets = secrets.clone();
                s.running = false;
            }
        }

        // Секреты (пароль PostgreSQL) — в «одноразовую витрину»
        // для фронтенда: пользователь увидит их один раз после установки.
        {
            let mut pending = pending_arc.lock().expect("pending_secrets poisoned");
            *pending = secrets.clone();
        }

        let _ = app.emit("toolchain:install_done", &working_plan);
    });

    Ok(())
}

/// Секреты, сгенерированные ПОСЛЕДНЕЙ установкой (пароль PostgreSQL).
/// Забираются «одноразово»: после вызова витрина очищается, чтобы
/// старые пароли не всплывали на следующей странице окружения.
#[tauri::command]
pub fn tc_take_new_secrets(state: State<'_, ToolchainState>) -> std::collections::HashMap<String, String> {
    let pending = state.pending_secrets();
    let mut guard = pending.lock().expect("pending_secrets poisoned");
    std::mem::take(&mut *guard)
}

/// Текущий статус установки: план с состояниями задач и секреты.
/// Пока running=true — установка идёт. Если вернёт null — ещё не запускалась.
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
    Ok(())
}

/// Локальное состояние Toolchain Manager (state.json): установленные
/// инструменты, секреты, время последней проверки. Для страницы
/// окружения и автодополнений.
#[tauri::command]
pub fn tc_get_metadata(state: State<'_, ToolchainState>) -> ToolchainMetadata {
    state
        .metadata()
        .lock()
        .expect("metadata poisoned")
        .data()
        .clone()
}

/// Health-отчёт по всему окружению: для каждого инструмента каталога
/// — прогон health_checks из tools.json + общий score. Отдаётся
/// фронтенду страницы окружения.
#[tauri::command]
pub async fn tc_get_health_report(
    state: State<'_, ToolchainState>,
) -> Result<HealthReport, String> {
    Ok(core::health::run_health_report(state.definitions()).await)
}

/// Прокладка из Tauri в ядро: шлёт события установки на фронтенд.
struct AppEventSink {
    app: tauri::AppHandle,
}

impl core::console::EventSink for AppEventSink {
    fn emit(&self, event: ToolchainEvent) {
        let _ = self.app.emit("toolchain:task_event", &event);
    }
}