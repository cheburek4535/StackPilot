use std::fs;
use std::path::{Path, PathBuf};

use super::fs_manager::EnvironmentFsManager;
use super::models::{
    BindingDiagnostics, BindingWarning, EnvironmentBinding, EnvironmentDiskUsage,
    StandaloneExportResult,
};

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

    /// Delete a binding by ID and removes its sandbox directory.
    fn delete(&self, binding_id: &str) -> Result<(), String>;

    /// Find a binding associated with a project path.
    fn find_by_project_path(
        &self,
        project_path: &str,
    ) -> Result<Option<EnvironmentBinding>, String>;

    /// Validate a binding and return diagnostics.
    fn validate(&self, binding: &EnvironmentBinding) -> Result<BindingDiagnostics, String>;

    /// Get or create the default global environment.
    fn get_or_create_default(&self) -> Result<EnvironmentBinding, String>;

    /// Bind a project path to an environment and persist the association.
    fn bind_project(&self, binding_id: &str, project_path: &str) -> Result<EnvironmentBinding, String>;

    /// Unbind a project path from an environment.
    fn unbind_project(&self, binding_id: &str, project_path: &str) -> Result<EnvironmentBinding, String>;

    /// Export standalone activation scripts for an environment into a target folder.
    fn export_standalone(
        &self,
        binding_id: &str,
        target_dir: Option<&str>,
    ) -> Result<StandaloneExportResult, String>;

    /// Calculate disk usage for an environment sandbox.
    fn calculate_disk_usage(&self, binding_id: &str) -> Result<EnvironmentDiskUsage, String>;

    /// Clean up temporary sandbox cache and rebuild fresh shims.
    fn cleanup_sandbox(&self, binding_id: &str) -> Result<EnvironmentDiskUsage, String>;
}

pub struct JsonEnvironmentBindingService {
    bindings_dir: PathBuf,
    environments_dir: PathBuf,
}

impl JsonEnvironmentBindingService {
    pub fn new(bindings_dir: PathBuf) -> Self {
        let environments_dir = bindings_dir
            .parent()
            .unwrap_or(&bindings_dir)
            .join("environments");
        Self {
            bindings_dir,
            environments_dir,
        }
    }

    pub fn with_environments_dir(bindings_dir: PathBuf, environments_dir: PathBuf) -> Self {
        Self {
            bindings_dir,
            environments_dir,
        }
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
                                log::warn!(
                                    "Warning: skipping corrupt binding {}: {}",
                                    path.display(),
                                    e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("Warning: cannot read {}: {}", path.display(), e);
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

        let mut to_save = binding.clone();

        // If isolated, ensure environment sandbox directory and tool shims exist
        if to_save.is_isolated() {
            match EnvironmentFsManager::ensure_sandbox(&self.environments_dir, &to_save) {
                Ok(env_dir) => {
                    to_save.env_dir = Some(env_dir.to_string_lossy().into_owned());
                }
                Err(err) => {
                    log::warn!("Failed to setup sandbox for {}: {}", to_save.binding_id, err);
                }
            }
        }

        let path = self.binding_path(&to_save.binding_id);
        let content = serde_json::to_string_pretty(&to_save)
            .map_err(|e| format!("Failed to serialize binding: {}", e))?;

        // Atomic write: temp file + rename
        let parent = path
            .parent()
            .ok_or_else(|| "Binding path has no parent directory".to_string())?;
        let tmp = parent.join(format!("{}.tmp", to_save.binding_id));
        fs::write(&tmp, &content)
            .map_err(|e| format!("Failed to write binding temp file: {}", e))?;
        fs::rename(&tmp, &path).map_err(|e| format!("Failed to commit binding: {}", e))?;

        Ok(to_save)
    }

    fn delete(&self, binding_id: &str) -> Result<(), String> {
        let path = self.binding_path(binding_id);
        if !path.exists() {
            return Err(format!("Binding '{}' not found", binding_id));
        }
        let _ = fs::remove_file(&path);
        // Clean up sandbox shims directory
        let _ = EnvironmentFsManager::remove_sandbox(&self.environments_dir, binding_id);
        Ok(())
    }

    fn find_by_project_path(
        &self,
        project_path: &str,
    ) -> Result<Option<EnvironmentBinding>, String> {
        let proj = Path::new(project_path);

        // 1. Check primary .stackpilot/environment.json descriptor
        let sp_dir_meta = proj.join(".stackpilot").join("environment.json");
        if sp_dir_meta.is_file() {
            if let Ok(content) = fs::read_to_string(&sp_dir_meta) {
                if let Ok(file_binding) = serde_json::from_str::<EnvironmentBinding>(&content) {
                    if let Ok(existing) = self.get(&file_binding.binding_id) {
                        return Ok(Some(existing));
                    } else {
                        // Project was moved or cloned from another machine: restore into local registry
                        let mut restored = file_binding;
                        restored.bind_project(project_path);
                        if let Ok(saved) = self.save(&restored) {
                            return Ok(Some(saved));
                        }
                    }
                }
            }
        }

        // 2. Check legacy .stackpilot.json specifying environment_id
        let sp_meta = proj.join(".stackpilot.json");
        if sp_meta.is_file() {
            if let Ok(content) = fs::read_to_string(&sp_meta) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(env_id) = val.get("environment_id").and_then(|v| v.as_str()) {
                        if let Ok(b) = self.get(env_id) {
                            return Ok(Some(b));
                        }
                    }
                }
            }
        }

