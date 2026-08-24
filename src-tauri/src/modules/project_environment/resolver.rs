use std::path::Path;

use crate::platform::environment::EnvironmentOverlay;

use super::models::EnvironmentBinding;

/// Resolve an `EnvironmentBinding` into an `EnvironmentOverlay`.
///
/// The overlay prepends tool-specific PATH entries (from executable_path
/// parent dirs and path_entries) before the global PATH, sets environment
/// variables, and removes specified variables.
///
/// Resolution rules:
/// - executable_path: the parent directory is prepended to PATH
/// - path_entries: each is prepended to PATH in order
/// - managed_path_entries: each is prepended to PATH in order
/// - env_vars: set in the overlay
/// - env_vars_remove: removed in the overlay
/// - No binding (None) returns an empty overlay (preserves host behavior)
pub fn resolve_binding_overlay(binding: &EnvironmentBinding) -> EnvironmentOverlay {
    let mut overlay = EnvironmentOverlay::new();

    // 1. Collect tool-specific paths
    let mut tool_paths: Vec<String> = Vec::new();

    for (_tool_id, tool) in &binding.tool_overrides {
        // Prepend parent directory of executable_path
        if let Some(ref exe) = tool.executable_path {
            if let Some(parent) = Path::new(exe).parent() {
                let parent_str = parent.to_string_lossy().into_owned();
                if !parent_str.is_empty() && is_absolute_path(&parent_str) {
                    tool_paths.push(parent_str);
                }
            }
        }
        // Prepend tool-specific path_entries
        for entry in &tool.path_entries {
            if is_absolute_path(entry) {
                tool_paths.push(entry.clone());
            }
        }
    }

    // 2. Add managed_path_entries (project-level PATH)
    for entry in &binding.managed_path_entries {
        if is_absolute_path(entry) {
            tool_paths.push(entry.clone());
        }
    }

    // 3. Build overlay with paths (tool paths before managed paths before global)
    for path in &tool_paths {
        overlay = overlay.prepend_path(path);
    }

    // 4. Set environment variables
    for (key, value) in &binding.env_vars {
        overlay = overlay.set_var(key, value);
    }

    // 5. Remove environment variables
    for key in &binding.env_vars_remove {
        overlay = overlay.remove_var(key);
    }

    overlay
}

