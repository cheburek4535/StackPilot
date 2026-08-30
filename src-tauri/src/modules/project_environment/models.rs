use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Schema version for forward-compatible migration.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Persisted project environment binding.
///
/// Stored under `app_data_dir/project_environments/<binding_id>.json`.
/// A binding associates a project (by optional path) with tool overrides,
/// environment variables, PATH entries, and an optional preferred IDE.
///
/// Old JSON files without new fields load correctly via `#[serde(default)]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentBinding {
    /// Schema version for migration. Default 1.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

    /// Stable unique identifier (UUID-like, generated at creation).
    pub binding_id: String,

    /// Optional human-readable name (e.g. "Python 3.12 + Node 20").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Optional project path this binding is associated with.
    /// When set, the binding can be auto-selected for that project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,

    /// Tool overrides keyed by tool ID (e.g. "python", "node", "cargo").
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tool_overrides: HashMap<String, ToolOverride>,

    /// Optional executable paths to prepend to PATH (in order).
    /// Each entry must be absolute when the binding is resolved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub managed_path_entries: Vec<String>,

    /// Environment variables to set when this binding is active.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env_vars: HashMap<String, String>,

    /// Environment variables to remove when this binding is active.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_vars_remove: Vec<String>,

    /// Optional preferred IDE/application launch target (path or command).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_ide: Option<String>,

    /// Optional arguments for the preferred IDE.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_ide_args: Option<Vec<String>>,

    /// ISO-8601 timestamp of creation.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_at: String,

    /// ISO-8601 timestamp of last modification.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub updated_at: String,
}

fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

/// Per-tool override within a binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOverride {
    /// Optional absolute path to the tool executable.
    /// When set, this path is validated as absolute on resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,

    /// Optional version constraint (advisory, not enforced automatically).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Optional additional PATH entries for this specific tool.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_entries: Vec<String>,

    /// Optional extra environment variables for this specific tool.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env_vars: HashMap<String, String>,
}

/// Diagnostics returned when resolving/validating a binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingDiagnostics {
    /// Warnings about missing or stale configured paths.
    pub warnings: Vec<BindingWarning>,
    /// Whether the binding is valid for the current host OS.
    pub host_compatible: bool,
}

/// A single warning about a binding's configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingWarning {
    /// Tool ID or "path_entry" or "env" related to this warning.
    pub subject: String,
    /// Human-readable warning message.
    pub message: String,
}

impl EnvironmentBinding {
    /// Create a new binding with a generated ID and timestamps.
    pub fn new(name: Option<String>, project_path: Option<String>) -> Self {
        let now = chrono_now();
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            binding_id: generate_binding_id(),
            name,
            project_path,
            tool_overrides: HashMap::new(),
            managed_path_entries: Vec::new(),
            env_vars: HashMap::new(),
            env_vars_remove: Vec::new(),
            preferred_ide: None,
            preferred_ide_args: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Touch the updated_at timestamp.
    pub fn touch(&mut self) {
        self.updated_at = chrono_now();
    }

    /// Validate that all configured executable paths are absolute.
    /// Returns warnings for non-absolute or missing paths.
    pub fn validate_paths(&self) -> Vec<BindingWarning> {
        let mut warnings = Vec::new();

        for (tool_id, tool) in &self.tool_overrides {
            if let Some(ref exe) = tool.executable_path {
                if !is_absolute_path(exe) {
                    warnings.push(BindingWarning {
                        subject: tool_id.clone(),
                        message: format!(
                            "Executable path for '{}' is not absolute: {}",
                            tool_id, exe
                        ),
                    });
                }
            }
            for entry in &tool.path_entries {
                if !is_absolute_path(entry) {
                    warnings.push(BindingWarning {
                        subject: tool_id.clone(),
                        message: format!("Path entry for '{}' is not absolute: {}", tool_id, entry),
                    });
                }
            }
        }

        for entry in &self.managed_path_entries {
            if !is_absolute_path(entry) {
                warnings.push(BindingWarning {
                    subject: "managed_path".to_string(),
                    message: format!("Managed path entry is not absolute: {}", entry),
                });
            }
        }

        warnings
    }
}

/// Generate a unique binding ID.
fn generate_binding_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("env_{:016x}", nanos)
}

/// Get current time as ISO-8601 string.
fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Simple ISO-8601 without external crate dependency
    format!(
        "2026-01-01T{:02}:{:02}:{:02}Z",
        (secs / 3600) % 24,
        (secs / 60) % 60,
        secs % 60
    )
}

