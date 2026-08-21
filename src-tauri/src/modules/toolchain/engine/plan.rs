// ============================================================
// Canonical plan types (engine/plan.rs)
// ============================================================
//
// The plan is built and owned by the backend only. It carries typed
// installation phases, per-task integrity metadata, dependency edges,
// PATH changes (explicit, separately represented), admin/disk facts,
// capabilities and risk warnings. The client renders it; it can never
// produce or mutate one.

use serde::{Deserialize, Serialize};

use super::request::OperationKind;
use crate::modules::toolchain::models::PlatformCapabilities;

/// Typed job/task phases (contract §4 of the pipeline prompt).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Validating,
    Preparing,
    Downloading,
    Verifying,
    Installing,
    Configuring,
    UpdatingPath,
    CheckingHealth,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

/// Terminal/live status of a whole job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Partial,
    Failed,
    Cancelled,
    Interrupted,
}

impl JobStatus {
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            JobStatus::Succeeded
                | JobStatus::Partial
                | JobStatus::Failed
                | JobStatus::Cancelled
                | JobStatus::Interrupted
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Succeeded => "succeeded",
            JobStatus::Partial => "partial",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
            JobStatus::Interrupted => "interrupted",
        }
    }
}

/// Why a task needs no work — always truthful, never fabricated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoopReason {
    /// Installed at an acceptable version; reinstall only via explicit repair.
    AlreadyInstalled { version: String },
    /// Update requested but no newer acceptable target exists.
    UpdateUnavailable { version: String },
    /// Dual tool managed by docker-compose; host install is opt-in.
    DockerManaged,
}

/// What the executor should do for a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskAction {
    InstallNew {
        target_version: Option<String>,
    },
    Update {
        current_version: String,
        target_version: Option<String>,
    },
    RepairPath,
    HealthCheck,
    NoOp(NoopReason),
}

impl TaskAction {
    pub fn is_noop(&self) -> bool {
        matches!(self, TaskAction::NoOp(_))
    }
}

/// Where the tool lives/installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Host,
    Docker,
}

/// Backend-selected source. Id/kind/description/integrity come from the
/// catalog; the raw URL stays catalog-internal and never round-trips
/// through the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedSource {
    pub kind: String,
    pub id: String,
    pub description: String,
    /// SHA-256 from the catalog (hex). None = source ships no digest:
    /// execution requires explicit user confirmation (unverified).
    pub sha256: Option<String>,
    pub needs_admin: bool,
}

impl SelectedSource {
    pub fn unverified(&self) -> bool {
        self.sha256.is_none()
    }
}

/// Live state of one planned task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineTaskStatus {
    Pending,
    Running { phase: Phase },
    Succeeded { version: String },
    Failed { error: String },
    Cancelled,
    Interrupted,
}

impl EngineTaskStatus {
    pub fn terminal(&self) -> bool {
        !matches!(
            self,
            EngineTaskStatus::Pending | EngineTaskStatus::Running { .. }
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            EngineTaskStatus::Pending => "pending",
            EngineTaskStatus::Running { .. } => "running",
            EngineTaskStatus::Succeeded { .. } => "succeeded",
            EngineTaskStatus::Failed { .. } => "failed",
            EngineTaskStatus::Cancelled => "cancelled",
            EngineTaskStatus::Interrupted => "interrupted",
        }
    }
}

/// One canonical task: backend-resolved facts only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanTask {
    pub task_id: String,
    pub tool_id: String,
    pub display: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub action: TaskAction,
    #[serde(default)]
    pub source: Option<SelectedSource>,
    pub size_mb: u32,
    pub needs_admin: bool,
    /// Tool ids this task depends on (dependency closure edges).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// PATH entries the tool requires — explicit representation; applied
    /// only by approved jobs, audited in the final result.
    #[serde(default)]
    pub path_entries: Vec<String>,
    #[serde(default)]
    pub install_options: Vec<String>,
    pub execution_mode: ExecutionMode,
    pub status: EngineTaskStatus,
}

/// Risk/warning items shown on the review screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanWarning {
    UnverifiedSource { tool_id: String, source_id: String },
    AdminRequired { tool_id: String },
    ReinstallOnBroken { tool_id: String },
}

/// Explicit PATH change record for the final result (before/after diff).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathChangeRecord {
    pub tool_id: String,
    pub added: Vec<String>,
}

/// The canonical plan. Produced exclusively by `planner::build_plan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalPlan {
    pub plan_id: String,
    pub operation: OperationKind,
    pub os: String,
    pub created_at: String,
    /// Fingerprint of the inputs the plan was built from (request +
    /// detected versions + selected sources). Persisted with the plan.
    pub fingerprint: String,
    pub tasks: Vec<PlanTask>,
    pub total_size_mb: u64,
    pub free_space_mb: u64,
    pub enough_space: bool,
    pub needs_admin_any: bool,
    pub capabilities: PlatformCapabilities,
    pub warnings: Vec<PlanWarning>,
}

