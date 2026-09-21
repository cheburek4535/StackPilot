use std::collections::HashMap;

use super::host::HostOs;

/// Overlay of environment variables and PATH entries to apply to a child process.
///
/// By default the inherited environment is preserved; the overlay adds or
/// overrides specific variables and PATH entries on top of it.
///
/// Environment overlays are execution-scoped: they apply to a single
/// command execution (hidden process, visible terminal, script, or
/// application launch where supported). The same overlay instance can
/// be reused across multiple executions for consistency.
#[derive(Debug, Clone, Default)]
pub struct EnvironmentOverlay {
    /// PATH entries to prepend (in order).
    pub path_prepend: Vec<String>,
    /// Environment variables to set (key → value).
    pub vars_set: HashMap<String, String>,
    /// Environment variables to remove.
    pub vars_remove: Vec<String>,
    /// Keys whose values should be redacted in diagnostic output.
    redact_keys: Vec<String>,
}

/// Patterns that indicate a variable likely holds a secret.
const SECRET_PATTERNS: &[&str] = &[
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "API_KEY",
    "APIKEY",
    "PRIVATE_KEY",
    "CREDENTIAL",
    "AUTH",
    "ACCESS_KEY",
    "SECRET_KEY",
    "DATABASE_URL",
    "REDIS_URL",
    "MONGO_URL",
    "AMQP_URL",
];

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
        let key = key.into();
        if is_likely_secret(&key) {
            self.redact_keys.push(key.clone());
        }
        self.vars_set.insert(key, value.into());
        self
    }

    /// Remove an environment variable.
    pub fn remove_var(mut self, key: impl Into<String>) -> Self {
        self.vars_remove.push(key.into());
        self
    }

    /// Mark a key for redaction in diagnostic output.
    pub fn redact_key(mut self, key: impl Into<String>) -> Self {
        self.redact_keys.push(key.into());
        self
    }

    /// Returns true if the overlay has any entries that need applying.
    pub fn is_empty(&self) -> bool {
        self.path_prepend.is_empty() && self.vars_set.is_empty() && self.vars_remove.is_empty()
    }

    /// Get a redacted copy of the environment variables for diagnostics.
    /// Secret values are replaced with `[REDACTED]`.
    pub fn redacted_vars(&self) -> HashMap<String, String> {
        self.vars_set
            .iter()
            .map(|(k, v)| {
                if self.redact_keys.contains(k) || is_likely_secret(k) {
                    (k.clone(), "[REDACTED]".to_string())
                } else {
                    (k.clone(), v.clone())
                }
            })
            .collect()
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

/// Check if an environment key name likely holds a secret value.
pub fn is_likely_secret(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    SECRET_PATTERNS.iter().any(|pat| upper.contains(pat))
}

/// Build a new PATH string by prepending entries to the inherited PATH.
///
/// On Windows the separator is `;` and comparison is case-insensitive.
/// On Unix the separator is `:` and comparison is case-sensitive.
/// Duplicate entries (already present in the inherited PATH) are not
/// re-prepended.
pub fn build_overlay_path(prepend: &[String], os: HostOs) -> String {
    let inherited = std::env::var("PATH").unwrap_or_default();
    build_overlay_path_with_inherited(prepend, &inherited, os)
}

/// Testable core of [`build_overlay_path`]: the inherited PATH is an explicit
/// argument, so tests never touch (and never race) the process environment.
pub fn build_overlay_path_with_inherited(
    prepend: &[String],
    inherited: &str,
    os: HostOs,
) -> String {
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
        assert!(overlay.is_empty());
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
        assert!(!overlay.is_empty());
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
        let prepend = vec![r"C:\Users\test\bin".to_string(), "/opt/new".to_string()];
        let result = build_overlay_path_with_inherited(&prepend, inherited, HostOs::Windows);

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
    }

    #[test]
    fn build_overlay_path_deduplicates_case_sensitive_unix() {
        let prepend = vec!["/usr/bin".to_string(), "/opt/new".to_string()];
        let result = build_overlay_path_with_inherited(
            &prepend,
            "/usr/bin:/usr/local/bin",
            HostOs::Linux,
        );

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
    }

    #[test]
    fn build_overlay_path_prepends_in_order() {
        // Prepended entries must appear in front of the inherited PATH, in
        // order. Uses the pure core so parallel tests mutating PATH (or
        // reading it) cannot make this flaky.
        let prepend = vec![
            "/prepended_first".to_string(),
            "/prepended_second".to_string(),
        ];
        let result = build_overlay_path_with_inherited(
            &prepend,
            "/usr/bin:/bin",
            HostOs::Linux,
        );

        assert!(
            result.starts_with("/prepended_first:/prepended_second:/usr/bin:/bin"),
            "prepended entries must come first, in order; got: {result}"
        );
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

    // ========================================================================
    // Secret redaction tests
    // ========================================================================

    #[test]
    fn is_likely_secret_detection() {
        assert!(is_likely_secret("DATABASE_URL"));
        assert!(is_likely_secret("API_KEY"));
        assert!(is_likely_secret("SECRET_TOKEN"));
        assert!(is_likely_secret("MY_PASSWORD"));
        assert!(is_likely_secret("AWS_SECRET_ACCESS_KEY"));
        assert!(!is_likely_secret("PORT"));
        assert!(!is_likely_secret("NODE_ENV"));
        assert!(!is_likely_secret("MY_VAR"));
    }

    #[test]
    fn set_var_auto_redacts_secrets() {
        let overlay = EnvironmentOverlay::new()
            .set_var("DATABASE_URL", "postgres://user:pass@localhost/db")
            .set_var("PORT", "3000");

        assert!(overlay.redact_keys.contains(&"DATABASE_URL".to_string()));
        assert!(!overlay.redact_keys.contains(&"PORT".to_string()));
    }

    #[test]
    fn redacted_vars_masks_secret_values() {
        let overlay = EnvironmentOverlay::new()
            .set_var("DATABASE_URL", "postgres://user:pass@localhost/db")
            .set_var("PORT", "3000");

        let redacted = overlay.redacted_vars();
        assert_eq!(redacted.get("DATABASE_URL").unwrap(), "[REDACTED]");
        assert_eq!(redacted.get("PORT").unwrap(), "3000");
    }

    #[test]
    fn explicit_redact_key() {
        let overlay = EnvironmentOverlay::new()
            .set_var("CUSTOM_SECRET", "value")
            .redact_key("CUSTOM_SECRET");

        let redacted = overlay.redacted_vars();
        assert_eq!(redacted.get("CUSTOM_SECRET").unwrap(), "[REDACTED]");
    }

    #[test]
    fn empty_overlay_is_empty() {
        let overlay = EnvironmentOverlay::new();
        assert!(overlay.is_empty());
    }

    #[test]
    fn non_empty_overlay_is_not_empty() {
        let overlay = EnvironmentOverlay::new().set_var("A", "1");
        assert!(!overlay.is_empty());
    }
}
