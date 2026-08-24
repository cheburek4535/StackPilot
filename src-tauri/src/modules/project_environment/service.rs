use std::fs;
use std::path::{Path, PathBuf};

use super::models::{BindingDiagnostics, BindingWarning, EnvironmentBinding};

/// CRUD service for project environment bindings.
///
/// Bindings are stored as individual JSON files under
/// `<app_data_dir>/project_environments/<binding_id>.json`.
pub trait EnvironmentBindingService: Send + Sync {
    /// List all saved bindings.
    fn list(&self) -> Result<Vec<EnvironmentBinding>, String>;

    /// Get a single binding by ID.
    fn get(&self, binding_id: &str) -> Result<EnvironmentBinding, String>;

    /// Save (create or update) a binding. Returns the saved binding.
    fn save(&self, binding: &EnvironmentBinding) -> Result<EnvironmentBinding, String>;

    /// Delete a binding by ID.
    fn delete(&self, binding_id: &str) -> Result<(), String>;

    /// Find a binding associated with a project path.
    fn find_by_project_path(
        &self,
        project_path: &str,
    ) -> Result<Option<EnvironmentBinding>, String>;

    /// Validate a binding and return diagnostics.
    fn validate(&self, binding: &EnvironmentBinding) -> Result<BindingDiagnostics, String>;
}

pub struct JsonEnvironmentBindingService {
    bindings_dir: PathBuf,
}

impl JsonEnvironmentBindingService {
    pub fn new(bindings_dir: PathBuf) -> Self {
        Self { bindings_dir }
    }

    fn binding_path(&self, binding_id: &str) -> PathBuf {
        self.bindings_dir.join(format!("{}.json", binding_id))
    }
}

impl EnvironmentBindingService for JsonEnvironmentBindingService {
    fn list(&self) -> Result<Vec<EnvironmentBinding>, String> {
        let mut bindings = Vec::new();
        if !self.bindings_dir.exists() {
            return Ok(bindings);
        }
        let entries = fs::read_dir(&self.bindings_dir)
            .map_err(|e| format!("Failed to read bindings dir: {}", e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "json") {
                match fs::read_to_string(&path) {
                    Ok(content) => {
                        match serde_json::from_str::<EnvironmentBinding>(&content) {
                            Ok(binding) => bindings.push(binding),
                            Err(e) => {
                                // Skip corrupt files, don't fail the whole list
                                eprintln!(
                                    "Warning: skipping corrupt binding {}: {}",
                                    path.display(),
                                    e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Warning: cannot read {}: {}", path.display(), e);
                    }
                }
            }
        }
        Ok(bindings)
    }

    fn get(&self, binding_id: &str) -> Result<EnvironmentBinding, String> {
        let path = self.binding_path(binding_id);
        let content = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read binding '{}': {}", binding_id, e))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse binding '{}': {}", binding_id, e))
    }

    fn save(&self, binding: &EnvironmentBinding) -> Result<EnvironmentBinding, String> {
        // Ensure directory exists
        fs::create_dir_all(&self.bindings_dir)
            .map_err(|e| format!("Failed to create bindings dir: {}", e))?;

        let path = self.binding_path(&binding.binding_id);
        let content = serde_json::to_string_pretty(binding)
            .map_err(|e| format!("Failed to serialize binding: {}", e))?;

        // Atomic write: temp file + rename
        let parent = path
            .parent()
            .ok_or_else(|| "Binding path has no parent directory".to_string())?;
        let tmp = parent.join(format!("{}.tmp", binding.binding_id));
        fs::write(&tmp, &content)
            .map_err(|e| format!("Failed to write binding temp file: {}", e))?;
        fs::rename(&tmp, &path).map_err(|e| format!("Failed to commit binding: {}", e))?;

        Ok(binding.clone())
    }

    fn delete(&self, binding_id: &str) -> Result<(), String> {
        let path = self.binding_path(binding_id);
        if !path.exists() {
            return Err(format!("Binding '{}' not found", binding_id));
        }
        fs::remove_file(&path)
            .map_err(|e| format!("Failed to delete binding '{}': {}", binding_id, e))
    }

    fn find_by_project_path(
        &self,
        project_path: &str,
    ) -> Result<Option<EnvironmentBinding>, String> {
        let normalized = normalize_path(project_path);
        let bindings = self.list()?;
        for binding in bindings {
            if let Some(ref pp) = binding.project_path {
                if normalize_path(pp) == normalized {
                    return Ok(Some(binding));
                }
            }
        }
        Ok(None)
    }

    fn validate(&self, binding: &EnvironmentBinding) -> Result<BindingDiagnostics, String> {
        let mut warnings = binding.validate_paths();

        // Check for missing executables
        for (tool_id, tool) in &binding.tool_overrides {
            if let Some(ref exe) = tool.executable_path {
                if is_absolute_path(exe) && !Path::new(exe).exists() {
                    warnings.push(BindingWarning {
                        subject: tool_id.clone(),
                        message: format!(
                            "Configured executable for '{}' does not exist: {}",
                            tool_id, exe
                        ),
                    });
                }
            }
        }

        // Check managed path entries exist
        for entry in &binding.managed_path_entries {
            if is_absolute_path(entry) && !Path::new(entry).exists() {
                warnings.push(BindingWarning {
                    subject: "managed_path".to_string(),
                    message: format!("Managed path entry does not exist: {}", entry),
                });
            }
        }

        Ok(BindingDiagnostics {
            warnings,
            host_compatible: true, // Always compatible at foundation level
        })
    }
}

/// Normalize path separators for comparison.
fn normalize_path(path: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        path.replace('/', "\\").to_lowercase()
    }
    #[cfg(not(target_os = "windows"))]
    {
        path.to_string()
    }
}

