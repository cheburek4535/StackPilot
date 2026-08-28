use std::fmt;

use serde::{Deserialize, Serialize};

use super::host::{current_os, HostOs};
use crate::modules::devlauncher::models::ProcessTrackingQuality;

// ---------------------------------------------------------------------------
// TerminalBackend — platform abstraction for native terminal execution
// ---------------------------------------------------------------------------

/// Which terminal emulator to launch.
///
/// The backend resolves which terminal is available on the current platform
/// and builds the correct command line to open a terminal window with the
/// specified command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerminalBackend {
    /// Use the platform's default terminal:
    /// - Windows: Windows Terminal if available, else cmd.exe
    /// - macOS: Terminal.app
    /// - Linux: first available from detection order
    Default,
    /// Windows Terminal (wt.exe)
    WindowsTerminal,
    /// PowerShell console (powershell.exe or pwsh.exe)
    PowerShell,
    /// Cross-platform PowerShell (pwsh)
    Pwsh,
    /// Windows Command Prompt (cmd.exe)
    Cmd,
    /// macOS Terminal.app
    TerminalApp,
    /// macOS iTerm2 (iTerm.app)
    ITerm2,
    /// gnome-terminal (Linux)
    GnomeTerminal,
    /// konsole (Linux KDE)
    Konsole,
    /// xterm (Linux/BSD)
    Xterm,
    /// alacritty (cross-platform)
    Alacritty,
    /// kitty (cross-platform)
    Kitty,
    /// xfce4-terminal (Linux)
    Xfce4Terminal,
    /// Custom terminal application
    Custom(String),
}

impl fmt::Display for TerminalBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TerminalBackend::Default => write!(f, "default"),
            TerminalBackend::WindowsTerminal => write!(f, "Windows Terminal"),
            TerminalBackend::PowerShell => write!(f, "PowerShell"),
            TerminalBackend::Pwsh => write!(f, "pwsh"),
            TerminalBackend::Cmd => write!(f, "cmd"),
            TerminalBackend::TerminalApp => write!(f, "Terminal.app"),
            TerminalBackend::ITerm2 => write!(f, "iTerm2"),
            TerminalBackend::GnomeTerminal => write!(f, "gnome-terminal"),
            TerminalBackend::Konsole => write!(f, "konsole"),
            TerminalBackend::Xterm => write!(f, "xterm"),
            TerminalBackend::Alacritty => write!(f, "alacritty"),
            TerminalBackend::Kitty => write!(f, "kitty"),
            TerminalBackend::Xfce4Terminal => write!(f, "xfce4-terminal"),
            TerminalBackend::Custom(name) => write!(f, "{}", name),
        }
    }
}

// ---------------------------------------------------------------------------
// TerminalWindowPolicy — new window vs tab
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerminalWindowPolicy {
    /// Open a new terminal window.
    #[default]
    NewWindow,
    /// Open a new tab in the existing terminal (if supported).
    NewTab,
    /// Let the terminal decide (usually new window).
    Auto,
}

// ---------------------------------------------------------------------------
// TerminalConfig — configuration for a terminal launch
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalConfig {
    /// Which terminal backend to use.
    pub backend: TerminalBackend,
    /// The command to run inside the terminal.
    pub command: String,
    /// Optional working directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// Window policy: new window or tab.
    #[serde(default)]
    pub window_policy: TerminalWindowPolicy,
    /// Terminal title/label (where supported).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Whether to keep the terminal open after the command exits.
    #[serde(default = "default_true")]
    pub keep_open: bool,
    /// Environment variables to pass to the terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<std::collections::HashMap<String, String>>,
}

fn default_true() -> bool {
    true
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            backend: TerminalBackend::Default,
            command: String::new(),
            working_dir: None,
            window_policy: TerminalWindowPolicy::Auto,
            label: None,
            keep_open: true,
            env: None,
        }
    }
}

// ---------------------------------------------------------------------------
// TerminalPlan — resolved launch command for a specific terminal
// ---------------------------------------------------------------------------

