//! Dedicated Docker preflight and readiness service.
//!
//! Distinguishes between various Docker failure modes and provides
//! structured diagnostics for each. Docker readiness operations are
//! separate from project actions вЂ” a Docker failure does not disable
//! unrelated project actions.

use std::net::ToSocketAddrs;
use std::path::Path;
use std::time::Duration;

use super::command_resolver::resolve_executable;

/// Canonical docker compose file names, in docker's own discovery order.
pub const COMPOSE_FILE_NAMES: &[&str] = &[
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];

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
    /// When the daemon is down, attempt to auto-launch Docker Desktop /
    /// the Docker daemon once before falling back to plain waiting.
    pub auto_launch: bool,
}

impl Default for DockerReadinessCheck {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            poll_interval: Duration::from_secs(1),
            service_name: None,
            port_check: None,
            health_check: None,
            auto_launch: false,
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
    /// Structured launcher for the Docker Desktop APPLICATION on this host.
    ///
    /// - **Windows** — unchanged: the app resolvers (App Paths, install
    ///   dirs, Start Menu shortcuts including Store/MSIX aliases).
    /// - **macOS** — the canonical `open -a Docker`.
    /// - **Linux** — the GUI binary on PATH or in the deb/rpm install
    ///   directory (`/opt/docker-desktop`), then the `.desktop` entry (custom
    ///   installs), then the `docker desktop` CLI plugin (Docker Desktop
    ///   4.26+), then the systemd user unit older packages registered.
    ///
    /// The bare `docker` CLI is NEVER returned: launching it just prints help
    /// (it cannot start the daemon), which made the "Open Docker Desktop"
    /// step report success on Linux while nothing opened.
    pub fn desktop_launcher() -> Option<(String, Vec<String>)> {
        match crate::platform::host::current_os() {
            crate::platform::host::HostOs::Windows => {
                if let Some(path) = crate::platform::ide::resolve_ide_executable("Docker Desktop") {
                    return Some((path, Vec::new()));
                }
                let launcher =
                    crate::platform::app_launcher::resolve_application("Docker Desktop", None, None);
                if launcher.found && !launcher.is_flatpak {
                    return Some((launcher.program, launcher.args));
                }
                // Start Menu shortcut: covers Store/MSIX installs that
                // register neither App Paths nor a plain exe path. The
                // structured launcher carries the `shell:AppsFolder` alias.
                #[cfg(target_os = "windows")]
                {
                    if let Some(l) =
                        crate::platform::app_launcher::start_menu_launcher("Docker Desktop")
                    {
                        return Some((l.program, l.args));
                    }
                }
                None
            }
            crate::platform::host::HostOs::Macos => Some((
                "open".to_string(),
                vec!["-a".to_string(), "Docker".to_string()],
            )),
            crate::platform::host::HostOs::Linux => Self::linux_desktop_launcher(),
        }
    }

    /// Linux Docker Desktop launcher (see [`Self::desktop_launcher`]).
    #[cfg(target_os = "linux")]
    fn linux_desktop_launcher() -> Option<(String, Vec<String>)> {
        // 1. A `docker-desktop` CLI/wrapper on PATH (some packages install
        //    /usr/bin/docker-desktop).
        if let Some(path) = resolve_executable("docker-desktop", None) {
            return Some((path.to_string_lossy().into_owned(), Vec::new()));
        }
        // 2. The deb/rpm install layout. This is the binary the desktop entry
        //    itself launches, and it opens the Dashboard window.
        for candidate in [
            "/opt/docker-desktop/bin/docker-desktop",
            "/usr/lib/docker-desktop/bin/docker-desktop",
        ] {
            if Path::new(candidate).is_file() {
                return Some((candidate.to_string(), Vec::new()));
            }
        }
        // 3. The desktop entry, for installs in non-standard locations.
        if let Some((program, args)) = desktop_entry_exec("docker-desktop.desktop") {
            return Some((program, args));
        }
        // 4. `docker desktop start` (Docker Desktop 4.26+). Starts the app
        //    and its engine; the GUI opens when a desktop session exists.
        if Self::desktop_plugin_available() {
            if let Some(cli) = Self::resolve_cli() {
                return Some((
                    cli.to_string_lossy().into_owned(),
                    vec!["desktop".to_string(), "start".to_string()],
                ));
            }
        }
        // 5. The systemd user unit older Docker Desktop packages installed.
        if linux_systemd_user_unit_exists("docker-desktop.service") {
            return Some((
                "systemctl".to_string(),
                vec![
                    "--user".to_string(),
                    "start".to_string(),
                    "docker-desktop".to_string(),
                ],
            ));
        }
        None
    }

