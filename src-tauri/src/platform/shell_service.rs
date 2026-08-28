//! Shell validation and availability service.
//!
//! Validates that a requested shell is available on the current OS,
//! provides fallback options, and returns structured diagnostics
//! including the detected OS and whether automatic fallback is allowed.

use super::host::{current_os, HostOs};
use super::shell::{default_shell_for_platform, parse_shell, resolve_shell, ShellKind};
use std::fmt;

/// Result of validating shell availability.
#[derive(Debug, Clone)]
pub struct ShellValidationResult {
    /// The shell that was requested.
    pub requested: ShellKind,
    /// The shell that will actually be used (after resolution/fallback).
    pub effective: ShellKind,
    /// The detected host OS.
    pub detected_os: HostOs,
    /// Available fallback shells on this OS.
    pub fallback_options: Vec<ShellKind>,
    /// Whether automatic fallback to another shell is permitted.
    pub auto_fallback_allowed: bool,
    /// Diagnostic messages collected during validation.
    pub diagnostics: Vec<ShellDiagnostic>,
}

/// Severity of a shell diagnostic message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

/// A structured diagnostic from shell validation.
#[derive(Debug, Clone)]
pub struct ShellDiagnostic {
    pub severity: ShellDiagnosticSeverity,
    pub message: String,
}

impl fmt::Display for ShellValidationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Shell validation: requested={}, effective={}, OS={}, fallbacks={:?}",
            self.requested, self.effective, self.detected_os, self.fallback_options
        )
    }
}

/// Validate that a shell is available on the current OS and return
/// a complete diagnostic result.
///
/// This is the authoritative shell validation function. It checks:
/// - OS compatibility (e.g. `cmd` on Unix is an error).
/// - Whether the shell binary is actually on PATH.
/// - What fallback options exist.
/// - Whether automatic fallback is allowed.
pub fn validate_shell(shell: ShellKind) -> ShellValidationResult {
    let os = current_os();
    let resolved = resolve_shell(shell);
    let fallback_options = available_shells_for_os(os);
    let compatible = is_shell_compatible(resolved, os);
    let available_on_path = is_shell_on_path(resolved);
    let mut diagnostics = Vec::new();

    // Check OS compatibility.
    if !compatible {
        diagnostics.push(ShellDiagnostic {
            severity: ShellDiagnosticSeverity::Error,
            message: format!(
                "Shell '{}' is not available on {}. Available shells: {}",
                resolved,
                os,
                fallback_options
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }

    // Check PATH availability.
    if compatible && !available_on_path {
        diagnostics.push(ShellDiagnostic {
            severity: ShellDiagnosticSeverity::Warning,
            message: format!(
                "Shell '{}' is compatible with {} but was not found on PATH. \
                 It may need to be installed.",
                resolved, os
            ),
        });
    }

    // Determine effective shell.
    let effective = if compatible && available_on_path {
        resolved
    } else if compatible {
        // Compatible but not on PATH — use platform default as fallback.
        let fallback = default_shell_for_platform();
        diagnostics.push(ShellDiagnostic {
            severity: ShellDiagnosticSeverity::Warning,
            message: format!(
                "Falling back from '{}' to '{}' (platform default).",
                resolved, fallback
            ),
        });
        fallback
    } else {
        // Incompatible — use platform default.
        let fallback = default_shell_for_platform();
        diagnostics.push(ShellDiagnostic {
            severity: ShellDiagnosticSeverity::Info,
            message: format!(
                "Auto-fallback: using '{}' instead of incompatible '{}'.",
                fallback, resolved
            ),
        });
        fallback
    };

    // Auto-fallback is allowed for Default and when the requested shell
    // is compatible but not found.
    let auto_fallback_allowed =
        matches!(shell, ShellKind::Default) || (compatible && !available_on_path);

    ShellValidationResult {
        requested: shell,
        effective,
        detected_os: os,
        fallback_options,
        auto_fallback_allowed,
        diagnostics,
    }
}

/// Check if a shell is compatible with the given OS.
pub fn is_shell_compatible(shell: ShellKind, os: HostOs) -> bool {
    match (shell, os) {
        (ShellKind::Cmd | ShellKind::PowerShell, HostOs::Linux | HostOs::Macos) => false,
        (ShellKind::Sh | ShellKind::Bash | ShellKind::Zsh, HostOs::Windows) => false,
        _ => true,
    }
}

/// Check if a shell binary is available on the system PATH.
pub fn is_shell_on_path(shell: ShellKind) -> bool {
    let (exe, _) = super::shell::shell_executable(shell);
    which::which(exe).is_ok()
}

/// List all shells that are compatible with and potentially available
/// on the given OS.
pub fn available_shells_for_os(os: HostOs) -> Vec<ShellKind> {
    match os {
        HostOs::Windows => vec![ShellKind::Cmd, ShellKind::PowerShell, ShellKind::Pwsh],
        HostOs::Linux | HostOs::Macos => vec![
            ShellKind::Sh,
            ShellKind::Bash,
            ShellKind::Zsh,
            ShellKind::Pwsh,
        ],
    }
}

/// Validate a shell string (as used in step definitions) and return
/// the effective shell or an error.
pub fn validate_shell_string(shell_str: &str) -> Result<ShellKind, ShellValidationError> {
    let parsed = parse_shell_string(shell_str)?;
    let result = validate_shell(parsed);

    if !result.diagnostics.is_empty()
        && result
            .diagnostics
            .iter()
            .any(|d| d.severity == ShellDiagnosticSeverity::Error)
    {
        return Err(ShellValidationError {
            requested: shell_str.to_string(),
            effective: result.effective,
            message: result
                .diagnostics
                .iter()
                .filter(|d| d.severity == ShellDiagnosticSeverity::Error)
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("; "),
        });
    }

    Ok(result.effective)
}

/// Parse a shell string into a typed [`ShellKind`].
fn parse_shell_string(s: &str) -> Result<ShellKind, ShellValidationError> {
    parse_shell(s).map_err(|e| ShellValidationError {
        requested: s.to_string(),
        effective: default_shell_for_platform(),
        message: e.to_string(),
    })
}

/// Error returned when a shell string is invalid or unavailable.
#[derive(Debug, Clone)]
pub struct ShellValidationError {
    pub requested: String,
    pub effective: ShellKind,
    pub message: String,
}

impl fmt::Display for ShellValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Shell '{}' is invalid or unavailable: {}",
            self.requested, self.message
        )
    }
}

