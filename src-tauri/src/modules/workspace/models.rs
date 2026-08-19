use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectContext {
    pub profile_name: String,
    pub project_path: Option<String>,
    pub description: String,
    pub stack: Vec<String>,
    pub opened_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub started_at: String,
    pub duration_secs: u64,
    pub process_count: usize,
    pub error_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProcessStatus {
    Running,
    Exited(i32),
    Killed,
    Crashed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedProcess {
    pub id: String,
    pub pid: u32,
    pub label: String,
    pub status: ProcessStatus,
    pub started_at: String,
    pub duration_secs: u64,
    pub restarts: u32,
    pub last_error: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessLogs {
    pub stdout_lines: Vec<String>,
    pub stderr_lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessOutputEvent {
    pub process_id: String,
    pub stream: String,
    pub line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessStatusEvent {
    pub process_id: String,
    pub status: ProcessStatus,
    pub error: Option<String>,
}
