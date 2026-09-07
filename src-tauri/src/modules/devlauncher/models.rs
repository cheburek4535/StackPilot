use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::Path;

/// Process tracking quality — single source of truth lives in the workspace
/// module; re-exported here for the V2 domain model.
pub use crate::modules::workspace::models::ProcessTrackingQuality;

/// The schema version written to every persisted V2 profile.
pub const PROFILE_SCHEMA_VERSION: &str = "2";

/// Validate that a shell name is supported on the current OS.
pub fn validate_shell_for_os(shell: &str) -> Result<(), String> {
    let os = crate::platform::host::current_os();
    let lower = shell.to_ascii_lowercase();
    match lower.as_str() {
        "sh" | "bash" | "zsh" | "fish" => {
            if cfg!(target_os = "windows") {
                // These are available via WSL/Git Bash but not native — warn.
                Ok(())
            } else {
                Ok(())
            }
        }
        "cmd" | "powershell" | "pwsh" => {
            if cfg!(not(target_os = "windows")) {
                Err(format!(
                    "Shell '{}' is only available on Windows, but current OS is {}",
                    shell, os
                ))
            } else {
                Ok(())
            }
        }
        _ => Err(format!(
            "Unsupported shell '{}'. Valid values: sh, bash, zsh, fish, cmd, powershell, pwsh.",
            shell
        )),
    }
}

// ---------------------------------------------------------------------------
// Legacy types — kept for backward compatibility. DO NOT REMOVE.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionType {
    RunCommand {
        command: String,
        working_dir: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        persistent: Option<bool>,
    },
    OpenApplication {
        path: String,
        #[serde(default)]
        args: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        args_list: Option<Vec<String>>,
    },
    OpenUrl {
        url: String,
    },
    WaitForUrl {
        url: String,
        timeout_secs: u64,
    },
    WaitForPort {
        host: String,
        port: u16,
        timeout_secs: u64,
    },
    Delay {
        seconds: u64,
    },
    ExecuteScript {
        script: String,
        shell: Option<String>,
    },
}

