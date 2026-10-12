use std::fmt;

use super::host::{current_os, HostOs};

/// Typed representation of available shells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShellKind {
    /// Resolves to `cmd` on Windows, `/bin/sh` on Unix.
    Default,
    Sh,
    Bash,
    Zsh,
    /// fish (Unix)
    Fish,
    Cmd,
    PowerShell,
    /// Cross-platform PowerShell (pwsh) — allowed on any OS when available.
    Pwsh,
}

/// Error returned when a shell string cannot be parsed into a [`ShellKind`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedShell(pub String);

impl fmt::Display for UnsupportedShell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unsupported shell: '{}'. valid values: sh, bash, zsh, fish, cmd, powershell, pwsh",
            self.0
        )
    }
}

impl std::error::Error for UnsupportedShell {}

impl fmt::Display for ShellKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShellKind::Default => write!(f, "default"),
            ShellKind::Sh => write!(f, "sh"),
            ShellKind::Bash => write!(f, "bash"),
            ShellKind::Zsh => write!(f, "zsh"),
            ShellKind::Fish => write!(f, "fish"),
            ShellKind::Cmd => write!(f, "cmd"),
            ShellKind::PowerShell => write!(f, "powershell"),
            ShellKind::Pwsh => write!(f, "pwsh"),
        }
    }
}

/// Parse a legacy shell string into a typed [`ShellKind`].
///
/// Returns `Err(UnsupportedShell)` for unrecognized values.
pub fn parse_shell(s: &str) -> Result<ShellKind, UnsupportedShell> {
    match s.trim().to_lowercase().as_str() {
        "" | "default" => Ok(ShellKind::Default),
        "sh" => Ok(ShellKind::Sh),
        "bash" => Ok(ShellKind::Bash),
        "zsh" => Ok(ShellKind::Zsh),
        "fish" => Ok(ShellKind::Fish),
        "cmd" | "cmd.exe" => Ok(ShellKind::Cmd),
        "powershell" | "pwsh.exe" => Ok(ShellKind::PowerShell),
        "pwsh" => Ok(ShellKind::Pwsh),
        other => Err(UnsupportedShell(other.to_string())),
    }
}

/// Resolve [`ShellKind::Default`] to the platform-native default shell.
///
/// On Windows returns `ShellKind::Cmd`, on Unix returns `ShellKind::Sh`.
pub fn default_shell_for_platform() -> ShellKind {
    match current_os() {
        HostOs::Windows => ShellKind::Cmd,
        HostOs::Linux | HostOs::Macos => ShellKind::Sh,
    }
}

/// Resolve any [`ShellKind`] to a concrete shell.
///
/// `ShellKind::Default` is resolved via [`default_shell_for_platform`].
pub fn resolve_shell(shell: ShellKind) -> ShellKind {
    match shell {
        ShellKind::Default => default_shell_for_platform(),
        other => other,
    }
}

/// Returns the executable name and base flag for a resolved shell.
///
/// `Cmd` → `("cmd", "/C")` — `/C` executes the following command string and
/// exits. This must be a single argument: cmd.exe rejects combined flags
/// like `/D /C` when passed as one quoted argument, so the caller builds
/// `cmd /C <command>` with separate args.
/// `PowerShell` → `("powershell", "-Command")`.
/// `Pwsh` → `("pwsh", "-Command")`.
/// Unix shells → `(name, "-c")` on Linux and `(name, "-lc")` on macOS.
///
/// The login flag (`-l`) is deliberately NOT used on Linux: the distro
/// `/etc/profile` scripts (Debian/Ubuntu in particular) *overwrite* `PATH`,
/// which silently discarded the environment overlay's prepended toolchain
/// paths for every captured command, script and visible step. macOS keeps
/// `-lc` because GUI-launched apps inherit a minimal `PATH` there and the
/// login profile (`path_helper`) is what makes Homebrew tools resolvable.
pub fn shell_executable(shell: ShellKind) -> (&'static str, &'static str) {
    match shell {
        ShellKind::Cmd => ("cmd", "/C"),
        ShellKind::PowerShell => ("powershell", "-Command"),
        ShellKind::Pwsh => ("pwsh", "-Command"),
        ShellKind::Sh => ("sh", unix_shell_flag()),
        ShellKind::Bash => ("bash", unix_shell_flag()),
        ShellKind::Zsh => ("zsh", unix_shell_flag()),
        ShellKind::Fish => ("fish", unix_shell_flag()),
        ShellKind::Default => {
            let resolved = default_shell_for_platform();
            shell_executable(resolved)
        }
    }
}

/// Command flag for non-interactive Unix shells: `-lc` (login) on macOS,
/// `-c` on Linux (see [`shell_executable`]).
fn unix_shell_flag() -> &'static str {
    if cfg!(target_os = "macos") {
        "-lc"
    } else {
        "-c"
    }
}

