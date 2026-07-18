use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedProcess {
    pub id: String,
    pub pid: u32,
    pub label: String,
    pub status: ProcessStatus,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProcessStatus {
    Running,
    Exited(i32),
    Killed,
    Crashed,
}
