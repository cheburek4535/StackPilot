use std::fmt;
#[cfg(any(target_os = "windows", target_os = "linux"))]
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::host::{current_os, HostOs};
use crate::modules::devlauncher::models::ProcessTrackingQuality;

/// Whether Windows Terminal is usable on this machine, cached for the
/// process lifetime (availability cannot change while the app runs).
#[cfg(target_os = "windows")]
static WINDOWS_TERMINAL_AVAILABLE: OnceLock<bool> = OnceLock::new();

/// Whether the freedesktop `xdg-terminal-exec` helper can resolve a default
/// terminal. Probed at most once per process: the answer cannot change while
/// the app runs, and the probe runs a helper script (never cheap enough for
/// every terminal spawn).
#[cfg(target_os = "linux")]
static LINUX_XDG_TERMINAL_EXEC: OnceLock<bool> = OnceLock::new();

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
    /// ptyxis (GNOME Terminal for GNOME 47+, Linux)
    Ptyxis,
    /// freedesktop `xdg-terminal-exec` default-terminal launcher (Linux)
    XdgTerminalExec,
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
            TerminalBackend::Ptyxis => write!(f, "ptyxis"),
            TerminalBackend::XdgTerminalExec => write!(f, "xdg-terminal-exec"),
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
    /// Capture the inner-command output into a sidecar log file next to
    /// `startup_marker` (see [`startup_log_path`]). POSIX terminals tee the
    /// output there, so the orchestrator can report the REAL failure reason
    /// ("port is already allocated", "no configuration file provided", ...)
    /// instead of only "check the terminal output" — the terminal window
    /// belongs to the emulator and its scrollback is not machine-readable.
    #[serde(default)]
    pub capture_output: bool,
    /// Optional file the inner POSIX command writes its own PID (and process
    /// group) to, before running. Terminal emulators detach: the tracked
    /// launcher process exits as soon as the window is up, so Stop/Cancel
    /// cannot reach the real command tree through it. The recorded process
    /// group is the only handle that can stop a dev server the user started
    /// from a terminal step. Windows backends keep the launcher-based kill
    /// (`taskkill /T`) and never write this file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid_file: Option<String>,
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
            capture_output: false,
            pid_file: None,
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
            if windows_terminal_available() {
                TerminalBackend::WindowsTerminal
            } else {
                TerminalBackend::Cmd
            }
        }
        HostOs::Macos => TerminalBackend::TerminalApp,
        HostOs::Linux => {
            // 1. The freedesktop default-terminal helper knows the user's
            //    preferred terminal AND its exact command-line syntax (GNOME
            //    Ptyxis, Console, foot, WezTerm, ...), which per-terminal
            //    heuristics cannot. Preferred whenever it can actually find a
            //    terminal on this machine.
            if linux_xdg_terminal_exec_available() {
                return TerminalBackend::XdgTerminalExec;
            }
            // 2. Explicit terminals in preference order. Ptyxis and GNOME
            //    Console are the modern GNOME terminals; x-terminal-emulator
            //    is the distro's own configured default (Debian/Ubuntu
            //    alternatives), so it is honored before the raw fallbacks.
            let candidates: [(&str, TerminalBackend); 8] = [
                ("ptyxis", TerminalBackend::Ptyxis),
                ("gnome-terminal", TerminalBackend::GnomeTerminal),
                ("konsole", TerminalBackend::Konsole),
                (
                    "x-terminal-emulator",
                    TerminalBackend::Custom("x-terminal-emulator".to_string()),
                ),
                ("xfce4-terminal", TerminalBackend::Xfce4Terminal),
                ("alacritty", TerminalBackend::Alacritty),
                ("kitty", TerminalBackend::Kitty),
                ("xterm", TerminalBackend::Xterm),
            ];
            for (cand, backend) in candidates {
                if which_exists(cand) {
                    // Prefer a backend that knows the terminal's exact command
                    // syntax. `x-terminal-emulator` is a distro alternatives
                    // symlink (Debian/Ubuntu): its target decides whether the
                    // `-e sh -c` contract or a structured `--` invocation is
                    // correct.
                    if let Ok(path) = which::which(cand) {
                        if let Ok(real) = std::fs::canonicalize(&path) {
                            if let Some(mapped) =
                                linux_backend_for_terminal_name(&real.to_string_lossy())
                            {
                                return mapped;
                            }
                        }
                    }
                    return backend;
                }
            }
            // 3. The user's $TERMINAL choice (the same variable the desktop
            //    files use) before a bare xterm attempt.
            if let Ok(term) = std::env::var("TERMINAL") {
                let term = term.trim();
                if !term.is_empty() && which_exists(term) {
                    if let Some(mapped) = which::which(term)
                        .ok()
                        .and_then(|p| std::fs::canonicalize(p).ok())
                        .and_then(|real| linux_backend_for_terminal_name(&real.to_string_lossy()))
                    {
                        return mapped;
                    }
                    return TerminalBackend::Custom(term.to_string());
                }
            }
            // Last resort: xterm is the safest `-hold -e sh -c` contract
            // even when it is not on PATH — the spawn error is then reported
            // as an actual failure instead of silently doing nothing.
            TerminalBackend::Xterm
        }
    }
}

