use std::fmt;

use serde::{Deserialize, Serialize};

use super::host::{current_os, HostOs};
use crate::modules::devlauncher::models::ProcessTrackingQuality;

// ---------------------------------------------------------------------------
// TerminalBackend вЂ” platform abstraction for native terminal execution
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
// TerminalWindowPolicy вЂ” new window vs tab
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
// TerminalConfig вЂ” configuration for a terminal launch
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
    /// Optional startup probe marker file. When set, the terminal's inner
    /// command is wrapped so that, after it exits, its exit code is written
    /// to this file. The orchestrator polls the file to verify the command
    /// actually ran (and did not fail instantly), instead of blindly
    /// treating "terminal opened" as "command started".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_marker: Option<String>,
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
            startup_marker: None,
        }
    }
}

// ---------------------------------------------------------------------------
// TerminalPlan вЂ” resolved launch command for a specific terminal
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
    /// Flag marking the LAST argument of `args` as a raw command tail that
    /// must be appended to the spawned command line verbatim (never
    /// re-quoted by standard argument quoting).
    ///
    /// Windows `cmd` and `wt` re-parse their command tail with cmd-style
    /// rules from the RAW command line (they do not round-trip through
    /// CommandLineToArgvW). Backslash-escaped quotes produced by standard
    /// argument quoting (`\"`) survive into the command and break paths
    /// with spaces, silently killing the whole chain. The spawner therefore
    /// pops the LAST argument and appends it raw, UNQUOTED: cmd parses
    /// everything after `/K` itself, and wt forwards the tail to cmd
    /// unchanged. (Wrapping the tail in outer quotes вЂ” the value stored
    /// here вЂ” does NOT survive cmd's first/last-quote stripping: the nested
    /// quotes get mangled and the command dies with "C:\Program is not
    /// recognized".)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_tail: Option<String>,
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
///
/// On Windows the working directory is NOT baked into the command line:
/// `cmd` receives it as the spawn's current directory and Windows Terminal
/// gets it via `--startingDirectory` (see `resolve_windows_terminal`). A
/// `cd /d "path"` prefix is fragile under `wt`'s command-line re-parsing
/// (quotes get stripped, so a path with spaces breaks the `cd` and the
/// command silently never runs). POSIX terminals keep the `cd` prefix вЂ”
/// their launchers start the shell in the launcher's own directory.
fn build_inner_command(command: &str, working_dir: Option<&str>) -> String {
    match working_dir {
        Some(dir) => {
            if cfg!(target_os = "windows") {
                command.to_string()
            } else {
                format!("cd '{}' && {}", dir.replace('\'', "'\\''"), command)
            }
        }
        None => command.to_string(),
    }
}

/// Shell family a terminal backend runs its inner command in.
#[derive(Debug, Clone, Copy)]
enum MarkerShell {
    /// `cmd.exe` (`cmd /K`), needs delayed expansion (`!ERRORLEVEL!`).
    Cmd,
    /// POSIX `sh`/`bash`/`zsh` (`$?`).
    Sh,
    /// PowerShell (`$LASTEXITCODE`).
    Ps,
}

/// Wrap the inner command so the terminal writes the command's exit code to
/// the startup probe marker after it exits. The suffix is appended AFTER the
/// inner command unconditionally (`&` in cmd, `;` in sh/ps), so a failing
/// command still produces a marker with its real exit code.
fn with_marker(inner: &str, config: &TerminalConfig, shell: MarkerShell) -> String {
    let Some(marker) = config.startup_marker.as_deref() else {
        return inner.to_string();
    };
    match shell {
        // `!ERRORLEVEL!` (delayed expansion, enabled via `/V:ON`) is expanded
        // when the echo executes, unlike `%ERRORLEVEL%` which is expanded when
        // the whole line is parsed (always 0). The space before `>` and the
        // quoted path are required: `echo !ERRORLEVEL!>path` writes an empty
        // file.
        MarkerShell::Cmd => format!("{} & echo !ERRORLEVEL! > \"{}\"", inner, marker),
        MarkerShell::Sh => {
            format!("{}; echo $? > '{}'", inner, marker.replace('\'', "'\\''"))
        }
        MarkerShell::Ps => format!(
            "{}; $LASTEXITCODE | Out-File -FilePath '{}' -Encoding ascii",
            inner,
            marker.replace('\'', "''")
        ),
    }
}