impl ActionType {
    pub fn is_persistent(&self) -> bool {
        matches!(
            self,
            ActionType::RunCommand {
                persistent: Some(true),
                ..
            }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchAction {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub action_type: ActionType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PreferredIde {
    Vscode,
    Pycharm,
    Goland,
    Idea,
    Webstorm,
    Xcode,
    VisualStudio,
    Custom(String),
}

impl PreferredIde {
    pub fn cli_name(&self) -> &str {
        match self {
            PreferredIde::Vscode => "code",
            PreferredIde::Pycharm => "pycharm",
            PreferredIde::Goland => "goland",
            PreferredIde::Idea => "idea",
            PreferredIde::Webstorm => "webstorm",
            PreferredIde::Xcode => "xed",
            PreferredIde::VisualStudio => "devenv",
            PreferredIde::Custom(name) => name,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            PreferredIde::Vscode => "VS Code",
            PreferredIde::Pycharm => "PyCharm",
            PreferredIde::Goland => "GoLand",
            PreferredIde::Idea => "IntelliJ IDEA",
            PreferredIde::Webstorm => "WebStorm",
            PreferredIde::Xcode => "Xcode",
            PreferredIde::VisualStudio => "Visual Studio",
            PreferredIde::Custom(name) => name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchProfile {
    pub name: String,
    pub description: String,
    pub project_path: Option<String>,
    pub actions: Vec<LaunchAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_binding_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_ide: Option<PreferredIde>,
    /// Schema marker carried through the legacy round-trip so the frontend
    /// can detect V2 profiles returned by legacy listing APIs. Absent on
    /// genuine v1 profiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    /// Stable profile ID carried through the legacy round-trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The full V2 step graph carried through the legacy round-trip so the
    /// frontend can launch V2 profiles without an extra fetch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<LaunchStep>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionStatus {
    Success { message: String },
    Failed { error: String },
    Skipped { reason: String },
}

// ---------------------------------------------------------------------------
// V2 ID generation — UUID v4, collision-safe, timestamp-independent.
// ---------------------------------------------------------------------------

pub fn generate_stable_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn default_schema_version() -> String {
    PROFILE_SCHEMA_VERSION.to_string()
}

fn default_true() -> bool {
    true
}

fn default_timeout_secs() -> u64 {
    120
}

fn default_retry_delay_ms() -> u64 {
    1000
}

pub fn default_now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ---------------------------------------------------------------------------
// V2 Domain Model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchProfileV2 {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default = "generate_stable_id")]
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "project_path"
    )]
    pub project_root: Option<String>,
    #[serde(default)]
    pub steps: Vec<LaunchStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_binding_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_ide: Option<PreferredIde>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_execution_mode: Option<ExecutionMode>,
    /// Unknown top-level fields from the JSON document are preserved here
    /// so newer schema fields written by future versions survive a
    /// round-trip through this version of the model.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchStep {
    pub id: String,
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub kind: StepKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<ExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<CompletionPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_policy: Option<FailurePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<RetryPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
    /// Unknown per-step fields from the JSON document are preserved here
    /// (forward compatibility: fields written by future schema versions
    /// survive a round-trip through this version of the model).
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

// ---------------------------------------------------------------------------
// StepKind
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StepKind {
    RunCommand {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command_spec: Option<CommandSpec>,
    },
    RunScript {
        script: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell: Option<String>,
    },
    OpenApplication {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        args: Option<Vec<String>>,
    },
    OpenUrl {
        url: String,
    },
    WaitForPort {
        host: String,
        port: u16,
        /// Additional ports to accept during the readiness wait. The step
        /// succeeds when ANY of `[port] + candidate_ports` opens. Dev servers
        /// (Vite, Next.js, Expo) auto-increment their port when the primary
        /// one is taken, so generated profiles carry the real candidates and
        /// the wait no longer times out against a busy port.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        candidate_ports: Vec<u16>,
    },
    WaitForUrl {
        url: String,
    },
    /// Waits until the Docker daemon answers (polling `docker version`).
    /// Timeout is taken from the step's `timeout` field (default 30s).
    WaitForDocker {},
    Delay {
        seconds: u64,
    },
    OpenTerminal {
        command: String,
    },
    OpenFolder {
        path: String,
    },
}

// ---------------------------------------------------------------------------
// CommandSpec
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Captured,
    VisibleTerminal,
    Detached,
}

// ---------------------------------------------------------------------------
// ExecutionMode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    OneShot,
    LongRunning,
}

// ---------------------------------------------------------------------------
// CompletionPolicy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompletionPolicy {
    ExitSuccess,
    ProcessStarted,
    PortOpen {
        host: String,
        port: u16,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
    },
    UrlReady {
        url: String,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
    },
    DelayElapsed {
        #[serde(default)]
        seconds: u64,
    },
    ExternalLaunchAccepted,
    Manual,
}

// ---------------------------------------------------------------------------
// FailurePolicy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    StopRun,
    SkipDependents,
    WarnAndContinue,
}

// ---------------------------------------------------------------------------
// RetryPolicy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_retries: u32,
    #[serde(default = "default_retry_delay_ms")]
    pub delay_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_multiplier: Option<f64>,
}

// ---------------------------------------------------------------------------
// LaunchRun
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchRun {
    pub run_id: String,
    pub profile_id: String,
    pub profile_name: String,
    pub status: RunStatus,
    pub steps: Vec<StepExecutionState>,
    pub created_at: String,
    pub finished_at: Option<String>,
    pub cancelled: bool,
    pub diagnostics: Vec<Diagnostic>,
}

// ---------------------------------------------------------------------------
// RunStatus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    PartialSuccess,
}

// ---------------------------------------------------------------------------
// StepExecutionState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepExecutionState {
    pub step_id: String,
    pub status: StepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries_remaining: Option<u32>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    #[serde(default)]
    pub attempts: Vec<StepAttempt>,
}

// ---------------------------------------------------------------------------
// StepAttempt
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepAttempt {
    pub attempt_number: u32,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: StepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// StepStatus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
    Cancelled,
    Retrying,
}

// ---------------------------------------------------------------------------
// ManagedProcess (V2)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedProcess {
    pub id: String,
    pub pid: u32,
    pub label: String,
    pub status: crate::modules::workspace::models::ProcessStatus,
    pub started_at: String,
    pub duration_secs: u64,
    pub restarts: u32,
    pub last_error: Option<String>,
    pub session_id: Option<String>,
    #[serde(default)]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking_quality: Option<ProcessTrackingQuality>,
}

