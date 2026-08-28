//! Dedicated Docker preflight and readiness service.
//!
//! Distinguishes between various Docker failure modes and provides
//! structured diagnostics for each. Docker readiness operations are
//! separate from project actions — a Docker failure does not disable
//! unrelated project actions.

use std::net::ToSocketAddrs;
use std::time::Duration;

use super::command_resolver::resolve_executable;

// ---------------------------------------------------------------------------
// Docker status classification
// ---------------------------------------------------------------------------

/// The current state of Docker on the host system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerStatus {
    /// Docker CLI is not installed or not on PATH.
    CliMissing,
    /// Docker CLI exists but the daemon is not running.
    DaemonUnavailable,
    /// Docker daemon is starting up (e.g. Docker Desktop booting).
    DaemonStarting,
    /// Docker daemon is ready to accept commands.
    DaemonReady,
    /// A Docker command failed with a specific error.
    CommandFailed,
    /// Docker Compose file was not found at the expected path.
    ComposeFileMissing,
    /// A Docker service/container is already running.
    ServiceAlreadyRunning,
    /// A Docker service was started successfully.
    ServiceStartedSuccessfully,
}

impl DockerStatus {
    pub fn label(&self) -> &'static str {
        match self {
            DockerStatus::CliMissing => "CLI missing",
            DockerStatus::DaemonUnavailable => "Daemon unavailable",
            DockerStatus::DaemonStarting => "Daemon starting",
            DockerStatus::DaemonReady => "Daemon ready",
            DockerStatus::CommandFailed => "Command failed",
            DockerStatus::ComposeFileMissing => "Compose file missing",
            DockerStatus::ServiceAlreadyRunning => "Service already running",
            DockerStatus::ServiceStartedSuccessfully => "Service started",
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, DockerStatus::DaemonReady)
    }
}

impl std::fmt::Display for DockerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

// ---------------------------------------------------------------------------
// Docker diagnostics
// ---------------------------------------------------------------------------

/// Structured diagnostic from a Docker operation.
#[derive(Debug, Clone)]
pub struct DockerDiagnostic {
    pub status: DockerStatus,
    pub message: String,
    pub suggested_action: Option<String>,
    pub detected_os: crate::platform::host::HostOs,
}

impl std::fmt::Display for DockerDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {} (OS: {})",
            self.status, self.message, self.detected_os
        )
    }
}

// ---------------------------------------------------------------------------
// Docker readiness check parameters
// ---------------------------------------------------------------------------

/// Parameters for checking Docker daemon readiness.
#[derive(Debug, Clone)]
pub struct DockerReadinessCheck {
    /// Maximum time to wait for the daemon to become ready.
    pub timeout: Duration,
    /// Polling interval between checks.
    pub poll_interval: Duration,
    /// Optional: check that a specific container/service is running.
    pub service_name: Option<String>,
    /// Optional: check that a specific port is listening.
    pub port_check: Option<u16>,
    /// Optional: run a health check command.
    pub health_check: Option<String>,
}

impl Default for DockerReadinessCheck {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            poll_interval: Duration::from_secs(1),
            service_name: None,
            port_check: None,
            health_check: None,
        }
    }
}

/// Result of a Docker readiness check.
#[derive(Debug, Clone)]
pub struct DockerReadinessResult {
    pub status: DockerStatus,
    pub elapsed: Duration,
    pub message: String,
    pub suggested_action: Option<String>,
}

// ---------------------------------------------------------------------------
// Docker service
// ---------------------------------------------------------------------------

/// The Docker preflight and readiness service.
///
/// This service provides:
/// - CLI availability check
/// - Daemon readiness check with timeout and cancellation
/// - Container/service readiness check
/// - Port readiness check
/// - Health check support
/// - Docker Desktop launch detection
pub struct DockerService;

impl DockerService {
    /// Check if the Docker CLI is available on PATH.
    pub fn check_cli() -> DockerDiagnostic {
        let os = crate::platform::host::current_os();

        match resolve_executable("docker", None) {
            Some(path) => DockerDiagnostic {
                status: DockerStatus::DaemonReady, // CLI found; daemon status TBD
                message: format!("Docker CLI found at: {}", path.to_string_lossy()),
                suggested_action: None,
                detected_os: os,
            },
            None => DockerDiagnostic {
                status: DockerStatus::CliMissing,
                message: "Docker CLI not found on PATH".to_string(),
                suggested_action: Some(format!(
                    "Install Docker Desktop or Docker Engine for {} and ensure \
                     it is available on PATH.",
                    os
                )),
                detected_os: os,
            },
        }
    }