impl CanonicalPlan {
    /// Task ids that will actually run (no-ops excluded).
    pub fn actionable_tool_ids(&self) -> Vec<String> {
        self.tasks
            .iter()
            .filter(|t| !t.action.is_noop())
            .map(|t| t.tool_id.clone())
            .collect()
    }

    pub fn requested_tool_ids(&self) -> Vec<String> {
        self.tasks.iter().map(|t| t.tool_id.clone()).collect()
    }

    pub fn find_task(&self, task_id: &str) -> Option<&PlanTask> {
        self.tasks.iter().find(|t| t.task_id == task_id)
    }

    pub fn unverified_tools(&self) -> Vec<String> {
        self.warnings
            .iter()
            .filter_map(|w| match w {
                PlanWarning::UnverifiedSource { tool_id, .. } => Some(tool_id.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn admin_tools(&self) -> Vec<String> {
        self.tasks
            .iter()
            .filter(|t| t.needs_admin)
            .map(|t| t.tool_id.clone())
            .collect()
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_serialization_is_snake_case() {
        assert_eq!(
            serde_json::to_string(&Phase::UpdatingPath).unwrap(),
            "\"updating_path\""
        );
        assert_eq!(
            serde_json::to_string(&Phase::CheckingHealth).unwrap(),
            "\"checking_health\""
        );
        let phase: Phase = serde_json::from_str("\"downloading\"").unwrap();
        assert_eq!(phase, Phase::Downloading);
    }

    #[test]
    fn job_status_terminal_flags() {
        assert!(!JobStatus::Running.terminal());
        assert!(!JobStatus::Queued.terminal());
        assert!(JobStatus::Succeeded.terminal());
        assert!(JobStatus::Partial.terminal());
        assert!(JobStatus::Failed.terminal());
        assert!(JobStatus::Cancelled.terminal());
        assert!(JobStatus::Interrupted.terminal());
    }

    #[test]
    fn noop_action_is_detected() {
        let noop = TaskAction::NoOp(NoopReason::AlreadyInstalled {
            version: "22".into(),
        });
        let real = TaskAction::InstallNew {
            target_version: None,
        };
        assert!(noop.is_noop());
        assert!(!real.is_noop());
    }

    #[test]
    fn plan_helpers_report_truthful_counts() {
        let plan = CanonicalPlan {
            plan_id: "p1".into(),
            operation: OperationKind::Install,
            os: "windows".into(),
            created_at: "now".into(),
            fingerprint: "f".into(),
            tasks: vec![
                PlanTask {
                    task_id: "p1:git".into(),
                    tool_id: "git".into(),
                    display: "Git".into(),
                    icon: None,
                    action: TaskAction::InstallNew {
                        target_version: None,
                    },
                    source: None,
                    size_mb: 10,
                    needs_admin: true,
                    depends_on: vec![],
                    path_entries: vec![],
                    install_options: vec![],
                    execution_mode: ExecutionMode::Host,
                    status: EngineTaskStatus::Pending,
                },
                PlanTask {
                    task_id: "p1:node".into(),
                    tool_id: "node".into(),
                    display: "Node".into(),
                    icon: None,
                    action: TaskAction::NoOp(NoopReason::AlreadyInstalled {
                        version: "24".into(),
                    }),
                    source: None,
                    size_mb: 0,
                    needs_admin: false,
                    depends_on: vec![],
                    path_entries: vec![],
                    install_options: vec![],
                    execution_mode: ExecutionMode::Host,
                    status: EngineTaskStatus::Pending,
                },
            ],
            total_size_mb: 10,
            free_space_mb: 1000,
            enough_space: true,
            needs_admin_any: true,
            capabilities: PlatformCapabilities::default(),
            warnings: vec![PlanWarning::AdminRequired {
                tool_id: "git".into(),
            }],
        };

        assert_eq!(plan.actionable_tool_ids(), vec!["git"]);
        assert_eq!(plan.requested_tool_ids(), vec!["git", "node"]);
        assert_eq!(plan.admin_tools(), vec!["git"]);
        assert!(plan.find_task("p1:node").is_some());
        assert!(plan.find_task("missing").is_none());
    }

    #[test]
    fn interrupted_status_is_terminal() {
        assert!(!EngineTaskStatus::Pending.terminal());
        assert!(!EngineTaskStatus::Running {
            phase: Phase::Installing
        }
        .terminal());
        assert!(EngineTaskStatus::Interrupted.terminal());
        assert!(EngineTaskStatus::Cancelled.terminal());
    }
}
