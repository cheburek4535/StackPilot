use crate::modules::plugins::mini_ide::models::*;
use crate::modules::plugins::mini_ide::IdeService;

pub struct IdeState {
    pub service: Box<dyn IdeService>,
}

impl IdeState {
    pub fn new() -> Self {
        Self {
            service: Box::new(crate::modules::plugins::mini_ide::service::DefaultIdeService::new()),
        }
    }
}

#[tauri::command]
pub fn get_completions(
    state: tauri::State<'_, IdeState>,
    file_path: String,
    line: usize,
    column: usize,
) -> Vec<CompletionItem> {
    state.service.get_completions(&file_path, line, column)
}

#[tauri::command]
pub fn get_diagnostics(state: tauri::State<'_, IdeState>, file_path: String) -> Vec<Diagnostic> {
    state.service.get_diagnostics(&file_path)
}

#[tauri::command]
pub fn get_hover(
    state: tauri::State<'_, IdeState>,
    file_path: String,
    line: usize,
    column: usize,
) -> Option<HoverInfo> {
    state.service.get_hover(&file_path, line, column)
}

#[tauri::command]
pub fn go_to_definition(
    state: tauri::State<'_, IdeState>,
    file_path: String,
    line: usize,
    column: usize,
) -> Option<Location> {
    state.service.go_to_definition(&file_path, line, column)
}

#[tauri::command]
pub fn format_code(
    state: tauri::State<'_, IdeState>,
    file_path: String,
    content: String,
) -> Result<String, String> {
    state.service.format_code(&file_path, &content)
}
