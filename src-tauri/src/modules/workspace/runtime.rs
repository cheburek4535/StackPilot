use crate::modules::workspace::models::{ProcessStatus, TrackedProcess};

/// Service for runtime process monitoring.
/// Aggregates process data from ProcessManager and can add
/// session-scoped filtering, restart logic, etc.
pub trait RuntimeService: Send + Sync {
    fn filter_session_processes(&self, processes: &[TrackedProcess], session_id: Option<&str>) -> Vec<TrackedProcess>;
    fn count_by_status(&self, processes: &[TrackedProcess]) -> RuntimeCounts;
}

#[derive(Debug, Clone)]
pub struct RuntimeCounts {
    pub running: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub killed: usize,
}

pub struct DefaultRuntimeService;

impl RuntimeService for DefaultRuntimeService {
    fn filter_session_processes(&self, processes: &[TrackedProcess], session_id: Option<&str>) -> Vec<TrackedProcess> {
        match session_id {
            Some(sid) => processes.iter().filter(|p| p.session_id.as_deref() == Some(sid)).cloned().collect(),
            None => processes.to_vec(), // если сессии нет — все процессы
        }
    }

    fn count_by_status(&self, processes: &[TrackedProcess]) -> RuntimeCounts {
        let mut counts = RuntimeCounts { running: 0, succeeded: 0, failed: 0, killed: 0 };
        for p in processes {
            match p.status {
                ProcessStatus::Running => counts.running += 1,
                ProcessStatus::Exited(0) => counts.succeeded += 1,
                ProcessStatus::Exited(_) => counts.failed += 1,
                ProcessStatus::Crashed => counts.failed += 1,
                ProcessStatus::Killed => counts.killed += 1,
            }
        }
        counts
    }
}

impl DefaultRuntimeService {
    pub fn new() -> Self {
        Self
    }
}
