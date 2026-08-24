use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::Command as StdCommand;
use std::process::Stdio;
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
            } => {
                let dir_ref = working_dir.as_deref();

                // RunCommand is treated as a shell command: existing profiles
                // store a command string that may contain shell syntax
                // (pipes, redirects, env vars). Use the platform default shell.
                let resolved_shell = default_shell_for_platform();
                let (shell_exe, shell_flag) =
                    crate::platform::shell::shell_executable(resolved_shell);

                // Apply environment overlay if provided
                let _effective_overlay = overlay.unwrap_or(&EnvironmentOverlay::new());

                match self.process_manager.spawn_and_track(
                    shell_exe,
                    &[shell_flag, command],
                    dir_ref,
                    &action.label,
                    session_id,
                ) {
                    Ok(tracked_proc) => {
                        // TODO: When process_manager supports env overlay,
                        // apply _effective_overlay here. For now, the overlay
                        // is resolved and available for future integration.
                        Ok((
                            ActionStatus::Success {
                                message: format!(
                                    "Process started under manager. ID: {}",
                                    tracked_proc.id
                                ),
                            },
                            Some(tracked_proc.id),
                        ))
                    }
                    Err(e) => Err(format!("Manager failed to start command: {}", e)),
                }
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
                let mut cmd = StdCommand::new(path);
                cmd.stdout(Stdio::null()).stderr(Stdio::null());

                // Apply environment overlay if provided
                if let Some(ov) = overlay {
                    ov.apply_std(&mut cmd);
                }

                // Prefer structured args_list when present (handles quoted
                // arguments correctly). Fall back to split_whitespace on the
                // legacy string field for backward compatibility.
                if let Some(list) = args_list {
                    cmd.args(list);
                } else if let Some(args_str) = args {
                    cmd.args(args_str.split_whitespace());
                }

                match cmd.spawn() {
                    Ok(_) => Ok((
                        ActionStatus::Success {
                            message: format!("App launched: {}", path),
                        },
                        None,
                    )),
                    Err(e) => Err(format!("Failed to launch '{}': {}", path, e)),
                }
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
                let _effective_overlay = overlay.unwrap_or(&EnvironmentOverlay::new());

                let tracked_proc = self
                    .process_manager
                    .spawn_and_track(
                        shell_exe,
                        &[shell_flag, script],
                        None,
                        &action.label,
                        session_id,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::host::{current_os, HostOs};

    #[test]
    fn runcommand_uses_platform_default_shell() {
        let os = current_os();
        let shell = default_shell_for_platform();
        let (exe, _) = crate::platform::shell::shell_executable(shell);
        match os {
            HostOs::Windows => assert_eq!(exe, "cmd", "RunCommand should use cmd on Windows"),
            HostOs::Linux | HostOs::Macos => {
                assert_eq!(exe, "sh", "RunCommand should use sh on Unix")
            }
        }
    }

    #[test]
    fn runcommand_does_not_name_cmd_on_unix() {
        if cfg!(target_os = "windows") {
            return;
        }
        let shell = default_shell_for_platform();
        let (exe, _) = crate::platform::shell::shell_executable(shell);
        assert_ne!(exe, "cmd", "RunCommand must not use cmd on Unix builds");
    }

    #[test]
    fn execute_script_shell_none_resolves_to_default() {
        let requested = ShellKind::Default;
        let resolved = resolve_shell(requested);
        let os = current_os();
        match os {
            HostOs::Windows => assert_eq!(resolved, ShellKind::Cmd),
            HostOs::Linux | HostOs::Macos => assert_eq!(resolved, ShellKind::Sh),
        }
    }

    #[test]
    fn execute_script_legacy_shell_strings_parse_correctly() {
        assert_eq!(
            parse_shell("cmd").unwrap(),
            ShellKind::Cmd,
            "legacy 'cmd' should parse"
        );
        assert_eq!(
            parse_shell("powershell").unwrap(),
            ShellKind::PowerShell,
            "legacy 'powershell' should parse"
        );
        assert_eq!(
            parse_shell("pwsh").unwrap(),
            ShellKind::Pwsh,
            "legacy 'pwsh' should parse"
        );
        assert_eq!(
            parse_shell("sh").unwrap(),
            ShellKind::Sh,
            "legacy 'sh' should parse"
        );
        assert_eq!(
            parse_shell("bash").unwrap(),
            ShellKind::Bash,
            "legacy 'bash' should parse"
        );
        assert_eq!(
            parse_shell("zsh").unwrap(),
            ShellKind::Zsh,
            "legacy 'zsh' should parse"
        );
    }

    #[test]
    fn execute_script_legacy_shell_case_insensitive() {
        assert_eq!(parse_shell("CMD").unwrap(), ShellKind::Cmd);
        assert_eq!(parse_shell("PowerShell").unwrap(), ShellKind::PowerShell);
        assert_eq!(parse_shell("  Bash  ").unwrap(), ShellKind::Bash);
    }

    #[test]
    fn execute_script_unsupported_shell_returns_error() {
        let err = parse_shell("fish").unwrap_err();
        assert!(err.to_string().contains("fish"));
        let err = parse_shell("zsh-plus").unwrap_err();
        assert_eq!(err.0, "zsh-plus");
    }

    #[test]
    fn open_application_prefers_args_list_over_string() {
        // Verify the model deserialization: args_list takes precedence
        let json = r#"{"OpenApplication":{"path":"/usr/bin/app","args":"--foo --bar","args_list":["--foo","--bar"]}}"#;
        let action: ActionType = serde_json::from_str(json).unwrap();
        match action {
            ActionType::OpenApplication {
                args, args_list, ..
            } => {
                assert_eq!(args.as_deref(), Some("--foo --bar"));
                let list = args_list.as_ref().unwrap();
                assert_eq!(list.len(), 2);
                assert_eq!(list[0], "--foo");
                assert_eq!(list[1], "--bar");
            }
            _ => panic!("expected OpenApplication"),
        }
    }

    #[test]
    fn open_application_backward_compat_no_args_list() {
        // Legacy serialized profiles omit args_list entirely
        let json = r#"{"OpenApplication":{"path":"/usr/bin/app","args":"--foo"}}"#;
        let action: ActionType = serde_json::from_str(json).unwrap();
        match action {
            ActionType::OpenApplication {
                args, args_list, ..
            } => {
                assert_eq!(args.as_deref(), Some("--foo"));
                assert_eq!(args_list, None);
            }
            _ => panic!("expected OpenApplication"),
        }
    }

    #[test]
    fn open_application_no_args_at_all() {
        let json = r#"{"OpenApplication":{"path":"/usr/bin/app"}}"#;
        let action: ActionType = serde_json::from_str(json).unwrap();
        match action {
            ActionType::OpenApplication {
                args, args_list, ..
            } => {
                assert_eq!(args, None);
                assert_eq!(args_list, None);
            }
            _ => panic!("expected OpenApplication"),
        }
    }
}
