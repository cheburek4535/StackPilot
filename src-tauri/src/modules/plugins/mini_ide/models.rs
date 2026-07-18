use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionItem {
    pub label: String,
    pub kind: String,
    pub detail: Option<String>,
    pub insert_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub line: usize,
    pub column: usize,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoverInfo {
    pub contents: String,
    pub range_start: (usize, usize),
    pub range_end: (usize, usize),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub uri: String,
    pub line: usize,
    pub column: usize,
}
