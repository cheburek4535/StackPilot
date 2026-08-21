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

pub trait LaunchEngine: Send + Sync {
    /// Executes a single action.
    ///
    /// Returns the action status and, when the action spawned a tracked
    /// process (RunCommand / ExecuteScript), that process's id — the caller
    /// links it to the session so the session timer can see its lifecycle.
    fn execute_action(
        &self,
        action: &LaunchAction,
        session_id: Option<String>,
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

                match self.process_manager.spawn_and_track(
                    "cmd",
                    &["/C", command],
                    dir_ref,
                    &action.label,
                    session_id,
                ) {
                    Ok(tracked_proc) => Ok((
                        ActionStatus::Success {
                            message: format!(
                                "Процесс запущен под контролем менеджера. ID: {}",
                                tracked_proc.id
                            ),
                        },
                        Some(tracked_proc.id),
                    )),
                    Err(e) => Err(format!("Менеджер не смог запустить команду: {}", e)),
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

            ActionType::OpenApplication { path, args } => {
                let mut cmd = StdCommand::new(path);
                cmd.stdout(Stdio::null()).stderr(Stdio::null());
                if let Some(args_str) = args {
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
                let shell_name = shell.as_deref().unwrap_or("cmd");
                let flag = match shell_name {
                    "cmd" => "/C",
                    "powershell" | "pwsh" => "-Command",
                    _ => "-c",
                };

                // 1. Запускаем скрипт под контролем менеджера, чтобы он появился в UI
                let tracked_proc = self
                    .process_manager
                    .spawn_and_track(shell_name, &[flag, script], None, &action.label, session_id)
                    .map_err(|e| format!("Ошибка запуска скрипта: {}", e))?;

                let proc_id = tracked_proc.id;

                // 2. Запускаем цикл неблокирующего ожидания (Polling)
                loop {
                    // Засыпаем на 500 миллисекунд, чтобы не перегружать процессор частыми запросами
                    thread::sleep(Duration::from_millis(500));

                    // 3. Опрашиваем менеджер о состоянии нашего скрипта
                    match self.process_manager.refresh_status(&proc_id) {
                        Ok(ProcessStatus::Running) => {
                            // Скрипт еще работает. Ничего не делаем, цикл идет на следующий круг
                            continue;
                        }
                        Ok(ProcessStatus::Exited(0)) => {
                            // Скрипт успешно завершился! Выходим из цикла с успехом
                            return Ok((
                                ActionStatus::Success {
                                    message: format!("Скрипт успешно выполнен: {}", script),
                                },
                                Some(proc_id),
                            ));
                        }
                        Ok(ProcessStatus::Exited(code)) => {
                            // Скрипт завершился, но с ошибкой
                            return Ok((
                                ActionStatus::Failed {
                                    error: format!("Скрипт завершился с кодом ошибки {}", code),
                                },
                                Some(proc_id),
                            ));
                        }
                        Ok(ProcessStatus::Crashed) => {
                            return Ok((
                                ActionStatus::Failed {
                                    error: "Скрипт аварийно завершил работу (Crashed)".to_string(),
                                },
                                Some(proc_id),
                            ));
                        }
                        Ok(ProcessStatus::Killed) => {
                            return Ok((
                                ActionStatus::Failed {
                                    error: "Выполнение скрипта было принудительно остановлено"
                                        .to_string(),
                                },
                                Some(proc_id),
                            ));
                        }
                        Err(e) => {
                            // Произошла какая-то системная ошибка при проверке
                            return Err(format!("Ошибка мониторинга скрипта: {}", e));
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