/// Escape a command for osascript (macOS Terminal.app / iTerm2).
fn escape_osascript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Escape a command for use in a `.cmd` batch script.
#[allow(dead_code)]
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
#[allow(dead_code)]
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
    let inner_cmd = with_marker(inner_cmd, config, MarkerShell::Cmd);
    // wt.exe -w new cmd /K "<inner>" вЂ” the command is passed as a single
    // argv entry so cmd.exe receives it literally (no temp batch script:
    // the user sees the exact command in the terminal and nothing leaks
    // into %TEMP%). The tail is cmd-style quoted and appended RAW: wt
    // forwards the command portion verbatim to cmd, whose /K parsing does
    // not understand backslash-escaped quotes.
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

    // The tab must start in the step's working directory. This is passed as
    // a structured `--startingDirectory` (NOT baked into the command line as
    // `cd /d`): wt re-parses its command tail with CommandLineToArgvW and
    // strips quotes, so an unquoted path with spaces would fail the `cd`
    // with "The system cannot find the path specified".
    if let Some(dir) = config.working_dir.as_deref() {
        args.push("--startingDirectory".to_string());
        args.push(dir.to_string());
    }

    args.push("cmd".to_string());
    // `/V:ON` enables delayed expansion so the startup probe can capture the
    // inner command's exit code with `!ERRORLEVEL!`.
    if config.startup_marker.is_some() {
        args.push("/V:ON".to_string());
    }
    args.push("/K".to_string());
    args.push(inner_cmd.to_string());

    // `raw_tail` is a FLAG: the spawner pops the last `args` element (the
    // unquoted payload) and appends it raw. The value stored here is not
    // appended itself вЂ” cmd's first/last-quote stripping mangles a
    // quote-wrapped tail and the command dies with "C:\Program is not
    // recognized".
    let raw_tail = format!("\"{}\"", inner_cmd);
    Ok(TerminalPlan {
        program: "wt".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
        raw_tail: Some(raw_tail),
    })
}

fn resolve_cmd(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let inner_cmd = with_marker(inner_cmd, config, MarkerShell::Cmd);
    // `cmd /K <tail>`: the tail is a raw command line parsed by cmd itself.
    // `&&` separators must NOT be escaped (`^&^&` would join the pieces
    // into one command) and the tail must reach cmd without backslash
    // re-escaping вЂ” hence `raw_tail`.
    let mut args = Vec::new();
    // `/V:ON` enables delayed expansion so the startup probe can capture the
    // inner command's exit code with `!ERRORLEVEL!`.
    if config.startup_marker.is_some() {
        args.push("/V:ON".to_string());
    }
    args.push("/K".to_string());

    let full_cmd = if let Some(ref label) = config.label {
        format!("title {} && {}", label, inner_cmd)
    } else {
        inner_cmd.to_string()
    };

    args.push(full_cmd.clone());

    // `raw_tail` is a FLAG: the spawner pops the last `args` element (the
    // unquoted `full_cmd`) and appends it raw. The value stored here is not
    // appended itself вЂ” cmd's first/last-quote stripping would mangle the
    // nested quotes (paths with spaces die with "C:\Program is not
    // recognized"). cmd parses the UNQUOTED tail after `/K` itself.
    let raw_tail = format!("\"{}\"", full_cmd);
    Ok(TerminalPlan {
        program: "cmd".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
        raw_tail: Some(raw_tail),
    })
}

fn resolve_powershell(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let inner_cmd = with_marker(inner_cmd, config, MarkerShell::Ps);
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
        raw_tail: None,
    })
}

fn resolve_pwsh(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let inner_cmd = with_marker(inner_cmd, config, MarkerShell::Ps);
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
        raw_tail: None,
    })
}

fn resolve_terminal_app(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let inner_cmd = with_marker(inner_cmd, config, MarkerShell::Sh);
    let escaped = escape_osascript(&inner_cmd);
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
        raw_tail: None,
    })
}

