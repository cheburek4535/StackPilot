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
pub mod file_explorer;

use std::sync::Arc;
use crate::modules::workspace::process_manager::ProcessManager;

pub struct WorkspaceState {
    pub process_manager: Arc<dyn ProcessManager>,
    pub project: project::DefaultProjectService,
    pub overview: overview::DefaultOverviewService,
    pub runtime: runtime::DefaultRuntimeService,
    pub session: session::DefaultSessionService,
    pub logs: logs::DefaultLogsService,
    pub problems: problems::DefaultProblemsService,
    pub info: info::DefaultInfoService,
    pub file_explorer: file_explorer::DefaultFileExplorerService,
}

impl WorkspaceState {
    pub fn new(process_manager: Arc<dyn ProcessManager>) -> Self {
        Self {
            process_manager,
            project: project::DefaultProjectService::new(),
            overview: overview::DefaultOverviewService::new(),
            runtime: runtime::DefaultRuntimeService::new(),
            session: session::DefaultSessionService::new(),
            logs: logs::DefaultLogsService::new(),
            problems: problems::DefaultProblemsService::new(),
            info: info::DefaultInfoService::new(),
            file_explorer: file_explorer::DefaultFileExplorerService::new(),
        }
    }
}