        // 3. Match against registered bound_projects or project_path
        let normalized = normalize_path(project_path);
        let bindings = self.list()?;
        for binding in bindings {
            if let Some(ref pp) = binding.project_path {
                if normalize_path(pp) == normalized {
                    return Ok(Some(binding));
                }
            }
            if binding.bound_projects.iter().any(|p| normalize_path(p) == normalized) {
                return Ok(Some(binding));
            }
        }

        Ok(None)
    }

    fn get_or_create_default(&self) -> Result<EnvironmentBinding, String> {
        match self.get("default") {
            Ok(default_env) => Ok(default_env),
            Err(_) => {
                let default_env = EnvironmentBinding::new_global_default();
                self.save(&default_env)
            }
        }
    }

    fn bind_project(&self, binding_id: &str, project_path: &str) -> Result<EnvironmentBinding, String> {
        let mut binding = self.get(binding_id)?;
        binding.bind_project(project_path);
        let saved = self.save(&binding)?;

        // Write dual descriptor: .stackpilot/environment.json (full metadata) and .stackpilot.json (legacy)
        let proj = Path::new(project_path);
        if proj.is_dir() {
            let sp_dir = proj.join(".stackpilot");
            let _ = fs::create_dir_all(&sp_dir);
            let sp_env_file = sp_dir.join("environment.json");
            if let Ok(content) = serde_json::to_string_pretty(&saved) {
                let _ = fs::write(sp_env_file, content);
            }

            let sp_json_path = proj.join(".stackpilot.json");
            let mut obj = if sp_json_path.exists() {
                fs::read_to_string(&sp_json_path)
                    .ok()
                    .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default()
            } else {
                serde_json::Map::new()
            };
            obj.insert("environment_id".to_string(), serde_json::Value::String(binding_id.to_string()));
            obj.insert("environment_name".to_string(), serde_json::Value::String(saved.name.clone().unwrap_or_default()));
            obj.insert("isolation_mode".to_string(), serde_json::Value::String(if saved.is_isolated() { "isolated".to_string() } else { "global".to_string() }));
            let _ = fs::write(&sp_json_path, serde_json::to_string_pretty(&obj).unwrap_or_default());
        }

        Ok(saved)
    }

    fn unbind_project(&self, binding_id: &str, project_path: &str) -> Result<EnvironmentBinding, String> {
        let mut binding = self.get(binding_id)?;
        binding.unbind_project(project_path);
        self.save(&binding)
    }

    fn export_standalone(
        &self,
        binding_id: &str,
        target_dir: Option<&str>,
    ) -> Result<StandaloneExportResult, String> {
        let binding = self.get(binding_id)?;
        let resolved_target = match target_dir.filter(|s| !s.trim().is_empty()) {
            Some(dir) => PathBuf::from(dir),
            None => {
                if let Some(ref pp) = binding.project_path {
                    PathBuf::from(pp)
                } else if let Some(first) = binding.bound_projects.first() {
                    PathBuf::from(first)
                } else {
                    return Err(format!(
                        "Environment '{}' has no associated project path. Please specify a target directory.",
                        binding_id
                    ));
                }
            }
        };

        if !resolved_target.exists() {
            fs::create_dir_all(&resolved_target).map_err(|e| {
                format!(
                    "Failed to create target export directory '{}': {}",
                    resolved_target.display(),
                    e
                )
            })?;
        }

        let (overlay, _) = super::resolver::resolve_with_diagnostics(&binding);
        let created_files = EnvironmentFsManager::export_standalone(&resolved_target, &binding, &overlay)?;

        Ok(StandaloneExportResult {
            target_dir: resolved_target.to_string_lossy().into_owned(),
            created_files,
        })
    }

    fn calculate_disk_usage(&self, binding_id: &str) -> Result<EnvironmentDiskUsage, String> {
        let binding = self.get(binding_id)?;
        let env_path = self.environments_dir.join(&binding.binding_id);

        if !env_path.exists() {
            return Ok(EnvironmentDiskUsage {
                binding_id: binding.binding_id,
                size_bytes: 0,
                size_display: "0 B".to_string(),
                path: None,
            });
        }

        let bytes = dir_size(&env_path);
        Ok(EnvironmentDiskUsage {
            binding_id: binding.binding_id,
            size_bytes: bytes,
            size_display: format_bytes(bytes),
            path: Some(env_path.to_string_lossy().into_owned()),
        })
    }

    fn cleanup_sandbox(&self, binding_id: &str) -> Result<EnvironmentDiskUsage, String> {
        let binding = self.get(binding_id)?;
        let env_path = self.environments_dir.join(&binding.binding_id);

        if env_path.exists() {
            let _ = fs::remove_dir_all(&env_path);
        }

        // Re-create fresh sandbox shims if isolated
        if binding.is_isolated() {
            let _ = EnvironmentFsManager::ensure_sandbox(&self.environments_dir, &binding);
        }

        self.calculate_disk_usage(binding_id)
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
        path.replace('/', "\\").trim_end_matches('\\').to_lowercase()
    }
    #[cfg(target_os = "macos")]
    {
        path.trim_end_matches('/').to_lowercase()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        path.trim_end_matches('/').to_string()
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

/// Recursively calculate directory size in bytes.
fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    total += dir_size(&entry.path());
                } else {
                    total += meta.len();
                }
            }
        }
    }
    total
}