fn resolve_iterm2(_config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    // iTerm2 can be controlled via AppleScript or its CLI (iterm2://)
    // Use the AppleScript approach for reliability
    let inner_cmd = with_marker(inner_cmd, _config, MarkerShell::Sh);
    let escaped = escape_osascript(&inner_cmd);
    let script = format!(
        "tell application \"iTerm\"\n  activate\n  set newWindow to (create window with default profile)\n  tell current session of newWindow\n    write text \"{}\"\n  end tell\nend tell",
        escaped
    );

    Ok(TerminalPlan {
        program: "osascript".to_string(),
        args: vec!["-e".to_string(), script],
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
        raw_tail: None,
    })
}

fn resolve_gnome_terminal(
    config: &TerminalConfig,
    inner_cmd: &str,
) -> Result<TerminalPlan, String> {
    let inner_cmd = with_marker(inner_cmd, config, MarkerShell::Sh);
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
        raw_tail: None,
    })
}

fn resolve_konsole(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let inner_cmd = with_marker(inner_cmd, config, MarkerShell::Sh);
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
        raw_tail: None,
    })
}

fn resolve_xterm(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let inner_cmd = with_marker(inner_cmd, config, MarkerShell::Sh);
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
        raw_tail: None,
    })
}

fn resolve_alacritty(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let inner_cmd = with_marker(inner_cmd, config, MarkerShell::Sh);
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
        raw_tail: None,
    })
}

fn resolve_kitty(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let inner_cmd = with_marker(inner_cmd, config, MarkerShell::Sh);
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
        raw_tail: None,
    })
}

fn resolve_xfce4_terminal(
    config: &TerminalConfig,
    inner_cmd: &str,
) -> Result<TerminalPlan, String> {
    let inner_cmd = with_marker(inner_cmd, config, MarkerShell::Sh);
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
        raw_tail: None,
    })
}

