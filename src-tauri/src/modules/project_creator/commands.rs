use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{Emitter, State};

use super::models::*;
use super::{ProjectCreatorState, EXECUTION_SNAPSHOT_LIMIT};

#[tauri::command]
pub fn get_wizard_tree(state: State<'_, ProjectCreatorState>) -> WizardTreeData {
    state.wizard.get_wizard_tree().clone()
}

#[tauri::command]
pub fn get_project_types(state: State<'_, ProjectCreatorState>) -> Vec<ProjectTypeDef> {
    state.wizard.get_project_types().to_vec()
}

#[tauri::command]
pub fn start_wizard(
    state: State<'_, ProjectCreatorState>,
    project_path: Option<String>,
) -> WizardSession {
    state.wizard.start_session(project_path)
}

#[tauri::command]
pub fn submit_wizard_answer(
    state: State<'_, ProjectCreatorState>,
    session: WizardSession,
    question_id: String,
    answers: Vec<String>,
) -> WizardSession {
    state.wizard.submit_answer(&session, &question_id, answers)
}

#[tauri::command]
pub fn ping_project_creator() -> Result<String, String> {
    Ok("ProjectCreator module is loaded".to_string())
}

#[tauri::command]
pub fn check_project_folder_exists(path: String) -> Result<bool, String> {
    Ok(std::path::Path::new(&path).exists())
}

/// ОС, на которой работает приложение ("windows", "macos", "linux").
/// Фронтенд использует для блокировки платформозависимых опций.
#[tauri::command]
pub fn get_host_platform() -> String {
    super::validate::current_os().to_string()
}

/// Проверяет выбранный стек на ограничения (лимиты, конфликты,
/// платформы, типы проектов, языки по сторонам, целостность генерации).
/// Возвращает все проблемы — единый путь validate_context (нормализация +
/// validate_stack + duplicate_framework_write_paths).
#[tauri::command]
pub fn validate_project_stack(
    state: State<'_, ProjectCreatorState>,
    project_type: Option<String>,
    backend_languages: Vec<String>,
    frontend_languages: Vec<String>,
    frameworks: Vec<String>,
) -> Vec<super::validate::StackIssue> {
    super::validate::validate_context(
        state.wizard.get_wizard_tree(),
        &mut guard_context(
            project_type,
            &backend_languages,
            &frontend_languages,
            &frameworks,
        ),
        super::validate::current_os(),
    )
}

/// Контекст для guard-проверки движка (пути генерации считаются только по
/// фреймворкам; языки/инструменты на результат не влияют).
fn guard_context(
    project_type: Option<String>,
    backend_languages: &[String],
    frontend_languages: &[String],
    frameworks: &[String],
) -> WizardContext {
    let mut languages = backend_languages.to_vec();
    languages.extend(frontend_languages.iter().cloned());
    WizardContext {
        project_path: None,
        project_name: None,
        project_type,
        is_existing: false,
        languages,
        backend_languages: backend_languages.to_vec(),
        frontend_languages: frontend_languages.to_vec(),
        frameworks: frameworks.to_vec(),
        tools: vec![],
        local_infra_tools: vec![],
        features: vec![],
        infrastructure: vec![],
        docker: false,
        testing: false,
        ci: false,
        git_init: false,
        vscode_config: false,
        answers: Default::default(),
        environment_binding_id: None,
    }
}

/// Первая блокирующая ошибка стека, если она есть (иначе None).
#[tauri::command]
pub fn validate_project_stack_error(
    state: State<'_, ProjectCreatorState>,
    project_type: Option<String>,
    backend_languages: Vec<String>,
    frontend_languages: Vec<String>,
    frameworks: Vec<String>,
) -> Option<String> {
    let issues = super::validate::validate_context(
        state.wizard.get_wizard_tree(),
        &mut guard_context(
            project_type,
            &backend_languages,
            &frontend_languages,
            &frameworks,
        ),
        super::validate::current_os(),
    );
    super::validate::first_error(&issues)
}

#[tauri::command]
pub async fn analyze_project_technologies(
    state: State<'_, ProjectCreatorState>,
    path: String,
) -> Result<AnalysisReport, String> {
    // Обход дерева проекта — потенциально секунды; выполняем вне главного
    // потока, чтобы UI не зависал во время анализа.
    let analyzer = Arc::clone(&state.analyzer);
    let path_buf = PathBuf::from(&path);
    tauri::async_runtime::spawn_blocking(move || analyzer.analyze(&path_buf))
        .await
        .map_err(|e| format!("Analyze task failed: {e}"))?
}

/// Рекомендации для текущего выбора стека: парные фреймворки,
/// побочные фреймворки и инструменты (см. recommend.rs).
#[tauri::command]
pub fn get_stack_recommendations(
    state: State<'_, ProjectCreatorState>,
    project_type: Option<String>,
    backend_languages: Vec<String>,
    frontend_languages: Vec<String>,
    frameworks: Vec<String>,
) -> super::recommend::StackRecommendations {
    super::recommend::recommend_stack(
        state.wizard.get_wizard_tree(),
        project_type.as_deref(),
        &backend_languages,
        &frontend_languages,
        &frameworks,
    )
}

