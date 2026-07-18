pub mod models;
pub mod profile_manager;
pub mod launch_engine;
pub mod analyzer;
pub mod commands;

use std::path::PathBuf;
use std::sync::Arc;
use crate::modules::devlauncher::profile_manager::ProfileManager;
use crate::modules::devlauncher::launch_engine::LaunchEngine;
use crate::modules::devlauncher::analyzer::ProjectAnalyzer;

/// Состояние модуля DevLauncher, регистрируется в Tauri отдельно.
pub struct DevLauncherState {
    pub profile_manager: Arc<dyn ProfileManager>,
    pub launch_engine: Arc<dyn LaunchEngine>,
    pub analyzer: Arc<dyn ProjectAnalyzer>,
}

impl DevLauncherState {
    pub fn new(
        profiles_dir: PathBuf,
        launch_engine: Arc<dyn LaunchEngine>,
        analyzer: Arc<dyn ProjectAnalyzer>,
    ) -> Self {
        Self {
            profile_manager: Arc::new(
                profile_manager::JsonProfileManager::new(profiles_dir),
            ),
            launch_engine,
            analyzer,
        }
    }
}