// ---------------------------------------------------------------------------
// ProcessTrackingQuality (V2) — defined in workspace::models, re-exported.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Diagnostic
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    pub source: LogSource,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogSource {
    Process,
    Orchestrator,
    Preflight,
    Readiness,
    User,
}

// ---------------------------------------------------------------------------
// Log retrieval types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunLogs {
    pub run_id: String,
    pub stdout: Vec<String>,
    pub stderr: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepLogs {
    pub run_id: String,
    pub step_id: String,
    pub process_id: String,
    pub stdout: Vec<String>,
    pub stderr: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncation: Option<(
        crate::modules::workspace::models::LogTruncation,
        crate::modules::workspace::models::LogTruncation,
    )>,
}

// ---------------------------------------------------------------------------
// Platform capabilities (returned by get_platform_capabilities)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformCapabilities {
    pub os: String,
    pub arch: String,
    pub shells: Vec<String>,
    pub has_docker: bool,
    pub has_compose: bool,
    pub default_terminal: String,
    pub supports_terminal_windows: bool,
}

// ---------------------------------------------------------------------------
// From conversions: Legacy → V2
// ---------------------------------------------------------------------------

impl From<LaunchAction> for LaunchStep {
    fn from(action: LaunchAction) -> Self {
        let (kind, visibility, execution_mode, completion, working_directory, timeout) =
            match action.action_type {
                ActionType::RunCommand {
                    command,
                    working_dir,
                    persistent,
                } => {
                    let vis = if persistent == Some(true) {
                        Some(Visibility::VisibleTerminal)
                    } else {
                        Some(Visibility::Captured)
                    };
                    let mode = if persistent == Some(true) {
                        Some(ExecutionMode::LongRunning)
                    } else {
                        Some(ExecutionMode::OneShot)
                    };
                    (
                        StepKind::RunCommand {
                            command,
                            command_spec: None,
                        },
                        vis,
                        mode,
                        None,
                        working_dir,
                        None,
                    )
                }
                ActionType::OpenApplication {
                    path,
                    args,
                    args_list,
                } => {
                    let args_vec = args_list
                        .or_else(|| args.map(|a| a.split_whitespace().map(String::from).collect()));
                    (
                        StepKind::OpenApplication {
                            path,
                            args: args_vec,
                        },
                        Some(Visibility::Detached),
                        None,
                        Some(CompletionPolicy::ExternalLaunchAccepted),
                        None,
                        None,
                    )
                }
                ActionType::OpenUrl { url } => (
                    StepKind::OpenUrl { url },
                    None,
                    None,
                    Some(CompletionPolicy::ExternalLaunchAccepted),
                    None,
                    None,
                ),
                ActionType::WaitForUrl { url, timeout_secs } => (
                    StepKind::WaitForUrl { url: url.clone() },
                    None,
                    None,
                    Some(CompletionPolicy::UrlReady { url, timeout_secs }),
                    None,
                    Some(timeout_secs),
                ),
                ActionType::WaitForPort {
                    host,
                    port,
                    timeout_secs,
                } => (
                    StepKind::WaitForPort {
                        host: host.clone(),
                        port,
                        candidate_ports: Vec::new(),
                    },
                    None,
                    None,
                    Some(CompletionPolicy::PortOpen {
                        host,
                        port,
                        timeout_secs,
                    }),
                    None,
                    Some(timeout_secs),
                ),
                ActionType::Delay { seconds } => (
                    StepKind::Delay { seconds },
                    None,
                    None,
                    Some(CompletionPolicy::DelayElapsed { seconds }),
                    None,
                    None,
                ),
                ActionType::ExecuteScript { script, shell } => (
                    StepKind::RunScript { script, shell },
                    Some(Visibility::Captured),
                    Some(ExecutionMode::OneShot),
                    None,
                    None,
                    None,
                ),
            };

        let mut metadata = HashMap::new();
        metadata.insert("migrated_from".to_string(), "legacy_action".to_string());

        LaunchStep {
            id: action.id,
            label: action.label,
            enabled: action.enabled,
            kind,
            depends_on: Vec::new(),
            working_directory,
            environment: None,
            visibility,
            execution_mode,
            completion,
            timeout,
            failure_policy: None,
            retry_policy: None,
            metadata: Some(metadata),
            extra: Map::new(),
        }
    }
}

