use crate::modules::workspace::models::{ProcessStatus, TrackedProcess};

#[derive(Debug, Clone)]
pub struct OverviewData {
    pub total_processes: usize,
    pub running_processes: usize,
    pub error_count: usize,
    pub total_restarts: u32,
    pub session_uptime_secs: u64,
}

pub trait OverviewService: Send + Sync {
    fn compute_overview(
        &self,
        processes: &[TrackedProcess],
        session_started_at: Option<u64>,
    ) -> OverviewData;
}

pub struct DefaultOverviewService;

impl OverviewService for DefaultOverviewService {
    fn compute_overview(
        &self,
        processes: &[TrackedProcess],
        session_started_at: Option<u64>,
    ) -> OverviewData {
        let running = processes
            .iter()
            .filter(|p| p.status == ProcessStatus::Running)
            .count();
        let errors = processes
            .iter()
            .filter(|p| {
                matches!(p.status, ProcessStatus::Crashed)
                    || (matches!(p.status, ProcessStatus::Exited(c) if c != 0))
            })
            .count();
        let restarts: u32 = processes.iter().map(|p| p.restarts).sum();
        let uptime = session_started_at
            .map(|start| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                now.saturating_sub(start)
            })
            .unwrap_or(0);

        OverviewData {
            total_processes: processes.len(),
            running_processes: running,
            error_count: errors,
            total_restarts: restarts,
            session_uptime_secs: uptime,
        }
    }
}

impl DefaultOverviewService {
    pub fn new() -> Self {
        Self
    }
}