/// Format bytes into a human-readable string (KB, MB, GB).
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::project_environment::models::ToolOverride;
    use std::collections::HashMap;

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
                executable_path: Some(abs("/nonexistent/python3.12")),
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

    #[test]
    fn test_export_standalone_generates_scripts() {
        let base_dir = temp_dir("export_standalone_svc");
        let export_target = base_dir.join("my_project");
        let svc = JsonEnvironmentBindingService::new(base_dir.clone());

        let mut b = EnvironmentBinding::new(Some("ExportEnv".into()), Some(export_target.to_string_lossy().into_owned()));
        b.env_vars.insert("CUSTOM_KEY".into(), "CUSTOM_VAL".into());
        let saved = svc.save(&b).unwrap();

        let result = svc.export_standalone(&saved.binding_id, None).unwrap();
        assert_eq!(result.target_dir, export_target.to_string_lossy());
        assert!(result.created_files.contains(&"activate.bat".to_string()));
        assert!(result.created_files.contains(&"deactivate.bat".to_string()));
        assert!(result.created_files.contains(&"activate.ps1".to_string()));
        assert!(result.created_files.contains(&"activate.sh".to_string()));

        assert!(export_target.join("activate.bat").exists());
        assert!(export_target.join("deactivate.bat").exists());
        assert!(export_target.join("activate.ps1").exists());
        assert!(export_target.join("activate.sh").exists());

        let bat_text = fs::read_to_string(export_target.join("activate.bat")).unwrap();
        assert!(bat_text.contains("CUSTOM_KEY=CUSTOM_VAL"));
        assert!(bat_text.contains("ExportEnv"));

        let sh_text = fs::read_to_string(export_target.join("activate.sh")).unwrap();
        assert!(sh_text.contains("CUSTOM_KEY=\"CUSTOM_VAL\""));

        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn test_calculate_disk_usage_and_cleanup() {
        let base_dir = temp_dir("disk_usage_svc");
        let svc = JsonEnvironmentBindingService::new(base_dir.clone());

        let b = EnvironmentBinding::new(Some("UsageEnv".into()), None);
        let saved = svc.save(&b).unwrap();

        // 1. Calculate usage on newly created sandbox
        let usage = svc.calculate_disk_usage(&saved.binding_id).unwrap();
        assert_eq!(usage.binding_id, saved.binding_id);
        // It has .stackpilot-env.json metadata file created
        assert!(usage.size_bytes > 0);

        // 2. Put some dummy cache files into the sandbox
        let env_dir = base_dir.parent().unwrap().join("environments").join(&saved.binding_id);
        fs::create_dir_all(&env_dir).unwrap();
        fs::write(env_dir.join("test_cache.tmp"), vec![0u8; 10000]).unwrap();

        let updated_usage = svc.calculate_disk_usage(&saved.binding_id).unwrap();
        assert!(updated_usage.size_bytes >= 10000);

        // 3. Cleanup sandbox
        let clean_usage = svc.cleanup_sandbox(&saved.binding_id).unwrap();
        assert!(!env_dir.join("test_cache.tmp").exists());
        assert!(clean_usage.size_bytes < 10000);

        let _ = fs::remove_dir_all(base_dir);
    }
}
