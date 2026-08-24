//! Project environment binding foundation.
//!
//! This module provides a persisted per-project environment binding model,
//! stored under StackPilot app data. A binding associates a project with
//! tool overrides, managed PATH entries, environment variables, and an
//! optional preferred IDE.
//!
//! # Design
//!
//! - Bindings are stored as individual JSON files under
//!   `app_data_dir/project_environments/<binding_id>.json`.
//! - The `EnvironmentBinding` model uses `#[serde(default)]` throughout
//!   so old state/profile files without environment fields still load.
//! - Resolution converts a binding into an `EnvironmentOverlay` from the
//!   `platform::environment` module — no competing abstraction.
//! - No binding (None) preserves old host-environment behavior exactly.
//!
//! # Extension points
//!
//! Templates may later create environment profiles via the service API.
//! This session provides the foundation only and does not claim automatic
//! language-runtime provisioning.

pub mod commands;
pub mod models;
pub mod resolver;
pub mod service;

use service::{EnvironmentBindingService, JsonEnvironmentBindingService};
use std::path::PathBuf;
use std::sync::Arc;

/// Tauri-managed state for the project environment module.
pub struct ProjectEnvironmentState {
    pub binding_service: Arc<dyn EnvironmentBindingService>,
}

impl ProjectEnvironmentState {
    pub fn new(bindings_dir: PathBuf) -> Self {
        Self {
            binding_service: Arc::new(JsonEnvironmentBindingService::new(bindings_dir)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use models::*;
    use resolver::*;
    use std::collections::HashMap;

    #[test]
    fn full_lifecycle_create_save_resolve() {
        use service::EnvironmentBindingService;

        let dir = std::env::temp_dir().join(format!("sp_env_lifecycle_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let svc = JsonEnvironmentBindingService::new(dir.clone());

        // Create a binding
        let mut binding = EnvironmentBinding::new(
            Some("Python 3.12 + Node 20".into()),
            Some("/home/user/project".into()),
        );
        binding.tool_overrides.insert(
            "python".into(),
            ToolOverride {
                executable_path: Some("/usr/bin/python3.12".into()),
                version: Some("3.12".into()),
                path_entries: vec![],
                env_vars: HashMap::new(),
            },
        );
        binding.tool_overrides.insert(
            "node".into(),
            ToolOverride {
                executable_path: Some("/usr/local/bin/node".into()),
                version: Some("20".into()),
                path_entries: vec![],
                env_vars: HashMap::new(),
            },
        );
        binding
            .env_vars
            .insert("VIRTUAL_ENV".into(), "/home/user/venv".into());

        // Save
        let saved = svc.save(&binding).unwrap();
        assert_eq!(saved.binding_id, binding.binding_id);

        // Get
        let loaded = svc.get(&binding.binding_id).unwrap();
        assert_eq!(loaded.name, Some("Python 3.12 + Node 20".into()));

        // Find by project path
        let found = svc.find_by_project_path("/home/user/project").unwrap();
        assert!(found.is_some());

        // Resolve to overlay
        let overlay = resolve_binding_overlay(&loaded);
        assert!(!overlay.path_prepend.is_empty());
        assert_eq!(
            overlay.vars_set.get("VIRTUAL_ENV").unwrap(),
            "/home/user/venv"
        );

        // Validate
        let diag = svc.validate(&loaded).unwrap();
        assert!(diag.host_compatible);

        // Delete
        svc.delete(&binding.binding_id).unwrap();
        assert!(svc.get(&binding.binding_id).is_err());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn no_binding_preserves_host_behavior() {
        let binding = EnvironmentBinding::new(None, None);
        let overlay = resolve_binding_overlay(&binding);
        assert!(overlay.path_prepend.is_empty());
        assert!(overlay.vars_set.is_empty());
        assert!(overlay.vars_remove.is_empty());
    }

    #[test]
    fn binding_precedence_tool_before_managed_before_global() {
        let mut binding = EnvironmentBinding::new(None, None);
        binding.tool_overrides.insert(
            "python".into(),
            ToolOverride {
                executable_path: Some("/opt/python/bin/python3".into()),
                version: None,
                path_entries: vec!["/opt/python/lib".into()],
                env_vars: HashMap::new(),
            },
        );
        binding
            .managed_path_entries
            .push("/project/.venv/bin".into());

        let overlay = resolve_binding_overlay(&binding);

        // Tool executable parent dir should be first
        assert_eq!(overlay.path_prepend[0], "/opt/python/bin");
        // Tool path_entries should be second
        assert_eq!(overlay.path_prepend[1], "/opt/python/lib");
        // Managed paths should be last
        assert_eq!(overlay.path_prepend[2], "/project/.venv/bin");
    }

    #[test]
    fn missing_configured_executable_diagnostics() {
        use service::EnvironmentBindingService;

        let dir = std::env::temp_dir().join(format!("sp_env_diagnostics_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let svc = JsonEnvironmentBindingService::new(dir.clone());
        let mut binding = EnvironmentBinding::new(None, None);
        binding.tool_overrides.insert(
            "python".into(),
            ToolOverride {
                executable_path: Some("/nonexistent/python3.12".into()),
                version: None,
                path_entries: vec![],
                env_vars: HashMap::new(),
            },
        );

        let diag = svc.validate(&binding).unwrap();
        assert!(diag
            .warnings
            .iter()
            .any(|w| w.message.contains("does not exist")));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn platform_path_validation_windows() {
        let mut binding = EnvironmentBinding::new(None, None);
        binding.tool_overrides.insert(
            "tool".into(),
            ToolOverride {
                executable_path: Some("relative/tool".into()),
                version: None,
                path_entries: vec!["also/relative".into()],
                env_vars: HashMap::new(),
            },
        );

        let warnings = binding.validate_paths();
        // Both should be flagged as non-absolute
        assert!(warnings.len() >= 2);
    }
}
