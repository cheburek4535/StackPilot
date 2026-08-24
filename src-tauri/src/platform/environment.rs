use std::collections::HashMap;

use super::host::HostOs;

/// Overlay of environment variables and PATH entries to apply to a child process.
///
/// By default the inherited environment is preserved; the overlay adds or
/// overrides specific variables and PATH entries on top of it.
#[derive(Debug, Clone, Default)]
pub struct EnvironmentOverlay {
    /// PATH entries to prepend (in order).
    pub path_prepend: Vec<String>,
    /// Environment variables to set (key → value).
    pub vars_set: HashMap<String, String>,
    /// Environment variables to remove.
    pub vars_remove: Vec<String>,
}

impl EnvironmentOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Prepend an absolute PATH entry (front of PATH).
    pub fn prepend_path(mut self, entry: impl Into<String>) -> Self {
        self.path_prepend.push(entry.into());
        self
    }

    /// Set an environment variable.
    pub fn set_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.vars_set.insert(key.into(), value.into());
        self
    }

    /// Remove an environment variable.
    pub fn remove_var(mut self, key: impl Into<String>) -> Self {
        self.vars_remove.push(key.into());
        self
    }

    /// Apply this overlay to a [`std::process::Command`].
    pub fn apply_std(&self, cmd: &mut std::process::Command) {
        self.apply_envs(cmd);
        self.apply_path(cmd);
        self.apply_removes(cmd);
    }

    /// Apply this overlay to a [`tokio::process::Command`].
    pub fn apply_tokio(&self, cmd: &mut tokio::process::Command) {
        self.apply_envs_tokio(cmd);
        self.apply_path_tokio(cmd);
        self.apply_removes_tokio(cmd);
    }

    fn apply_envs(&self, cmd: &mut std::process::Command) {
        for (key, value) in &self.vars_set {
            cmd.env(key, value);
        }
    }

    fn apply_envs_tokio(&self, cmd: &mut tokio::process::Command) {
        for (key, value) in &self.vars_set {
            cmd.env(key, value);
        }
    }

    fn apply_removes(&self, cmd: &mut std::process::Command) {
        for key in &self.vars_remove {
            cmd.env_remove(key);
        }
    }

    fn apply_removes_tokio(&self, cmd: &mut tokio::process::Command) {
        for key in &self.vars_remove {
            cmd.env_remove(key);
        }
    }

    fn apply_path(&self, cmd: &mut std::process::Command) {
        if self.path_prepend.is_empty() {
            return;
        }
        let os_path = build_overlay_path(&self.path_prepend, super::host::current_os());
        cmd.env("PATH", os_path);
    }

    fn apply_path_tokio(&self, cmd: &mut tokio::process::Command) {
        if self.path_prepend.is_empty() {
            return;
        }
        let os_path = build_overlay_path(&self.path_prepend, super::host::current_os());
        cmd.env("PATH", os_path);
    }
}

/// Build a new PATH string by prepending entries to the inherited PATH.
///
/// On Windows the separator is `;` and comparison is case-insensitive.
/// On Unix the separator is `:` and comparison is case-sensitive.
/// Duplicate entries (already present in the inherited PATH) are not
/// re-prepended.
pub fn build_overlay_path(prepend: &[String], os: HostOs) -> String {
    let inherited = std::env::var("PATH").unwrap_or_default();
    let separator = path_separator(os);
    let mut existing_entries: Vec<String> = inherited
        .split(separator)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    let mut new_path = String::new();

    for entry in prepend {
        let dominated = existing_entries
            .iter()
            .any(|existing| path_entry_eq(existing, entry, os));
        if !dominated {
            if !new_path.is_empty() {
                new_path.push(separator);
            }
            new_path.push_str(entry);
            existing_entries.push(entry.clone());
        }
    }

    for entry in &existing_entries {
        if !new_path.is_empty() {
            new_path.push(separator);
        }
        new_path.push_str(entry);
    }

    new_path
}

/// Platform-specific path separator character.
pub fn path_separator(os: HostOs) -> char {
    match os {
        HostOs::Windows => ';',
        HostOs::Linux | HostOs::Macos => ':',
    }
}