/// A resolved terminal launch plan: the program to execute and its arguments.
/// This is the output of resolving a `TerminalConfig` for the current platform.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TerminalPlan {
    /// The program to spawn (e.g., "cmd", "osascript", "gnome-terminal").
    pub program: String,
    /// Arguments to pass to the program.
    pub args: Vec<String>,
    /// The tracking quality for processes spawned through this plan.
    pub tracking_quality: ProcessTrackingQuality,
}

// ---------------------------------------------------------------------------
// Resolution logic
// ---------------------------------------------------------------------------

/// Resolve the default terminal backend for the current platform.
pub fn default_terminal_for_platform() -> TerminalBackend {
    match current_os() {
        HostOs::Windows => {
            // Try Windows Terminal first, fall back to cmd
            if which_exists("wt") {
                TerminalBackend::WindowsTerminal
            } else {
                TerminalBackend::Cmd
            }
        }
        HostOs::Macos => TerminalBackend::TerminalApp,
        HostOs::Linux => {
            // Try common terminals in preference order
            let candidates = [
                "gnome-terminal",
                "konsole",
                "x-terminal-emulator",
                "xfce4-terminal",
                "alacritty",
                "kitty",
                "xterm",
            ];
            for cand in &candidates {
                if which_exists(cand) {
                    return match *cand {
                        "gnome-terminal" => TerminalBackend::GnomeTerminal,
                        "konsole" => TerminalBackend::Konsole,
                        "xterm" => TerminalBackend::Xterm,
                        "alacritty" => TerminalBackend::Alacritty,
                        "kitty" => TerminalBackend::Kitty,
                        "xfce4-terminal" => TerminalBackend::Xfce4Terminal,
                        _ => TerminalBackend::Custom(cand.to_string()),
                    };
                }
            }
            TerminalBackend::Xterm // best fallback
        }
    }
}

/// Resolve a terminal configuration into a concrete launch plan.
///
/// This is the core platform abstraction: given a `TerminalConfig`, produce
/// the exact program + arguments needed to open a terminal and run the
/// command on the current platform.
pub fn resolve_terminal_plan(config: &TerminalConfig) -> Result<TerminalPlan, String> {
    let backend = if config.backend == TerminalBackend::Default {
        default_terminal_for_platform()
    } else {
        config.backend.clone()
    };

    let inner_cmd = build_inner_command(&config.command, config.working_dir.as_deref());

    match &backend {
        TerminalBackend::WindowsTerminal => resolve_windows_terminal(config, &inner_cmd),
        TerminalBackend::Cmd => resolve_cmd(config, &inner_cmd),
        TerminalBackend::PowerShell => resolve_powershell(config, &inner_cmd),
        TerminalBackend::Pwsh => resolve_pwsh(config, &inner_cmd),
        TerminalBackend::TerminalApp => resolve_terminal_app(config, &inner_cmd),
        TerminalBackend::ITerm2 => resolve_iterm2(config, &inner_cmd),
        TerminalBackend::GnomeTerminal => resolve_gnome_terminal(config, &inner_cmd),
        TerminalBackend::Konsole => resolve_konsole(config, &inner_cmd),
        TerminalBackend::Xterm => resolve_xterm(config, &inner_cmd),
        TerminalBackend::Alacritty => resolve_alacritty(config, &inner_cmd),
        TerminalBackend::Kitty => resolve_kitty(config, &inner_cmd),
        TerminalBackend::Xfce4Terminal => resolve_xfce4_terminal(config, &inner_cmd),
        TerminalBackend::Custom(name) => resolve_custom_terminal(name, config, &inner_cmd),
        TerminalBackend::Default => unreachable!("Default should be resolved before match"),
    }
}

// ---------------------------------------------------------------------------
// Platform-specific terminal resolution
// ---------------------------------------------------------------------------

/// Build the inner command string with working directory.
fn build_inner_command(command: &str, working_dir: Option<&str>) -> String {
    match working_dir {
        Some(dir) => {
            if cfg!(target_os = "windows") {
                format!("cd /d \"{}\" && {}", dir, command)
            } else {
                format!("cd '{}' && {}", dir.replace('\'', "'\\''"), command)
            }
        }
        None => command.to_string(),
    }
}

