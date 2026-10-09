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

/// Get or create the canonical default global environment.
#[tauri::command]
pub fn pe_get_or_create_default(
    state: State<'_, ProjectEnvironmentState>,
) -> Result<EnvironmentBinding, String> {
    state.binding_service.get_or_create_default()
}

/// Bind a project path to an environment.
#[tauri::command]
pub fn pe_bind_project(
    state: State<'_, ProjectEnvironmentState>,
    binding_id: String,
    project_path: String,
) -> Result<EnvironmentBinding, String> {
    state.binding_service.bind_project(&binding_id, &project_path)
}

/// Unbind a project path from an environment.
#[tauri::command]
pub fn pe_unbind_project(
    state: State<'_, ProjectEnvironmentState>,
    binding_id: String,
    project_path: String,
) -> Result<EnvironmentBinding, String> {
    state.binding_service.unbind_project(&binding_id, &project_path)
}

/// Export standalone activation scripts into the target project directory.
#[tauri::command]
pub fn pe_export_standalone(
    state: State<'_, ProjectEnvironmentState>,
    binding_id: String,
    target_dir: Option<String>,
) -> Result<StandaloneExportResult, String> {
    state.binding_service.export_standalone(&binding_id, target_dir.as_deref())
}

/// Calculate disk usage for an environment sandbox.
#[tauri::command]
pub fn pe_calculate_disk_usage(
    state: State<'_, ProjectEnvironmentState>,
    binding_id: String,
) -> Result<EnvironmentDiskUsage, String> {
    state.binding_service.calculate_disk_usage(&binding_id)
}

/// Clean up temporary sandbox cache and rebuild fresh shims.
#[tauri::command]
pub fn pe_cleanup_sandbox(
    state: State<'_, ProjectEnvironmentState>,
    binding_id: String,
) -> Result<EnvironmentDiskUsage, String> {
    state.binding_service.cleanup_sandbox(&binding_id)
}

/// Spawn a visible native terminal preloaded with this environment overlay.
#[tauri::command]
pub async fn pe_open_terminal(
    state: State<'_, ProjectEnvironmentState>,
    workspace: State<'_, crate::modules::workspace::WorkspaceState>,
    binding_id: String,
    project_path: Option<String>,
) -> Result<(), String> {
    let binding = state.binding_service.get(&binding_id)?;
    let (overlay, _) = resolve_with_diagnostics(&binding);

    let working_dir = project_path
        .or_else(|| binding.project_path.clone())
        .or_else(|| binding.bound_projects.first().cloned());

    let session_id = crate::modules::workspace::session::ensure_session(&workspace);
    let manager = std::sync::Arc::clone(&workspace.process_manager);

    let title = format!(
        "StackPilot [{}]",
        binding.name.as_deref().unwrap_or(&binding.binding_id)
    );

    #[cfg(target_os = "windows")]
    let (cmd, args) = (
        "cmd.exe".to_string(),
        vec!["/k".to_string(), format!("title {}", title)],
    );
    #[cfg(not(target_os = "windows"))]
    let (cmd, args) = {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| {
            if cfg!(target_os = "macos") {
                "/bin/zsh".to_string()
            } else {
                "/bin/bash".to_string()
            }
        });
        (shell, vec![])
    };

    let args_refs: Vec<String> = args;

    tauri::async_runtime::spawn_blocking(move || {
        let args_str_refs: Vec<&str> = args_refs.iter().map(|s| s.as_str()).collect();
        manager.spawn_visible(
            &cmd,
            &args_str_refs,
            working_dir.as_deref(),
            &title,
            session_id,
            Some(&overlay),
        )
    })
    .await
    .map_err(|e| format!("Spawn terminal failed: {e}"))??;

    Ok(())
}