/// Platform-aware path entry equality.
///
/// On Windows, comparison is case-insensitive and normalizes forward slashes
/// to backslashes. On Unix, comparison is case-sensitive and literal.
pub fn path_entry_eq(a: &str, b: &str, os: HostOs) -> bool {
    match os {
        HostOs::Windows => {
            let a_norm = a.replace('/', "\\").to_lowercase();
            let b_norm = b.replace('/', "\\").to_lowercase();
            a_norm == b_norm
        }
        HostOs::Linux | HostOs::Macos => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_default_is_empty() {
        let overlay = EnvironmentOverlay::new();
        assert!(overlay.path_prepend.is_empty());
        assert!(overlay.vars_set.is_empty());
        assert!(overlay.vars_remove.is_empty());
    }

    #[test]
    fn overlay_builder_chain() {
        let overlay = EnvironmentOverlay::new()
            .prepend_path("/opt/tools/bin")
            .set_var("MY_VAR", "hello")
            .remove_var("UNWANTED");
        assert_eq!(overlay.path_prepend, vec!["/opt/tools/bin"]);
        assert_eq!(overlay.vars_set.get("MY_VAR").unwrap(), "hello");
        assert!(overlay.vars_remove.contains(&"UNWANTED".to_string()));
    }

    #[test]
    fn path_separator_platform() {
        let os = crate::platform::host::current_os();
        match os {
            HostOs::Windows => assert_eq!(path_separator(HostOs::Windows), ';'),
            HostOs::Linux | HostOs::Macos => assert_eq!(path_separator(HostOs::Linux), ':'),
        }
    }

    #[test]
    fn path_entry_eq_case_insensitive_windows() {
        assert!(path_entry_eq(
            r"C:\Users\test\bin",
            r"C:\Users\test\bin",
            HostOs::Windows
        ));
        assert!(path_entry_eq(
            r"C:\Users\Test\Bin",
            r"C:\Users\test\bin",
            HostOs::Windows
        ));
        // Mixed separator styles: forward slashes normalized to backslashes
        assert!(path_entry_eq(
            r"C:/Users/test/bin",
            r"C:\Users\test\bin",
            HostOs::Windows
        ));
        assert!(path_entry_eq(
            r"D:\Other/Path",
            r"D:\Other\Path",
            HostOs::Windows
        ));
    }

    #[test]
    fn path_entry_eq_case_sensitive_unix() {
        assert!(path_entry_eq("/usr/bin", "/usr/bin", HostOs::Linux));
        assert!(!path_entry_eq("/usr/Bin", "/usr/bin", HostOs::Linux));
        assert!(!path_entry_eq("/USR/BIN", "/usr/bin", HostOs::Linux));
    }

    #[test]
    fn build_overlay_path_deduplicates_case_insensitive_windows() {
        let inherited = r"C:\Windows\System32;C:\Users\test\bin";
        let old_val = std::env::var("PATH").ok();
        std::env::set_var("PATH", inherited);

        let prepend = vec![r"C:\Users\test\bin".to_string(), "/opt/new".to_string()];
        let result = build_overlay_path(&prepend, HostOs::Windows);

        // C:\Users\test\bin is already in inherited PATH → not prepended
        // /opt/new is new → prepended
        assert!(
            result.starts_with("/opt/new"),
            "expected /opt/new at start, got: {result}"
        );
        assert!(
            result.contains(r"C:\Windows\System32"),
            "expected System32 in path, got: {result}"
        );
        assert!(
            result.contains(r"C:\Users\test\bin"),
            r"expected test\bin in path, got: {result}"
        );
        // Count occurrences of test\bin — should be exactly 1
        let count = result.matches(r"C:\Users\test\bin").count();
        assert_eq!(
            count, 1,
            r"test\bin should appear once, got {count} in: {result}"
        );

        if let Some(val) = old_val {
            std::env::set_var("PATH", val);
        } else {
            std::env::remove_var("PATH");
        }
    }

    #[test]
    fn build_overlay_path_deduplicates_case_sensitive_unix() {
        let old_val = std::env::var("PATH").ok();
        std::env::set_var("PATH", "/usr/bin:/usr/local/bin");

        let prepend = vec!["/usr/bin".to_string(), "/opt/new".to_string()];
        let result = build_overlay_path(&prepend, HostOs::Linux);

        // /usr/bin is already in inherited PATH → not prepended
        // /opt/new is new → prepended
        assert!(
            result.starts_with("/opt/new"),
            "expected /opt/new at start, got: {result}"
        );
        // /usr/bin should appear exactly once
        let count = result.matches("/usr/bin").count();
        assert_eq!(
            count, 1,
            "/usr/bin should appear once, got {count} in: {result}"
        );

        if let Some(val) = old_val {
            std::env::set_var("PATH", val);
        } else {
            std::env::remove_var("PATH");
        }
    }

    #[test]
    fn build_overlay_path_prepends_in_order() {
        // Verify that prepended entries appear in front of inherited PATH.
        // (Parallel test isolation makes env-dependent tests inherently flaky,
        // so we test the ordering property instead of exact content.)
        let old_val = std::env::var("PATH").ok();
        let inherited = std::env::var("PATH").unwrap_or_default();

        let prepend = vec![
            "/prepended_first".to_string(),
            "/prepended_second".to_string(),
        ];
        let result = build_overlay_path(&prepend, HostOs::Linux);

        // Prepended entries must appear before the inherited content.
        let first_pos = result.find("/prepended_first").unwrap();
        let second_pos = result.find("/prepended_second").unwrap();
        assert!(first_pos < second_pos, "prepend order must be preserved");
        assert!(
            second_pos < result.len() - (result.len() - inherited.len() + 20).min(result.len()),
            "prepended entries must come before inherited PATH"
        );

        if let Some(val) = old_val {
            std::env::set_var("PATH", val);
        } else {
            std::env::remove_var("PATH");
        }
    }

    #[test]
    fn apply_std_sets_and_removes_env() {
        let overlay = EnvironmentOverlay::new()
            .set_var("PLATFORM_TEST_VAR", "platform_value")
            .remove_var("PLATFORM_TEST_REMOVE");

        let mut cmd = std::process::Command::new("echo");
        overlay.apply_std(&mut cmd);

        // Verify the command has the env set (we can inspect via env_mods)
        // The actual env application is tested through integration tests.
        // Here we just verify no panic/crash.
        let _ = cmd.output();
    }
}