impl From<LaunchStep> for LaunchAction {
    fn from(step: LaunchStep) -> Self {
        let action_type = match step.kind {
            StepKind::RunCommand { command, .. } => {
                let persistent = step.execution_mode.as_ref().map(|m| match m {
                    ExecutionMode::LongRunning => true,
                    ExecutionMode::OneShot => false,
                });
                ActionType::RunCommand {
                    command,
                    working_dir: step.working_directory,
                    persistent,
                }
            }
            StepKind::RunScript { script, shell } => ActionType::ExecuteScript { script, shell },
            StepKind::OpenApplication { path, args } => ActionType::OpenApplication {
                path,
                args: None,
                args_list: args,
            },
            StepKind::OpenUrl { url } => ActionType::OpenUrl { url },
            StepKind::WaitForPort { host, port, .. } => {
                let timeout = step
                    .completion
                    .as_ref()
                    .and_then(|c| match c {
                        CompletionPolicy::PortOpen { timeout_secs, .. } => Some(*timeout_secs),
                        _ => None,
                    })
                    .unwrap_or(120);
                ActionType::WaitForPort {
                    host,
                    port,
                    timeout_secs: timeout,
                }
            }
            StepKind::WaitForUrl { url } => {
                let timeout = step
                    .completion
                    .as_ref()
                    .and_then(|c| match c {
                        CompletionPolicy::UrlReady { timeout_secs, .. } => Some(*timeout_secs),
                        _ => None,
                    })
                    .unwrap_or(120);
                ActionType::WaitForUrl {
                    url,
                    timeout_secs: timeout,
                }
            }
            StepKind::Delay { seconds } => ActionType::Delay { seconds },
            StepKind::WaitForDocker {} => ActionType::RunCommand {
                command: "docker version --format {{.Server.Version}}".to_string(),
                working_dir: step.working_directory.clone(),
                persistent: Some(false),
            },
            StepKind::OpenTerminal { command } => ActionType::RunCommand {
                command,
                working_dir: step.working_directory.clone(),
                persistent: Some(true),
            },
            StepKind::OpenFolder { path } => ActionType::OpenApplication {
                path,
                args: None,
                args_list: None,
            },
        };

        LaunchAction {
            id: step.id,
            label: step.label,
            enabled: step.enabled,
            action_type,
        }
    }
}

impl From<LaunchProfile> for LaunchProfileV2 {
    fn from(profile: LaunchProfile) -> Self {
        let steps: Vec<LaunchStep> = profile.actions.into_iter().map(LaunchStep::from).collect();
        LaunchProfileV2 {
            schema_version: PROFILE_SCHEMA_VERSION.to_string(),
            id: generate_stable_id(),
            name: profile.name,
            description: profile.description,
            project_root: profile.project_path,
            steps,
            environment_binding_id: profile.environment_binding_id,
            preferred_ide: profile.preferred_ide,
            default_execution_mode: None,
            extra: Map::new(),
        }
    }
}

