use crate::modules::plugins::mini_ide::models::*;

/// Trait that defines IDE capabilities.
/// Default implementation returns empty/no-op results.
pub trait IdeService: Send + Sync {
    fn get_completions(&self, file_path: &str, line: usize, column: usize) -> Vec<CompletionItem>;
    fn get_diagnostics(&self, file_path: &str) -> Vec<Diagnostic>;
    fn get_hover(&self, file_path: &str, line: usize, column: usize) -> Option<HoverInfo>;
    fn go_to_definition(&self, file_path: &str, line: usize, column: usize) -> Option<Location>;
    fn format_code(&self, file_path: &str, content: &str) -> Result<String, String>;
}

pub struct DefaultIdeService;

impl DefaultIdeService {
    pub fn new() -> Self {
        Self
    }
}

impl IdeService for DefaultIdeService {
    fn get_completions(&self, _file_path: &str, _line: usize, _column: usize) -> Vec<CompletionItem> {
        Vec::new()
    }

    fn get_diagnostics(&self, _file_path: &str) -> Vec<Diagnostic> {
        Vec::new()
    }

    fn get_hover(&self, _file_path: &str, _line: usize, _column: usize) -> Option<HoverInfo> {
        None
    }

    fn go_to_definition(&self, _file_path: &str, _line: usize, _column: usize) -> Option<Location> {
        None
    }

    fn format_code(&self, _file_path: &str, content: &str) -> Result<String, String> {
        Ok(content.to_string())
    }
}