/// Configure .vscode/settings.json in project_path for this environment.
#[tauri::command]
pub fn pe_configure_vscode_environment(
    state: State<'_, ProjectEnvironmentState>,
    binding_id: String,
    project_path: String,
) -> Result<(), String> {
    let binding = state.binding_service.get(&binding_id)?;
    let (overlay, _) = resolve_with_diagnostics(&binding);

    let proj = std::path::Path::new(&project_path);
    if !proj.is_dir() {
        return Err(format!("Project directory '{}' does not exist", project_path));
    }
    let vscode_dir = proj.join(".vscode");
    let _ = std::fs::create_dir_all(&vscode_dir);
    let settings_path = vscode_dir.join("settings.json");

    let mut settings: serde_json::Map<String, serde_json::Value> = if settings_path.exists() {
        std::fs::read_to_string(&settings_path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default()
    } else {
        serde_json::Map::new()
    };

    if !overlay.path_prepend.is_empty() || !overlay.vars_set.is_empty() {
        let (env_key, path_val) = match crate::platform::host::current_os() {
            crate::platform::host::HostOs::Windows => {
                let s = overlay.path_prepend.join(";");
                ("terminal.integrated.env.windows", format!("{};${{env:PATH}}", s))
            }
            crate::platform::host::HostOs::Macos => {
                let s = overlay.path_prepend.iter().map(|p| p.replace('\\', "/")).collect::<Vec<_>>().join(":");
                ("terminal.integrated.env.osx", format!("{}:${{env:PATH}}", s))
            }
            crate::platform::host::HostOs::Linux => {
                let s = overlay.path_prepend.iter().map(|p| p.replace('\\', "/")).collect::<Vec<_>>().join(":");
                ("terminal.integrated.env.linux", format!("{}:${{env:PATH}}", s))
            }
        };

        let mut term_env = settings
            .get(env_key)
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();

        if !overlay.path_prepend.is_empty() {
            term_env.insert("PATH".to_string(), serde_json::Value::String(path_val));
        }

        for (k, v) in &overlay.vars_set {
            term_env.insert(k.clone(), serde_json::Value::String(v.clone()));
        }

        settings.insert(
            env_key.to_string(),
            serde_json::Value::Object(term_env),
        );
    }

    if let Some(tool) = binding.tool_overrides.get("python") {
        if let Some(ref exe) = tool.executable_path {
            settings.insert(
                "python.defaultInterpreterPath".to_string(),
                serde_json::Value::String(exe.clone()),
            );
        }
    }

    std::fs::write(
        &settings_path,
        serde_json::to_string_pretty(&settings).unwrap_or_default(),
    )
    .map_err(|e| format!("Failed to write .vscode/settings.json: {e}"))?;

    Ok(())
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

    /// Convert a Unix-style absolute path into a platform-appropriate absolute
    /// path so path helpers treat it the same on every OS.
    fn abs(p: &str) -> String {
        #[cfg(target_os = "windows")]
        {
            format!("C:{}", p.replace('/', "\\"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            p.to_string()
        }
    }

    fn abs_parent(p: &str) -> String {
        #[cfg(target_os = "windows")]
        {
            match p.rfind('\\') {
                Some(i) => p[..i].to_string(),
                None => p.to_string(),
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            std::path::Path::new(p)
                .parent()
                .map(|x| x.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string())
        }
    }

    #[test]
    fn create_and_list_bindings() {
        let dir = temp_dir("cmd_list");
        let svc = JsonEnvironmentBindingService::new(dir.clone());

        let b1 =
            EnvironmentBinding::new(Some("First".into()), Some("/path/to/project1".into()));
        svc.save(&b1).unwrap();
        let b2 = EnvironmentBinding::new(Some("Second".into()), None);
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

        let exe = abs("/usr/bin/python3");
        let expected_parent = abs_parent(&exe);
        let mut binding = EnvironmentBinding::new(None, None);
        binding.tool_overrides.insert(
            "python".into(),
            ToolOverride {
                executable_path: Some(exe),
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

        assert!(overlay.path_prepend.contains(&expected_parent));
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