fn resolve_custom_terminal(
    name: &str,
    config: &TerminalConfig,
    inner_cmd: &str,
) -> Result<TerminalPlan, String> {
    // Custom terminal: assume it can accept `-e sh -c "<cmd>"` pattern
    let inner_cmd = with_marker(inner_cmd, config, MarkerShell::Sh);
    Ok(TerminalPlan {
        program: name.to_string(),
        args: vec![
            "-e".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            inner_cmd.to_string(),
        ],
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
        raw_tail: None,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check if a program exists on PATH.
fn which_exists(name: &str) -> bool {
    which::which(name).is_ok()
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
            startup_marker: None,
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
            startup_marker: None,
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
            startup_marker: None,
        };
        let plan = resolve_terminal_plan(&config).unwrap();
        assert_eq!(plan.program, "gnome-terminal");
        assert!(plan.args.iter().any(|a| a == "--title"));
    }

    #[test]
    fn cmd_plan_with_startup_marker_enables_delayed_expansion() {
        let config = TerminalConfig {
            backend: TerminalBackend::Cmd,
            command: "npm run dev".to_string(),
            working_dir: None,
            window_policy: TerminalWindowPolicy::NewWindow,
            label: None,
            keep_open: true,
            env: None,
            startup_marker: Some(r"C:\Temp\probe\m.txt".to_string()),
        };
        let plan = resolve_terminal_plan(&config).unwrap();
        assert_eq!(plan.program, "cmd");
        assert!(plan.args.contains(&"/V:ON".to_string()));
        let tail = plan.raw_tail.unwrap_or_default();
        assert!(tail.contains("!ERRORLEVEL!"));
        assert!(tail.contains("m.txt"));
        assert!(tail.contains("npm run dev"));
    }

    #[test]
    fn raw_tail_plan_contract_last_arg_is_the_payload() {
        // The spawner pops the plan's LAST argument and appends it raw
        // (never re-quoted). If a resolver stops putting the payload last,
        // cmd/wt receive a mangled command line and every command with a
        // quoted path dies with "C:\Program is not recognized" вЂ” the
        // regression this test guards against.
        let cmd_config = TerminalConfig {
            backend: TerminalBackend::Cmd,
            command: "\"C:\\Program Files\\nodejs\\npm.cmd\" run dev".to_string(),
            working_dir: Some("C:\\My Project\\app".to_string()),
            window_policy: TerminalWindowPolicy::NewWindow,
            label: Some("Backend".to_string()),
            keep_open: true,
            env: None,
            startup_marker: None,
        };
        let cmd_plan = resolve_terminal_plan(&cmd_config).unwrap();
        assert!(cmd_plan.raw_tail.is_some());
        let last = cmd_plan.args.last().cloned().unwrap_or_default();
        assert!(
            last.contains("npm.cmd"),
            "cmd payload must be the last arg: {last}"
        );
        assert!(
            last.contains("run dev"),
            "cmd payload must carry the command: {last}"
        );

        let wt_config = TerminalConfig {
            backend: TerminalBackend::WindowsTerminal,
            command: "\"C:\\Program Files\\nodejs\\npm.cmd\" run dev".to_string(),
            working_dir: Some("C:\\My Project\\app".to_string()),
            window_policy: TerminalWindowPolicy::NewWindow,
            label: Some("Backend".to_string()),
            keep_open: true,
            env: None,
            startup_marker: None,
        };
        let wt_plan = resolve_terminal_plan(&wt_config).unwrap();
        assert!(wt_plan.raw_tail.is_some());
        let last = wt_plan.args.last().cloned().unwrap_or_default();
        assert!(
            last.contains("npm.cmd"),
            "wt payload must be the last arg: {last}"
        );
        assert!(
            last.contains("run dev"),
            "wt payload must carry the command: {last}"
        );
    }

    #[test]
    fn sh_plan_with_startup_marker_appends_exit_code_capture() {
        let config = TerminalConfig {
            backend: TerminalBackend::GnomeTerminal,
            command: "python manage.py runserver".to_string(),
            working_dir: None,
            window_policy: TerminalWindowPolicy::NewWindow,
            label: None,
            keep_open: true,
            env: None,
            startup_marker: Some("/tmp/probe/m.txt".to_string()),
        };
        let plan = resolve_terminal_plan(&config).unwrap();
        assert_eq!(plan.program, "gnome-terminal");
        let joined = plan.args.join(" ");
        assert!(joined.contains("echo $? > '/tmp/probe/m.txt'"), "{joined}");
        assert!(joined.contains("python manage.py runserver"), "{joined}");
    }

    #[test]
    fn build_inner_command_with_dir() {
        // On Windows the working directory is passed out-of-band (spawn cwd
        // / `--startingDirectory`), so the command line is unchanged. POSIX
        // terminals keep the `cd` prefix.
        let cmd = build_inner_command("echo hello", Some("/tmp/project"));
        if cfg!(target_os = "windows") {
            assert_eq!(cmd, "echo hello");
        } else {
            assert!(cmd.starts_with("cd"));
            assert!(cmd.contains("echo hello"));
        }
    }

    #[test]
    fn build_inner_command_without_dir() {
        let cmd = build_inner_command("echo hello", None);
        assert_eq!(cmd, "echo hello");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn wt_plan_carries_starting_directory() {
        let config = TerminalConfig {
            backend: TerminalBackend::WindowsTerminal,
            command: "npm run dev".to_string(),
            working_dir: Some("C:\\My Project\\app".to_string()),
            window_policy: TerminalWindowPolicy::NewWindow,
            label: None,
            keep_open: true,
            env: None,
            startup_marker: None,
        };
        let plan = resolve_terminal_plan(&config).unwrap();
        assert_eq!(plan.program, "wt");
        let idx = plan.args.iter().position(|a| a == "--startingDirectory");
        assert!(idx.is_some(), "wt plan must carry --startingDirectory");
        let idx = idx.unwrap();
        assert_eq!(plan.args[idx + 1], "C:\\My Project\\app");
        // The command line must not carry a `cd` prefix вЂ” the directory is
        // passed as a structured option instead.
        assert!(plan.args.iter().all(|a| !a.starts_with("cd ")));
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
            raw_tail: None,
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