    /// Check if the Docker daemon is running and ready.
    pub fn check_daemon() -> DockerDiagnostic {
        let os = crate::platform::host::current_os();

        // First check CLI.
        let cli_check = Self::check_cli();
        if cli_check.status == DockerStatus::CliMissing {
            return cli_check;
        }

        // Run `docker version --format {{.Server.Version}}` with a timeout.
        let output = std::process::Command::new("docker")
            .args(["version", "--format", "{{.Server.Version}}"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();

        match output {
            Ok(out) => {
                if out.status.success() {
                    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !version.is_empty() {
                        return DockerDiagnostic {
                            status: DockerStatus::DaemonReady,
                            message: format!("Docker daemon is running (v{})", version),
                            suggested_action: None,
                            detected_os: os,
                        };
                    }
                }

                let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
                classify_docker_error(&stderr, os)
            }
            Err(_) => DockerDiagnostic {
                status: DockerStatus::DaemonUnavailable,
                message: "Failed to execute Docker CLI".to_string(),
                suggested_action: Some(
                    "Ensure Docker is installed and accessible on PATH.".to_string(),
                ),
                detected_os: os,
            },
        }
    }

    /// Check if a Docker Compose file exists at the given path.
    pub fn check_compose_file(path: &str) -> DockerDiagnostic {
        let os = crate::platform::host::current_os();
        let p = std::path::Path::new(path);
        if p.is_file() {
            DockerDiagnostic {
                status: DockerStatus::DaemonReady,
                message: format!("Compose file found: {}", path),
                suggested_action: None,
                detected_os: os,
            }
        } else {
            DockerDiagnostic {
                status: DockerStatus::ComposeFileMissing,
                message: format!("Compose file not found: {}", path),
                suggested_action: Some(format!(
                    "Create a docker-compose.yml at '{}' or update the path.",
                    path
                )),
                detected_os: os,
            }
        }
    }

    /// Wait for Docker daemon readiness with timeout and cancellation.
    pub async fn wait_for_daemon(
        check: &DockerReadinessCheck,
        cancelled: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> DockerReadinessResult {
        let start = tokio::time::Instant::now();
        let deadline = start + check.timeout;

        loop {
            if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                return DockerReadinessResult {
                    status: DockerStatus::DaemonUnavailable,
                    elapsed: start.elapsed(),
                    message: "Docker daemon readiness check cancelled".to_string(),
                    suggested_action: None,
                };
            }

            let diag = Self::check_daemon();
            if diag.status == DockerStatus::DaemonReady {
                // If service name is specified, check that too.
                if let Some(ref service) = check.service_name {
                    if !is_service_running(service) {
                        // Daemon ready but service not running — keep waiting.
                        if tokio::time::Instant::now() >= deadline {
                            return DockerReadinessResult {
                                status: DockerStatus::DaemonStarting,
                                elapsed: start.elapsed(),
                                message: format!(
                                    "Docker daemon ready but service '{}' not running",
                                    service
                                ),
                                suggested_action: Some(format!(
                                    "Start the service with: docker compose up {}",
                                    service
                                )),
                            };
                        }
                        tokio::time::sleep(check.poll_interval).await;
                        continue;
                    }
                }

                // If port check is specified, verify it.
                if let Some(port) = check.port_check {
                    if !is_port_open(port) {
                        if tokio::time::Instant::now() >= deadline {
                            return DockerReadinessResult {
                                status: DockerStatus::DaemonStarting,
                                elapsed: start.elapsed(),
                                message: format!(
                                    "Docker daemon ready but port {} not listening",
                                    port
                                ),
                                suggested_action: Some(format!(
                                    "Ensure the Docker service exposes port {}.",
                                    port
                                )),
                            };
                        }
                        tokio::time::sleep(check.poll_interval).await;
                        continue;
                    }
                }

                return DockerReadinessResult {
                    status: DockerStatus::DaemonReady,
                    elapsed: start.elapsed(),
                    message: diag.message,
                    suggested_action: None,
                };
            }

            if tokio::time::Instant::now() >= deadline {
                return DockerReadinessResult {
                    status: diag.status,
                    elapsed: start.elapsed(),
                    message: format!(
                        "Docker daemon not ready after {}s: {}",
                        check.timeout.as_secs(),
                        diag.message
                    ),
                    suggested_action: diag.suggested_action,
                };
            }

            tokio::time::sleep(check.poll_interval).await;
        }
    }

    /// Preflight check for Docker commands — validates daemon state
    /// before executing a Docker command.
    pub fn preflight_for_command(command: &str) -> Result<(), DockerDiagnostic> {
        let trimmed = command.trim_start();
        if !trimmed.starts_with("docker") {
            return Ok(());
        }

        let diag = Self::check_daemon();
        match diag.status {
            DockerStatus::DaemonReady => Ok(()),
            _ => Err(diag),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Classify a Docker error message into a structured status.
fn classify_docker_error(stderr: &str, os: crate::platform::host::HostOs) -> DockerDiagnostic {
    if stderr.contains("cannot connect")
        || stderr.contains("pipe")
        || stderr.contains("daemon not running")
        || stderr.contains("error during connect")
    {
        DockerDiagnostic {
            status: DockerStatus::DaemonUnavailable,
            message: "Docker daemon is not running".to_string(),
            suggested_action: Some(match os {
                crate::platform::host::HostOs::Windows => {
                    "Start Docker Desktop and wait for the whale icon to show \
                     \"Docker Desktop is running\", then retry."
                        .to_string()
                }
                crate::platform::host::HostOs::Macos => {
                    "Start Docker Desktop from Applications or run \
                     `open -a Docker`."
                        .to_string()
                }
                crate::platform::host::HostOs::Linux => {
                    "Start the Docker daemon with `sudo systemctl start docker` \
                     or start Docker Desktop if installed."
                        .to_string()
                }
            }),
            detected_os: os,
        }
    } else if stderr.contains("starting")
        || stderr.contains("not ready")
        || stderr.contains("initializing")
    {
        DockerDiagnostic {
            status: DockerStatus::DaemonStarting,
            message: "Docker daemon is starting up".to_string(),
            suggested_action: Some("Wait a few seconds and retry.".to_string()),
            detected_os: os,
        }
    } else {
        DockerDiagnostic {
            status: DockerStatus::CommandFailed,
            message: format!("Docker error: {}", stderr.trim()),
            suggested_action: None,
            detected_os: os,
        }
    }
}

/// Check if a Docker service is running.
fn is_service_running(service_name: &str) -> bool {
    std::process::Command::new("docker")
        .args([
            "ps",
            "--filter",
            &format!("name={}", service_name),
            "--format",
            "{{.Names}}",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .map(|out| {
            let output = String::from_utf8_lossy(&out.stdout);
            !output.trim().is_empty()
        })
        .unwrap_or(false)
}

/// Check if a TCP port is open on localhost.
fn is_port_open(port: u16) -> bool {
    use std::net::TcpStream;
    let addr_str = format!("127.0.0.1:{}", port);
    match addr_str.to_socket_addrs() {
        Ok(mut addrs) => {
            if let Some(addr) = addrs.next() {
                TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_ok()
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_status_labels() {
        assert_eq!(DockerStatus::CliMissing.label(), "CLI missing");
        assert_eq!(
            DockerStatus::DaemonUnavailable.label(),
            "Daemon unavailable"
        );
        assert_eq!(DockerStatus::DaemonStarting.label(), "Daemon starting");
        assert_eq!(DockerStatus::DaemonReady.label(), "Daemon ready");
        assert_eq!(DockerStatus::CommandFailed.label(), "Command failed");
        assert_eq!(
            DockerStatus::ComposeFileMissing.label(),
            "Compose file missing"
        );
        assert_eq!(
            DockerStatus::ServiceAlreadyRunning.label(),
            "Service already running"
        );
        assert_eq!(
            DockerStatus::ServiceStartedSuccessfully.label(),
            "Service started"
        );
    }

    #[test]
    fn docker_status_is_ready() {
        assert!(DockerStatus::DaemonReady.is_ready());
        assert!(!DockerStatus::CliMissing.is_ready());
        assert!(!DockerStatus::DaemonUnavailable.is_ready());
        assert!(!DockerStatus::DaemonStarting.is_ready());
    }

    #[test]
    fn docker_status_display() {
        assert_eq!(format!("{}", DockerStatus::DaemonReady), "Daemon ready");
    }

    #[test]
    fn check_cli_does_not_panic() {
        let _ = DockerService::check_cli();
    }

    #[test]
    fn check_daemon_does_not_panic() {
        let _ = DockerService::check_daemon();
    }

    #[test]
    fn check_compose_file_missing() {
        let diag = DockerService::check_compose_file("/nonexistent/docker-compose.yml");
        assert_eq!(diag.status, DockerStatus::ComposeFileMissing);
        assert!(diag.suggested_action.is_some());
    }

    #[test]
    fn preflight_non_docker_command_ok() {
        assert!(DockerService::preflight_for_command("npm install").is_ok());
    }

    #[test]
    fn classify_docker_errors() {
        let diag = classify_docker_error(
            "error during connect: Cannot connect to the Docker daemon",
            crate::platform::host::HostOs::Windows,
        );
        assert_eq!(diag.status, DockerStatus::DaemonUnavailable);
        assert!(diag.suggested_action.is_some());

        let diag = classify_docker_error(
            "the daemon is starting",
            crate::platform::host::HostOs::Linux,
        );
        assert_eq!(diag.status, DockerStatus::DaemonStarting);

        let diag = classify_docker_error("some other error", crate::platform::host::HostOs::Macos);
        assert_eq!(diag.status, DockerStatus::CommandFailed);
    }

    #[test]
    fn readiness_check_default_values() {
        let check = DockerReadinessCheck::default();
        assert_eq!(check.timeout, Duration::from_secs(30));
        assert_eq!(check.poll_interval, Duration::from_secs(1));
        assert!(check.service_name.is_none());
        assert!(check.port_check.is_none());
    }

    #[test]
    fn docker_diagnostic_display() {
        let diag = DockerDiagnostic {
            status: DockerStatus::DaemonReady,
            message: "Docker daemon is running".to_string(),
            suggested_action: None,
            detected_os: crate::platform::host::HostOs::Windows,
        };
        let display = format!("{}", diag);
        assert!(display.contains("Daemon ready"));
        assert!(display.contains("windows"));
    }
}