    /// Non-Linux stub.
    #[cfg(not(target_os = "linux"))]
    #[allow(dead_code)]
    fn linux_desktop_launcher() -> Option<(String, Vec<String>)> {
        None
    }

    /// Whether the `docker desktop` CLI plugin is installed for this host.
    #[cfg(target_os = "linux")]
    fn desktop_plugin_available() -> bool {
        let mut candidates: Vec<std::path::PathBuf> = vec![
            std::path::PathBuf::from("/usr/lib/docker/cli-plugins/docker-desktop"),
            std::path::PathBuf::from("/usr/local/lib/docker/cli-plugins/docker-desktop"),
            std::path::PathBuf::from("/usr/libexec/docker/cli-plugins/docker-desktop"),
        ];
        if let Ok(home) = std::env::var("HOME") {
            candidates.push(
                std::path::PathBuf::from(home).join(".docker/cli-plugins/docker-desktop"),
            );
        }
        if let Ok(config) = std::env::var("DOCKER_CONFIG") {
            candidates.push(std::path::PathBuf::from(config).join("cli-plugins/docker-desktop"));
        }
        candidates.iter().any(|p| p.is_file())
    }

    /// Non-Linux stub.
    #[cfg(not(target_os = "linux"))]
    #[allow(dead_code)]
    fn desktop_plugin_available() -> bool {
        false
    }

