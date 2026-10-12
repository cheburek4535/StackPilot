pub mod commands;
pub mod file_explorer;
pub mod info;
pub mod logs;
pub mod models;
pub mod overview;
pub mod problems;
pub mod process_manager;
pub mod process_supervisor;
pub mod project;
pub mod runtime;
pub mod session;

use crate::modules::workspace::process_manager::ProcessManager;
use std::path::PathBuf;
use std::sync::Arc;

#[allow(dead_code)]
pub struct WorkspaceState {
    pub process_manager: Arc<dyn ProcessManager>,
    pub project: project::DefaultProjectService,
    pub overview: overview::DefaultOverviewService,
    pub runtime: runtime::DefaultRuntimeService,
    pub session: session::DefaultSessionService,
    pub logs: logs::DefaultLogsService,
    pub problems: problems::DefaultProblemsService,
    pub info: info::DefaultInfoService,
    pub file_explorer: Arc<file_explorer::DefaultFileExplorerService>,
}

impl WorkspaceState {
    pub fn new(process_manager: Arc<dyn ProcessManager>, data_dir: Option<PathBuf>) -> Self {
        let project = project::DefaultProjectService::new();
        let file_explorer = Arc::new(file_explorer::DefaultFileExplorerService::new(Arc::new(
            project.clone(),
        )));
        Self {
            process_manager,
            project,
            overview: overview::DefaultOverviewService::new(),
            runtime: runtime::DefaultRuntimeService::new(),
            session: session::DefaultSessionService::new(data_dir),
            logs: logs::DefaultLogsService::new(),
            problems: problems::DefaultProblemsService::new(),
            info: info::DefaultInfoService::new(),
            file_explorer,
        }
    }
}