#[tauri::command]
pub fn preview_project_recipe(
    state: State<'_, ProjectCreatorState>,
    context: WizardContext,
    project_path: String,
) -> Result<RecipePreview, String> {
    // Единый барьер: нормализация + правила стека + целостность генерации
    // (два фреймворка, пишущие один файл, — Error). Контекст нормализуется
    // in-place и в каноническом виде уходит в plan().
    let mut context = context;
    if let Some(err) = super::validate::first_error(&super::validate::validate_context(
        state.wizard.get_wizard_tree(),
        &mut context,
        super::validate::current_os(),
    )) {
        return Err(err);
    }
    let path = PathBuf::from(&project_path);
    let plan = state.engine.plan(&context, &path)?;
    Ok(state.engine.preview(&plan))
}

/// Предпросмотр файловой структуры проекта: дерево файлов с уровнями
/// достоверности (certain/expected/unknown), содержимое файлов, которые
/// создаём мы, и список удалаемых опциональных шагов.
///
/// Отличие от preview_project_recipe: НЕ вызывает validate_context —
/// превью показывается до заполнения имени/пуля, на основе уже
/// выбранных фреймворков и инструментов.
///
/// `removed_step_ids` — опциональные шаги, помеченные пользователем на
/// удаление: они исключаются из плана ДО построения дерева, поэтому
/// предпросмотр и генерация всегда совпадают (одна схема).
#[tauri::command]
pub fn preview_project_files(
    state: State<'_, ProjectCreatorState>,
    context: WizardContext,
    project_path: String,
    removed_step_ids: Vec<String>,
) -> Result<ProjectFilePreview, String> {
    let mut context = context;
    // Без валидации — превью строится по текущему набору фреймворков/инструментов.
    // Нормализуем минимально: project_type + project_name.
    if context.project_type.is_none() {
        context.project_type = Some("empty".into());
    }
    if context.project_name.is_none() || context.project_name.as_deref() == Some("") {
        context.project_name = Some("preview".into());
    }
    let path = if project_path.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(&project_path)
    };
    // Если plan() не удаётся (неполный контекст), строим пустой preview
    // вместо ошибки — пользователь ещё не закончил выбор стека.
    match state.engine.plan(&context, &path) {
        Ok(plan) => match super::engine::apply_step_removals(plan, &removed_step_ids) {
            Ok(plan) => Ok(super::engine::build_project_file_preview(&plan)),
            Err(e) => Err(e),
        },
        Err(_e) => Ok(ProjectFilePreview {
            files: Vec::new(),
            layout: super::models::LayoutSummary {
                class: "unknown".into(),
                generated_directories: Vec::new(),
                root_owner: None,
                framework_placement: Vec::new(),
            },
            removable_step_ids: Vec::new(),
            summary: super::models::ProjectPreviewSummary {
                certain_count: 0,
                expected_count: 0,
                unknown_count: 0,
                dir_count: 0,
            },
        }),
    }
}

/// Запустить выполнение плана проекта. `removed_step_ids` — опциональные
/// шаги (git_*, vscode_*, readme...), исключённые пользователем: они
/// удаляются из плана ДО выполнения, генерация идёт по той же схеме,
/// что показывал предпросмотр.
#[tauri::command]
pub async fn start_project_execution(
    app: tauri::AppHandle,
    state: State<'_, ProjectCreatorState>,
    context: WizardContext,
    project_path: String,
    removed_step_ids: Vec<String>,
) -> Result<ExecutionPlan, String> {
    // Единый барьер перед выполнением: нормализация + правила стека +
    // целостность генерации. Контекст нормализуется in-place.
    let mut context = context;
    if let Some(err) = super::validate::first_error(&super::validate::validate_context(
        state.wizard.get_wizard_tree(),
        &mut context,
        super::validate::current_os(),
    )) {
        return Err(err);
    }
    let path = PathBuf::from(&project_path);
    let plan = state.engine.plan(&context, &path)?;
    let plan = super::engine::apply_step_removals(plan, &removed_step_ids)?;
    let plan_clone = plan.clone();
    let engine = Arc::clone(&state.engine);
    let app_clone = app.clone();
    let events_store = Arc::clone(&state.execution_events);
    let running_flag = Arc::clone(&state.execution_running);

    running_flag.store(true, Ordering::SeqCst);
    *events_store
        .lock()
        .map_err(|_| "execution state poisoned".to_string())? = Vec::new();

    tokio::spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<ExecutionEvent>(32);

        // Forward events from channel to Tauri frontend and keep a bounded
        // buffer so a re-mounting Create tab can restore the live progress.
        let forward_app = app.clone();
        let events_store_f = Arc::clone(&events_store);
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let _ = forward_app.emit("project_creator:step_event", &event);
                if let Ok(mut buf) = events_store_f.lock() {
                    if buf.len() >= EXECUTION_SNAPSHOT_LIMIT {
                        let drop_count = buf.len() - EXECUTION_SNAPSHOT_LIMIT + 1;
                        buf.drain(..drop_count);
                    }
                    buf.push(event);
                }
            }
        });

        engine.execute(plan_clone, tx).await;

        let _ = app_clone.emit("project_creator:execution_done", ());
        running_flag.store(false, Ordering::SeqCst);
    });

    Ok(plan)
}

/// Снимок текущего/последнего выполнения проекта: работает ли оно ещё и
/// буфер событий. Используется фронтендом при восстановлении вкладки Create
/// (переключение вкладок не убивает выполнение на бэкенде).
#[tauri::command]
pub fn project_execution_snapshot(
    state: State<'_, ProjectCreatorState>,
) -> Result<ExecutionSnapshot, String> {
    let events = state
        .execution_events
        .lock()
        .map_err(|_| "execution state poisoned".to_string())?
        .clone();
    let running = state.execution_running.load(Ordering::SeqCst);
    Ok(ExecutionSnapshot { running, events })
}