/// Check if a path is absolute (platform-aware).
fn is_absolute_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    #[cfg(target_os = "windows")]
    {
        let bytes = path.as_bytes();
        (bytes.len() >= 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/'))
            || (bytes.len() >= 2 && bytes[0] == b'\\' && bytes[1] == b'\\')
    }
    #[cfg(not(target_os = "windows"))]
    {
        path.starts_with('/')
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::project_environment::models::ToolOverride;
    use std::collections::HashMap;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sp_env_binding_test_{}_{}",
            std::process::id(),
            name
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn list_empty_when_no_dir() {
        let dir = temp_dir("list_empty");
        let svc = JsonEnvironmentBindingService::new(dir.clone());
        // Directory doesn't exist yet
        let _ = fs::remove_dir_all(&dir);
        let list = svc.list().unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn save_and_get_round_trip() {
        let dir = temp_dir("save_get");
        let svc = JsonEnvironmentBindingService::new(dir.clone());
        let mut b = EnvironmentBinding::new(Some("Test".into()), None);
        b.tool_overrides.insert(
            "python".into(),
            ToolOverride {
                executable_path: Some("/usr/bin/python3".into()),
                version: Some("3.11".into()),
                path_entries: vec![],
                env_vars: HashMap::new(),
            },
        );

        svc.save(&b).unwrap();
        let loaded = svc.get(&b.binding_id).unwrap();
        assert_eq!(loaded.binding_id, b.binding_id);
        assert_eq!(loaded.name, Some("Test".into()));
        assert_eq!(
            loaded.tool_overrides["python"].executable_path,
            Some("/usr/bin/python3".into())
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn list_returns_all() {
        let dir = temp_dir("list_all");
        let svc = JsonEnvironmentBindingService::new(dir.clone());

        let b1 = EnvironmentBinding::new(Some("First".into()), None);
        let b2 = EnvironmentBinding::new(Some("Second".into()), None);
        svc.save(&b1).unwrap();
        svc.save(&b2).unwrap();

        let list = svc.list().unwrap();
        assert_eq!(list.len(), 2);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn delete_removes_binding() {
        let dir = temp_dir("delete");
        let svc = JsonEnvironmentBindingService::new(dir.clone());
        let b = EnvironmentBinding::new(None, None);
        svc.save(&b).unwrap();
        assert!(svc.get(&b.binding_id).is_ok());

        svc.delete(&b.binding_id).unwrap();
        assert!(svc.get(&b.binding_id).is_err());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn delete_nonexistent_returns_error() {
        let dir = temp_dir("delete_nonexist");
        let svc = JsonEnvironmentBindingService::new(dir.clone());
        let result = svc.delete("env_nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn find_by_project_path_matches() {
        let dir = temp_dir("find_project");
        let svc = JsonEnvironmentBindingService::new(dir.clone());
        let b = EnvironmentBinding::new(None, Some("/home/user/project".into()));
        svc.save(&b).unwrap();

        let found = svc.find_by_project_path("/home/user/project").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().binding_id, b.binding_id);

        let not_found = svc.find_by_project_path("/other/path").unwrap();
        assert!(not_found.is_none());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn validate_missing_executable_warns() {
        let dir = temp_dir("validate_missing");
        let svc = JsonEnvironmentBindingService::new(dir.clone());
        let mut b = EnvironmentBinding::new(None, None);
        b.tool_overrides.insert(
            "python".into(),
            ToolOverride {
                executable_path: Some("/nonexistent/python3.12".into()),
                version: None,
                path_entries: vec![],
                env_vars: HashMap::new(),
            },
        );

        let diag = svc.validate(&b).unwrap();
        assert!(diag
            .warnings
            .iter()
            .any(|w| w.message.contains("does not exist")));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn validate_non_absolute_path_warns() {
        let dir = temp_dir("validate_abs");
        let svc = JsonEnvironmentBindingService::new(dir.clone());
        let mut b = EnvironmentBinding::new(None, None);
        b.tool_overrides.insert(
            "node".into(),
            ToolOverride {
                executable_path: Some("node".into()),
                version: None,
                path_entries: vec![],
                env_vars: HashMap::new(),
            },
        );

        let diag = svc.validate(&b).unwrap();
        assert!(diag
            .warnings
            .iter()
            .any(|w| w.message.contains("not absolute")));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn validate_clean_binding_no_warnings() {
        let dir = temp_dir("validate_clean");
        let svc = JsonEnvironmentBindingService::new(dir.clone());
        let b = EnvironmentBinding::new(None, None);
        let diag = svc.validate(&b).unwrap();
        assert!(diag.warnings.is_empty());
        assert!(diag.host_compatible);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_creates_atomic_temp_file() {
        let dir = temp_dir("atomic_save");
        let svc = JsonEnvironmentBindingService::new(dir.clone());
        let b = EnvironmentBinding::new(None, None);
        svc.save(&b).unwrap();

        // No .tmp file should remain after successful save
        let tmp_exists = fs::read_dir(&dir).unwrap().any(|e| {
            e.unwrap()
                .path()
                .extension()
                .map_or(false, |ext| ext == "tmp")
        });
        assert!(!tmp_exists);

        let _ = fs::remove_dir_all(dir);
    }
}
