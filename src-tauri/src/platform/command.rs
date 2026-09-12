use super::host::{current_os, HostOs};
use super::shell::{default_shell_for_platform, resolve_shell, ShellKind};
#[cfg(target_os = "windows")]
use super::{suppress_child_console, suppress_child_console_async};

/// How a command should be launched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandMode {
    /// Run the program directly (no shell wrapping).
    Direct,
    /// Run through a shell: shell + flag + single script argument.
    Shell(ShellKind),
}

/// Error returned when an explicit shell/OS combination is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandBuildError {
    /// Shell is only available on Windows but running on Unix.
    WindowsOnlyShell { shell: ShellKind, os: HostOs },
    /// Shell is only available on Unix but running on Windows.
    UnixOnlyShell { shell: ShellKind, os: HostOs },
}

impl std::fmt::Display for CommandBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandBuildError::WindowsOnlyShell { shell, os } => {
                write!(
                    f,
                    "shell '{shell}' is only available on Windows, current OS is {os}"
                )
            }
            CommandBuildError::UnixOnlyShell { shell, os } => {
                write!(
                    f,
                    "shell '{shell}' is only available on Unix, current OS is {os}"
                )
            }
        }
    }
}

impl std::error::Error for CommandBuildError {}

/// Build a [`tokio::process::Command`] for the given program and arguments.
///
/// # Direct mode
///
/// The executable and arguments are passed directly to `Command::arg(s)`.
/// No shell parsing or concatenation is used. This is the correct mode for
/// running executables that accept arguments natively (e.g. `node`, `cargo`).
///
/// # Shell mode
///
/// The command is constructed as `shell <flag> <script_text>` where `<script_text>`
/// is the full script to run inside the shell. On Windows, batch files (`.cmd`/`.bat`)
/// are always launched through `cmd /C`.
///
/// # Errors
///
/// Returns `Err(CommandBuildError)` when an explicit shell is incompatible with
/// the current OS (e.g. `ShellKind::Cmd` on Linux).
pub fn build_tokio_command(
    program: &str,
    args: &[String],
    mode: CommandMode,
) -> Result<tokio::process::Command, CommandBuildError> {
    let os = current_os();

    match mode {
        CommandMode::Direct => {
            let resolved = resolve_program_for_direct(program, os);
            let mut cmd = tokio::process::Command::new(&resolved);
            cmd.args(args);
            #[cfg(target_os = "windows")]
            suppress_child_console_async(&mut cmd);
            Ok(cmd)
        }
        CommandMode::Shell(shell) => {
            validate_shell_os(shell, os)?;
            let resolved_shell = resolve_shell(shell);
            let (exe, flag) = super::shell::shell_executable(resolved_shell);
            let mut cmd = tokio::process::Command::new(exe);
            cmd.arg(flag);
            // Build the full script line for the shell
            let script = build_script_line(program, args, resolved_shell);
            append_shell_script_tokio(&mut cmd, resolved_shell, script);
            #[cfg(target_os = "windows")]
            suppress_child_console_async(&mut cmd);
            Ok(cmd)
        }
    }
}

/// Build a [`std::process::Command`] for the given program and arguments.
pub fn build_std_command(
    program: &str,
    args: &[String],
    mode: CommandMode,
) -> Result<std::process::Command, CommandBuildError> {
    let os = current_os();

    match mode {
        CommandMode::Direct => {
            let resolved = resolve_program_for_direct(program, os);
            let mut cmd = std::process::Command::new(&resolved);
            cmd.args(args);
            #[cfg(target_os = "windows")]
            suppress_child_console(&mut cmd);
            Ok(cmd)
        }
        CommandMode::Shell(shell) => {
            validate_shell_os(shell, os)?;
            let resolved_shell = resolve_shell(shell);
            let (exe, flag) = super::shell::shell_executable(resolved_shell);
            let mut cmd = std::process::Command::new(exe);
            cmd.arg(flag);
            let script = build_script_line(program, args, resolved_shell);
            append_shell_script_std(&mut cmd, resolved_shell, script);
            #[cfg(target_os = "windows")]
            suppress_child_console(&mut cmd);
            Ok(cmd)
        }
    }
}

