use tauri::State;

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
