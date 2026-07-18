pub mod models;
pub mod process_manager;
pub mod commands;
pub mod project;
pub mod overview;
pub mod runtime;
pub mod session;
pub mod logs;
pub mod problems;
pub mod info;

use std::sync::Arc;
use crate::modules::workspace::process_manager::ProcessManager;

pub struct WorkspaceState {
    pub process_manager: Arc<dyn ProcessManager>,
    pub project: project::ProjectState,
    pub overview: overview::OverviewService,
    pub runtime: runtime::RuntimeService,
    pub session: session::SessionService,
    pub logs: logs::LogsService,
    pub problems: problems::ProblemsService,
    pub info: info::InfoService,
}

impl WorkspaceState {
    pub fn new(process_manager: Arc<dyn ProcessManager>) -> Self {
        Self {
            process_manager,
            project: project::ProjectState::new(),
            overview: overview::OverviewService::new(),
            runtime: runtime::RuntimeService::new(),
            session: session::SessionService::new(),
            logs: logs::LogsService::new(),
            problems: problems::ProblemsService::new(),
            info: info::InfoService::new(),
        }
    }
}
