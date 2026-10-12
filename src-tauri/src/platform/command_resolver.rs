//! Authoritative command resolution for all platforms.
//!
//! This module is the single source of truth for resolving commands from
//! user-facing strings into structured, spawnable specifications. Every
//! other module (launch_engine, orchestrator, process_manager) must
//! delegate to this resolver instead of implementing ad-hoc resolution.
//!
//! # Design principles
//!
//! - Arguments remain structured (`Vec<String>`) until a shell script
//!   is intentionally constructed.
//! - `split_whitespace` is never used for parsing user arguments.
//! - Every resolution returns structured diagnostics.
//! - Flatpak and wrapper commands are resolved as structured launchers
//!   (e.g. `program="flatpak", args=["run", "<id>"]`), never as a
//!   single concatenated path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::host::{current_os, HostOs};
use super::paths::is_batch_file;
use super::shell::{parse_shell, ShellKind};

// ---------------------------------------------------------------------------
// Structured command specification
// ---------------------------------------------------------------------------

/// Structured command specification вЂ” the authoritative representation
/// of a command to execute. Unlike a flat string, this preserves the
/// program, arguments, shell intent, and environment as distinct fields.
///
/// Arguments are always structured (`Vec<String>`). They are only
/// concatenated into a shell script when the operation explicitly
/// requires shell execution (e.g. `ExecuteScript`, terminal launch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCommand {
    /// The program to execute (absolute path or bare name resolvable via PATH).
    pub program: String,
    /// Structured arguments вЂ” never joined with `split_whitespace`.
    pub args: Vec<String>,
    /// If set, the command must be executed through this shell.
    pub shell: Option<ShellKind>,
    /// Optional inline script content (for `ExecuteScript` steps).
    pub script: Option<String>,
    /// Optional environment overlay scoped to this command execution.
    pub env: Option<HashMap<String, String>>,
    /// Optional working directory override.
    pub cwd: Option<String>,
}