/// Determine the correct command mode for a given program and optional
/// explicit shell, preserving existing Windows batch-file behavior.
///
/// This function encodes the platform-specific dispatch rules:
///
/// - **Windows batch files** (`.cmd`/`.bat`): always `cmd /D /C`.
/// - **PowerShell heuristics**: if the command looks like PowerShell
///   (`$?`, `$LASTEXITCODE`, `Get-`, `Set-`, etc.), use `powershell -Command`.
/// - **Unix**: by default, run through `sh -c` to preserve existing behavior.
/// - **Explicit shell**: use the caller's choice.
pub fn infer_command_mode(
    program: &str,
    args: &[String],
    explicit_shell: Option<ShellKind>,
) -> CommandMode {
    if let Some(shell) = explicit_shell {
        return CommandMode::Shell(shell);
    }

    let os = current_os();
    match os {
        HostOs::Windows => infer_windows_mode(program, args),
        HostOs::Linux | HostOs::Macos => {
            // Unix: run through shell to preserve existing behavior
            CommandMode::Shell(ShellKind::Sh)
        }
    }
}

/// Windows-specific mode inference preserving existing behavior.
fn infer_windows_mode(program: &str, _args: &[String]) -> CommandMode {
    if is_powershell_command(program) {
        return CommandMode::Shell(ShellKind::PowerShell);
    }
    if is_batch_file(program) || is_batch_file(&resolve_windows_program_name(program)) {
        return CommandMode::Shell(ShellKind::Cmd);
    }
    CommandMode::Direct
}

/// Check if a command looks like a PowerShell command based on heuristic
/// patterns (preserved from original `build_command` logic).
pub fn is_powershell_command(command: &str) -> bool {
    command.starts_with("powershell")
        || command.starts_with("pwsh")
        || command.contains("Get-")
        || command.contains("Set-")
        || command.contains("Invoke-")
        || command.contains("New-")
        || command.contains("$?")
        || command.contains("$LASTEXITCODE")
}

/// Resolve a Windows program name for batch detection.
///
/// The npm ecosystem installs commands as `.cmd` files; `CreateProcess`
/// cannot run `.cmd`/`.bat` without cmd.exe, so we add the extension.
pub fn resolve_windows_program_name(command: &str) -> String {
    let trimmed = command.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.ends_with(".cmd")
        || lower.ends_with(".bat")
        || lower.ends_with(".exe")
        || trimmed.contains('\\')
        || trimmed.contains('/')
    {
        return trimmed.to_string();
    }
    match lower.as_str() {
        "npx" | "npm" | "pnpm" | "yarn" | "vite" | "nest" | "turbo" | "nx" | "tsx" | "nodemon"
        | "expo" | "next" | "nuxt" | "eslint" | "prettier" | "sass" | "rimraf" | "cross-env"
        | "concurrently" | "wait-on" | "serve" | "webpack" | "rollup" | "parcel" | "jest"
        | "vitest" | "mocha" | "ts-node" | "dotenv" | "husky" | "lint-staged" | "stylelint"
        | "tailwindcss" | "postcss" => {
            format!("{trimmed}.cmd")
        }
        "composer" => "composer.bat".to_string(),
        "mix" => "mix.bat".to_string(),
        "flutter" => "flutter.bat".to_string(),
        "mvn" => "mvn.cmd".to_string(),
        "mvnw" => "mvnw.cmd".to_string(),
        "gradle" => "gradle.bat".to_string(),
        "gradlew" => "gradlew.bat".to_string(),
        _ => trimmed.to_string(),
    }
}

