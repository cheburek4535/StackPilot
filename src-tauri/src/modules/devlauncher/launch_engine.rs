use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::modules::devlauncher::models::*;
use crate::modules::workspace::models::ProcessStatus;
use crate::modules::workspace::process_manager::ProcessManager;
use crate::platform::environment::EnvironmentOverlay;
use crate::platform::host::current_os;
use crate::platform::shell::{
    default_shell_for_platform, is_windows_only_shell, parse_shell, resolve_shell, ShellKind,
};

pub trait LaunchEngine: Send + Sync {
    /// Executes a single action with an optional environment overlay.
    ///
    /// When overlay is Some, RunCommand and ExecuteScript apply it to the
    /// spawned process (PATH prepend, env set/remove). OpenApplication
    /// applies it when the action targets a configured IDE path.
    /// When overlay is None, old host-environment behavior is preserved.
    ///
    /// Returns the action status and, when the action spawned a tracked
    /// process (RunCommand / ExecuteScript), that process's id — the caller
    /// links it to the session so the session timer can see its lifecycle.
    fn execute_action(
        &self,
        action: &LaunchAction,
        session_id: Option<String>,
        overlay: Option<&EnvironmentOverlay>,
    ) -> Result<(ActionStatus, Option<String>), String>;

    /// Launch the preferred IDE for a project. This is called before running
    /// actions so the user sees their IDE open immediately with the project.
    ///
    /// `vscode_path` overrides the default "code" CLI name when the IDE is
    /// VSCode, allowing the user to configure a custom path in Settings.
    ///
    /// Returns `Ok(true)` if the IDE was launched, `Ok(false)` if no
    /// preferred IDE is set or it could not be found, and `Err` on failure.
    fn launch_ide(
        &self,
        ide: &PreferredIde,
        project_path: Option<&str>,
        overlay: Option<&EnvironmentOverlay>,
        vscode_path: Option<&str>,
    ) -> Result<bool, String>;
}

pub struct ProcessLaunchEngine {
    process_manager: Arc<dyn ProcessManager>,
}

impl ProcessLaunchEngine {
    pub fn new(process_manager: Arc<dyn ProcessManager>) -> Self {
        Self { process_manager }
    }
}