impl From<LaunchProfileV2> for LaunchProfile {
    fn from(profile: LaunchProfileV2) -> Self {
        let actions: Vec<LaunchAction> = profile
            .steps
            .clone()
            .into_iter()
            .map(LaunchAction::from)
            .collect();
        LaunchProfile {
            name: profile.name,
            description: profile.description,
            project_path: profile.project_root,
            actions,
            environment_binding_id: profile.environment_binding_id,
            preferred_ide: profile.preferred_ide,
            schema_version: Some(profile.schema_version),
            id: Some(profile.id),
            steps: Some(profile.steps),
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy migration — full profile conversion
// ---------------------------------------------------------------------------

/// Convert a legacy `LaunchProfile` (schema v1: flat `actions` array) into a
/// V2 step graph.
///
/// Migration behavior (documented in docs/devlauncher-progress.md):
/// - Action IDs, labels and `enabled` flags are preserved verbatim.
/// - `RunCommand` actions become `RunCommand` steps. The legacy
///   `working_dir` is preserved in `working_directory`; `persistent: true`
///   maps to `VisibleTerminal` + `LongRunning`, otherwise `Captured` +
///   `OneShot`.
/// - `OpenApplication` preserves `path`; `args_list` wins over the legacy
///   whitespace-split `args` string.
/// - `WaitForUrl` / `WaitForPort` keep their timeout in both `timeout` and
///   the `completion` policy (`UrlReady` / `PortOpen`).
/// - `Delay`, `ExecuteScript` map 1:1.
/// - Legacy execution was strictly sequential, so steps are chained with
///   `depends_on` pointing at the previous action's step. This preserves
///   the observable legacy behavior (a later `WaitForPort` does not race
///   its server start) instead of the contract's literal empty `depends_on`,
///   which would run all migrated steps in parallel.
/// - `project_path`, `environment_binding_id` and `preferred_ide` carry
///   over unchanged.
pub fn migrate_legacy_profile(profile: LaunchProfile) -> LaunchProfileV2 {
    let mut steps: Vec<LaunchStep> = profile.actions.into_iter().map(LaunchStep::from).collect();
    for i in 1..steps.len() {
        let previous = steps[i - 1].id.clone();
        steps[i].depends_on.push(previous);
    }
    LaunchProfileV2 {
        schema_version: PROFILE_SCHEMA_VERSION.to_string(),
        id: generate_stable_id(),
        name: profile.name,
        description: profile.description,
        project_root: profile.project_path,
        steps,
        environment_binding_id: profile.environment_binding_id,
        preferred_ide: profile.preferred_ide,
        default_execution_mode: None,
        extra: Map::new(),
    }
}

// ---------------------------------------------------------------------------
// Path model — resolution of relative step directories
// ---------------------------------------------------------------------------

/// Resolve a step's working directory against the profile's explicit project
/// root.
///
/// - Absolute paths pass through unchanged.
/// - Relative paths (e.g. `./backend`) are joined onto `project_root`.
/// - When both are absent, `None` is returned (the platform default working
///   directory applies).
///
/// A saved profile must never depend on the globally active workspace
/// project to resolve its paths — the profile's own `project_root` is the
/// only base used here.
pub fn resolve_working_directory(
    project_root: Option<&str>,
    working_directory: Option<&str>,
) -> Option<String> {
    let dir = working_directory?;
    let path = Path::new(dir);
    if path.is_absolute() {
        return Some(dir.to_string());
    }
    let root = project_root?;
    // Normalize the joined path: `Path::join` keeps `.` components
    // (`/proj/./backend`), which are valid but ugly in logs and misleading
    // in step metadata. CurDir components are dropped lexically.
    let mut joined = std::path::PathBuf::from(root);
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                // Pop only when it does not escape the accumulated prefix.
                if joined.file_name().is_some() {
                    joined.pop();
                }
            }
            other => joined.push(other.as_os_str()),
        }
    }
    Some(joined.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// Defaults for CompletionPolicy based on StepKind + ExecutionMode
// ---------------------------------------------------------------------------

impl StepKind {
    pub fn default_completion(&self, execution_mode: Option<&ExecutionMode>) -> CompletionPolicy {
        match self {
            StepKind::RunCommand { .. } | StepKind::RunScript { .. } => match execution_mode {
                Some(ExecutionMode::LongRunning) => CompletionPolicy::ProcessStarted,
                _ => CompletionPolicy::ExitSuccess,
            },
            StepKind::OpenApplication { .. } | StepKind::OpenFolder { .. } => {
                CompletionPolicy::ExternalLaunchAccepted
            }
            StepKind::OpenUrl { .. } => CompletionPolicy::ExternalLaunchAccepted,
            StepKind::WaitForPort { host, port, .. } => CompletionPolicy::PortOpen {
                host: host.clone(),
                port: *port,
                timeout_secs: default_timeout_secs(),
            },
            StepKind::WaitForUrl { url } => CompletionPolicy::UrlReady {
                url: url.clone(),
                timeout_secs: default_timeout_secs(),
            },
            StepKind::WaitForDocker {} => CompletionPolicy::ProcessStarted,
            StepKind::Delay { seconds } => CompletionPolicy::DelayElapsed { seconds: *seconds },
            StepKind::OpenTerminal { .. } => CompletionPolicy::ProcessStarted,
        }
    }

    pub fn default_visibility(&self) -> Visibility {
        match self {
            StepKind::RunCommand { .. } | StepKind::RunScript { .. } => Visibility::Captured,
            StepKind::OpenTerminal { .. } => Visibility::VisibleTerminal,
            _ => Visibility::Detached,
        }
    }

    pub fn default_execution_mode(&self) -> ExecutionMode {
        match self {
            StepKind::RunCommand { .. } | StepKind::RunScript { .. } => ExecutionMode::OneShot,
            StepKind::OpenTerminal { .. } => ExecutionMode::LongRunning,
            _ => ExecutionMode::OneShot,
        }
    }

    /// Default failure policy for a step given its dependency state.
    ///
    /// Leaf steps (no dependents) default to `WarnAndContinue`: their
    /// failure is isolated to themselves. Every step WITH dependents
    /// defaults to `SkipDependents`: a failed step (install, build, docker
    /// compose, readiness wait) makes only its own downstream chain useless,
    /// but it must NEVER take down independent branches of the run — most
    /// importantly, a failing leaf must not abort a `docker compose up` that
    /// is still bringing the project's containers up. A hard `StopRun` abort
    /// (which kills every in-flight process and marks all concurrent steps as
    /// "Run aborted by a failing step; process terminated") is reserved for
    /// profiles that explicitly opt in via `failure_policy`.
    pub fn default_failure_policy(&self, has_dependents: bool) -> FailurePolicy {
        if !has_dependents {
            return FailurePolicy::WarnAndContinue;
        }
        FailurePolicy::SkipDependents
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_round_trip_preserves_action_ids() {
        let action = LaunchAction {
            id: "act_test_1".to_string(),
            label: "Test".to_string(),
            enabled: true,
            action_type: ActionType::RunCommand {
                command: "echo hello".to_string(),
                working_dir: Some("./backend".to_string()),
                persistent: Some(false),
            },
        };
        let step = LaunchStep::from(action.clone());
        assert_eq!(step.id, "act_test_1");
        assert_eq!(step.working_directory.as_deref(), Some("./backend"));
        let back = LaunchAction::from(step);
        assert_eq!(back.id, "act_test_1");
    }

    #[test]
    fn wait_for_port_candidate_ports_json_round_trip() {
        let step = LaunchStep {
            id: "s1".to_string(),
            label: "wait".to_string(),
            enabled: true,
            kind: StepKind::WaitForPort {
                host: "127.0.0.1".to_string(),
                port: 5173,
                candidate_ports: vec![5174, 5175],
            },
            depends_on: vec![],
            working_directory: None,
            environment: None,
            visibility: None,
            execution_mode: None,
            completion: None,
            timeout: Some(60),
            failure_policy: None,
            retry_policy: None,
            metadata: None,
            extra: Map::new(),
        };
        let json = serde_json::to_string(&step).unwrap();
        assert!(json.contains("candidate_ports"), "{json}");
        let back: LaunchStep = serde_json::from_str(&json).unwrap();
        match back.kind {
            StepKind::WaitForPort {
                port,
                candidate_ports,
                ..
            } => {
                assert_eq!(port, 5173);
                assert_eq!(candidate_ports, vec![5174, 5175]);
            }
            _ => panic!("expected WaitForPort"),
        }
    }

    #[test]
    fn legacy_profile_to_v2_preserves_name() {
        let profile = LaunchProfile {
            name: "My Profile".to_string(),
            description: "desc".to_string(),
            project_path: Some("/tmp".to_string()),
            actions: vec![],
            environment_binding_id: None,
            preferred_ide: Some(PreferredIde::Vscode),
            schema_version: None,
            id: None,
            steps: None,
        };
        let v2 = LaunchProfileV2::from(profile);
        assert_eq!(v2.name, "My Profile");
        assert_eq!(v2.project_root, Some("/tmp".to_string()));
        assert_eq!(v2.schema_version, "2");
        assert!(v2.preferred_ide.is_some());
    }

    #[test]
    fn v2_to_legacy_preserves_steps_as_actions() {
        let v2 = LaunchProfileV2 {
            schema_version: "2".to_string(),
            id: "test-id".to_string(),
            name: "P".to_string(),
            description: "d".to_string(),
            project_root: None,
            steps: vec![LaunchStep {
                id: "s1".to_string(),
                label: "step1".to_string(),
                enabled: true,
                kind: StepKind::RunCommand {
                    command: "ls".to_string(),
                    command_spec: None,
                },
                depends_on: vec![],
                working_directory: None,
                environment: None,
                visibility: None,
                execution_mode: None,
                completion: None,
                timeout: None,
                failure_policy: None,
                retry_policy: None,
                metadata: None,
                extra: Map::new(),
            }],
            environment_binding_id: None,
            preferred_ide: None,
            default_execution_mode: None,
            extra: Map::new(),
        };
        let legacy = LaunchProfile::from(v2);
        assert_eq!(legacy.actions.len(), 1);
        assert_eq!(legacy.actions[0].id, "s1");
    }

    #[test]
    fn default_completion_for_wait_for_port() {
        let kind = StepKind::WaitForPort {
            host: "localhost".to_string(),
            port: 3000,
            candidate_ports: vec![3001, 3002],
        };
        let policy = kind.default_completion(None);
        match policy {
            CompletionPolicy::PortOpen {
                port, timeout_secs, ..
            } => {
                assert_eq!(port, 3000);
                assert_eq!(timeout_secs, 120);
            }
            _ => panic!("expected PortOpen"),
        }
    }

    #[test]
    fn default_completion_for_delay() {
        let kind = StepKind::Delay { seconds: 5 };
        let policy = kind.default_completion(None);
        match policy {
            CompletionPolicy::DelayElapsed { seconds } => assert_eq!(seconds, 5),
            _ => panic!("expected DelayElapsed"),
        }
    }

    #[test]
    fn default_completion_for_run_command_oneshot() {
        let kind = StepKind::RunCommand {
            command: "echo hi".to_string(),
            command_spec: None,
        };
        let policy = kind.default_completion(Some(&ExecutionMode::OneShot));
        assert!(matches!(policy, CompletionPolicy::ExitSuccess));
    }

    #[test]
    fn default_completion_for_run_command_longrunning() {
        let kind = StepKind::RunCommand {
            command: "npm start".to_string(),
            command_spec: None,
        };
        let policy = kind.default_completion(Some(&ExecutionMode::LongRunning));
        assert!(matches!(policy, CompletionPolicy::ProcessStarted));
    }

    #[test]
    fn v2_json_round_trip() {
        let profile = LaunchProfileV2 {
            schema_version: "2".to_string(),
            id: generate_stable_id(),
            name: "Test".to_string(),
            description: "desc".to_string(),
            project_root: Some("/tmp".to_string()),
            steps: vec![LaunchStep {
                id: "s1".to_string(),
                label: "step".to_string(),
                enabled: true,
                kind: StepKind::RunCommand {
                    command: "echo hi".to_string(),
                    command_spec: None,
                },
                depends_on: vec![],
                working_directory: None,
                environment: None,
                visibility: Some(Visibility::Captured),
                execution_mode: Some(ExecutionMode::OneShot),
                completion: Some(CompletionPolicy::ExitSuccess),
                timeout: None,
                failure_policy: Some(FailurePolicy::StopRun),
                retry_policy: Some(RetryPolicy {
                    max_retries: 3,
                    delay_ms: 1000,
                    backoff_multiplier: Some(2.0),
                }),
                metadata: None,
                extra: Map::new(),
            }],
            environment_binding_id: None,
            preferred_ide: None,
            default_execution_mode: None,
            extra: Map::new(),
        };

        let json = serde_json::to_string(&profile).unwrap();
        let back: LaunchProfileV2 = serde_json::from_str(&json).unwrap();
        assert_eq!(back.steps[0].id, "s1");
        assert!(back.steps[0].retry_policy.is_some());
    }

    #[test]
    fn unknown_fields_preserved_on_round_trip() {
        let json = r#"{
            "schema_version": "2",
            "id": "abc",
            "name": "P",
            "description": "d",
            "project_root": "/tmp",
            "steps": [
                {
                    "id": "s1",
                    "label": "step",
                    "enabled": true,
                    "kind": { "type": "run_command", "command": "ls" },
                    "future_step_field": { "anything": 1 }
                }
            ],
            "future_profile_field": "keep-me"
        }"#;
        let profile: LaunchProfileV2 = serde_json::from_str(json).unwrap();
        assert_eq!(
            profile.extra.get("future_profile_field").unwrap(),
            "keep-me"
        );
        assert_eq!(
            profile.steps[0]
                .extra
                .get("future_step_field")
                .and_then(|v| v.get("anything"))
                .unwrap(),
            1
        );
        let round: LaunchProfileV2 =
            serde_json::from_str(&serde_json::to_string(&profile).unwrap()).unwrap();
        assert_eq!(round.extra.get("future_profile_field").unwrap(), "keep-me");
        assert!(round.steps[0].extra.contains_key("future_step_field"));
    }

    #[test]
    fn migration_chains_steps_sequentially() {
        let legacy = LaunchProfile {
            name: "Legacy".to_string(),
            description: "d".to_string(),
            project_path: Some("/proj".to_string()),
            actions: vec![
                LaunchAction {
                    id: "a1".to_string(),
                    label: "Start server".to_string(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm run dev".to_string(),
                        working_dir: Some("./backend".to_string()),
                        persistent: Some(true),
                    },
                },
                LaunchAction {
                    id: "a2".to_string(),
                    label: "Wait port".to_string(),
                    enabled: false,
                    action_type: ActionType::WaitForPort {
                        host: "127.0.0.1".to_string(),
                        port: 3000,
                        timeout_secs: 45,
                    },
                },
            ],
            environment_binding_id: Some("env-1".to_string()),
            preferred_ide: Some(PreferredIde::Pycharm),
            schema_version: None,
            id: None,
            steps: None,
        };
        let v2 = migrate_legacy_profile(legacy);
        assert_eq!(v2.schema_version, PROFILE_SCHEMA_VERSION);
        assert_eq!(v2.project_root.as_deref(), Some("/proj"));
        assert_eq!(v2.environment_binding_id.as_deref(), Some("env-1"));
        assert!(matches!(v2.preferred_ide, Some(PreferredIde::Pycharm)));
        assert_eq!(v2.steps.len(), 2);
        assert_eq!(v2.steps[0].id, "a1");
        assert!(v2.steps[0].depends_on.is_empty());
        assert_eq!(v2.steps[1].id, "a2");
        assert_eq!(v2.steps[1].depends_on, vec!["a1".to_string()]);
        assert!(!v2.steps[1].enabled);
        assert_eq!(v2.steps[1].timeout, Some(45));
        assert_eq!(v2.steps[0].working_directory.as_deref(), Some("./backend"));
        assert_eq!(v2.steps[0].execution_mode, Some(ExecutionMode::LongRunning));
        assert_eq!(v2.steps[0].visibility, Some(Visibility::VisibleTerminal));
        assert!(v2.steps[0]
            .metadata
            .as_ref()
            .unwrap()
            .contains_key("migrated_from"));
    }

    #[test]
    fn resolve_working_directory_model() {
        let root = "/proj";
        // Expected join rendered through Path on the current platform so
        // separator style never matters.
        let joined = |rel: &str| Path::new(root).join(rel).to_string_lossy().into_owned();
        // Relative directories resolve against the project root, and `.`
        // components are stripped from the result.
        assert_eq!(
            resolve_working_directory(Some(root), Some("./backend")),
            Some(joined("backend"))
        );
        assert_eq!(
            resolve_working_directory(Some(root), Some("backend")),
            Some(joined("backend"))
        );
        // Absolute paths pass through unchanged.
        let abs = if cfg!(windows) {
            r"C:\abs\path"
        } else {
            "/abs/path"
        };
        assert_eq!(
            resolve_working_directory(Some(root), Some(abs)),
            Some(abs.to_string())
        );
        assert_eq!(resolve_working_directory(Some(root), None), None);
        assert_eq!(resolve_working_directory(None, Some("./x")), None);
        assert_eq!(resolve_working_directory(None, None), None);
    }
}