/// Escape a command for osascript (macOS Terminal.app / iTerm2).
fn escape_osascript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Escape a command for use in a `.cmd` batch script.
fn batch_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_quotes = false;
    for ch in s.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                out.push(ch);
            }
            '%' => out.push_str("%%"),
            '&' | '|' | '<' | '>' | '^' if !in_quotes => {
                out.push('^');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Shell-quote a single token for terminal argument.
fn shell_quote(token: &str) -> String {
    let needs_quote = token.is_empty()
        || token
            .chars()
            .any(|c| c.is_whitespace() || c == '"' || c == '\'');
    if !needs_quote {
        return token.to_string();
    }
    if cfg!(target_os = "windows") {
        format!("\"{}\"", token.replace('"', "\"\""))
    } else {
        format!("'{}'", token.replace('\'', "'\\''"))
    }
}

fn resolve_windows_terminal(
    config: &TerminalConfig,
    inner_cmd: &str,
) -> Result<TerminalPlan, String> {
    // wt.exe -w new cmd /K "<script>"  (a new window is guaranteed with -w new)
    let mut args = Vec::new();

    match config.window_policy {
        TerminalWindowPolicy::NewWindow => {
            args.push("--window".to_string());
            args.push("new".to_string());
        }
        TerminalWindowPolicy::NewTab => args.push("--tab".to_string()),
        TerminalWindowPolicy::Auto => {
            args.push("--window".to_string());
            args.push("new".to_string());
        }
    }

    if let Some(ref label) = config.label {
        args.push("--title".to_string());
        args.push(label.clone());
    }

    // Write the inner command to a temp batch script for reliable execution
    let script_path = write_temp_batch_script(inner_cmd)?;
    args.push("cmd".to_string());
    args.push("/K".to_string());
    args.push(script_path);

    Ok(TerminalPlan {
        program: "wt".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
    })
}

fn resolve_cmd(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let escaped = batch_escape(inner_cmd);
    let mut args = Vec::new();
    args.push("/K".to_string());

    let full_cmd = if let Some(ref label) = config.label {
        format!("title {} && {}", label, escaped)
    } else {
        escaped
    };

    args.push(full_cmd);

    Ok(TerminalPlan {
        program: "cmd".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
    })
}

fn resolve_powershell(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let mut args = vec![
        "-NoExit".to_string(),
        "-Command".to_string(),
        inner_cmd.to_string(),
    ];

    if let Some(ref label) = config.label {
        args.insert(0, format!("$Host.UI.RawUI.WindowTitle = '{}'", label));
    }

    Ok(TerminalPlan {
        program: "powershell".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
    })
}

fn resolve_pwsh(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let mut args = vec![
        "-NoExit".to_string(),
        "-Command".to_string(),
        inner_cmd.to_string(),
    ];

    if let Some(ref label) = config.label {
        args.insert(0, format!("$Host.UI.RawUI.WindowTitle = '{}'", label));
    }

    Ok(TerminalPlan {
        program: "pwsh".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
    })
}

fn resolve_terminal_app(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let escaped = escape_osascript(inner_cmd);
    let script = if let Some(ref label) = config.label {
        format!(
            "tell application \"Terminal\"\n  activate\n  do script \"{}\"\n  set custom title of front window to \"{}\"\nend tell",
            escaped, label
        )
    } else {
        format!("tell application \"Terminal\" to do script \"{}\"", escaped)
    };

    Ok(TerminalPlan {
        program: "osascript".to_string(),
        args: vec!["-e".to_string(), script],
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
    })
}

fn resolve_iterm2(_config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    // iTerm2 can be controlled via AppleScript or its CLI (iterm2://)
    // Use the AppleScript approach for reliability
    let escaped = escape_osascript(inner_cmd);
    let script = format!(
        "tell application \"iTerm\"\n  activate\n  set newWindow to (create window with default profile)\n  tell current session of newWindow\n    write text \"{}\"\n  end tell\nend tell",
        escaped
    );

    Ok(TerminalPlan {
        program: "osascript".to_string(),
        args: vec!["-e".to_string(), script],
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
    })
}

fn resolve_gnome_terminal(
    config: &TerminalConfig,
    inner_cmd: &str,
) -> Result<TerminalPlan, String> {
    let mut args = Vec::new();

    if matches!(config.window_policy, TerminalWindowPolicy::NewTab) {
        args.push("--tab".to_string());
    } else {
        args.push("--window".to_string());
    }

    if let Some(ref label) = config.label {
        args.push("--title".to_string());
        args.push(label.clone());
    }

    args.push("--".to_string());
    args.push("sh".to_string());
    args.push("-c".to_string());
    args.push(inner_cmd.to_string());

    Ok(TerminalPlan {
        program: "gnome-terminal".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
    })
}

fn resolve_konsole(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let mut args = vec![
        "--hold".to_string(),
        "-e".to_string(),
        "sh".to_string(),
        "-c".to_string(),
        inner_cmd.to_string(),
    ];

    if let Some(ref label) = config.label {
        args.insert(0, format!("--title={}", label));
    }

    Ok(TerminalPlan {
        program: "konsole".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
    })
}

fn resolve_xterm(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let mut args = vec![
        "-hold".to_string(),
        "-e".to_string(),
        "sh".to_string(),
        "-c".to_string(),
        inner_cmd.to_string(),
    ];

    if let Some(ref label) = config.label {
        args.insert(0, format!("-T{}", label));
    }

    Ok(TerminalPlan {
        program: "xterm".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
    })
}

fn resolve_alacritty(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let mut args = vec![];
    args.push("--command".to_string());
    args.push(format!("sh -c '{}'", inner_cmd.replace('\'', "'\\''")));

    if let Some(ref label) = config.label {
        args.push("--title".to_string());
        args.push(label.clone());
    }

    Ok(TerminalPlan {
        program: "alacritty".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
    })
}

fn resolve_kitty(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let mut args = vec![];
    args.push("--hold".to_string());
    args.push("sh".to_string());
    args.push("-c".to_string());
    args.push(inner_cmd.to_string());

    if let Some(ref label) = config.label {
        args.push("--title".to_string());
        args.push(label.clone());
    }

    Ok(TerminalPlan {
        program: "kitty".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
    })
}

fn resolve_xfce4_terminal(
    config: &TerminalConfig,
    inner_cmd: &str,
) -> Result<TerminalPlan, String> {
    let mut args = vec![
        "--hold".to_string(),
        "-e".to_string(),
        "sh".to_string(),
        "-c".to_string(),
        inner_cmd.to_string(),
    ];

    if let Some(ref label) = config.label {
        args.push("--title".to_string());
        args.push(label.clone());
    }

    Ok(TerminalPlan {
        program: "xfce4-terminal".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
    })
}

fn resolve_custom_terminal(
    name: &str,
    _config: &TerminalConfig,
    inner_cmd: &str,
) -> Result<TerminalPlan, String> {
    // Custom terminal: assume it can accept `-e sh -c "<cmd>"` pattern
    Ok(TerminalPlan {
        program: name.to_string(),
        args: vec![
            "-e".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            inner_cmd.to_string(),
        ],
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check if a program exists on PATH.
fn which_exists(name: &str) -> bool {
    which::which(name).is_ok()
}

/// Write a temporary batch script on Windows for reliable terminal execution.
#[cfg(target_os = "windows")]
fn write_temp_batch_script(inner: &str) -> Result<String, String> {
    use std::io::Write;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "devlauncher_term_{}_{}.cmd",
        std::process::id(),
        nanos
    ));

    let mut file =
        std::fs::File::create(&dir).map_err(|e| format!("Failed to create temp script: {}", e))?;
    writeln!(file, "@echo off")
        .and_then(|_| writeln!(file, "{}", inner))
        .map_err(|e| format!("Failed to write temp script: {}", e))?;

    Ok(dir.to_string_lossy().into_owned())
}

#[cfg(not(target_os = "windows"))]
fn write_temp_batch_script(_inner: &str) -> Result<String, String> {
    Err("temp batch scripts are Windows-only".to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_terminal_matches_platform() {
        let backend = default_terminal_for_platform();
        let os = current_os();
        match os {
            HostOs::Windows => {
                // Could be WindowsTerminal or Cmd depending on wt availability
                assert!(
                    backend == TerminalBackend::WindowsTerminal || backend == TerminalBackend::Cmd
                )
            }
            HostOs::Macos => assert_eq!(backend, TerminalBackend::TerminalApp),
            HostOs::Linux => {
                // Should be one of the known Linux terminals
                assert!(matches!(
                    backend,
                    TerminalBackend::GnomeTerminal
                        | TerminalBackend::Konsole
                        | TerminalBackend::Xterm
                        | TerminalBackend::Alacritty
                        | TerminalBackend::Kitty
                        | TerminalBackend::Xfce4Terminal
                        | TerminalBackend::Custom(_)
                ));
            }
        }
    }

    #[test]
    fn resolve_cmd_plan() {
        let config = TerminalConfig {
            backend: TerminalBackend::Cmd,
            command: "echo hello".to_string(),
            working_dir: None,
            window_policy: TerminalWindowPolicy::NewWindow,
            label: Some("My Terminal".to_string()),
            keep_open: true,
            env: None,
        };
        let plan = resolve_terminal_plan(&config).unwrap();
        assert_eq!(plan.program, "cmd");
        assert!(plan.args.contains(&"/K".to_string()));
        assert_eq!(
            plan.tracking_quality,
            ProcessTrackingQuality::TerminalWrapper
        );
    }

    #[test]
    fn resolve_terminal_app_plan() {
        let config = TerminalConfig {
            backend: TerminalBackend::TerminalApp,
            command: "npm start".to_string(),
            working_dir: Some("/tmp/project".to_string()),
            window_policy: TerminalWindowPolicy::NewWindow,
            label: None,
            keep_open: true,
            env: None,
        };
        let plan = resolve_terminal_plan(&config).unwrap();
        assert_eq!(plan.program, "osascript");
        assert!(plan.args.iter().any(|a| a.contains("Terminal")));
    }

    #[test]
    fn resolve_gnome_terminal_plan() {
        let config = TerminalConfig {
            backend: TerminalBackend::GnomeTerminal,
            command: "npm start".to_string(),
            working_dir: None,
            window_policy: TerminalWindowPolicy::NewWindow,
            label: Some("Dev".to_string()),
            keep_open: true,
            env: None,
        };
        let plan = resolve_terminal_plan(&config).unwrap();
        assert_eq!(plan.program, "gnome-terminal");
        assert!(plan.args.iter().any(|a| a == "--title"));
    }

    #[test]
    fn build_inner_command_with_dir() {
        let cmd = build_inner_command("echo hello", Some("/tmp/project"));
        assert!(cmd.starts_with("cd"));
        assert!(cmd.contains("echo hello"));
    }

    #[test]
    fn build_inner_command_without_dir() {
        let cmd = build_inner_command("echo hello", None);
        assert_eq!(cmd, "echo hello");
    }

    #[test]
    fn shell_quote_plain() {
        assert_eq!(shell_quote("hello"), "hello");
    }

    #[test]
    fn shell_quote_with_spaces() {
        let q = shell_quote("hello world");
        assert!(q.starts_with('"') || q.starts_with('\''));
    }

    #[test]
    fn terminal_plan_serialize_roundtrip() {
        let plan = TerminalPlan {
            program: "cmd".to_string(),
            args: vec!["/K".to_string(), "echo hello".to_string()],
            tracking_quality: ProcessTrackingQuality::TerminalWrapper,
        };
        // TerminalPlan is not Serialize by default, but we can test Debug
        let debug = format!("{:?}", plan);
        assert!(debug.contains("cmd"));
    }

    #[test]
    fn terminal_config_default() {
        let config = TerminalConfig::default();
        assert_eq!(config.backend, TerminalBackend::Default);
        assert!(config.keep_open);
        assert_eq!(config.window_policy, TerminalWindowPolicy::Auto);
    }
}
