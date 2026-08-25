pub mod analyzer;
pub mod commands;
pub mod file_watcher;
pub mod launch_engine;
pub mod models;
pub mod profile_builder;
pub mod profile_manager;

use crate::modules::devlauncher::analyzer::ProjectAnalyzer;
use crate::modules::devlauncher::file_watcher::FileWatcher;
use crate::modules::devlauncher::launch_engine::LaunchEngine;
use crate::modules::devlauncher::profile_manager::ProfileManager;
use crate::modules::project_environment::service::EnvironmentBindingService;
use std::path::PathBuf;
use std::sync::Arc;

/// Состояние модуля DevLauncher, регистрируется в Tauri отдельно.
pub struct DevLauncherState {
    pub profile_manager: Arc<dyn ProfileManager>,
    pub launch_engine: Arc<dyn LaunchEngine>,
    pub analyzer: Arc<dyn ProjectAnalyzer>,
    /// Optional binding service for resolving environment overlays.
    /// When None, launch actions use host environment (backward compat).
    pub binding_service: Option<Arc<dyn EnvironmentBindingService>>,
    /// File watcher for live-reload: monitors project directories for changes
    /// and emits Tauri events so the frontend can auto-restart services.
    pub file_watcher: Arc<FileWatcher>,
}

impl DevLauncherState {
    pub fn new(
        profiles_dir: PathBuf,
        launch_engine: Arc<dyn LaunchEngine>,
        analyzer: Arc<dyn ProjectAnalyzer>,
    ) -> Self {
        Self {
            profile_manager: Arc::new(profile_manager::JsonProfileManager::new(profiles_dir)),
            launch_engine,
            analyzer,
            binding_service: None,
            file_watcher: Arc::new(FileWatcher::new()),
        }
    }
    
    pub fn with_binding_service(
        mut self,
        binding_service: Arc<dyn EnvironmentBindingService>,
    ) -> Self {
        self.binding_service = Some(binding_service);
        self
    }
}