    /// Resolve the docker CLI executable. PATH is tried first; when the
    /// CLI is missing there (GUI-launched apps do not inherit shell-profile
    /// PATH entries), the Docker Desktop bundled CLI is used.
    pub fn resolve_cli() -> Option<std::path::PathBuf> {
        if let Some(path) = resolve_executable("docker", None) {
            return Some(path);
        }
        #[cfg(target_os = "windows")]
        {
            let mut candidates: Vec<std::path::PathBuf> = vec![std::path::PathBuf::from(
                r"C:\Program Files\Docker\Docker\resources\bin\docker.exe",
            )];
            if let Ok(pf) = std::env::var("ProgramFiles") {
                candidates.push(
                    std::path::PathBuf::from(pf)
                        .join("Docker")
                        .join("Docker")
                        .join("resources")
                        .join("bin")
                        .join("docker.exe"),
                );
            }
            for cand in candidates {
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            for cand in [
                "/Applications/Docker.app/Contents/Resources/bin/docker",
                "/usr/local/bin/docker",
                "/opt/homebrew/bin/docker",
            ] {
                let p = std::path::Path::new(cand);
                if p.is_file() {
                    return Some(p.to_path_buf());
                }
            }
        }
        #[cfg(target_os = "linux")]
        {
            for cand in [
                "/usr/bin/docker",
                "/usr/local/bin/docker",
                "/snap/bin/docker",
            ] {
                let p = std::path::Path::new(cand);
                if p.is_file() {
                    return Some(p.to_path_buf());
                }
            }
            // Docker Desktop for Linux ships its own CLI in the user's home;
            // on a Desktop-only machine (no distro package) the daemon is
            // running while `docker` is missing from PATH — every docker step
            // then reported "CLI missing".
            if let Ok(home) = std::env::var("HOME") {
                let bundled = std::path::PathBuf::from(home)
                    .join(".docker")
                    .join("desktop")
                    .join("bin")
                    .join("docker");
                if bundled.is_file() {
                    return Some(bundled);
                }
            }
        }
        None
    }

    /// Check if the Docker CLI is available.
    pub fn check_cli() -> DockerDiagnostic {
        let os = crate::platform::host::current_os();

        match Self::resolve_cli() {
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
        Self::check_daemon_bounded(Duration::from_secs(8))
    }

    /// Check daemon readiness with a hard per-attempt timeout.
    ///
    /// A cold-starting engine (Docker Desktop booting its WSL2 backend,
    /// the named pipe already existing) routinely makes `docker version`
    /// block for tens of seconds per call. An unbounded call would stall
    /// the whole readiness wait loop; bounding each attempt keeps the wait
    /// responsive and cancellation-friendly.
    pub fn check_daemon_bounded(timeout: Duration) -> DockerDiagnostic {
        let os = crate::platform::host::current_os();

        // First check CLI (PATH, then Docker Desktop bundled CLI).
        let cli_check = Self::check_cli();
        if cli_check.status == DockerStatus::CliMissing {
            return cli_check;
        }
        // `check_cli` and `resolve_cli` must agree; a divergence here is a
        // hard failure, never a panic вЂ” a panic inside the async wait path
        // would hang the whole run (the step task never completes).
        let Some(cli) = Self::resolve_cli() else {
            return DockerDiagnostic {
                status: DockerStatus::CliMissing,
                message: "Docker CLI not found on PATH".to_string(),
                suggested_action: cli_check.suggested_action,
                detected_os: os,
            };
        };

        // Run `docker version --format {{.Server.Version}}` and poll with
        // `try_wait` so the attempt itself cannot block past `timeout`.
        match run_cli_bounded(
            &cli,
            &["version", "--format", "{{.Server.Version}}"],
            timeout,
        ) {
            // The attempt timed out / was killed: the daemon may still be booting.
            None => DockerDiagnostic {
                status: DockerStatus::DaemonStarting,
                message: format!(
                    "Docker daemon did not answer within {}s (still starting?)",
                    timeout.as_secs()
                ),
                suggested_action: None,
                detected_os: os,
            },
            // A successful `docker version` means the engine responded. The
            // version may legitimately be empty on some configs вЂ” that is
            // still a ready daemon, not an error.
            Some((true, out, _)) => {
                let version = out.trim().to_string();
                let suffix = if version.is_empty() {
                    String::new()
                } else {
                    format!(" (v{})", version)
                };
                DockerDiagnostic {
                    status: DockerStatus::DaemonReady,
                    message: format!("Docker daemon is running{}", suffix),
                    suggested_action: None,
                    detected_os: os,
                }
            }
            Some((false, _, err)) => classify_docker_error(&err.to_lowercase(), os),
        }
    }

    /// Locate a docker-compose configuration file for `dir`. The directory
    /// itself is searched first (docker compose's own default lookup order);
    /// when the file lives one level down (infra/, deploy/, docker/), the
    /// shallow scan finds it too. Common build/source directories that never
    /// host compose files are skipped. Returns the absolute path of the file,
    /// or None when the project has no compose configuration.
    pub fn find_compose_file(dir: &std::path::Path) -> Option<std::path::PathBuf> {
        if !dir.is_dir() {
            return None;
        }
        for name in COMPOSE_FILE_NAMES {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        let mut subdirs: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter(|e| {
                if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    return false;
                }
                let name = e.file_name().to_string_lossy().to_lowercase();
                !matches!(name.as_str(), "node_modules" | ".git")
            })
            .map(|e| e.path())
            .collect();
        subdirs.sort();
        for subdir in subdirs {
            for name in COMPOSE_FILE_NAMES {
                let candidate = subdir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        None
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
    ///
    /// When `check.auto_launch` is set, Docker Desktop / the Docker daemon
    /// is launched once (best-effort) as soon as the daemon is detected as
    /// unavailable, so a cold boot does not immediately fail the wait.
    pub async fn wait_for_daemon(
        check: &DockerReadinessCheck,
        cancelled: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> DockerReadinessResult {
        let start = tokio::time::Instant::now();
        let deadline = start + check.timeout;
        let mut auto_launched = false;
        // Log each missing service/port only once (not on every poll).
        let mut logged_service_wait = false;
        let mut logged_port_wait = false;

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
            // A missing CLI will never fix itself during the wait: fail
            // fast with the actionable diagnostic instead of burning the
            // full timeout on polling nothing.
            if diag.status == DockerStatus::CliMissing {
                return DockerReadinessResult {
                    status: DockerStatus::CliMissing,
                    elapsed: start.elapsed(),
                    message: diag.message,
                    suggested_action: diag.suggested_action,
                };
            }
            if diag.status == DockerStatus::DaemonReady {
                // If service name is specified, check that too.
                if let Some(ref service) = check.service_name {
                    if !is_service_running(service) {
                        if !logged_service_wait {
                            logged_service_wait = true;
                            log::info!(
                                "[docker] daemon ready but service '{}' not running; \
                                 waiting (bounded by {}s)",
                                service,
                                check.timeout.as_secs()
                            );
                        }
                        // Daemon ready but service not running вЂ” keep waiting.
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
                        if !logged_port_wait {
                            logged_port_wait = true;
                            log::info!(
                                "[docker] daemon ready but port {} not listening; \
                                 waiting (bounded by {}s)",
                                port,
                                check.timeout.as_secs()
                            );
                        }
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

            // Auto-launch the daemon once when it is unavailable. The launch
            // is fire-and-forget: the wait loop below reports readiness
            // as soon as `docker version` answers.
            if check.auto_launch && !auto_launched {
                auto_launched = true;
                log::info!("[docker] daemon not ready; attempting to auto-launch");
                if let Err(diag) = Self::try_launch_docker() {
                    log::warn!("[docker] auto-launch failed: {}", diag);
                }
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

    /// Ensure the Docker daemon is running: auto-launch it when it is down,
    /// then wait for readiness (bounded by `check.timeout`).
    ///
    /// The caller can inspect [`Self::check_daemon`] / [`Self::try_launch_docker`]
    /// beforehand to surface diagnostics about the launch attempt.
    pub async fn ensure_daemon(
        check: &DockerReadinessCheck,
        cancelled: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> DockerReadinessResult {
        let mut check = check.clone();
        if Self::check_daemon().status != DockerStatus::DaemonReady {
            check.auto_launch = true;
        }
        Self::wait_for_daemon(&check, cancelled).await
    }

    /// Attempt to start Docker Desktop / the Docker daemon on the current
    /// platform. Fire-and-forget: the launcher is spawned detached and no
    /// output is captured.
    ///
    /// Returns `Ok(())` when a launch attempt was made. Returns `Err` with
    /// a structured diagnostic when nothing could be attempted (the app is
    /// not installed anywhere we look).
    pub fn try_launch_docker() -> Result<(), DockerDiagnostic> {
        let os = crate::platform::host::current_os();
        let err_diag = |message: String, action: Option<String>| DockerDiagnostic {
            status: DockerStatus::DaemonUnavailable,
            message,
            suggested_action: action,
            detected_os: os,
        };

        match os {
            crate::platform::host::HostOs::Windows => {
                // 1. App Paths / PATH resolution of "Docker Desktop".
                let launcher = crate::platform::app_launcher::resolve_application(
                    "Docker Desktop",
                    None,
                    None,
                );
                if launcher.found && !launcher.is_flatpak {
                    let args: Vec<String> = launcher.args.clone();
                    if let Err(e) = spawn_detached(&launcher.program, &args) {
                        return Err(err_diag(
                            format!("Failed to launch Docker Desktop: {}", e),
                            Some(
                                "Start Docker Desktop manually from the Start menu \
                                 or the desktop shortcut."
                                    .to_string(),
                            ),
                        ));
                    }
                    log::info!(
                        "[docker] launched Docker Desktop via '{}'",
                        launcher.program
                    );
                    Self::best_effort_engine_start();
                    return Ok(());
                }
                // 2. Well-known install locations.
                let mut candidates: Vec<String> = vec![
                    r"C:\Program Files\Docker\Docker\Docker Desktop.exe".to_string(),
                    r"C:\Program Files\Docker\Docker\resources\Docker Desktop.exe".to_string(),
                ];
                if let Ok(pf) = std::env::var("ProgramFiles") {
                    candidates.push(format!(r"{}\Docker\Docker\Docker Desktop.exe", pf));
                }
                if let Ok(la) = std::env::var("LocalAppData") {
                    candidates.push(format!(r"{}\Docker\Docker Desktop.exe", la));
                }
                for candidate in candidates {
                    if Path::new(&candidate).is_file() {
                        if let Err(e) = spawn_detached(&candidate, &[]) {
                            return Err(err_diag(
                                format!("Failed to launch Docker Desktop: {}", e),
                                Some("Start Docker Desktop manually.".to_string()),
                            ));
                        }
                        log::info!("[docker] launched Docker Desktop via '{}'", candidate);
                        Self::best_effort_engine_start();
                        return Ok(());
                    }
                }
                // 3. The GUI is not installed, but the CLI may still exist
                // (Docker Engine / docker-machine): start the engine CLI.
                Self::best_effort_engine_start();
                Err(err_diag(
                    "Docker Desktop not found; cannot auto-launch the daemon".to_string(),
                    Some(
                        "Install Docker Desktop or start it manually, then retry the profile."
                            .to_string(),
                    ),
                ))
            }
            crate::platform::host::HostOs::Macos => {
                // `open -a Docker` is the canonical way to start Docker Desktop.
                if let Err(e) = spawn_detached("open", &["-a".to_string(), "Docker".to_string()]) {
                    return Err(err_diag(
                        format!("Failed to launch Docker Desktop: {}", e),
                        Some("Start Docker Desktop from /Applications manually.".to_string()),
                    ));
                }
                log::info!("[docker] launched Docker Desktop via `open -a Docker`");
                Self::best_effort_engine_start();
                Ok(())
            }
            crate::platform::host::HostOs::Linux => {
                // 1. `docker desktop start` (Docker Desktop 4.26+): boots the
                //    app AND its engine, needs no display session, and is
                //    idempotent. Preferred over the GUI binary for the daemon
                //    wait because the wait only cares about the engine.
                if Self::desktop_plugin_available() {
                    if let Some(cli) = Self::resolve_cli() {
                        let args = vec!["desktop".to_string(), "start".to_string()];
                        if let Err(e) = spawn_detached(&cli.to_string_lossy(), &args) {
                            log::warn!(
                                "[docker] failed to run `{} desktop start`: {}",
                                cli.display(),
                                e
                            );
                        } else {
                            log::info!(
                                "[docker] started Docker Desktop via `{} desktop start`",
                                cli.display()
                            );
                            return Ok(());
                        }
                    }
                }
                // 2. The GUI launcher (deb/rpm installs, custom paths, the
                //    systemd user unit) — same resolver the "Open Docker
                //    Desktop" step uses, so both stay in sync.
                if let Some((program, args)) = Self::desktop_launcher() {
                    if let Err(e) = spawn_detached(&program, &args) {
                        log::warn!("[docker] failed to launch Docker Desktop: {}", e);
                    } else {
                        log::info!("[docker] launched Docker Desktop via '{}'", program);
                        return Ok(());
                    }
                }
                // 3. Docker Engine installed as a system service (no Docker
                //    Desktop at all). The launch is best-effort: without
                //    privileges the daemon stays down and the wait loop
                //    reports the authoritative state.
                if resolve_executable("systemctl", None).is_some() {
                    let _ =
                        spawn_detached("systemctl", &["start".to_string(), "docker".to_string()]);
                    log::info!("[docker] attempted `systemctl start docker`");
                    return Ok(());
                }
                Err(err_diag(
                    "No way to auto-start Docker on this system".to_string(),
                    Some(
                        "Start Docker Desktop (`docker desktop start`) or the Docker Engine \
                         (`sudo systemctl start docker`), then retry the profile."
                            .to_string(),
                    ),
                ))
            }
        }
    }

    /// Fire the `docker desktop start` engine bootstrap and log any failure.
    /// The launch itself is best-effort вЂ” the daemon wait loop reports the
    /// authoritative state вЂ” so a failure here is a log line, not an abort.
    fn best_effort_engine_start() {
        match Self::docker_desktop_start() {
            Ok(()) => {}
            Err(diag) => log::error!("[docker] engine start failed: {}", diag),
        }
    }

    /// Best-effort start of the Docker Desktop ENGINE when the GUI is
    /// already running but the daemon pipe/endpoint is missing (the WSL2
    /// backend frequently ends up in this state after sleep or a crash).
    /// `docker desktop start` (Docker Desktop 4.26+) boots the engine
    /// without restarting the GUI and is idempotent. Older versions print
    /// an unknown-command error to stderr (discarded) вЂ” harmless.
    ///
    /// Returns `Ok(())` when a launch attempt was made; `Err` carries a
    /// structured diagnostic when the CLI is unavailable or the spawn failed.
    fn docker_desktop_start() -> Result<(), DockerDiagnostic> {
        let os = crate::platform::host::current_os();
        let Some(cli) = Self::resolve_cli() else {
            return Err(DockerDiagnostic {
                status: DockerStatus::DaemonUnavailable,
                message: "Docker CLI not found; cannot start the Docker Desktop engine".to_string(),
                suggested_action: Some(
                    "Install Docker Desktop or start it manually, then retry the profile."
                        .to_string(),
                ),
                detected_os: os,
            });
        };
        let mut cmd = std::process::Command::new(&cli);
        cmd.args(["desktop", "start"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(target_os = "windows")]
        crate::platform::suppress_child_console(&mut cmd);
        match cmd.spawn() {
            Ok(_) => {
                log::info!(
                    "[docker] started Desktop engine via `{} desktop start`",
                    cli.display()
                );
                Ok(())
            }
            Err(e) => Err(DockerDiagnostic {
                status: DockerStatus::DaemonUnavailable,
                message: format!("Failed to start Docker Desktop engine: {}", e),
                suggested_action: Some("Start Docker Desktop manually.".to_string()),
                detected_os: os,
            }),
        }
    }

    /// Preflight check for Docker commands вЂ” validates daemon state
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

/// Read the `Exec=` line of a desktop entry from the standard application
/// directories. Used to launch Docker Desktop on Linux installs that do not
/// follow the deb/rpm layout; `%`-field codes are dropped because there is no
/// file/URL argument to substitute.
#[cfg(target_os = "linux")]
fn desktop_entry_exec(file_name: &str) -> Option<(String, Vec<String>)> {
    let mut dirs: Vec<std::path::PathBuf> = vec![std::path::PathBuf::from("/usr/share/applications")];
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(std::path::PathBuf::from(home).join(".local/share/applications"));
    }
    if let Ok(data_dirs) = std::env::var("XDG_DATA_DIRS") {
        for dir in data_dirs.split(':').filter(|d| !d.trim().is_empty()) {
            dirs.push(std::path::PathBuf::from(dir).join("applications"));
        }
    }
    for dir in dirs {
        let Ok(text) = std::fs::read_to_string(dir.join(file_name)) else {
            continue;
        };
        for line in text.lines() {
            let Some(exec) = line.trim().strip_prefix("Exec=") else {
                continue;
            };
            let tokens: Vec<&str> = exec
                .split_whitespace()
                .filter(|t| !t.starts_with('%'))
                .collect();
            let Some((program, args)) = tokens.split_first() else {
                continue;
            };
            return Some((
                program.to_string(),
                args.iter().map(|s| s.to_string()).collect(),
            ));
        }
    }
    None
}

/// Non-Linux stub.
#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
fn desktop_entry_exec(_file_name: &str) -> Option<(String, Vec<String>)> {
    None
}

/// Whether a systemd user unit with this name exists (Linux). Used to detect
/// the Docker Desktop service older packages installed; the unit lives in the
/// user's systemd directory, so a plain `systemctl --user start` bootstraps
/// Docker Desktop without any GUI assumptions.
#[cfg(target_os = "linux")]
fn linux_systemd_user_unit_exists(unit: &str) -> bool {
    let mut dirs: Vec<std::path::PathBuf> = vec![std::path::PathBuf::from("/usr/lib/systemd/user")];
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(
            std::path::PathBuf::from(home)
                .join(".config/systemd/user"),
        );
    }
    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        dirs.push(std::path::PathBuf::from(config_home).join("systemd/user"));
    }
    dirs.iter().any(|dir| dir.join(unit).is_file())
}

/// Non-Linux stub.
#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
fn linux_systemd_user_unit_exists(_unit: &str) -> bool {
    false
}

/// Spawn a GUI application detached (fire-and-forget, no output capture,
/// new process group on Windows so it outlives the parent; `setsid` on Unix
/// for the same reason).
fn spawn_detached(program: &str, args: &[String]) -> Result<(), String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }
    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }
    cmd.spawn()
        .map(|child| {
            // Reap the detached child so Unix does not accumulate zombies
            // (dropping a Child does not wait for it).
            std::thread::spawn(move || {
                let mut child = child;
                let _ = child.wait();
            });
        })
        .map_err(|e| format!("Failed to launch '{}': {}", program, e))
}

/// Classify a Docker error message into a structured status.
fn classify_docker_error(stderr: &str, os: crate::platform::host::HostOs) -> DockerDiagnostic {
    if stderr.contains("cannot connect")
        || stderr.contains("pipe")
        || stderr.contains("daemon not running")
        || stderr.contains("error during connect")
        || stderr.contains("is the docker daemon running")
        || stderr.contains("connection refused")
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
                    "Start Docker Desktop (`docker desktop start`) or the Docker Engine \
                     (`sudo systemctl start docker`), then retry."
                        .to_string()
                }
            }),
            detected_os: os,
        }
    } else if stderr.contains("permission denied")
        && (stderr.contains("docker.sock") || stderr.contains("socket"))
    {
        // The classic "user is not in the docker group" case: the daemon is
        // running fine, the socket just refuses the connection. Without this
        // branch the user only saw a raw "Docker error: permission denied
        // while trying to connect..." with no hint.
        DockerDiagnostic {
            status: DockerStatus::DaemonUnavailable,
            message: "Permission denied on the Docker socket".to_string(),
            suggested_action: Some(match os {
                crate::platform::host::HostOs::Windows | crate::platform::host::HostOs::Macos => {
                    "Start Docker Desktop and make sure it is running, then retry.".to_string()
                }
                crate::platform::host::HostOs::Linux => {
                    "Add your user to the docker group (`sudo usermod -aG docker $USER`), \
                     then log out and back in вЂ” or use Docker Desktop, which manages \
                     the socket for the current user."
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
///
/// Uses the resolved CLI (never the bare `docker` name вЂ” GUI-launched apps
/// may lack it on PATH), and each attempt is bounded so a hung `docker ps`
/// cannot block the readiness wait loop indefinitely.
fn is_service_running(service_name: &str) -> bool {
    let Some(cli) = DockerService::resolve_cli() else {
        log::warn!("[docker] is_service_running: docker CLI not resolvable");
        return false;
    };
    run_cli_bounded(
        &cli,
        &[
            "ps",
            "--filter",
            &format!("name={}", service_name),
            "--format",
            "{{.Names}}",
        ],
        Duration::from_secs(5),
    )
    .map(|(_, out, _)| !out.trim().is_empty())
    .unwrap_or(false)
}

/// List the container IDs of running services for the compose project in
/// `dir`. Runs `docker compose ps --status running --quiet` inside `dir`
/// (compose's own config discovery), so it verifies the SAME project the
/// `docker compose up` step booted. The check is captured вЂ” it never opens
/// a terminal window. Returns an empty vector when no container is running
/// or when the check cannot complete (CLI missing, command error, timeout).
pub fn compose_running_ids(dir: &std::path::Path) -> Vec<String> {
    let Some(cli) = DockerService::resolve_cli() else {
        log::warn!("[docker] compose_running_ids: docker CLI not resolvable");
        return Vec::new();
    };
    run_cli_bounded_in(
        &cli,
        &["compose", "ps", "--status", "running", "--quiet"],
        Some(dir),
        Duration::from_secs(10),
    )
    .map(|(_, out, _)| {
        out.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

/// Run a Docker CLI command with a hard per-attempt timeout.
///
/// Returns `Some((success, stdout, stderr))` when the process finished within
/// `timeout`, or `None` when it was killed after exceeding the timeout (or
/// could not be spawned). Spawn/poll failures are logged вЂ” never panicked.
fn run_cli_bounded(
    cli: &std::path::Path,
    args: &[&str],
    timeout: Duration,
) -> Option<(bool, String, String)> {
    run_cli_bounded_in(cli, args, None, timeout)
}

/// [`run_cli_bounded`] with an explicit working directory for the child.
fn run_cli_bounded_in(
    cli: &std::path::Path,
    args: &[&str],
    dir: Option<&std::path::Path>,
    timeout: Duration,
) -> Option<(bool, String, String)> {
    let mut cmd = std::process::Command::new(cli);
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(target_os = "windows")]
    crate::platform::suppress_child_console(&mut cmd);
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log::error!(
                "[docker] failed to execute {} {}: {}",
                cli.display(),
                args.join(" "),
                e
            );
            return None;
        }
    };

    let deadline = std::time::Instant::now() + timeout;
    let exit = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                log::error!(
                    "[docker] failed to poll {} {}: {}",
                    cli.display(),
                    args.join(" "),
                    e
                );
                break None;
            }
        }
    };

    let status = exit?;

    let mut out = String::new();
    let mut err = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        use std::io::Read;
        let _ = stdout.read_to_string(&mut out);
    }
    if let Some(mut stderr) = child.stderr.take() {
        use std::io::Read;
        let _ = stderr.read_to_string(&mut err);
    }
    let _ = child.wait();
    Some((status.success(), out, err))
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
        assert_eq!(check.timeout, Duration::from_secs(120));
        assert_eq!(check.poll_interval, Duration::from_secs(1));
        assert!(check.service_name.is_none());
        assert!(check.port_check.is_none());
        assert!(!check.auto_launch);
    }

    #[test]
    fn try_launch_docker_does_not_panic() {
        // No assertion on the outcome: the machine may or may not have
        // Docker installed. The call must simply not panic.
        let _ = DockerService::try_launch_docker();
    }

    /// The "Open Docker Desktop" launcher must never be the bare `docker`
    /// CLI: on Linux that CLI only prints help, so the step reported success
    /// while no window ever opened. The CLI is only acceptable as
    /// `docker desktop start` (CLI plugin, Docker Desktop 4.26+).
    #[test]
    fn desktop_launcher_is_never_the_bare_docker_cli() {
        let Some((program, args)) = DockerService::desktop_launcher() else {
            return; // Docker Desktop is not installed on this machine.
        };
        let exe = std::path::Path::new(&program)
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if exe == "docker" {
            assert_eq!(
                args,
                vec!["desktop".to_string(), "start".to_string()],
                "the docker CLI may only be used as `docker desktop start`"
            );
        }
    }

    #[test]
    fn missing_systemd_user_unit_is_not_found() {
        assert!(!linux_systemd_user_unit_exists(
            "stackpilot-unit-that-does-not-exist.service"
        ));
    }

    #[test]
    fn spawn_detached_missing_program_errors() {
        let result = spawn_detached("this_program_definitely_does_not_exist_xyz_98765", &[]);
        assert!(result.is_err());
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