/// Resolve a binding and return diagnostics about what was applied.
pub fn resolve_with_diagnostics(binding: &EnvironmentBinding) -> (EnvironmentOverlay, Vec<String>) {
    let mut diagnostics = Vec::new();

    // Validate paths before resolving
    let warnings = binding.validate_paths();
    for w in &warnings {
        diagnostics.push(format!("[{}] {}", w.subject, w.message));
    }

    let overlay = resolve_binding_overlay(binding);

    // Log what we're applying
    if !overlay.path_prepend.is_empty() {
        diagnostics.push(format!(
            "Prepending {} PATH entries",
            overlay.path_prepend.len()
        ));
    }
    if !overlay.vars_set.is_empty() {
        diagnostics.push(format!(
            "Setting {} environment variables",
            overlay.vars_set.len()
        ));
    }
    if !overlay.vars_remove.is_empty() {
        diagnostics.push(format!(
            "Removing {} environment variables",
            overlay.vars_remove.len()
        ));
    }

    (overlay, diagnostics)
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

    fn make_binding() -> EnvironmentBinding {
        EnvironmentBinding {
            schema_version: 1,
            binding_id: "env_test".into(),
            name: Some("Test".into()),
            project_path: None,
            tool_overrides: HashMap::new(),
            managed_path_entries: Vec::new(),
            env_vars: HashMap::new(),
            env_vars_remove: Vec::new(),
            preferred_ide: None,
            preferred_ide_args: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn no_binding_returns_empty_overlay() {
        let binding = make_binding();
        let overlay = resolve_binding_overlay(&binding);
        assert!(overlay.path_prepend.is_empty());
        assert!(overlay.vars_set.is_empty());
        assert!(overlay.vars_remove.is_empty());
    }

    #[test]
    fn executable_path_parent_prepended_to_path() {
        let mut binding = make_binding();
        binding.tool_overrides.insert(
            "python".into(),
            ToolOverride {
                executable_path: Some("/usr/local/bin/python3.12".into()),
                version: None,
                path_entries: vec![],
                env_vars: HashMap::new(),
            },
        );

        let overlay = resolve_binding_overlay(&binding);
        assert!(overlay.path_prepend.contains(&"/usr/local/bin".to_string()));
    }

    #[test]
    fn tool_path_entries_prepended() {
        let mut binding = make_binding();
        binding.tool_overrides.insert(
            "java".into(),
            ToolOverride {
                executable_path: None,
                version: None,
                path_entries: vec!["/opt/java/bin".into(), "/opt/java/lib".into()],
                env_vars: HashMap::new(),
            },
        );

        let overlay = resolve_binding_overlay(&binding);
        assert!(overlay.path_prepend.contains(&"/opt/java/bin".to_string()));
        assert!(overlay.path_prepend.contains(&"/opt/java/lib".to_string()));
    }

    #[test]
    fn managed_path_entries_prepended() {
        let mut binding = make_binding();
        binding
            .managed_path_entries
            .push("/project/.venv/bin".into());
        binding
            .managed_path_entries
            .push("/project/node_modules/.bin".into());

        let overlay = resolve_binding_overlay(&binding);
        assert!(overlay
            .path_prepend
            .contains(&"/project/.venv/bin".to_string()));
        assert!(overlay
            .path_prepend
            .contains(&"/project/node_modules/.bin".to_string()));
    }

    #[test]
    fn env_vars_are_set() {
        let mut binding = make_binding();
        binding
            .env_vars
            .insert("VIRTUAL_ENV".into(), "/home/user/venv".into());
        binding
            .env_vars
            .insert("NODE_ENV".into(), "development".into());

        let overlay = resolve_binding_overlay(&binding);
        assert_eq!(
            overlay.vars_set.get("VIRTUAL_ENV").unwrap(),
            "/home/user/venv"
        );
        assert_eq!(overlay.vars_set.get("NODE_ENV").unwrap(), "development");
    }

    #[test]
    fn env_vars_remove_are_set() {
        let mut binding = make_binding();
        binding.env_vars_remove.push("OLD_VAR".into());
        binding.env_vars_remove.push("ANOTHER".into());

        let overlay = resolve_binding_overlay(&binding);
        assert!(overlay.vars_remove.contains(&"OLD_VAR".to_string()));
        assert!(overlay.vars_remove.contains(&"ANOTHER".to_string()));
    }

    #[test]
    fn relative_executable_path_not_prepended() {
        let mut binding = make_binding();
        binding.tool_overrides.insert(
            "node".into(),
            ToolOverride {
                executable_path: Some("node".into()), // relative
                version: None,
                path_entries: vec![],
                env_vars: HashMap::new(),
            },
        );

        let overlay = resolve_binding_overlay(&binding);
        assert!(overlay.path_prepend.is_empty());
    }

    #[test]
    fn relative_path_entry_not_prepended() {
        let mut binding = make_binding();
        binding.tool_overrides.insert(
            "tool".into(),
            ToolOverride {
                executable_path: None,
                version: None,
                path_entries: vec!["relative/path".into()],
                env_vars: HashMap::new(),
            },
        );

        let overlay = resolve_binding_overlay(&binding);
        assert!(overlay.path_prepend.is_empty());
    }

    #[test]
    fn mixed_tool_and_managed_paths() {
        let mut binding = make_binding();
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
            .managed_path_entries
            .push("/project/.venv/bin".into());

        let overlay = resolve_binding_overlay(&binding);
        // Tool paths come before managed paths
        let py_pos = overlay
            .path_prepend
            .iter()
            .position(|p| p == "/usr/bin")
            .unwrap();
        let venv_pos = overlay
            .path_prepend
            .iter()
            .position(|p| p == "/project/.venv/bin")
            .unwrap();
        assert!(
            py_pos < venv_pos,
            "tool paths should come before managed paths"
        );
    }

    #[test]
    fn resolve_with_diagnostics_reports_warnings() {
        let mut binding = make_binding();
        binding.tool_overrides.insert(
            "bad".into(),
            ToolOverride {
                executable_path: Some("not_absolute".into()),
                version: None,
                path_entries: vec![],
                env_vars: HashMap::new(),
            },
        );

        let (_overlay, diagnostics) = resolve_with_diagnostics(&binding);
        assert!(diagnostics.iter().any(|d| d.contains("not absolute")));
    }

    #[test]
    fn resolve_with_diagnostics_empty_binding() {
        let binding = make_binding();
        let (overlay, diagnostics) = resolve_with_diagnostics(&binding);
        assert!(overlay.path_prepend.is_empty());
        // No path prepend or var set diagnostics for empty binding
        assert!(diagnostics
            .iter()
            .all(|d| !d.contains("Prepending") && !d.contains("Setting")));
    }

    #[test]
    fn multiple_tools_paths_in_order() {
        let mut binding = make_binding();
        binding.tool_overrides.insert(
            "python".into(),
            ToolOverride {
                executable_path: Some("/opt/python/bin/python3".into()),
                version: None,
                path_entries: vec![],
                env_vars: HashMap::new(),
            },
        );
        binding.tool_overrides.insert(
            "node".into(),
            ToolOverride {
                executable_path: Some("/opt/node/bin/node".into()),
                version: None,
                path_entries: vec![],
                env_vars: HashMap::new(),
            },
        );

        let overlay = resolve_binding_overlay(&binding);
        // Both should be present (order depends on HashMap iteration,
        // but both must be there)
        assert!(overlay
            .path_prepend
            .contains(&"/opt/python/bin".to_string()));
        assert!(overlay.path_prepend.contains(&"/opt/node/bin".to_string()));
    }
}
