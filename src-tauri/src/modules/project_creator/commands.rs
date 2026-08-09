use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{Emitter, State};


use super::models::*;
use super::engine::duplicate_framework_write_paths;
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
/// платформы, типы проектов, языки по сторонам). Возвращает все проблемы.
#[tauri::command]
pub fn validate_project_stack(
    state: State<'_, ProjectCreatorState>,
    project_type: Option<String>,
        backend_languages: Vec<String>,
        frontend_languages: Vec<String>,
        frameworks: Vec<String>,
    ) -> Vec<super::validate::StackIssue> {
    let mut issues = super::validate::validate_stack(
        state.wizard.get_wizard_tree(),
        project_type.as_deref(),
        &backend_languages,
        &frontend_languages,
        &frameworks,
        super::validate::current_os(),
    );
    // Guard движка: два фреймворка, пишущие один файл, сломают генерацию.
    issues.extend(
        duplicate_framework_write_paths(&guard_context(
            project_type,
            &backend_languages,
            &frontend_languages,
            &frameworks,
        ))
        .into_iter()
        .map(|message| super::validate::StackIssue {
            severity: super::validate::StackSeverity::Error,
            message,
        }),
    );
    issues
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
        features: vec![],
        infrastructure: vec![],
        docker: false,
        testing: false,
        ci: false,
        git_init: false,
        vscode_config: false,
        answers: Default::default(),
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
    let mut issues = super::validate::validate_stack(
        state.wizard.get_wizard_tree(),
        project_type.as_deref(),
        &backend_languages,
        &frontend_languages,
        &frameworks,
        super::validate::current_os(),
    );
    issues.extend(
        duplicate_framework_write_paths(&guard_context(
            project_type,
            &backend_languages,
            &frontend_languages,
            &frameworks,
        ))
        .into_iter()
        .map(|message| super::validate::StackIssue {
            severity: super::validate::StackSeverity::Error,
            message,
        }),
    );
    super::validate::first_error(&issues)
}

#[tauri::command]
pub fn analyze_project_technologies(
    state: State<'_, ProjectCreatorState>,
    path: String,
) -> Result<AnalysisReport, String> {
    let p = PathBuf::from(&path);
    state.analyzer.analyze(&p)
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
    if let Some(err) = super::validate::first_error(&super::validate::validate_stack(
        state.wizard.get_wizard_tree(),
        context.project_type.as_deref(),
        &context.backend_languages,
        &context.frontend_languages,
        &context.frameworks,
        super::validate::current_os(),
    )) {
        return Err(err);
    }
    // Легальные правила прошли, но фреймворки могут писать один файл —
    // такой стек сломает генерацию, отсекаем до предпросмотра.
    if let Some(err) = duplicate_framework_write_paths(&context).into_iter().next() {
        return Err(err);
    }
    let path = PathBuf::from(&project_path);
    let plan = state.engine.plan(&context, &path)?;
    Ok(state.engine.preview(&plan))
}

#[tauri::command]
pub async fn start_project_execution(
    app: tauri::AppHandle,
    state: State<'_, ProjectCreatorState>,
    context: WizardContext,
    project_path: String,
    ) -> Result<ExecutionPlan, String> {
    if let Some(err) = super::validate::first_error(&super::validate::validate_stack(
        state.wizard.get_wizard_tree(),
        context.project_type.as_deref(),
        &context.backend_languages,
        &context.frontend_languages,
        &context.frameworks,
        super::validate::current_os(),
    )) {
        return Err(err);
    }
    // Финальная проверка целостности генерации (дублирование файлов).
    if let Some(err) = duplicate_framework_write_paths(&context).into_iter().next() {
        return Err(err);
    }
    let path = PathBuf::from(&project_path);
    let plan = state.engine.plan(&context, &path)?;
    let plan_clone = plan.clone();
    let engine = Arc::clone(&state.engine);
    let app_clone = app.clone();
    let events_store = Arc::clone(&state.execution_events);
    let running_flag = Arc::clone(&state.execution_running);

    running_flag.store(true, Ordering::SeqCst);
    *events_store.lock().map_err(|_| "execution state poisoned".to_string())? = Vec::new();

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
