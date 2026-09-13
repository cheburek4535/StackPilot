pub mod analyzer;
pub mod commands;
pub mod docker_auth;
pub mod file_watcher;
pub mod launch_engine;
pub mod models;
pub mod orchestrator;
pub mod profile_builder;
pub mod profile_manager;
pub mod validation;

use crate::modules::devlauncher::analyzer::{ProjectAnalyzer, ProjectAnalyzerV2};
use crate::modules::devlauncher::file_watcher::FileWatcher;
use crate::modules::devlauncher::launch_engine::LaunchEngine;
use crate::modules::devlauncher::orchestrator::RunOrchestrator;
use crate::modules::devlauncher::profile_manager::JsonProfileManager;
use crate::modules::project_environment::service::EnvironmentBindingService;
use crate::modules::workspace::process_manager::ProcessManager;
use std::path::PathBuf;
use std::sync::Arc;

/// Состояние модуля DevLauncher, регистрируется в Tauri отдельно.
pub struct DevLauncherState {
    /// Concrete manager implementing both the legacy `ProfileManager` API
    /// and the versioned `ProfileManagerV2` API.
    pub profile_manager: Arc<JsonProfileManager>,
    pub launch_engine: Arc<dyn LaunchEngine>,
    pub analyzer: Arc<dyn ProjectAnalyzer>,
    /// Draft-profile analyzer (project model + confidence diagnostics).
    pub analyzer_v2: Arc<dyn ProjectAnalyzerV2>,
    /// Optional binding service for resolving environment overlays.
    /// When None, launch actions use host environment (backward compat).
    pub binding_service: Option<Arc<dyn EnvironmentBindingService>>,
    /// File watcher for live-reload: monitors project directories for changes
    /// and emits Tauri events so the frontend can auto-restart services.
    pub file_watcher: Arc<FileWatcher>,
    /// V2 run orchestrator — DAG-based, async, dependency-aware.
    pub orchestrator: Arc<RunOrchestrator>,
    /// Persistent Docker authorization state (`confirmed` once any docker
    /// step has succeeded). Shared with the orchestrator so failed docker
    /// steps get an auth hint and successful docker runs flip the flag.
    pub docker_auth: Arc<docker_auth::DockerAuthStore>,
}

impl DevLauncherState {
    pub fn new(
        profiles_dir: PathBuf,
        launch_engine: Arc<dyn LaunchEngine>,
        analyzer: Arc<dyn ProjectAnalyzer>,
        process_manager: Arc<dyn ProcessManager>,
        docker_auth: Arc<docker_auth::DockerAuthStore>,
    ) -> Self {
        let analyzer_v2: Arc<dyn ProjectAnalyzerV2> = Arc::new(analyzer::FsProjectAnalyzer);
        Self {
            profile_manager: Arc::new(profile_manager::JsonProfileManager::new(profiles_dir)),
            launch_engine,
            analyzer,
            analyzer_v2,
            binding_service: None,
            file_watcher: Arc::new(FileWatcher::new()),
            orchestrator: Arc::new(RunOrchestrator::new(process_manager, docker_auth.clone())),
            docker_auth,
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