/// Map a Linux terminal binary (name or path) to the backend that knows its
/// exact command-line syntax. Unknown terminals return `None`; the caller
/// then falls back to the generic `-e sh -c` contract that every
/// x-terminal-emulator-compatible program supports.
#[cfg(target_os = "linux")]
fn linux_backend_for_terminal_name(name: &str) -> Option<TerminalBackend> {
    let base = name
        .trim()
        .rsplit('/')
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase();
    match base.as_str() {
        "ptyxis" => Some(TerminalBackend::Ptyxis),
        "gnome-terminal" => Some(TerminalBackend::GnomeTerminal),
        "konsole" => Some(TerminalBackend::Konsole),
        "xfce4-terminal" => Some(TerminalBackend::Xfce4Terminal),
        "alacritty" => Some(TerminalBackend::Alacritty),
        "kitty" => Some(TerminalBackend::Kitty),
        "xterm" | "uxterm" | "urxvt" | "rxvt" => Some(TerminalBackend::Xterm),
        "xdg-terminal-exec" => Some(TerminalBackend::XdgTerminalExec),
        _ => None,
    }
}

/// Non-Linux stub.
#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
fn linux_backend_for_terminal_name(_name: &str) -> Option<TerminalBackend> {
    None
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
        TerminalBackend::Ptyxis => resolve_ptyxis(config, &inner_cmd),
        TerminalBackend::XdgTerminalExec => resolve_xdg_terminal_exec(config, &inner_cmd),
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
///
/// POSIX commands additionally record their process group (see
/// [`TerminalConfig::pid_file`]) and remove it again when they finish, so a
/// Stop/Cancel can kill the real command tree after the terminal launcher
/// has detached.
fn with_marker(inner: &str, config: &TerminalConfig, shell: MarkerShell) -> String {
    match shell {
        // `!ERRORLEVEL!` (delayed expansion, enabled via `/V:ON`) is expanded
        // when the echo executes, unlike `%ERRORLEVEL%` which is expanded when
        // the whole line is parsed (always 0). The space before `>` and the
        // quoted path are required: `echo !ERRORLEVEL!>path` writes an empty
        // file.
        MarkerShell::Cmd => {
            let Some(marker) = config.startup_marker.as_deref() else {
                return inner.to_string();
            };
            format!("{} & echo !ERRORLEVEL! > \"{}\"", inner, marker)
        }
        MarkerShell::Sh => with_posix_lifecycle(inner, config),
        MarkerShell::Ps => {
            let Some(marker) = config.startup_marker.as_deref() else {
                return inner.to_string();
            };
            format!(
                "{}; $LASTEXITCODE | Out-File -FilePath '{}' -Encoding ascii",
                inner,
                marker.replace('\'', "''")
            )
        }
    }
}

/// POSIX command lifecycle wrapper:
///
/// 1. record the command's PID + process group into `pid_file` (when set), so
///    Stop/Cancel can signal the real tree even though the terminal launcher
///    has already detached;
/// 2. run the command and write its exit code to `startup_marker` (when set);
/// 3. remove both helpers again.
///
/// When `capture_output` is set the output is additionally teed into a sidecar
/// log while the user still watches it in the terminal — terminal scrollback
/// is not machine-readable, and the orchestrator can then report the REAL
/// failure reason instead of only "check the terminal output". The marker is
/// written from inside the group, so it carries the command's real exit code
/// even though a pipeline's status is `tee`'s.
fn with_posix_lifecycle(inner: &str, config: &TerminalConfig) -> String {
    let pid_file_raw = config.pid_file.as_deref();
    let marker_raw = config.startup_marker.as_deref();
    let pid_file = pid_file_raw.map(|p| p.replace('\'', "'\\''"));
    let marker = marker_raw.map(|m| m.replace('\'', "'\\''"));

    let mut prefix = String::new();
    if let Some(pid_file) = &pid_file {
        // `ps -o pgid=` is the portable way to learn the process group; when
        // it is unavailable the group equals the PID for terminal foreground
        // jobs. The EXIT trap is a best-effort cleanup for signal
        // termination (a closed window, SIGTERM); the explicit `rm` below
        // handles the normal path (an `exec` of the interactive shell would
        // skip the trap).
        prefix.push_str(&format!(
            "{{ __dl_pidfile='{pid_file}'; __dl_pid=$$; \
             __dl_pgid=$(ps -o pgid= -p $$ 2>/dev/null | tr -d ' '); \
             if [ -z \"$__dl_pgid\" ]; then __dl_pgid=$__dl_pid; fi; \
             printf '%s %s\\n' \"$__dl_pid\" \"$__dl_pgid\" > \"$__dl_pidfile\"; \
             trap 'rm -f \"$__dl_pidfile\"' EXIT; }}; "
        ));
    }

    let cleanup = if pid_file.is_some() {
        "rm -f \"$__dl_pidfile\"; ".to_string()
    } else {
        String::new()
    };

    let body = match (&marker, config.capture_output) {
        (Some(marker), true) => {
            let log = startup_log_path(marker_raw.unwrap_or(marker)).replace('\'', "'\\''");
            format!(
                "{{ {inner}; __dl_exit=$?; echo \"$__dl_exit\" > '{marker}'; {cleanup} }} \
                 2>&1 | tee '{log}'"
            )
        }
        (Some(marker), false) => format!("{inner}; echo $? > '{marker}'; {cleanup}"),
        (None, _) => {
            if cleanup.is_empty() {
                inner.to_string()
            } else {
                format!("{inner}; {cleanup}")
            }
        }
    };

    format!("{prefix}{body}")
}

/// Sidecar log file used when [`TerminalConfig::capture_output`] is set.
/// Derived from the marker path so both the spawner and the orchestrator
/// agree without extra plumbing.
pub fn startup_log_path(marker: &str) -> String {
    format!("{}.log", marker)
}

/// Keep the terminal window open after the inner command exits.
///
/// Windows backends already hold the window themselves (`cmd /K`,
/// PowerShell `-NoExit`), so nothing is appended there. POSIX terminals
/// close their window as soon as the spawned shell exits, which on Linux
/// made every failing command's output unreadable (the terminal vanished
/// before the user could see why the step failed). Handing the terminal over
/// to an interactive shell is the POSIX analogue of `cmd /K`: the window
/// stays open with the full output above the prompt.
fn with_hold(cmd: String, shell: MarkerShell, keep_open: bool) -> String {
    if !keep_open {
        return cmd;
    }
    match shell {
        MarkerShell::Sh => format!(
            "{}; {{ [ -n \"$SHELL\" ] && [ -x \"$SHELL\" ] && exec \"$SHELL\" || exec /bin/sh; }}",
            cmd
        ),
        // cmd /K and PowerShell -NoExit keep the window open by themselves.
        MarkerShell::Cmd | MarkerShell::Ps => cmd,
    }
}

/// Apply the exit-code marker (when requested) and the keep-open hold to an
/// inner command for the given terminal shell family.
fn prepare_inner(inner: &str, config: &TerminalConfig, shell: MarkerShell) -> String {
    with_hold(with_marker(inner, config, shell), shell, config.keep_open)
}

/// Fresh pid-file path for a visible terminal command (POSIX only). The file
/// is written by the command itself inside the terminal window.
pub fn create_pid_file_path() -> String {
    std::env::temp_dir()
        .join(format!("devlauncher_term_pid_{}.txt", uuid::Uuid::new_v4()))
        .to_string_lossy()
        .into_owned()
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
    let inner_cmd = prepare_inner(inner_cmd, config, MarkerShell::Sh);
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
    let inner_cmd = prepare_inner(inner_cmd, _config, MarkerShell::Sh);
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
    let inner_cmd = prepare_inner(inner_cmd, config, MarkerShell::Sh);
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

/// Ptyxis (GNOME's terminal since GNOME 47) takes the command after `--`,
/// not `-e`; the working directory and title are structured options. Passing
/// `-e sh -c ...` happens to work on some builds but is not the documented
/// interface, and the `cd` prefix baked into the command (see
/// [`build_inner_command`]) breaks for paths the terminal would have to
/// expand itself.
fn resolve_ptyxis(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let inner_cmd = prepare_inner(inner_cmd, config, MarkerShell::Sh);
    let mut args = vec!["--new-window".to_string()];

    if let Some(dir) = config.working_dir.as_deref() {
        args.push("--working-directory".to_string());
        args.push(dir.to_string());
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
        program: "ptyxis".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
        raw_tail: None,
    })
}

/// The freedesktop default-terminal launcher resolves the user's terminal
/// (and the correct CLI syntax for it) from the desktop entries, so one plan
/// works for every Linux desktop.
fn resolve_xdg_terminal_exec(
    config: &TerminalConfig,
    inner_cmd: &str,
) -> Result<TerminalPlan, String> {
    let inner_cmd = prepare_inner(inner_cmd, config, MarkerShell::Sh);
    let mut args = Vec::new();

    if let Some(ref label) = config.label {
        args.push(format!("--title={}", label));
    }
    // `--dir` is the spec's structured working directory. The POSIX `cd`
    // prefix stays in the command as well: `--dir` is ignored by entries
    // that do not support it, while `cd` always works.
    if let Some(dir) = config.working_dir.as_deref() {
        args.push(format!("--dir={}", dir));
    }

    args.push("--".to_string());
    args.push("sh".to_string());
    args.push("-c".to_string());
    args.push(inner_cmd.to_string());

    Ok(TerminalPlan {
        program: "xdg-terminal-exec".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
        raw_tail: None,
    })
}

fn resolve_konsole(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let inner_cmd = prepare_inner(inner_cmd, config, MarkerShell::Sh);
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
    let inner_cmd = prepare_inner(inner_cmd, config, MarkerShell::Sh);
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

/// Alacritty takes the command as trailing arguments after `--command`
/// (`alacritty --command sh -c '<cmd>'`), NOT as one pre-joined shell string:
/// the old single-argument form was passed to `execve` verbatim, so alacritty
/// tried to run a file literally named `sh -c '...'`. Options must precede
/// `--command` — anything after it belongs to the spawned program.
fn resolve_alacritty(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let inner_cmd = prepare_inner(inner_cmd, config, MarkerShell::Sh);
    let mut args = vec![];

    if let Some(ref label) = config.label {
        args.push("--title".to_string());
        args.push(label.clone());
    }

    args.push("--command".to_string());
    args.push("sh".to_string());
    args.push("-c".to_string());
    args.push(inner_cmd.to_string());

    Ok(TerminalPlan {
        program: "alacritty".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
        raw_tail: None,
    })
}

/// Kitty takes options first and the program after them; the old order put
/// `--title` AFTER the command, so the title was passed to the shell as
/// `$0`/positional arguments instead of being applied to the window.
fn resolve_kitty(config: &TerminalConfig, inner_cmd: &str) -> Result<TerminalPlan, String> {
    let inner_cmd = prepare_inner(inner_cmd, config, MarkerShell::Sh);
    let mut args = vec![];

    if let Some(ref label) = config.label {
        args.push("--title".to_string());
        args.push(label.clone());
    }

    args.push("--hold".to_string());
    args.push("sh".to_string());
    args.push("-c".to_string());
    args.push(inner_cmd.to_string());

    Ok(TerminalPlan {
        program: "kitty".to_string(),
        args,
        tracking_quality: ProcessTrackingQuality::TerminalWrapper,
        raw_tail: None,
    })
}

/// xfce4-terminal treats everything after `-e` as the command, so the title
/// option must come BEFORE `-e` (the old order appended `--title` after the
/// command, which forwarded it to `sh` as an extra argument).
fn resolve_xfce4_terminal(
    config: &TerminalConfig,
    inner_cmd: &str,
) -> Result<TerminalPlan, String> {
    let inner_cmd = prepare_inner(inner_cmd, config, MarkerShell::Sh);
    let mut args = vec!["--hold".to_string()];

    if let Some(ref label) = config.label {
        args.push("--title".to_string());
        args.push(label.clone());
    }

    args.push("-e".to_string());
    args.push("sh".to_string());
    args.push("-c".to_string());
    args.push(inner_cmd.to_string());

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
    let inner_cmd = prepare_inner(inner_cmd, config, MarkerShell::Sh);
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

/// Whether the freedesktop `xdg-terminal-exec` helper can resolve a default
/// terminal on this machine. A binary alone is not enough: a host can ship
/// the helper without any usable terminal desktop entry, in which case every
/// launch would fail with a non-zero exit instead of opening a window. The
/// probe (`--print-id`) reports the selected entry without launching
/// anything, and the verdict is cached for the process lifetime.
#[cfg(target_os = "linux")]
fn linux_xdg_terminal_exec_available() -> bool {
    *LINUX_XDG_TERMINAL_EXEC
        .get_or_init(|| which_exists("xdg-terminal-exec") && xdg_terminal_exec_finds_terminal())
}

/// Non-Linux stub: the helper is a freedesktop/Linux tool.
#[cfg(not(target_os = "linux"))]
fn linux_xdg_terminal_exec_available() -> bool {
    false
}

#[cfg(target_os = "linux")]
fn xdg_terminal_exec_finds_terminal() -> bool {
    let mut child = match std::process::Command::new("xdg-terminal-exec")
        .arg("--print-id")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    // The probe must never block a terminal spawn: a broken helper script is
    // killed after a short grace period and treated as unavailable.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Whether Windows Terminal can actually be launched on this machine.
///
/// A PATH check alone is NOT enough: on Windows `wt.exe` is an App
/// Execution Alias — a reparse point in
/// `%LOCALAPPDATA%\Microsoft\WindowsApps` whose presence says nothing about
/// whether the Windows Terminal package is installed. Stale aliases (the
/// app was uninstalled, the alias was disabled, or the machine never had
/// the package) survive on disk and, when spawned, create a process that
/// exits immediately with a non-zero code — the startup probe then reports
/// "Process ... exited immediately after start" for every terminal step on
/// such machines. The verdict therefore comes from the AppX package
/// repository (the same registry store `Get-AppxPackage` reads): the
/// `Microsoft.WindowsTerminal_*` package is only registered there when the
/// terminal is actually installed. This is a pure registry read — spawning
/// `wt` for verification is NOT an option: `wt --version` makes Windows
/// Terminal pop its version-title Help window. The verdict is cached for
/// the process lifetime.
#[cfg(target_os = "windows")]
fn windows_terminal_available() -> bool {
    *WINDOWS_TERMINAL_AVAILABLE.get_or_init(windows_terminal_registered)
}

/// Query the AppX package repository for the Windows Terminal package.
///
/// Packages registered for the current user live under
/// `HKCU\Software\Classes\Local Settings\...\AppModel\Repository\Packages`;
/// provisioned-for-all-users packages are additionally listed under
/// `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages`.
/// Either hit means Windows Terminal is installed.
#[cfg(target_os = "windows")]
fn windows_terminal_registered() -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    const PER_USER_REPO: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
    const MACHINE_REPO: &str =
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";

    // `HKEY` is a `*mut c_void` alias; the predef constants (HKEY_CURRENT_USER,
    // HKEY_LOCAL_MACHINE) are its values.
    let has_windows_terminal = |hive: *mut std::ffi::c_void, path: &str| -> bool {
        match RegKey::predef(hive).open_subkey_with_flags(path, KEY_READ) {
            Ok(key) => key.enum_keys().any(|name| {
                name.map(|n| n.starts_with("Microsoft.WindowsTerminal_"))
                    .unwrap_or(false)
            }),
            Err(_) => false,
        }
    };

    has_windows_terminal(HKEY_CURRENT_USER, PER_USER_REPO)
        || has_windows_terminal(HKEY_LOCAL_MACHINE, MACHINE_REPO)
}

/// Non-Windows stub: the WindowsTerminal backend is never available there.
#[cfg(not(target_os = "windows"))]
fn windows_terminal_available() -> bool {
    false
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
                // Should be one of the known Linux terminals (or the
                // freedesktop default-terminal helper).
                assert!(matches!(
                    backend,
                    TerminalBackend::GnomeTerminal
                        | TerminalBackend::Ptyxis
                        | TerminalBackend::XdgTerminalExec
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

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_terminal_availability_is_bool_and_stable() {
        // Never panics and returns a consistent verdict for the process
        // lifetime (the availability cache is process-scoped).
        let a = windows_terminal_available();
        let b = windows_terminal_available();
        assert_eq!(a, b);
        let backend = default_terminal_for_platform();
        if a {
            assert_eq!(backend, TerminalBackend::WindowsTerminal);
        } else {
            assert_eq!(backend, TerminalBackend::Cmd);
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
            capture_output: false,
            pid_file: None,
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
            capture_output: false,
            pid_file: None,
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
            capture_output: false,
            pid_file: None,
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
            capture_output: false,
            pid_file: None,
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
            capture_output: false,
            pid_file: None,
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
            capture_output: false,
            pid_file: None,
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
            keep_open: false,
            env: None,
            startup_marker: Some("/tmp/probe/m.txt".to_string()),
            capture_output: false,
            pid_file: None,
        };
        let plan = resolve_terminal_plan(&config).unwrap();
        assert_eq!(plan.program, "gnome-terminal");
        let joined = plan.args.join(" ");
        assert!(joined.contains("echo $? > '/tmp/probe/m.txt'"), "{joined}");
        assert!(joined.contains("python manage.py runserver"), "{joined}");
        // keep_open = false: no interactive shell hand-off.
        assert!(!joined.contains("exec \"$SHELL\""), "{joined}");
    }

    /// POSIX terminals close their window when the spawned shell exits, which
    /// made every failing command's output unreadable on Linux. The hold is
    /// the POSIX analogue of `cmd /K`: an interactive shell keeps the window
    /// (and its scrollback) alive after the command finishes.
    #[test]
    fn posix_plan_holds_terminal_open() {
        let config = TerminalConfig {
            backend: TerminalBackend::Xterm,
            command: "npm run dev".to_string(),
            working_dir: None,
            window_policy: TerminalWindowPolicy::NewWindow,
            label: None,
            keep_open: true,
            env: None,
            startup_marker: None,
            capture_output: false,
            pid_file: None,
        };
        let plan = resolve_terminal_plan(&config).unwrap();
        let joined = plan.args.join(" ");
        assert!(joined.contains("exec \"$SHELL\""), "{joined}");
        // The hold must come AFTER the command, never before it.
        let cmd_pos = joined.find("npm run dev").expect("command present");
        let hold_pos = joined.find("exec \"$SHELL\"").expect("hold present");
        assert!(hold_pos > cmd_pos, "{joined}");
    }

    /// End-to-end check of the POSIX wrapper: the marker must carry the
    /// command's REAL exit code even though the output goes through `tee`,
    /// the sidecar log must contain both streams, and the pid file must be
    /// cleaned up when the command finishes.
    #[cfg(unix)]
    #[test]
    fn posix_lifecycle_wrapper_writes_marker_log_and_pidfile() {
        let dir = std::env::temp_dir().join(format!(
            "sp_term_lifecycle_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("marker.txt");
        let pid_file = dir.join("pid.txt");
        let config = TerminalConfig {
            backend: TerminalBackend::Xterm,
            command: "echo hello-from-wrapper; echo err-line >&2; false".to_string(),
            working_dir: None,
            window_policy: TerminalWindowPolicy::NewWindow,
            label: None,
            keep_open: false,
            env: None,
            startup_marker: Some(marker.to_string_lossy().into_owned()),
            capture_output: true,
            pid_file: Some(pid_file.to_string_lossy().into_owned()),
        };
        let plan = resolve_terminal_plan(&config).unwrap();
        let idx = plan
            .args
            .iter()
            .position(|a| a == "-c")
            .expect("xterm plan carries sh -c");
        let inner = plan.args[idx + 1].clone();

        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(&inner)
            .status()
            .expect("wrapper must run");
        assert!(status.success(), "wrapper itself must exit cleanly");

        let code: i32 = std::fs::read_to_string(&marker)
            .expect("marker written")
            .trim()
            .parse()
            .expect("marker holds a number");
        assert_eq!(code, 1, "marker must carry the command's real exit code");

        let log = std::fs::read_to_string(dir.join("marker.txt.log")).expect("log written");
        assert!(log.contains("hello-from-wrapper"), "{log}");
        assert!(log.contains("err-line"), "{log}");
        assert!(
            !pid_file.exists(),
            "pid file must be removed after the command"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The pid file must record a live process group while the command runs,
    /// so Stop/Cancel can kill a dev server started in a terminal window.
    #[cfg(unix)]
    #[test]
    fn posix_lifecycle_wrapper_records_live_process_group() {
        let dir = std::env::temp_dir().join(format!(
            "sp_term_pgid_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let pid_file = dir.join("pid.txt");
        let config = TerminalConfig {
            backend: TerminalBackend::Xterm,
            command: "sleep 30".to_string(),
            working_dir: None,
            window_policy: TerminalWindowPolicy::NewWindow,
            label: None,
            keep_open: false,
            env: None,
            startup_marker: None,
            capture_output: false,
            pid_file: Some(pid_file.to_string_lossy().into_owned()),
        };
        let plan = resolve_terminal_plan(&config).unwrap();
        let idx = plan.args.iter().position(|a| a == "-c").unwrap();
        let inner = plan.args[idx + 1].clone();

        // Spawn the wrapper as a session/group leader, exactly like a
        // terminal emulator does: the recorded pgid then belongs to the
        // command's own session, never to the app's process group.
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(&inner);
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let mut child = cmd.spawn().expect("wrapper must spawn");
        // Wait for the pid file to appear.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let content = loop {
            if let Ok(text) = std::fs::read_to_string(&pid_file) {
                if !text.trim().is_empty() {
                    break text;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "pid file was never written"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let nums: Vec<i32> = content
            .split_whitespace()
            .filter_map(|v| v.parse().ok())
            .collect();
        assert_eq!(nums.len(), 2, "pid file must hold pid and pgid: {content}");
        assert!(nums[0] > 1 && nums[1] > 1, "invalid recorded ids: {content}");
        assert_eq!(nums[1], nums[0], "session leader must be its own group");
        // The recorded group must be alive while the command runs.
        assert_eq!(
            unsafe { libc::kill(-nums[1], 0) },
            0,
            "recorded process group must be alive"
        );

        // The real kill path: the recorded group is signalled and the pid
        // file is consumed.
        assert!(
            crate::modules::workspace::process_manager::kill_recorded_process_group(&pid_file)
                .unwrap(),
            "the live recorded group must be killed"
        );
        let _ = child.wait();
        assert!(
            unsafe { libc::kill(-nums[1], 0) } != 0,
            "the recorded group must be gone after the kill"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Windows terminals hold the window themselves (`cmd /K`,
    /// PowerShell `-NoExit`) — the POSIX hold must never leak into their
    /// command lines.
    #[test]
    fn windows_plans_do_not_get_posix_hold() {
        for backend in [TerminalBackend::Cmd, TerminalBackend::PowerShell] {
            let config = TerminalConfig {
                backend,
                command: "npm run dev".to_string(),
                working_dir: None,
                window_policy: TerminalWindowPolicy::NewWindow,
                label: None,
                keep_open: true,
                env: None,
                startup_marker: None,
                capture_output: false,
                pid_file: None,
            };
            let plan = resolve_terminal_plan(&config).unwrap();
            let joined = plan.args.join(" ");
            assert!(!joined.contains("exec \"$SHELL\""), "{joined}");
        }
    }

    /// Compose bootstraps opt into output capture: the exit-code marker must
    /// still carry the REAL exit code while the output is teed to the sidecar
    /// log (a pipeline's status is `tee`'s, so the code is read before it).
    #[test]
    fn posix_capture_plan_tees_output_and_keeps_real_exit_code() {
        let config = TerminalConfig {
            backend: TerminalBackend::Xterm,
            command: "docker compose up -d".to_string(),
            working_dir: None,
            window_policy: TerminalWindowPolicy::NewWindow,
            label: None,
            keep_open: true,
            env: None,
            startup_marker: Some("/tmp/probe/m.txt".to_string()),
            capture_output: true,
            pid_file: None,
        };
        let plan = resolve_terminal_plan(&config).unwrap();
        let joined = plan.args.join(" ");
        assert!(joined.contains("__dl_exit=$?"), "{joined}");
        assert!(
            joined.contains("echo \"$__dl_exit\" > '/tmp/probe/m.txt'"),
            "{joined}"
        );
        assert!(joined.contains("tee '/tmp/probe/m.txt.log'"), "{joined}");
        assert!(joined.contains("docker compose up -d"), "{joined}");
        assert_eq!(startup_log_path("/tmp/probe/m.txt"), "/tmp/probe/m.txt.log");
    }

    /// Ptyxis (GNOME's default terminal) takes `--` plus structured options,
    /// not `-e`; the legacy custom-terminal pattern is not its interface.
    #[test]
    fn ptyxis_plan_uses_structured_options() {
        let config = TerminalConfig {
            backend: TerminalBackend::Ptyxis,
            command: "npm start".to_string(),
            working_dir: Some("/tmp/project".to_string()),
            window_policy: TerminalWindowPolicy::NewWindow,
            label: Some("Dev".to_string()),
            keep_open: false,
            env: None,
            startup_marker: None,
            capture_output: false,
            pid_file: None,
        };
        let plan = resolve_terminal_plan(&config).unwrap();
        assert_eq!(plan.program, "ptyxis");
        assert!(plan.args.iter().any(|a| a == "--new-window"));
        assert!(plan.args.iter().any(|a| a == "--working-directory"));
        assert!(plan.args.iter().any(|a| a == "--"));
        assert!(!plan.args.iter().any(|a| a == "-e"));
    }

    /// The freedesktop helper receives the command after `--`, with the
    /// title/dir as structured options; it selects the actual terminal.
    #[test]
    fn xdg_terminal_exec_plan_is_structured() {
        let config = TerminalConfig {
            backend: TerminalBackend::XdgTerminalExec,
            command: "npm start".to_string(),
            working_dir: Some("/tmp/project".to_string()),
            window_policy: TerminalWindowPolicy::NewWindow,
            label: Some("Dev".to_string()),
            keep_open: false,
            env: None,
            startup_marker: None,
            capture_output: false,
            pid_file: None,
        };
        let plan = resolve_terminal_plan(&config).unwrap();
        assert_eq!(plan.program, "xdg-terminal-exec");
        assert!(plan.args.iter().any(|a| a.starts_with("--title=")));
        assert!(plan.args.iter().any(|a| a == "--dir=/tmp/project"));
        assert!(plan.args.iter().any(|a| a == "--"));
    }

    /// Alacritty takes the command as trailing argv after `--command`; the
    /// old code passed one pre-joined string, which `execve` cannot run.
    #[test]
    fn alacritty_plan_passes_argv_not_prejoined_string() {
        let config = TerminalConfig {
            backend: TerminalBackend::Alacritty,
            command: "npm start".to_string(),
            working_dir: None,
            window_policy: TerminalWindowPolicy::NewWindow,
            label: None,
            keep_open: false,
            env: None,
            startup_marker: None,
            capture_output: false,
            pid_file: None,
        };
        let plan = resolve_terminal_plan(&config).unwrap();
        let idx = plan
            .args
            .iter()
            .position(|a| a == "--command")
            .expect("--command present");
        assert_eq!(plan.args[idx + 1], "sh");
        assert_eq!(plan.args[idx + 2], "-c");
        assert_eq!(plan.args[idx + 3], "npm start");
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
            capture_output: false,
            pid_file: None,
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