impl std::error::Error for ShellValidationError {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_shell_default_returns_platform_default() {
        let result = validate_shell(ShellKind::Default);
        assert_eq!(result.detected_os, current_os());
        assert!(!result.fallback_options.is_empty());
    }

    #[test]
    fn validate_shell_cmd_on_windows() {
        if cfg!(target_os = "windows") {
            let result = validate_shell(ShellKind::Cmd);
            assert_eq!(result.effective, ShellKind::Cmd);
            assert!(result.diagnostics.is_empty());
        }
    }

    #[test]
    fn validate_shell_compatible_check() {
        assert!(is_shell_compatible(ShellKind::Cmd, HostOs::Windows));
        assert!(!is_shell_compatible(ShellKind::Cmd, HostOs::Linux));
        assert!(!is_shell_compatible(ShellKind::Cmd, HostOs::Macos));
        assert!(!is_shell_compatible(ShellKind::Sh, HostOs::Windows));
        assert!(is_shell_compatible(ShellKind::Sh, HostOs::Linux));
        assert!(is_shell_compatible(ShellKind::Sh, HostOs::Macos));
        // pwsh is cross-platform
        assert!(is_shell_compatible(ShellKind::Pwsh, HostOs::Windows));
        assert!(is_shell_compatible(ShellKind::Pwsh, HostOs::Linux));
        assert!(is_shell_compatible(ShellKind::Pwsh, HostOs::Macos));
    }

    #[test]
    fn available_shells_windows() {
        let shells = available_shells_for_os(HostOs::Windows);
        assert!(shells.contains(&ShellKind::Cmd));
        assert!(shells.contains(&ShellKind::PowerShell));
        assert!(shells.contains(&ShellKind::Pwsh));
    }

    #[test]
    fn available_shells_unix() {
        let shells = available_shells_for_os(HostOs::Linux);
        assert!(shells.contains(&ShellKind::Sh));
        assert!(shells.contains(&ShellKind::Bash));
        assert!(shells.contains(&ShellKind::Zsh));
        assert!(shells.contains(&ShellKind::Pwsh));
    }

    #[test]
    fn validate_shell_string_valid() {
        let result = validate_shell_string("sh");
        assert!(result.is_ok());
    }

    #[test]
    fn validate_shell_string_invalid() {
        let result = validate_shell_string("fish");
        assert!(result.is_err());
    }

    #[test]
    fn validate_shell_string_empty() {
        let result = validate_shell_string("");
        assert!(result.is_ok()); // Empty resolves to Default
    }

    #[test]
    fn validation_result_display() {
        let result = validate_shell(ShellKind::Default);
        let display = format!("{}", result);
        assert!(display.contains("Shell validation"));
    }
}