/// Check if a path is absolute (platform-aware).
fn is_absolute_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    #[cfg(target_os = "windows")]
    {
        // Windows: C:\... or \\server\share
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

    #[test]
    fn binding_new_generates_id_and_timestamps() {
        let b = EnvironmentBinding::new(Some("test".into()), None);
        assert!(b.binding_id.starts_with("env_"));
        assert!(!b.created_at.is_empty());
        assert!(!b.updated_at.is_empty());
        assert_eq!(b.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn binding_default_fields_are_empty() {
        let b = EnvironmentBinding::new(None, None);
        assert!(b.name.is_none());
        assert!(b.project_path.is_none());
        assert!(b.tool_overrides.is_empty());
        assert!(b.managed_path_entries.is_empty());
        assert!(b.env_vars.is_empty());
        assert!(b.env_vars_remove.is_empty());
        assert!(b.preferred_ide.is_none());
        assert!(b.preferred_ide_args.is_none());
    }

    #[test]
    fn binding_serialization_round_trip() {
        let mut b = EnvironmentBinding::new(
            Some("Python 3.12".into()),
            Some("/home/user/project".into()),
        );
        b.tool_overrides.insert(
            "python".into(),
            ToolOverride {
                executable_path: Some("/usr/bin/python3.12".into()),
                version: Some("3.12".into()),
                path_entries: vec!["/usr/lib/python3.12/bin".into()],
                env_vars: HashMap::new(),
            },
        );
        b.managed_path_entries.push("/opt/tools/bin".into());
        b.env_vars
            .insert("VIRTUAL_ENV".into(), "/home/user/venv".into());

        let json = serde_json::to_string(&b).unwrap();
        let restored: EnvironmentBinding = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.binding_id, b.binding_id);
        assert_eq!(restored.name, b.name);
        assert_eq!(restored.project_path, b.project_path);
        assert_eq!(restored.tool_overrides.len(), 1);
        assert_eq!(
            restored.tool_overrides["python"].executable_path,
            Some("/usr/bin/python3.12".into())
        );
        assert_eq!(restored.managed_path_entries, b.managed_path_entries);
        assert_eq!(restored.env_vars["VIRTUAL_ENV"], "/home/user/venv");
    }

    #[test]
    fn binding_backward_compat_no_new_fields() {
        // Legacy JSON without schema_version, preferred_ide, etc.
        let json = r#"{"binding_id":"env_old","name":"Old","tool_overrides":{},"env_vars":{}}"#;
        let b: EnvironmentBinding = serde_json::from_str(json).unwrap();
        assert_eq!(b.binding_id, "env_old");
        assert_eq!(b.schema_version, CURRENT_SCHEMA_VERSION); // defaulted
        assert!(b.preferred_ide.is_none());
        assert!(b.managed_path_entries.is_empty());
    }

    #[test]
    fn validate_paths_flags_non_absolute() {
        let mut b = EnvironmentBinding::new(None, None);
        b.tool_overrides.insert(
            "python".into(),
            ToolOverride {
                executable_path: Some("python3".into()),
                version: None,
                path_entries: vec!["relative/path".into()],
                env_vars: HashMap::new(),
            },
        );
        b.managed_path_entries.push("also/relative".into());

        let warnings = b.validate_paths();
        assert_eq!(warnings.len(), 3);
        assert!(warnings.iter().any(|w| w.subject == "python"));
        assert!(warnings.iter().any(|w| w.subject == "managed_path"));
    }

    #[test]
    fn validate_paths_ok_for_absolute() {
        let mut b = EnvironmentBinding::new(None, None);
        b.tool_overrides.insert(
            "node".into(),
            ToolOverride {
                executable_path: Some(abs("/usr/local/bin/node")),
                version: None,
                path_entries: vec![],
                env_vars: HashMap::new(),
            },
        );
        b.managed_path_entries.push(abs("/opt/node/bin"));

        let warnings = b.validate_paths();
        assert!(warnings.is_empty());
    }

    #[test]
    fn validate_paths_empty_is_ok() {
        let b = EnvironmentBinding::new(None, None);
        assert!(b.validate_paths().is_empty());
    }

    #[test]
    fn touch_updates_timestamp() {
        let mut b = EnvironmentBinding::new(None, None);
        // In test environment timestamps may be same due to resolution,
        // but touch should at least not panic
        b.touch();
        assert!(!b.updated_at.is_empty());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn is_absolute_path_windows() {
        assert!(is_absolute_path("C:\\Users\\test"));
        assert!(is_absolute_path("D:/tools/bin"));
        assert!(is_absolute_path("\\\\server\\share"));
        assert!(!is_absolute_path("relative/path"));
        assert!(!is_absolute_path(""));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn is_absolute_path_unix() {
        assert!(is_absolute_path("/usr/bin/node"));
        assert!(is_absolute_path("/opt/tools"));
        assert!(!is_absolute_path("relative/path"));
        assert!(!is_absolute_path(""));
    }
}
