use std::path::PathBuf;
use std::sync::Arc;
use tauri::{Emitter, State};


use super::models::*;
use super::ProjectCreatorState;

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
    super::validate::validate_stack(
        state.wizard.get_wizard_tree(),
        project_type.as_deref(),
        &backend_languages,
        &frontend_languages,
        &frameworks,
        super::validate::current_os(),
    )
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
    let issues = super::validate::validate_stack(
        state.wizard.get_wizard_tree(),
        project_type.as_deref(),
        &backend_languages,
        &frontend_languages,
        &frameworks,
        super::validate::current_os(),
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
    let path = PathBuf::from(&project_path);
    let plan = state.engine.plan(&context, &path)?;
    let plan_clone = plan.clone();
    let engine = Arc::clone(&state.engine);
    let app_clone = app.clone();

    tokio::spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<ExecutionEvent>(32);

        // Forward events from channel to Tauri frontend
        let forward_app = app.clone();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let _ = forward_app.emit("project_creator:step_event", &event);
            }
        });

        engine.execute(plan_clone, tx).await;

        let _ = app_clone.emit("project_creator:execution_done", ());
    });

    Ok(plan)
}