impl ResolvedCommand {
    /// Create a direct command (no shell wrapping).
    pub fn direct(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            shell: None,
            script: None,
            env: None,
            cwd: None,
        }
    }

    /// Create a shell-wrapped command.
    pub fn via_shell(shell: ShellKind, program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            shell: Some(shell),
            script: None,
            env: None,
            cwd: None,
        }
    }

    /// Create a script execution command.
    pub fn script(script: String, shell: Option<ShellKind>) -> Self {
        Self {
            program: String::new(),
            args: Vec::new(),
            shell,
            script: Some(script),
            env: None,
            cwd: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Resolution diagnostics
// ---------------------------------------------------------------------------

/// Severity level for a resolution diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

/// A structured diagnostic emitted during command resolution.
#[derive(Debug, Clone)]
pub struct ResolutionDiagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
}

/// The result of resolving a command: the resolved command plus any
/// diagnostics collected during resolution.
#[derive(Debug)]
pub struct ResolvedResult {
    pub command: ResolvedCommand,
    pub diagnostics: Vec<ResolutionDiagnostic>,
}

// ---------------------------------------------------------------------------
// Platform path overlay configuration
// ---------------------------------------------------------------------------

/// Additional PATH directories to search during resolution, configured
/// per-project or per-profile.
#[derive(Debug, Clone, Default)]
pub struct PathOverlay {
    pub entries: Vec<String>,
}

impl PathOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_entry(mut self, entry: impl Into<String>) -> Self {
        self.entries.push(entry.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Windows-specific resolution
// ---------------------------------------------------------------------------

/// Known npm-ecosystem tool names that ship as `.cmd`/`.bat` shims on Windows.
#[allow(dead_code)]
const NPM_ECOSYSTEM_SHIMS: &[&str] = &[
    "npx",
    "npm",
    "pnpm",
    "yarn",
    "vite",
    "nest",
    "turbo",
    "nx",
    "tsx",
    "nodemon",
    "expo",
    "next",
    "nuxt",
    "eslint",
    "prettier",
    "sass",
    "rimraf",
    "cross-env",
    "concurrently",
    "wait-on",
    "serve",
    "webpack",
    "rollup",
    "parcel",
    "jest",
    "vitest",
    "mocha",
    "ts-node",
    "dotenv",
    "husky",
    "lint-staged",
    "stylelint",
    "tailwindcss",
    "postcss",
];

/// Resolve a Windows program name, appending `.cmd`/`.bat` for npm-ecosystem
/// shims that `CreateProcess` cannot run directly.
#[allow(dead_code)]
fn resolve_windows_name(name: &str) -> String {
    let trimmed = name.trim();
    let lower = trimmed.to_ascii_lowercase();

    // Already has extension or contains a path separator вЂ” pass through.
    if lower.ends_with(".cmd")
        || lower.ends_with(".bat")
        || lower.ends_with(".exe")
        || trimmed.contains('\\')
        || trimmed.contains('/')
    {
        return trimmed.to_string();
    }

    // Known npm ecosystem shims.
    if NPM_ECOSYSTEM_SHIMS.contains(&lower.as_str()) {
        return format!("{}.cmd", trimmed);
    }

    if lower == "composer" {
        return "composer.bat".to_string();
    }

    trimmed.to_string()
}

/// Resolve a program name on Windows via PATH, trying `.exe`, `.cmd`,
/// and `.bat` variants.
fn resolve_windows_path(name: &str) -> Option<PathBuf> {
    let candidates = [
        name.to_string(),
        format!("{}.cmd", name),
        format!("{}.bat", name),
    ];
    for candidate in &candidates {
        if let Ok(path) = which::which(candidate) {
            return Some(path);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Unix-specific resolution
// ---------------------------------------------------------------------------

/// Resolve a program name on Unix via PATH.
fn resolve_unix_path(name: &str) -> Option<PathBuf> {
    which::which(name).ok()
}

// ---------------------------------------------------------------------------
// Public resolution API
// ---------------------------------------------------------------------------

/// Resolve a bare command name or path to an absolute executable path.
///
/// This is the authoritative resolution function used by all modules.
/// On Windows it handles `.cmd`/`.bat` shim detection. On Unix it
/// delegates to `which`.
///
/// # Arguments
///
/// * `name` вЂ” bare command name (e.g. `"npm"`) or absolute/relative path.
/// * `path_overlay` вЂ” additional PATH entries to search before system PATH.
pub fn resolve_executable(name: &str, path_overlay: Option<&PathOverlay>) -> Option<PathBuf> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Absolute or relative path with separator вЂ” check directly.
    if trimmed.contains('/') || trimmed.contains('\\') {
        let p = Path::new(trimmed);
        if p.is_file() {
            return Some(p.to_path_buf());
        }
        // Windows: also try adding .exe/.cmd/.bat to a bare path.
        #[cfg(target_os = "windows")]
        {
            for suffix in [".exe", ".cmd", ".bat"] {
                let candidate = format!("{}{}", trimmed, suffix);
                if Path::new(&candidate).is_file() {
                    return Some(PathBuf::from(candidate));
                }
            }
        }
        return None;
    }

    // Search PATH overlay first, then system PATH.
    if let Some(overlay) = path_overlay {
        for entry in &overlay.entries {
            let candidate = Path::new(entry).join(trimmed);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    let os = current_os();
    match os {
        HostOs::Windows => resolve_windows_path(trimmed),
        HostOs::Linux | HostOs::Macos => resolve_unix_path(trimmed),
    }
}

/// Resolve a command string into a structured [`ResolvedCommand`].
///
/// This is the main entry point for turning a user-provided command
/// string into a spawnable specification. The resolution is
/// platform-aware and returns diagnostics for any issues encountered.
///
/// # Arguments
///
/// * `program` вЂ” the program name or path.
/// * `args` вЂ” structured arguments (never `split_whitespace`).
/// * `explicit_shell` вЂ” optional shell override from the step definition.
/// * `path_overlay` вЂ” additional PATH entries to search.
pub fn resolve_command(
    program: &str,
    args: Vec<String>,
    explicit_shell: Option<ShellKind>,
    path_overlay: Option<&PathOverlay>,
) -> ResolvedResult {
    let mut diagnostics = Vec::new();
    let trimmed = program.trim();

    if trimmed.is_empty() {
        return ResolvedResult {
            command: ResolvedCommand::direct(String::new(), args),
            diagnostics: vec![ResolutionDiagnostic {
                severity: DiagnosticSeverity::Error,
                message: "Empty command program".to_string(),
            }],
        };
    }

    let os = current_os();

    // Determine shell mode.
    let shell_mode = if let Some(shell) = explicit_shell {
        Some(shell)
    } else {
        None
    };

    // Resolve the program path.
    let resolved_program = match resolve_executable(trimmed, path_overlay) {
        Some(path) => {
            let path_str = path.to_string_lossy().into_owned();

            // Warn about batch shims when no explicit shell is set.
            if is_batch_file(&path_str) && shell_mode.is_none() {
                diagnostics.push(ResolutionDiagnostic {
                    severity: DiagnosticSeverity::Info,
                    message: format!(
                        "Resolved '{}' to batch shim '{}'. \
                         Will execute through cmd /C on Windows.",
                        trimmed, path_str
                    ),
                });
            }

            path_str
        }
        None => {
            diagnostics.push(ResolutionDiagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "Command '{}' not found on PATH. \
                     Ensure it is installed and accessible.",
                    trimmed
                ),
            });
            trimmed.to_string()
        }
    };

    // Determine effective shell.
    let effective_shell = shell_mode.or_else(|| {
        // On Windows, batch files must go through cmd.
        if is_batch_file(&resolved_program) {
            return Some(ShellKind::Cmd);
        }
        // On Unix, default to sh for backward compatibility.
        match os {
            HostOs::Windows => None,
            HostOs::Linux | HostOs::Macos => Some(ShellKind::Sh),
        }
    });

    let command = if let Some(shell) = effective_shell {
        ResolvedCommand::via_shell(shell, resolved_program, args)
    } else {
        ResolvedCommand::direct(resolved_program, args)
    };

    ResolvedResult {
        command,
        diagnostics,
    }
}

/// Resolve a command from a structured [`CommandSpec`]-like input.
///
/// This is a convenience wrapper that takes the same fields as
/// the model's `CommandSpec` and returns a fully resolved command.
pub fn resolve_from_spec(
    program: &str,
    args: &[String],
    shell: Option<&str>,
    env: Option<HashMap<String, String>>,
    cwd: Option<String>,
    path_overlay: Option<&PathOverlay>,
) -> ResolvedResult {
    let shell_kind = shell.and_then(|s| parse_shell(s).ok());
    let mut result = resolve_command(program, args.to_vec(), shell_kind, path_overlay);
    result.command.env = env;
    result.command.cwd = cwd;
    result
}

/// Resolve a full command string (e.g. `"docker compose up -d"`) into a
/// structured command.
///
/// This does NOT use `split_whitespace` for argument parsing. Instead,
/// it tokenizes the string using a proper tokenizer that respects
/// single quotes, double quotes, and backslash escaping.
pub fn resolve_command_string(
    command_str: &str,
    explicit_shell: Option<ShellKind>,
    path_overlay: Option<&PathOverlay>,
) -> ResolvedResult {
    let tokens = tokenize_command_string(command_str);
    if tokens.is_empty() {
        return ResolvedResult {
            command: ResolvedCommand::direct(String::new(), Vec::new()),
            diagnostics: vec![ResolutionDiagnostic {
                severity: DiagnosticSeverity::Error,
                message: "Empty command string".to_string(),
            }],
        };
    }

    let program = &tokens[0];
    let args = tokens[1..].to_vec();
    resolve_command(program, args, explicit_shell, path_overlay)
}

/// Resolve a command string into a spawnable `(program, args)` pair.
///
/// Uses the tokenizer-based resolver (never `split_whitespace`). When the
/// resolved command requires a shell (batch shims on Windows, shell-syntax
/// pipelines, or the default-shell resolution on Unix), the command is
/// re-assembled as a properly quoted shell script line and the shell itself
/// becomes the spawned program. The result is safe to pass to
/// `std::process::Command` / `Command::new(program).args(args)`.
pub fn resolve_command_target(command: &str) -> (String, Vec<String>) {
    let resolved = resolve_command_string(command, None, None).command;

    if resolved.program.is_empty() {
        // Empty or pure-shell-syntax command вЂ” run through the platform shell.
        let kind = super::shell::default_shell_for_platform();
        let (exe, flag) = super::shell::shell_executable(kind);
        return (exe.to_string(), vec![flag.to_string(), command.to_string()]);
    }

    if let Some(shell) = resolved.shell {
        // Windows batch shims (npm.cmd, code.cmd, ...) must NOT be wrapped
        // into a pre-built `cmd /C "<line>"` here. That single argv entry
        // contains spaces, so `std::process::Command` re-quotes it and the
        // final command line becomes `cmd /C ""C:\...\npm.cmd" install"`,
        // which cmd breaks at the first space (`'C:\Program' is not
        // recognized` в†’ exit 1) вЂ” the real reason `npm install` steps died
        // with "Process exited with error code 1" on every profile. Return
        // the shim path + args and let the process manager invoke it through
        // `cmd /C` with a RAW command tail (see `resolve_spawn_plan` /
        // `batch_shim_cmd_line`), which quotes the line exactly once.
        if cfg!(target_os = "windows") && shell == ShellKind::Cmd {
            return (resolved.program, resolved.args);
        }
        let (exe, flag) = super::shell::shell_executable(shell);
        let line = shell_script_line(&resolved.program, &resolved.args, shell);
        return (exe.to_string(), vec![flag.to_string(), line]);
    }

    (resolved.program, resolved.args)
}

/// Join a resolved program + args into a single shell script line, quoting
/// each token for the target shell.
pub fn shell_script_line(program: &str, args: &[String], shell: ShellKind) -> String {
    match shell {
        ShellKind::Cmd => {
            let mut line = super::command::win_quote_arg(program);
            for a in args {
                line.push(' ');
                line.push_str(&super::command::win_quote_arg(a));
            }
            line
        }
        ShellKind::PowerShell | ShellKind::Pwsh => {
            let mut line = program.to_string();
            for a in args {
                line.push(' ');
                line.push_str(a);
            }
            line
        }
        _ => {
            let mut line = program.to_string();
            for a in args {
                line.push(' ');
                line.push_str(&super::command::sh_quote(a));
            }
            line
        }
    }
}

/// Tokenize a command string respecting shell quoting rules.
///
/// - Single-quoted strings are literal (no escape processing).
/// - Double-quoted strings allow `\"` escaping.
/// - Backslash escapes the next character outside quotes.
///   EXCEPT on Windows, where `\` is a path separator and is always kept
///   literal вЂ” old profiles carry commands like `cmd /C "python -m venv
///   .venv && .venv\Scripts\python.exe -m pip install ..."` and stripping
///   the separators turned them into `.venvScriptspython.exe` (exit 1).
/// - Spaces separate tokens outside quotes.
///
/// This does NOT use `split_whitespace`.
fn tokenize_command_string(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
            }
            '\\' if !in_single_quote && cfg!(not(target_os = "windows")) => {
                // On Unix `\` escapes the next character — but ONLY when the
                // escaped character is a shell metacharacter. Treating every
                // backslash as an escape mangled Windows-style paths carried
                // by profiles created on another OS: `.venv\Scripts\python.exe`
                // became `.venvScriptspython.exe`, so the orchestrator's
                // cross-platform venv fallback (which accepts `Scripts`/`bin`)
                // never ran and the step failed with "not found".
                match chars.peek().copied() {
                    Some(next)
                        if next.is_whitespace()
                            || matches!(next, '"' | '\'' | '\\' | '&' | '|' | '<' | '>'
                                | '(' | ')' | ';' | '$' | '`') =>
                    {
                        chars.next();
                        current.push(next);
                    }
                    // Path separator (or a trailing backslash): keep literal.
                    _ => current.push('\\'),
                }
            }
            ' ' | '\t' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_plain_command() {
        let tokens = tokenize_command_string("npm install");
        assert_eq!(tokens, vec!["npm", "install"]);
    }

    #[test]
    fn tokenize_quoted_args() {
        let tokens = tokenize_command_string(r#"echo "hello world""#);
        assert_eq!(tokens, vec!["echo", "hello world"]);
    }

    #[test]
    fn tokenize_single_quoted() {
        let tokens = tokenize_command_string("echo 'hello world'");
        assert_eq!(tokens, vec!["echo", "hello world"]);
    }

    #[test]
    fn tokenize_backslash_escape() {
        // On Unix `\` escapes the next character when it is a shell
        // metacharacter (so `hello\ world` is one token). On Windows `\` is a
        // path separator вЂ” never an escape вЂ” so the backslash is kept and the
        // space still splits the tokens.
        let tokens = tokenize_command_string(r#"echo hello\ world"#);
        #[cfg(not(target_os = "windows"))]
        assert_eq!(tokens, vec!["echo", "hello world"]);
        #[cfg(target_os = "windows")]
        assert_eq!(tokens, vec!["echo", r"hello\", "world"]);
    }

    /// Profiles created on Windows carry backslash paths. On Unix the
    /// tokenizer must keep them intact so the orchestrator's cross-platform
    /// venv fallback (`Scripts` vs `bin`) can recognize and repair them.
    #[test]
    fn tokenize_preserves_windows_paths_on_unix() {
        #[cfg(not(target_os = "windows"))]
        {
            let tokens = tokenize_command_string(r".venv\Scripts\python.exe manage.py runserver");
            assert_eq!(
                tokens,
                vec![r".venv\Scripts\python.exe", "manage.py", "runserver"]
            );
            // A real escape (whitespace) still works.
            let tokens = tokenize_command_string(r"echo a\ b");
            assert_eq!(tokens, vec!["echo", "a b"]);
            // A literal backslash before a non-metacharacter stays literal.
            let tokens = tokenize_command_string(r"echo C:\temp\file");
            assert_eq!(tokens, vec!["echo", r"C:\temp\file"]);
        }
    }

    #[test]
    fn tokenize_preserves_windows_backslash_paths() {
        #[cfg(target_os = "windows")]
        {
            let tokens = tokenize_command_string(
                r#"cmd /C "python -m venv .venv && .venv\Scripts\python.exe -m pip install -r requirements.txt""#,
            );
            assert_eq!(
                tokens,
                vec![
                    "cmd",
                    "/C",
                    r"python -m venv .venv && .venv\Scripts\python.exe -m pip install -r requirements.txt"
                ]
            );
        }
    }

    #[test]
    fn tokenize_empty_string() {
        let tokens = tokenize_command_string("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn tokenize_multiple_spaces() {
        let tokens = tokenize_command_string("npm   install   --save");
        assert_eq!(tokens, vec!["npm", "install", "--save"]);
    }

    #[test]
    fn resolved_command_direct() {
        let cmd = ResolvedCommand::direct("node", vec!["-e".into(), "console.log(1)".into()]);
        assert_eq!(cmd.program, "node");
        assert_eq!(cmd.args, vec!["-e", "console.log(1)"]);
        assert!(cmd.shell.is_none());
    }

    #[test]
    fn resolved_command_via_shell() {
        let cmd = ResolvedCommand::via_shell(ShellKind::Cmd, "npm", vec!["install".into()]);
        assert_eq!(cmd.program, "npm");
        assert_eq!(cmd.shell, Some(ShellKind::Cmd));
    }

    #[test]
    fn resolved_command_script() {
        let cmd = ResolvedCommand::script("echo hello".to_string(), Some(ShellKind::Sh));
        assert!(cmd.script.is_some());
        assert_eq!(cmd.shell, Some(ShellKind::Sh));
    }

    #[test]
    fn resolve_empty_program_returns_error() {
        let result = resolve_command("", vec![], None, None);
        assert!(!result.diagnostics.is_empty());
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error));
    }

    #[test]
    fn resolve_command_string_parses_correctly() {
        let result = resolve_command_string("docker compose up -d", None, None);
        // Program may be resolved to full path if found in PATH
        assert!(result.command.program.contains("docker") || result.command.program == "docker");
        assert_eq!(result.command.args, vec!["compose", "up", "-d"]);
    }

    #[test]
    fn resolve_from_spec_preserves_env_and_cwd() {
        let mut env = HashMap::new();
        env.insert("NODE_ENV".to_string(), "development".to_string());
        let result = resolve_from_spec(
            "node",
            &["server.js".into()],
            None,
            Some(env),
            Some("/tmp/project".to_string()),
            None,
        );
        // Program may be resolved to full path if found in PATH
        assert!(result.command.program.contains("node") || result.command.program == "node");
        assert!(result.command.env.is_some());
        assert_eq!(result.command.cwd, Some("/tmp/project".to_string()));
    }

    #[test]
    fn resolve_executable_finds_sh_on_unix() {
        let name = if cfg!(target_os = "windows") {
            "cmd"
        } else {
            "sh"
        };
        let result = resolve_executable(name, None);
        assert!(result.is_some(), "{name} should be resolvable");
    }

    #[test]
    fn resolve_executable_returns_none_for_garbage() {
        let result = resolve_executable("this_program_definitely_does_not_exist_xyz_98765", None);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_executable_empty_returns_none() {
        assert!(resolve_executable("", None).is_none());
        assert!(resolve_executable("   ", None).is_none());
    }

    #[test]
    fn path_overlay_is_searched() {
        let overlay = PathOverlay::new().add_entry("/nonexistent/path");
        let result = resolve_executable("some_tool", Some(&overlay));
        // Should not find it, but should not panic.
        assert!(result.is_none());
    }

    #[test]
    fn windows_name_resolution_npm_ecosystem() {
        if cfg!(target_os = "windows") {
            assert_eq!(resolve_windows_name("npx"), "npx.cmd");
            assert_eq!(resolve_windows_name("npm"), "npm.cmd");
            assert_eq!(resolve_windows_name("pnpm"), "pnpm.cmd");
            assert_eq!(resolve_windows_name("yarn"), "yarn.cmd");
            assert_eq!(resolve_windows_name("composer"), "composer.bat");
        }
    }

    #[test]
    fn windows_name_keeps_extensions() {
        if cfg!(target_os = "windows") {
            assert_eq!(
                resolve_windows_name(r"C:\tools\script.cmd"),
                r"C:\tools\script.cmd"
            );
            assert_eq!(resolve_windows_name("node.exe"), "node.exe");
        }
    }

    #[test]
    fn resolve_command_string_with_quotes() {
        let result = resolve_command_string(r#"node -e "console.log('hello world')""#, None, None);
        // Program may be resolved to full path if found in PATH
        assert!(result.command.program.contains("node") || result.command.program == "node");
        assert_eq!(
            result.command.args,
            vec!["-e", "console.log('hello world')"]
        );
    }

    #[test]
    fn resolve_command_string_docker_compose() {
        let result =
            resolve_command_string("docker compose -f docker-compose.yml up -d", None, None);
        // Program may be resolved to full path if found in PATH
        assert!(result.command.program.contains("docker") || result.command.program == "docker");
        assert_eq!(
            result.command.args,
            vec!["compose", "-f", "docker-compose.yml", "up", "-d"]
        );
    }

    /// Regression test for the `npm install` в†’ exit code 1 bug: on Windows a
    /// batch shim (npm.cmd, code.cmd, ...) must be returned as the program
    /// itself so the process manager wraps it through `cmd /C` with a RAW
    /// command tail. Pre-wrapping it into `cmd /C "<quoted line>"` made
    /// `std::process::Command` double-quote the line, so cmd broke it at the
    /// first space and every install step died with `'C:\Program' is not
    /// recognized`.
    #[test]
    fn resolve_command_target_never_prewraps_windows_batch_shims() {
        // Backslash Windows path: the tokenizer must preserve `\` (it is a
        // path separator, not a shell escape on Windows) and the batch shim
        // must NOT be pre-wrapped into a `cmd /C "<quoted line>"` string.
        let (program, args) = resolve_command_target(r"C:\fake\tools\npm.cmd install");
        #[cfg(target_os = "windows")]
        {
            assert_eq!(program, r"C:\fake\tools\npm.cmd");
            assert_eq!(args, vec!["install"]);
            assert!(
                !program.eq_ignore_ascii_case("cmd"),
                "batch shims must not be pre-wrapped into cmd /C"
            );
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (program, args);
        }
    }

    #[test]
    fn resolve_command_target_wraps_shell_syntax_on_unix() {
        // Unix-style shell syntax still routes through the platform shell
        // (guarded: the assertion only holds on non-Windows).
        let (program, args) = resolve_command_target("echo hello | grep hello");
        #[cfg(not(target_os = "windows"))]
        {
            assert!(!program.is_empty());
            assert_eq!(args.len(), 2);
            assert!(args[0].starts_with('-'));
        }
        #[cfg(target_os = "windows")]
        {
            let _ = (program, args);
        }
    }
}
