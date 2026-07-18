use std::time::Duration;
use std::process::{Command, Stdio};
use std::thread;
use std::net::{TcpStream, ToSocketAddrs};
use std::io::{Write, BufReader, BufRead};
use crate::modules::devlauncher::models::*;

pub trait LaunchEngine: Send + Sync {
    fn execute_action(&self, action: &LaunchAction) -> Result<ActionStatus, String>;
}

pub struct ProcessLaunchEngine;

impl LaunchEngine for ProcessLaunchEngine {
    fn execute_action(&self, action: &LaunchAction) -> Result<ActionStatus, String> {
        if !action.enabled {
            return Ok(ActionStatus::Skipped {
                reason: format!("Action '{}' disabled", action.label),
            });
        }

        match &action.action_type {
            ActionType::RunCommand { command, working_dir } => {
                let mut cmd = Command::new("cmd");
                cmd.arg("/C").arg(command)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                if let Some(dir) = working_dir {
                    cmd.current_dir(dir);
                }
                match cmd.spawn() {
                    Ok(_) => Ok(ActionStatus::Success {
                        message: format!("Command started: {}", command),
                    }),
                    Err(e) => Err(format!("Failed to start process: {}", e)),
                }
            }

            ActionType::OpenUrl { url } => {
                match webbrowser::open(url) {
                    Ok(_) => Ok(ActionStatus::Success {
                        message: format!("Browser opened: {}", url),
                    }),
                    Err(e) => Err(format!("Failed to open browser: {}", e)),
                }
            }

            ActionType::OpenApplication { path, args } => {
                let mut cmd = Command::new(path);
                cmd.stdout(Stdio::null()).stderr(Stdio::null());
                if let Some(args_str) = args {
                    cmd.args(args_str.split_whitespace());
                }
                match cmd.spawn() {
                    Ok(_) => Ok(ActionStatus::Success {
                        message: format!("App launched: {}", path),
                    }),
                    Err(e) => Err(format!("Failed to launch app: {}", e)),
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
                        if let Ok(mut stream) = TcpStream::connect_timeout(addr, Duration::from_secs(2)) {
                            let request = format!(
                                "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                                parsed.path, parsed.host
                            );
                            if stream.write_all(request.as_bytes()).is_err() { continue; }
                            let mut reader = BufReader::new(&stream);
                            let mut first_line = String::new();
                            if reader.read_line(&mut first_line).is_err() { continue; }

                            let parts: Vec<&str> = first_line.split_whitespace().collect();
                            if let Some(code_str) = parts.get(1) {
                                if let Ok(code) = code_str.parse::<u16>() {
                                    if (200..400).contains(&code) {
                                        return Ok(ActionStatus::Success {
                                            message: format!("URL {} responded with {}", url, code),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    thread::sleep(Duration::from_secs(1));
                }
                Err(format!("Timeout: {} not available after {}s", url, timeout_secs))
            }

            ActionType::WaitForPort { host, port, timeout_secs } => {
                let addr_str = format!("{}:{}", host, port);
                let addrs = addr_str
                    .to_socket_addrs()
                    .map_err(|e| format!("DNS resolve failed: {}", e))?
                    .collect::<Vec<_>>();

                for _ in 0..*timeout_secs {
                    for addr in &addrs {
                        if TcpStream::connect_timeout(addr, Duration::from_secs(1)).is_ok() {
                            return Ok(ActionStatus::Success {
                                message: format!("Port {}:{} is open", host, port),
                            });
                        }
                    }
                    thread::sleep(Duration::from_secs(1));
                }
                Err(format!("Timeout: port {}:{} not open after {}s", host, port, timeout_secs))
            }

            ActionType::Delay { seconds } => {
                thread::sleep(Duration::from_secs(*seconds));
                Ok(ActionStatus::Success {
                    message: format!("Delay {}s completed", seconds),
                })
            }

            ActionType::ExecuteScript { script, shell } => {
                let shell_name = shell.as_deref().unwrap_or("cmd");
                let flag = match shell_name {
                    "cmd" => "/C",
                    "powershell" | "pwsh" => "-Command",
                    _ => "-c",
                };
                let exit = Command::new(shell_name)
                    .args([flag, script])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map_err(|e| format!("Failed to run shell '{}': {}", shell_name, e))?;

                if exit.success() {
                    Ok(ActionStatus::Success {
                        message: format!("Script executed: {}", script),
                    })
                } else {
                    let code = exit.code().unwrap_or(-1);
                    Ok(ActionStatus::Failed {
                        error: format!("Script exited with code {}", code),
                    })
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
            p.parse::<u16>().map_err(|_| format!("Invalid port in URL: {}", raw))?,
        ),
        None => (host_port.to_string(), 80),
    };
    Ok(ParsedUrl { host, port, path })
}