impl LaunchEngine for ProcessLaunchEngine {
    fn execute_action(
        &self,
        action: &LaunchAction,
        session_id: Option<String>,
        overlay: Option<&EnvironmentOverlay>,
    ) -> Result<(ActionStatus, Option<String>), String> {
        if !action.enabled {
            return Ok((
                ActionStatus::Skipped {
                    reason: format!("Action '{}' disabled", action.label),
                },
                None,
            ));
        }

        match &action.action_type {
            ActionType::RunCommand {
                command,
                working_dir,
                ..
            } => {
                let dir_ref = working_dir.as_deref();
                let empty_overlay = EnvironmentOverlay::new();
                let effective_overlay = overlay.unwrap_or(&empty_overlay);

                // Determine spawn mode:
                // - persistent == Some(true) → show in a native terminal window.
                // - otherwise → one-shot command, capture output to log viewer.
                let is_persistent = action.action_type.is_persistent();

                // Preflight: `docker` commands fail with cryptic pipe errors
                // when the daemon is not running. Check first and give an
                // actionable message instead of a crashed process.
                if let Err(msg) = preflight_check(command) {
                    return Ok((ActionStatus::Failed { error: msg }, None));
                }

                // Spawn exactly once. When an environment overlay is present
                // it is applied to that single child process — never spawn
                // twice (the old code leaked a duplicate process for
                // one-shot commands with an overlay).
                let tracked_proc = if is_persistent {
                    // Long-running command: open in a native terminal window.
                    // `spawn_visible` wraps the full command string in a shell
                    // that runs it inside a new terminal (cmd /K on Windows,
                    // Terminal.app on macOS, a terminal emulator on Linux).
                    let ov = if effective_overlay.is_empty() {
                        None
                    } else {
                        Some(effective_overlay)
                    };
                    self.process_manager
                        .spawn_visible(
                            command,
                            &[],
                            dir_ref,
                            &action.label,
                            session_id.clone(),
                            ov,
                        )
                        .map_err(|e| format!("Manager failed to start command: {}", e))?
                } else if !effective_overlay.is_empty() {
                    // One-shot command with env overlay: run through the
                    // platform shell so shell syntax works, with PATH/env
                    // applied directly to the child.
                    return self.spawn_one_shot_with_overlay(
                        command,
                        dir_ref,
                        &action.label,
                        session_id,
                        effective_overlay,
                    );
                } else {
                    // One-shot command: run via platform shell so shell syntax
                    // (pipes, redirects, env vars) works.
                    let resolved_shell = default_shell_for_platform();
                    let (shell_exe, shell_flag) =
                        crate::platform::shell::shell_executable(resolved_shell);

                    self.process_manager
                        .spawn_and_track(
                            shell_exe,
                            &[shell_flag, command],
                            dir_ref,
                            &action.label,
                            session_id.clone(),
                        )
                        .map_err(|e| format!("Manager failed to start command: {}", e))?
                };

                Ok((
                    ActionStatus::Success {
                        message: format!("Process started under manager. ID: {}", tracked_proc.id),
                    },
                    Some(tracked_proc.id),
                ))
            }

            ActionType::OpenUrl { url } => match webbrowser::open(url) {
                Ok(_) => Ok((
                    ActionStatus::Success {
                        message: format!("Browser opened: {}", url),
                    },
                    None,
                )),
                Err(e) => Err(format!("Failed to open browser: {}", e)),
            },

            ActionType::OpenApplication {
                path,
                args,
                args_list,
            } => {
                let mut args_vec: Vec<String> = Vec::new();
                if let Some(list) = args_list {
                    args_vec.extend(list.iter().cloned());
                } else if let Some(args_str) = args {
                    args_vec.extend(args_str.split_whitespace().map(String::from));
                }

                // Bare CLI names (`code`, `devenv`, `pycharm`) are resolved to
                // an absolute path when possible. `launch_detached` itself
                // also handles `.cmd` shims, but resolving here gives a
                // precise error message when the IDE truly isn't installed.
                let resolved_path = if path.contains('/') || path.contains('\\') {
                    path.clone()
                } else {
                    match crate::platform::ide::resolve_ide_executable(path) {
                        Some(p) => p,
                        None => {
                            return Err(format!(
                                "Application '{}' not found on this system. \
                                 Install it or use an absolute path in the action.",
                                path
                            ));
                        }
                    }
                };

                let args_refs: Vec<&str> = args_vec.iter().map(|s| s.as_str()).collect();

                // Use detached launch so the GUI app opens natively without
                // output suppression. VSCode, PyCharm, Docker Desktop, etc.
                let ide_dir = working_dir_for_ide(overlay, action, &resolved_path);
                self.process_manager
                    .launch_detached(&resolved_path, &args_refs, ide_dir)?;

                Ok((
                    ActionStatus::Success {
                        message: format!("App launched: {}", resolved_path),
                    },
                    None,
                ))
            }

            ActionType::WaitForUrl { url, timeout_secs } => {
                let parsed = parse_http_url(url)?;
                let addr_str = format!("{}:{}", parsed.host, parsed.port);
                let addrs = addr_str
                    .to_socket_addrs()
                    .map_err(|e| format!("DNS resolve failed: {}", e))?
                    .collect::<Vec<_>>();

                for _ in 0..*timeout_secs {
                    for addr in &addrs {
                        if let Ok(mut stream) =
                            TcpStream::connect_timeout(addr, Duration::from_secs(2))
                        {
                            let request = format!(
                                "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                                parsed.path, parsed.host
                            );
                            if stream.write_all(request.as_bytes()).is_err() {
                                continue;
                            }
                            let mut reader = BufReader::new(&stream);
                            let mut first_line = String::new();
                            if reader.read_line(&mut first_line).is_err() {
                                continue;
                            }

                            let parts: Vec<&str> = first_line.split_whitespace().collect();
                            if let Some(code_str) = parts.get(1) {
                                if let Ok(code) = code_str.parse::<u16>() {
                                    if (200..400).contains(&code) {
                                        return Ok((
                                            ActionStatus::Success {
                                                message: format!(
                                                    "URL {} responded with {}",
                                                    url, code
                                                ),
                                            },
                                            None,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    thread::sleep(Duration::from_secs(1));
                }
                Err(format!(
                    "Timeout: {} not available after {}s",
                    url, timeout_secs
                ))
            }

            ActionType::WaitForPort {
                host,
                port,
                timeout_secs,
            } => {
                let addr_str = format!("{}:{}", host, port);
                let addrs = addr_str
                    .to_socket_addrs()
                    .map_err(|e| format!("DNS resolve failed: {}", e))?
                    .collect::<Vec<_>>();

                for _ in 0..*timeout_secs {
                    for addr in &addrs {
                        if TcpStream::connect_timeout(addr, Duration::from_secs(1)).is_ok() {
                            return Ok((
                                ActionStatus::Success {
                                    message: format!("Port {}:{} is open", host, port),
                                },
                                None,
                            ));
                        }
                    }
                    thread::sleep(Duration::from_secs(1));
                }
                Err(format!(
                    "Timeout: port {}:{} not open after {}s",
                    host, port, timeout_secs
                ))
            }

            ActionType::Delay { seconds } => {
                thread::sleep(Duration::from_secs(*seconds));
                Ok((
                    ActionStatus::Success {
                        message: format!("Delay {}s completed", seconds),
                    },
                    None,
                ))
            }

            ActionType::ExecuteScript { script, shell } => {
                // Parse the shell string into a typed ShellKind.
                let requested_shell = match shell {
                    Some(s) => parse_shell(s).map_err(|e| {
                        let os = current_os();
                        format!(
                            "Unsupported shell '{}': {}. Host OS: {}. \
                             Valid values: sh, bash, zsh, cmd, powershell, pwsh.",
                            s, e, os
                        )
                    })?,
                    None => ShellKind::Default,
                };

                // Validate that the resolved shell is available on this OS.
                let resolved = resolve_shell(requested_shell);
                if is_windows_only_shell(resolved) && cfg!(not(target_os = "windows")) {
                    let os = current_os();
                    return Err(format!(
                        "Shell '{}' is only available on Windows, but current OS is {}. \
                         Use a Unix-compatible shell (sh, bash, zsh) or omit the shell \
                         parameter to use the platform default.",
                        resolved, os
                    ));
                }

                let (shell_exe, shell_flag) = crate::platform::shell::shell_executable(resolved);

                // Apply environment overlay if provided
                let empty_overlay = EnvironmentOverlay::new();
                let effective_overlay = overlay.unwrap_or(&empty_overlay);

                // With an overlay, spawn exactly once with PATH/env applied.
                if !effective_overlay.is_empty() {
                    return self.spawn_script_with_overlay(
                        shell_exe,
                        shell_flag,
                        script,
                        &action.label,
                        session_id,
                        effective_overlay,
                    );
                }

                let tracked_proc = self
                    .process_manager
                    .spawn_and_track(
                        shell_exe,
                        &[shell_flag, script],
                        None,
                        &action.label,
                        session_id.clone(),
                    )
                    .map_err(|e| format!("Script launch error: {}", e))?;

                let proc_id = tracked_proc.id;

                loop {
                    thread::sleep(Duration::from_millis(500));

                    match self.process_manager.refresh_status(&proc_id) {
                        Ok(ProcessStatus::Running) => {
                            continue;
                        }
                        Ok(ProcessStatus::Exited(0)) => {
                            return Ok((
                                ActionStatus::Success {
                                    message: format!("Script completed: {}", script),
                                },
                                Some(proc_id),
                            ));
                        }
                        Ok(ProcessStatus::Exited(code)) => {
                            return Ok((
                                ActionStatus::Failed {
                                    error: format!("Script exited with error code {}", code),
                                },
                                Some(proc_id),
                            ));
                        }
                        Ok(ProcessStatus::Crashed) => {
                            return Ok((
                                ActionStatus::Failed {
                                    error: "Script crashed".to_string(),
                                },
                                Some(proc_id),
                            ));
                        }
                        Ok(ProcessStatus::Killed) => {
                            return Ok((
                                ActionStatus::Failed {
                                    error: "Script was forcibly terminated".to_string(),
                                },
                                Some(proc_id),
                            ));
                        }
                        Err(e) => {
                            return Err(format!("Script monitoring error: {}", e));
                        }
                    }
                }
            }
        }
    }

    fn launch_ide(
        &self,
        ide: &PreferredIde,
        project_path: Option<&str>,
        _overlay: Option<&EnvironmentOverlay>,
        vscode_path: Option<&str>,
    ) -> Result<bool, String> {
        // For VSCode, use the user-configured path if provided
        let cli = match ide {
            PreferredIde::Vscode => vscode_path.unwrap_or("code"),
            _ => ide.cli_name(),
        };
        let resolved = match crate::platform::ide::resolve_ide_executable(cli) {
            Some(p) => p,
            None => {
                // IDE not installed — report gracefully without failing the run.
                return Ok(false);
            }
        };

        let mut args: Vec<String> = Vec::new();
        if let Some(project) = project_path {
            args.push(project.to_string());
        }

        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.process_manager
            .launch_detached(&resolved, &args_refs, None)
            .map(|_| true)
            .map_err(|e| {
                format!(
                    "Failed to launch IDE '{}' ({}): {}",
                    ide.label(),
                    resolved,
                    e
                )
            })
    }
}

/// Preflight check for commands that need a running service to be useful.
///
/// `docker` commands currently fail with cryptic npipe errors when the
/// Docker daemon is not running. Detect the two common failure modes and
/// return a clear, actionable message instead of a "crashed" process.
fn preflight_check(command: &str) -> Result<(), String> {
    let trimmed = command.trim_start();
    if !trimmed.starts_with("docker") {
        return Ok(());
    }

    // Docker CLI must exist at all.
    let docker_path = crate::platform::command::resolve_program_path("docker")
        .unwrap_or_else(|| "docker".to_string());
    let mut cmd = std::process::Command::new(&docker_path);
    cmd.args(["version", "--format", "{{.Server.Version}}"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let Ok(mut child) = cmd.spawn() else {
        return Err(
            "Docker CLI was not found. Install Docker Desktop (or Docker Engine) \
             and make sure it is available on PATH."
                .to_string(),
        );
    };
    let Ok(output) = child.wait_with_output() else {
        return Err("Docker preflight check failed to complete.".to_string());
    };

    if output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("cannot connect") || lower.contains("pipe") || lower.contains("daemon") {
        return Err(
            "Docker daemon is not running. Start Docker Desktop and wait for the \
             whale icon to show \"Docker Desktop is running\", then retry."
                .to_string(),
        );
    }
    Err(format!(
        "Docker is installed but not ready: {}",
        stderr.trim()
    ))
}

impl ProcessLaunchEngine {
    /// Spawn a one-shot command with the environment overlay applied directly
    /// to the child process (not through shell wrapping).
    fn spawn_one_shot_with_overlay(
        &self,
        command: &str,
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        overlay: &EnvironmentOverlay,
    ) -> Result<(ActionStatus, Option<String>), String> {
        // Run through the platform shell so shell syntax works, but apply the
        // overlay to the shell process.
        let resolved_shell = default_shell_for_platform();
        let (shell_exe, shell_flag) = crate::platform::shell::shell_executable(resolved_shell);
        let tracked = self
            .process_manager
            .spawn_and_track_with_overlay(
                shell_exe,
                &[shell_flag, command],
                working_dir,
                label,
                session_id,
                overlay,
            )
            .map_err(|e| format!("Manager failed to start command: {}", e))?;

        Ok((
            ActionStatus::Success {
                message: format!("Process started under manager. ID: {}", tracked.id),
            },
            Some(tracked.id),
        ))
    }

    /// Spawn a script with the environment overlay applied directly.
    fn spawn_script_with_overlay(
        &self,
        shell_exe: &str,
        shell_flag: &str,
        script: &str,
        label: &str,
        session_id: Option<String>,
        overlay: &EnvironmentOverlay,
    ) -> Result<(ActionStatus, Option<String>), String> {
        let tracked = self
            .process_manager
            .spawn_and_track_with_overlay(
                shell_exe,
                &[shell_flag, script],
                None,
                label,
                session_id,
                overlay,
            )
            .map_err(|e| format!("Script launch error: {}", e))?;

        Ok((
            ActionStatus::Success {
                message: format!("Script started: {}", script),
            },
            Some(tracked.id),
        ))
    }
}

/// Helper: for OpenApplication with an IDE path, run in the project directory
/// if the overlay specifies one.
fn working_dir_for_ide(
    _overlay: Option<&EnvironmentOverlay>,
    _action: &LaunchAction,
    _path: &str,
) -> Option<&'static str> {
    // IDE launches generally don't need a working dir; the IDE opens its own
    // window. We could pass the project path as an arg instead. For now,
    // return None to keep the current behavior (launch in app's cwd).
    None
}

impl EnvironmentOverlay {
    /// Returns true if the overlay has any entries that need applying.
    fn is_empty(&self) -> bool {
        self.path_prepend.is_empty() && self.vars_set.is_empty() && self.vars_remove.is_empty()
    }
}

struct ParsedUrl {
    host: String,
    port: u16,
    path: String,
}

fn parse_http_url(raw: &str) -> Result<ParsedUrl, String> {
    let without_proto = raw
        .strip_prefix("http://")
        .or_else(|| raw.strip_prefix("https://"))
        .unwrap_or(raw);
    let (host_port, path) = match without_proto.split_once('/') {
        Some((hp, p)) => (hp, format!("/{}", p)),
        None => (without_proto, "/".to_string()),
    };
    let (host, port) = match host_port.split_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|_| format!("Invalid port in URL: {}", raw))?,
        ),
        None => (host_port.to_string(), 80),
    };
    Ok(ParsedUrl { host, port, path })
}