/// Check if the given shell is Windows-only.
pub fn is_windows_only_shell(shell: ShellKind) -> bool {
    matches!(shell, ShellKind::Cmd | ShellKind::PowerShell)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_shell_default_variants() {
        assert_eq!(parse_shell("").unwrap(), ShellKind::Default);
        assert_eq!(parse_shell("default").unwrap(), ShellKind::Default);
    }

    #[test]
    fn parse_shell_known_shells() {
        assert_eq!(parse_shell("sh").unwrap(), ShellKind::Sh);
        assert_eq!(parse_shell("bash").unwrap(), ShellKind::Bash);
        assert_eq!(parse_shell("zsh").unwrap(), ShellKind::Zsh);
        assert_eq!(parse_shell("fish").unwrap(), ShellKind::Fish);
        assert_eq!(parse_shell("cmd").unwrap(), ShellKind::Cmd);
        assert_eq!(parse_shell("cmd.exe").unwrap(), ShellKind::Cmd);
        assert_eq!(parse_shell("powershell").unwrap(), ShellKind::PowerShell);
        assert_eq!(parse_shell("pwsh.exe").unwrap(), ShellKind::PowerShell);
        assert_eq!(parse_shell("pwsh").unwrap(), ShellKind::Pwsh);
    }

    #[test]
    fn parse_shell_case_insensitive() {
        assert_eq!(parse_shell("CMD").unwrap(), ShellKind::Cmd);
        assert_eq!(parse_shell("PwSh").unwrap(), ShellKind::Pwsh);
        assert_eq!(parse_shell("  Bash  ").unwrap(), ShellKind::Bash);
    }

    #[test]
    fn parse_shell_unsupported() {
        let err = parse_shell("nu").unwrap_err();
        assert_eq!(err, UnsupportedShell("nu".to_string()));
        assert!(err.to_string().contains("nu"));

        let err = parse_shell("zsh-plus").unwrap_err();
        assert_eq!(err.0, "zsh-plus");
    }

    #[test]
    fn shell_default_resolves_to_platform_native() {
        let resolved = default_shell_for_platform();
        let os = current_os();
        match os {
            HostOs::Windows => assert_eq!(resolved, ShellKind::Cmd),
            HostOs::Linux | HostOs::Macos => assert_eq!(resolved, ShellKind::Sh),
        }
    }

    #[test]
    fn resolve_shell_passes_explicit_through() {
        assert_eq!(resolve_shell(ShellKind::Bash), ShellKind::Bash);
        assert_eq!(resolve_shell(ShellKind::Cmd), ShellKind::Cmd);
        assert_eq!(resolve_shell(ShellKind::PowerShell), ShellKind::PowerShell);
    }

    #[test]
    fn resolve_shell_default_resolves() {
        let resolved = resolve_shell(ShellKind::Default);
        assert_eq!(resolved, default_shell_for_platform());
    }

    #[test]
    fn shell_executable_returns_correct_pairs() {
        let (exe, flag) = shell_executable(ShellKind::Cmd);
        assert_eq!(exe, "cmd");
        assert_eq!(flag, "/C");

        let (exe, flag) = shell_executable(ShellKind::PowerShell);
        assert_eq!(exe, "powershell");
        assert_eq!(flag, "-Command");

        let (exe, flag) = shell_executable(ShellKind::Pwsh);
        assert_eq!(exe, "pwsh");
        assert_eq!(flag, "-Command");

        let (exe, flag) = shell_executable(ShellKind::Sh);
        assert_eq!(exe, "sh");
        assert_eq!(flag, unix_shell_flag());

        let (exe, flag) = shell_executable(ShellKind::Bash);
        assert_eq!(exe, "bash");
        assert_eq!(flag, unix_shell_flag());

        let (exe, flag) = shell_executable(ShellKind::Zsh);
        assert_eq!(exe, "zsh");
        assert_eq!(flag, unix_shell_flag());

        let (exe, flag) = shell_executable(ShellKind::Fish);
        assert_eq!(exe, "fish");
        assert_eq!(flag, unix_shell_flag());
    }

    /// The login flag must not be used on Linux: distro profile scripts
    /// overwrite PATH and silently drop the environment overlay's prepended
    /// toolchain paths. macOS keeps `-lc` (GUI apps need path_helper).
    #[test]
    fn unix_shell_flag_matches_platform() {
        if cfg!(target_os = "macos") {
            assert_eq!(unix_shell_flag(), "-lc");
        } else {
            assert_eq!(unix_shell_flag(), "-c");
        }
    }

    #[test]
    fn shell_executable_default_matches_platform() {
        let (exe, _) = shell_executable(ShellKind::Default);
        let os = current_os();
        match os {
            HostOs::Windows => assert_eq!(exe, "cmd"),
            HostOs::Linux | HostOs::Macos => assert_eq!(exe, "sh"),
        }
    }

    #[test]
    fn windows_only_shell_detection() {
        assert!(is_windows_only_shell(ShellKind::Cmd));
        assert!(is_windows_only_shell(ShellKind::PowerShell));
        assert!(!is_windows_only_shell(ShellKind::Sh));
        assert!(!is_windows_only_shell(ShellKind::Bash));
        assert!(!is_windows_only_shell(ShellKind::Zsh));
        assert!(!is_windows_only_shell(ShellKind::Pwsh));
    }

    #[test]
    fn explicit_cmd_does_not_become_sh_on_unix() {
        // Even on Unix, explicit Cmd stays Cmd.
        let resolved = resolve_shell(ShellKind::Cmd);
        assert_eq!(resolved, ShellKind::Cmd);
    }

    #[test]
    fn explicit_sh_does_not_become_cmd_on_windows() {
        let resolved = resolve_shell(ShellKind::Sh);
        assert_eq!(resolved, ShellKind::Sh);
    }
}