/// Resolve a bare command name to a real executable path when possible.
///
/// On Windows this first checks whether the name is a batch shim on PATH
/// (e.g. `code.cmd`), which `CreateProcess` cannot run directly. Returns
/// the resolved absolute path when found.
pub fn resolve_program_path(program: &str) -> Option<String> {
    let trimmed = program.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Some(trimmed.to_string());
    }
    #[cfg(target_os = "windows")]
    {
        for candidate in [
            format!("{}.cmd", trimmed),
            format!("{}.bat", trimmed),
            trimmed.to_string(),
        ] {
            if let Ok(path) = which::which(&candidate) {
                return Some(path.to_string_lossy().into_owned());
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(path) = which::which(trimmed) {
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}

/// Resolve a program for direct execution on the given OS.
fn resolve_program_for_direct(program: &str, os: HostOs) -> String {
    match os {
        HostOs::Windows => resolve_windows_program_name(program),
        HostOs::Linux | HostOs::Macos => program.trim().to_string(),
    }
}

/// Outcome of resolving a command for spawning: the program to actually
/// launch plus how it must be invoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnPlan {
    /// The executable to pass to `Command::new`.
    pub program: String,
    /// True when the program is a Windows batch shim (`.cmd`/`.bat`) that
    /// must be launched through `cmd /C` because `CreateProcess` cannot
    /// execute batch files directly.
    pub batch_shim: bool,
}

/// Resolve an arbitrary command string into a spawnable [`SpawnPlan`].
///
/// - On Windows, bare npm-ecosystem names (`npm`, `npx`, `pnpm`, `yarn`,
///   `vite`, `nest`, `composer`) are expanded to their `.cmd`/`.bat` shims
///   and flagged as batch shims so the caller wraps them in `cmd /C`.
/// - Any other bare name is resolved through PATH (PATHEXT-aware), so
///   `code` → `code.cmd` or `cursor` → `Cursor.exe` also work.
/// - Existing absolute paths pass through unchanged.
/// - On Unix the program is passed through verbatim.
pub fn resolve_spawn_plan(program: &str) -> SpawnPlan {
    let os = current_os();
    let trimmed = program.trim();
    if trimmed.is_empty() {
        return SpawnPlan {
            program: program.to_string(),
            batch_shim: false,
        };
    }

    match os {
        HostOs::Windows => {
            // 1. Known npm-ecosystem shims and explicit paths.
            let resolved = resolve_windows_program_name(trimmed);
            let explicit_batch = is_batch_file(&resolved);

            if explicit_batch {
                return SpawnPlan {
                    program: resolved,
                    batch_shim: true,
                };
            }

            // 2. PATH resolution: prefer a real `.exe` (e.g. `docker.exe`),
            // otherwise fall back to `.cmd`/`.bat` shims (e.g. `code.cmd`).
            if !trimmed.contains('/') && !trimmed.contains('\\') {
                let lower = trimmed.to_ascii_lowercase();
                if !lower.ends_with(".exe") && !lower.ends_with(".cmd") && !lower.ends_with(".bat")
                {
                    for candidate in [
                        trimmed.to_string(),
                        format!("{}.cmd", trimmed),
                        format!("{}.bat", trimmed),
                    ] {
                        if let Ok(path) = which::which(&candidate) {
                            let found = path.to_string_lossy().into_owned();
                            let is_batch = is_batch_file(&found);
                            return SpawnPlan {
                                program: found,
                                batch_shim: is_batch,
                            };
                        }
                    }
                }
            }

            SpawnPlan {
                program: resolved,
                batch_shim: false,
            }
        }
        HostOs::Linux | HostOs::Macos => SpawnPlan {
            program: trimmed.to_string(),
            batch_shim: false,
        },
    }
}

/// Build the argv for launching a resolved batch shim through `cmd /C`:
/// `cmd /C "<shim>" <quoted args...>`.
pub fn batch_shim_cmd_line(program: &str, args: &[&str]) -> Vec<String> {
    let mut line = win_quote_arg(program);
    for arg in args {
        line.push(' ');
        line.push_str(&win_quote_arg(arg));
    }
    vec!["/C".to_string(), line]
}

/// Validate that a shell is available on the current OS.
fn validate_shell_os(shell: ShellKind, os: HostOs) -> Result<(), CommandBuildError> {
    match (shell, os) {
        (ShellKind::Cmd | ShellKind::PowerShell, HostOs::Linux | HostOs::Macos) => {
            Err(CommandBuildError::WindowsOnlyShell { shell, os })
        }
        (ShellKind::Sh | ShellKind::Bash | ShellKind::Zsh, HostOs::Windows) => {
            Err(CommandBuildError::UnixOnlyShell { shell, os })
        }
        _ => Ok(()),
    }
}

/// Build the script line passed to the shell.
///
/// For `cmd`: each argument is quoted per cmd.exe rules, joined by spaces.
/// For Unix shells: each argument is sh-quoted, joined by spaces.
/// For PowerShell: arguments are joined by spaces (PS handles quoting).
fn build_script_line(program: &str, args: &[String], shell: ShellKind) -> String {
    match shell {
        ShellKind::Cmd => {
            let mut line = win_quote_arg(program);
            for arg in args {
                line.push(' ');
                line.push_str(&win_quote_arg(arg));
            }
            line
        }
        ShellKind::PowerShell | ShellKind::Pwsh => {
            let mut line = program.to_string();
            for arg in args {
                line.push(' ');
                line.push_str(arg);
            }
            line
        }
        ShellKind::Sh | ShellKind::Bash | ShellKind::Zsh => {
            let mut line = program.to_string();
            for arg in args {
                line.push(' ');
                line.push_str(&sh_quote(arg));
            }
            line
        }
        ShellKind::Default => {
            let resolved = default_shell_for_platform();
            build_script_line(program, args, resolved)
        }
    }
}

/// Append the pre-quoted shell script to a command.
///
/// `cmd /C <script>` is special: the script is one opaque command-line tail,
/// not a regular argv entry. Rust's default argument escaping turns the
/// script's embedded quotes (e.g. `--before-dev-command "cd frontend && npm
/// run dev"`) into `\"...\"`, which cmd.exe does not understand — the value is
/// split on spaces and the `&&` breaks the command. `raw_arg` appends the
/// script verbatim, preserving the quoting produced by [`build_script_line`].
fn append_shell_script_tokio(
    cmd: &mut tokio::process::Command,
    shell: ShellKind,
    script: String,
) {
    #[cfg(not(target_os = "windows"))]
    let _ = shell;
    #[cfg(target_os = "windows")]
    {
        if shell == ShellKind::Cmd {
            // tokio::process::Command has an inherent `raw_arg` on Windows.
            cmd.raw_arg(&script);
            return;
        }
    }
    cmd.arg(script);
}

/// Same as [`append_shell_script_tokio`] for [`std::process::Command`].
fn append_shell_script_std(cmd: &mut std::process::Command, shell: ShellKind, script: String) {
    #[cfg(not(target_os = "windows"))]
    let _ = shell;
    #[cfg(target_os = "windows")]
    {
        if shell == ShellKind::Cmd {
            use std::os::windows::process::CommandExt;
            cmd.raw_arg(&script);
            return;
        }
    }
    cmd.arg(script);
}

/// Quote an argument for `cmd /C`: double quotes with internal quote doubling.
pub fn win_quote_arg(arg: &str) -> String {
    let needs_quote = arg.is_empty()
        || arg
            .chars()
            .any(|c| c.is_whitespace() || "&()[]{}<>^|%!\"".contains(c));
    if !needs_quote {
        arg.to_string()
    } else {
        format!("\"{}\"", arg.replace('"', "\"\""))
    }
}

/// Quote an argument for `sh -c`: single quotes with internal quote escaping.
pub fn sh_quote(arg: &str) -> String {
    let needs_quote = arg.is_empty()
        || arg
            .chars()
            .any(|c| c.is_whitespace() || "&;|<>()$`\\\"'*?[]~#!{}".contains(c));
    if !needs_quote {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// Convenience: detect if a command is a batch file.
fn is_batch_file(name: &str) -> bool {
    super::paths::is_batch_file(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Shell default selection tests
    // ========================================================================

    #[test]
    fn shell_default_matches_platform() {
        let os = current_os();
        let default = default_shell_for_platform();
        match os {
            HostOs::Windows => assert_eq!(default, ShellKind::Cmd),
            HostOs::Linux | HostOs::Macos => assert_eq!(default, ShellKind::Sh),
        }
    }

    // ========================================================================
    // Unsupported shell/platform combination tests
    // ========================================================================

    #[test]
    fn cmd_on_unix_is_error() {
        let os = HostOs::Linux;
        let result = validate_shell_os(ShellKind::Cmd, os);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("only available on Windows"));
    }

    #[test]
    fn powershell_on_unix_is_error() {
        let os = HostOs::Macos;
        let result = validate_shell_os(ShellKind::PowerShell, os);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("only available on Windows"));
    }

    #[test]
    fn sh_on_windows_is_error() {
        let os = HostOs::Windows;
        let result = validate_shell_os(ShellKind::Sh, os);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("only available on Unix"));
    }

    #[test]
    fn bash_on_windows_is_error() {
        let os = HostOs::Windows;
        let result = validate_shell_os(ShellKind::Bash, os);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("only available on Unix"));
    }

    #[test]
    fn zsh_on_windows_is_error() {
        let os = HostOs::Windows;
        let result = validate_shell_os(ShellKind::Zsh, os);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("only available on Unix"));
    }

    #[test]
    fn pwsh_on_any_os_is_ok() {
        assert!(validate_shell_os(ShellKind::Pwsh, HostOs::Windows).is_ok());
        assert!(validate_shell_os(ShellKind::Pwsh, HostOs::Linux).is_ok());
        assert!(validate_shell_os(ShellKind::Pwsh, HostOs::Macos).is_ok());
    }

    #[test]
    fn cmd_on_windows_is_ok() {
        assert!(validate_shell_os(ShellKind::Cmd, HostOs::Windows).is_ok());
    }

    #[test]
    fn powershell_on_windows_is_ok() {
        assert!(validate_shell_os(ShellKind::PowerShell, HostOs::Windows).is_ok());
    }

    // ========================================================================
    // Windows batch command construction tests
    // ========================================================================

    #[cfg(target_os = "windows")]
    #[test]
    fn batch_file_uses_cmd_shell() {
        let mode = infer_command_mode("npx.cmd", &["create-vite@latest".into()], None);
        assert_eq!(mode, CommandMode::Shell(ShellKind::Cmd));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn bat_file_uses_cmd_shell() {
        let mode = infer_command_mode("composer.bat", &["install".into()], None);
        assert_eq!(mode, CommandMode::Shell(ShellKind::Cmd));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn npm_ecosystem_resolves_to_cmd() {
        let mode = infer_command_mode("npm", &["install".into()], None);
        assert_eq!(mode, CommandMode::Shell(ShellKind::Cmd));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn unix_ecosystem_commands_run_through_sh() {
        // Unix: npm/npx/gradle/mvn идут через sh — .cmd-шимов там нет.
        for program in ["npm", "npx", "gradle", "mvn"] {
            let mode = infer_command_mode(program, &["install".into()], None);
            assert_eq!(mode, CommandMode::Shell(ShellKind::Sh), "{program}");
        }
    }

    #[test]
    fn powershell_heuristic_detection() {
        assert!(is_powershell_command("powershell -Command Get-Process"));
        assert!(is_powershell_command("pwsh -Command Set-Item"));
        assert!(is_powershell_command("$?"));
        assert!(is_powershell_command("$LASTEXITCODE"));
        assert!(is_powershell_command("Invoke-RestMethod"));
        assert!(is_powershell_command("New-Item"));
        assert!(!is_powershell_command("node"));
        assert!(!is_powershell_command("npm install"));
    }

    #[test]
    fn explicit_shell_overrides_heuristic() {
        let mode = infer_command_mode(
            "powershell -Command Get-Process",
            &[],
            Some(ShellKind::Pwsh),
        );
        assert_eq!(mode, CommandMode::Shell(ShellKind::Pwsh));
    }

    // ========================================================================
    // Unix direct command construction tests
    // ========================================================================

    #[test]
    fn unix_direct_command_not_wrapped_in_sh() {
        let os = HostOs::Linux;
        let resolved = resolve_program_for_direct("node", os);
        assert_eq!(resolved, "node");
        // No sh -c wrapping — arguments passed directly
    }

    #[test]
    fn build_tokio_command_direct_mode() {
        let args = vec!["-e".into(), "console.log('hello')".into()];
        let result = build_tokio_command("node", &args, CommandMode::Direct);
        assert!(result.is_ok());
    }

    #[test]
    fn build_tokio_command_shell_mode_on_current_os() {
        let os = current_os();
        let shell = match os {
            HostOs::Windows => ShellKind::Cmd,
            _ => ShellKind::Sh,
        };
        let args = vec!["echo".into(), "hello".into()];
        let result = build_tokio_command("echo", &args, CommandMode::Shell(shell));
        assert!(result.is_ok());
    }

    // ========================================================================
    // Shell special characters remain individual args in Direct mode
    // ========================================================================

    #[test]
    fn direct_mode_preserves_individual_args() {
        let args = vec![
            "-e".into(),
            "const x = 'hello world'; console.log(x)".into(),
            "--name".into(),
            "my project".into(),
        ];
        let result = build_tokio_command("node", &args, CommandMode::Direct);
        let cmd = result.unwrap();
        // The command should have the program and all args as separate entries
        // (no string concatenation for direct mode)
        // We verify the command was constructed without error
        let _ = cmd;
    }

    #[test]
    fn direct_mode_args_not_concatted() {
        // Arguments with shell metacharacters should remain individual args
        let args = vec!["install".into(), "@nestjs/cli".into(), "--save-dev".into()];
        let result = build_tokio_command("npm", &args, CommandMode::Direct);
        let cmd = result.unwrap();
        // Verify the command builds successfully with individual args
        let _ = cmd;
    }

    // ========================================================================
    // Windows program resolution tests
    // ========================================================================

    #[test]
    fn resolve_windows_program_npm_ecosystem() {
        assert_eq!(resolve_windows_program_name("npx"), "npx.cmd");
        assert_eq!(resolve_windows_program_name("npm"), "npm.cmd");
        assert_eq!(resolve_windows_program_name("pnpm"), "pnpm.cmd");
        assert_eq!(resolve_windows_program_name("yarn"), "yarn.cmd");
        assert_eq!(resolve_windows_program_name("vite"), "vite.cmd");
        assert_eq!(resolve_windows_program_name("nest"), "nest.cmd");
        assert_eq!(resolve_windows_program_name("composer"), "composer.bat");
    }

    #[test]
    fn resolve_windows_program_jvm_shims() {
        // Gradle/Maven поставляются как .bat/.cmd шимы: без расширения
        // CreateProcess (и PATH-поиск Rust) их не находит — «program not
        // found» при генерации wrapper'а.
        assert_eq!(resolve_windows_program_name("gradle"), "gradle.bat");
        assert_eq!(resolve_windows_program_name("gradlew"), "gradlew.bat");
        assert_eq!(resolve_windows_program_name("mvn"), "mvn.cmd");
        assert_eq!(resolve_windows_program_name("mvnw"), "mvnw.cmd");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn gradle_and_mvn_run_through_cmd_shell() {
        assert_eq!(
            infer_command_mode("gradle", &["wrapper".into()], None),
            CommandMode::Shell(ShellKind::Cmd)
        );
        assert_eq!(
            infer_command_mode("mvn", &["-v".into()], None),
            CommandMode::Shell(ShellKind::Cmd)
        );
    }

    #[test]
    fn resolve_windows_program_keeps_extensions() {
        assert_eq!(
            resolve_windows_program_name(r"C:\tools\script.cmd"),
            r"C:\tools\script.cmd"
        );
        assert_eq!(
            resolve_windows_program_name("C:\\tools\\script.bat"),
            "C:\\tools\\script.bat"
        );
        assert_eq!(
            resolve_windows_program_name(r"C:\Users\John Doe\venv\Scripts\pip.exe"),
            r"C:\Users\John Doe\venv\Scripts\pip.exe"
        );
    }

    #[test]
    fn resolve_windows_program_plain_commands() {
        assert_eq!(resolve_windows_program_name("node"), "node");
        assert_eq!(resolve_windows_program_name("python"), "python");
        assert_eq!(resolve_windows_program_name("dotnet"), "dotnet");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn spawn_plan_expands_known_shims() {
        let plan = resolve_spawn_plan("npm");
        assert_eq!(plan.program, "npm.cmd");
        assert!(plan.batch_shim);

        let plan = resolve_spawn_plan("node");
        assert!(!plan.batch_shim);

        // Absolute paths and explicit extensions pass through.
        let plan = resolve_spawn_plan(r"C:\Tools\script.cmd");
        assert_eq!(plan.program, r"C:\Tools\script.cmd");
        assert!(plan.batch_shim);

        let plan = resolve_spawn_plan(r"C:\Tools\app.exe");
        assert_eq!(plan.program, r"C:\Tools\app.exe");
        assert!(!plan.batch_shim);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn spawn_plan_unix_passes_through() {
        let plan = resolve_spawn_plan("npm");
        assert_eq!(plan.program, "npm");
        assert!(!plan.batch_shim);
    }

    #[test]
    fn batch_shim_line_quotes_program_and_args() {
        let v = batch_shim_cmd_line(
            r"C:\Users\Me\AppData\Local\Programs\Microsoft VS Code\bin\code.cmd",
            &[r"C:\my project"],
        );
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], "/C");
        assert!(v[1].contains(r"C:\Users\Me\AppData\Local\Programs\Microsoft VS Code\bin\code.cmd"));
        assert!(v[1].contains(r#""C:\my project""#));
    }

    // ========================================================================
    // win_quote_arg / sh_quote tests
    // ========================================================================

    #[test]
    fn win_quote_arg_plain() {
        assert_eq!(win_quote_arg("hello"), "hello");
        assert_eq!(win_quote_arg("node"), "node");
    }

    #[test]
    fn win_quote_arg_with_spaces() {
        assert_eq!(win_quote_arg("hello world"), "\"hello world\"");
    }

    #[test]
    fn win_quote_arg_with_quotes() {
        assert_eq!(win_quote_arg(r#"say "hi""#), r#""say ""hi""""#);
    }

    #[test]
    fn win_quote_arg_with_metachars() {
        assert_eq!(win_quote_arg("a&b"), "\"a&b\"");
        assert_eq!(win_quote_arg("a|b"), "\"a|b\"");
        assert_eq!(win_quote_arg("%PATH%"), "\"%PATH%\"");
    }

    #[test]
    fn win_quote_arg_empty() {
        assert_eq!(win_quote_arg(""), "\"\"");
    }

    #[test]
    fn sh_quote_plain() {
        assert_eq!(sh_quote("hello"), "hello");
        assert_eq!(sh_quote("node"), "node");
    }

    #[test]
    fn sh_quote_with_spaces() {
        assert_eq!(sh_quote("hello world"), "'hello world'");
    }

    #[test]
    fn sh_quote_with_single_quote() {
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn sh_quote_with_metachars() {
        assert_eq!(sh_quote("a&b;c"), "'a&b;c'");
        assert_eq!(sh_quote("$HOME"), "'$HOME'");
        assert_eq!(sh_quote("a|b"), "'a|b'");
    }

    #[test]
    fn sh_quote_empty() {
        assert_eq!(sh_quote(""), "''");
    }

    // ========================================================================
    // Shell script line tests
    // ========================================================================

    #[test]
    fn cmd_script_line_quoting() {
        let line = build_script_line(
            "node",
            &["-e".into(), "console.log(1)".into()],
            ShellKind::Cmd,
        );
        // cmd /D /C receives the script line
        assert!(line.starts_with("node -e "), "{line}");
        assert!(line.contains("\"console.log(1)\""), "{line}");
    }

    #[test]
    fn sh_script_line_quoting() {
        let line = build_script_line("echo", &["hello world".into()], ShellKind::Sh);
        assert_eq!(line, "echo 'hello world'");
    }

    #[test]
    fn sh_script_line_preserves_metachars() {
        let line = build_script_line(
            "node",
            &["-e".into(), "const x = 'hello';".into()],
            ShellKind::Sh,
        );
        // The argument stays protected in single quotes
        assert!(line.contains("'const x = '"), "{line}");
    }
}
