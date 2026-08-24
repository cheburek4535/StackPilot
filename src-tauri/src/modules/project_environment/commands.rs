use std::collections::HashMap;

use crate::modules::project_environment::models::*;
use crate::modules::project_environment::resolver::resolve_with_diagnostics;
use crate::modules::project_environment::ProjectEnvironmentState;
use tauri::State;

/// List all saved environment bindings.
#[tauri::command]
pub fn pe_list_bindings(
    state: State<'_, ProjectEnvironmentState>,
) -> Result<Vec<EnvironmentBinding>, String> {
    state.binding_service.list()
}

/// Get a single environment binding by ID.
#[tauri::command]
pub fn pe_get_binding(
    state: State<'_, ProjectEnvironmentState>,
    binding_id: String,
) -> Result<EnvironmentBinding, String> {
    state.binding_service.get(&binding_id)
}

/// Save (create or update) an environment binding.
#[tauri::command]
pub fn pe_save_binding(
    state: State<'_, ProjectEnvironmentState>,
    binding: EnvironmentBinding,
) -> Result<EnvironmentBinding, String> {
    state.binding_service.save(&binding)
}

/// Delete an environment binding by ID.
#[tauri::command]
pub fn pe_delete_binding(
    state: State<'_, ProjectEnvironmentState>,
    binding_id: String,
) -> Result<(), String> {
    state.binding_service.delete(&binding_id)
}

/// Find an environment binding associated with a project path.
#[tauri::command]
pub fn pe_find_binding_for_project(
    state: State<'_, ProjectEnvironmentState>,
    project_path: String,
) -> Result<Option<EnvironmentBinding>, String> {
    state.binding_service.find_by_project_path(&project_path)
}

/// Validate an environment binding and return diagnostics.
#[tauri::command]
pub fn pe_validate_binding(
    state: State<'_, ProjectEnvironmentState>,
    binding: EnvironmentBinding,
) -> Result<BindingDiagnostics, String> {
    state.binding_service.validate(&binding)
}

/// Resolve a binding into an EnvironmentOverlay (returns PATH entries and env vars).
/// Used by other modules to apply the binding to process commands.
#[tauri::command]
pub fn pe_resolve_overlay(
    state: State<'_, ProjectEnvironmentState>,
    binding_id: String,
) -> Result<ResolvedOverlay, String> {
    let binding = state.binding_service.get(&binding_id)?;
    let (overlay, diagnostics) = resolve_with_diagnostics(&binding);

    Ok(ResolvedOverlay {
        binding_id,
        path_prepend: overlay.path_prepend,
        vars_set: overlay.vars_set,
        vars_remove: overlay.vars_remove,
        diagnostics,
    })
}

/// Create a new empty binding with generated ID and timestamps.
#[tauri::command]
pub fn pe_create_binding(
    state: State<'_, ProjectEnvironmentState>,
    name: Option<String>,
    project_path: Option<String>,
) -> Result<EnvironmentBinding, String> {
    let binding = EnvironmentBinding::new(name, project_path);
    state.binding_service.save(&binding)
}

/// Response payload for pe_resolve_overlay.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResolvedOverlay {
    pub binding_id: String,
    pub path_prepend: Vec<String>,
    pub vars_set: HashMap<String, String>,
    pub vars_remove: Vec<String>,
    pub diagnostics: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::project_environment::models::*;
    use crate::modules::project_environment::resolver::resolve_with_diagnostics;
    use crate::modules::project_environment::service::*;
    use std::collections::HashMap;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sp_env_cmd_test_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn create_and_list_bindings() {
        let dir = temp_dir("cmd_list");
        let svc = JsonEnvironmentBindingService::new(dir.clone());

        let mut b1 =
            EnvironmentBinding::new(Some("First".into()), Some("/path/to/project1".into()));
        svc.save(&b1).unwrap();
        let mut b2 = EnvironmentBinding::new(Some("Second".into()), None);
        svc.save(&b2).unwrap();

        let list = svc.list().unwrap();
        assert_eq!(list.len(), 2);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn save_get_delete_cycle() {
        let dir = temp_dir("cmd_crud");
        let svc = JsonEnvironmentBindingService::new(dir.clone());

        let mut binding = EnvironmentBinding::new(Some("Test".into()), None);
        binding.tool_overrides.insert(
            "python".into(),
            ToolOverride {
                executable_path: Some("/usr/bin/python3".into()),
                version: Some("3.12".into()),
                path_entries: vec![],
                env_vars: HashMap::new(),
            },
        );

        let saved = svc.save(&binding).unwrap();
        let loaded = svc.get(&saved.binding_id).unwrap();
        assert_eq!(loaded.name, Some("Test".into()));

        svc.delete(&saved.binding_id).unwrap();
        assert!(svc.get(&saved.binding_id).is_err());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn find_binding_for_project() {
        let dir = temp_dir("cmd_find");
        let svc = JsonEnvironmentBindingService::new(dir.clone());

        let b =
            EnvironmentBinding::new(Some("My Project".into()), Some("/home/user/project".into()));
        svc.save(&b).unwrap();

        let found = svc.find_by_project_path("/home/user/project").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, Some("My Project".into()));

        let not_found = svc.find_by_project_path("/other/path").unwrap();
        assert!(not_found.is_none());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resolve_overlay_returns_paths_and_vars() {
        let dir = temp_dir("cmd_resolve");
        let svc = JsonEnvironmentBindingService::new(dir.clone());

        let mut binding = EnvironmentBinding::new(None, None);
        binding.tool_overrides.insert(
            "python".into(),
            ToolOverride {
                executable_path: Some("/usr/bin/python3".into()),
                version: None,
                path_entries: vec![],
                env_vars: HashMap::new(),
            },
        );
        binding
            .env_vars
            .insert("VIRTUAL_ENV".into(), "/venv".into());

        let saved = svc.save(&binding).unwrap();
        let loaded = svc.get(&saved.binding_id).unwrap();
        let (overlay, _diagnostics) = resolve_with_diagnostics(&loaded);

        assert!(overlay.path_prepend.contains(&"/usr/bin".to_string()));
        assert_eq!(overlay.vars_set.get("VIRTUAL_ENV").unwrap(), "/venv");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn validate_binding_returns_diagnostics() {
        let dir = temp_dir("cmd_validate");
        let svc = JsonEnvironmentBindingService::new(dir.clone());

        let mut binding = EnvironmentBinding::new(None, None);
        binding.tool_overrides.insert(
            "bad".into(),
            ToolOverride {
                executable_path: Some("not_absolute".into()),
                version: None,
                path_entries: vec![],
                env_vars: HashMap::new(),
            },
        );

        let diag = svc.validate(&binding).unwrap();
        assert!(!diag.warnings.is_empty());
        assert!(diag.host_compatible);

        let _ = std::fs::remove_dir_all(dir);
    }
}
