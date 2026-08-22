// ============================================================
// Request model — the ONLY input shape accepted from the frontend
// ============================================================
//
// The request may carry: operation kind, tool ids, explicit source
// choices, local-vs-Docker choices, confirmation flags and an optional
// version policy. `deny_unknown_fields` makes every other field
// (URLs, paths, arguments, versions, sizes, task states, dependency
// lists) a hard deserialization error: the client physically cannot
// express them.

use serde::{Deserialize, Serialize};

/// Canonical operations of the environment-management pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// Install selected tools (idempotent: already-installed tools become no-ops).
    Install,
    /// Update selected tools to a backend-resolved target version.
    Update,
    /// Repair PATH entries for selected tools (idempotent, audited).
    RepairPath,
    /// Re-run health checks for selected tools (read-only).
    HealthCheck,
}

impl OperationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            OperationKind::Install => "install",
            OperationKind::Update => "update",
            OperationKind::RepairPath => "repair_path",
            OperationKind::HealthCheck => "health_check",
        }
    }

    pub fn mutates_machine(&self) -> bool {
        matches!(
            self,
            OperationKind::Install | OperationKind::Update | OperationKind::RepairPath
        )
    }
}

/// Explicit local-vs-Docker choice for dual tools (postgresql, redis, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionChoice {
    Host,
    Docker,
}

/// Version policy knob. The backend still resolves concrete versions
/// from the catalog/sources; the client only picks a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionChannel {
    Recommended,
    Latest,
}

/// One requested tool. Only ids and explicit user choices — nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolRequest {
    pub tool_id: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub execution: Option<ExecutionChoice>,
    /// UI modules/components selection (Qt: qt-webengine, ...). Carried
    /// verbatim into the task; interpreted only by catalog-defined pipelines.
    #[serde(default)]
    pub install_options: Vec<String>,
    /// Explicit reinstall confirmation: bypasses install idempotency.
    #[serde(default)]
    pub force_reinstall: bool,
}

impl ToolRequest {
    pub fn id(tool_id: &str) -> Self {
        Self {
            tool_id: tool_id.to_string(),
            source_id: None,
            execution: None,
            install_options: Vec::new(),
            force_reinstall: false,
        }
    }
}

/// Restricted engine request. Everything not declared here is rejected
/// at the deserialization boundary (`deny_unknown_fields`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineRequest {
    pub operation: OperationKind,
    #[serde(default)]
    pub tools: Vec<ToolRequest>,
    #[serde(default)]
    pub version_channel: Option<VersionChannel>,
    /// Required when any selected source lacks integrity metadata.
    #[serde(default)]
    pub confirm_unverified_sources: bool,
    /// Required when any planned task needs admin elevation.
    #[serde(default)]
    pub confirm_admin_elevation: bool,
    /// Preview mode (tcx_build_plan): the plan is built for REVIEW only.
    /// Confirmation gates (unverified sources / admin) are surfaced as
    /// warnings + checkboxes instead of hard errors; execution
    /// (`preview = false`) still refuses unconfirmed plans. Preview
    /// never registers a job and never executes anything.
    #[serde(default)]
    pub preview: bool,
    /// Fingerprint of the plan the client reviewed (from the preview).
    /// When set, execution refuses to run if the freshly rebuilt plan
    /// differs — a changed environment is reported, never silently
    /// executed. None = legacy callers without preview.
    #[serde(default)]
    pub expected_plan_fingerprint: Option<String>,
}

impl EngineRequest {
    pub fn new(operation: OperationKind, tools: Vec<ToolRequest>) -> Self {
        Self {
            operation,
            tools,
            version_channel: None,
            confirm_unverified_sources: false,
            confirm_admin_elevation: false,
            preview: false,
            expected_plan_fingerprint: None,
        }
    }

    pub fn deduped_tools(&self) -> Vec<ToolRequest> {
        let mut seen = std::collections::HashSet::new();
        self.tools
            .iter()
            .filter(|t| seen.insert(t.tool_id.clone()))
            .cloned()
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
    fn unknown_fields_are_rejected() {
        let raw = r#"{
            "operation": "install",
            "tools": [{"tool_id": "git", "url": "https://evil.example/x.exe"}]
        }"#;
        let parsed: Result<EngineRequest, _> = serde_json::from_str(raw);
        assert!(parsed.is_err(), "arbitrary URL must be unrepresentable");
    }

    #[test]
    fn fake_version_field_is_rejected() {
        let raw = r#"{
            "operation": "install",
            "tools": [{"tool_id": "git", "version": "999.0"}]
        }"#;
        assert!(serde_json::from_str::<EngineRequest>(raw).is_err());
    }

    #[test]
    fn fake_task_state_is_rejected() {
        let raw = r#"{
            "operation": "install",
            "tools": [{"tool_id": "git"}],
            "tasks": [{"task_id": "t1", "state": {"succeeded": {"version": "1"}}}]
        }"#;
        assert!(serde_json::from_str::<EngineRequest>(raw).is_err());
    }

    #[test]
    fn arbitrary_dependency_list_is_rejected() {
        let raw = r#"{
            "operation": "install",
            "tools": [{"tool_id": "composer", "depends_on": ["evil"]}]
        }"#;
        assert!(serde_json::from_str::<EngineRequest>(raw).is_err());
    }

    #[test]
    fn minimal_request_parses_with_defaults() {
        let raw = r#"{"operation": "health_check", "tools": [{"tool_id": "node"}]}"#;
        let req: EngineRequest = serde_json::from_str(raw).unwrap();
        assert_eq!(req.operation, OperationKind::HealthCheck);
        assert!(!req.confirm_admin_elevation);
        assert!(!req.preview, "preview defaults to false (execution mode)");
        assert!(req.tools[0].source_id.is_none());
        assert_eq!(req.tools[0].execution, None);
    }

    #[test]
    fn preview_flag_roundtrips() {
        let raw = r#"{
            "operation": "install",
            "tools": [{"tool_id": "git"}],
            "preview": true
        }"#;
        let req: EngineRequest = serde_json::from_str(raw).unwrap();
        assert!(req.preview);
        let serialized = serde_json::to_string(&req).unwrap();
        assert!(serialized.contains("\"preview\":true"));
    }

    #[test]
    fn explicit_choices_survive_roundtrip() {
        let raw = r#"{
            "operation": "install",
            "tools": [
                {"tool_id": "postgresql", "execution": "host", "source_id": "winget:PostgreSQL.PostgreSQL"},
                {"tool_id": "qt", "install_options": ["qt-webengine"]}
            ],
            "version_channel": "recommended",
            "confirm_unverified_sources": true,
            "confirm_admin_elevation": true
        }"#;
        let req: EngineRequest = serde_json::from_str(raw).unwrap();
        assert_eq!(req.tools[0].execution, Some(ExecutionChoice::Host));
        assert_eq!(
            req.tools[0].source_id.as_deref(),
            Some("winget:PostgreSQL.PostgreSQL")
        );
        assert_eq!(req.version_channel, Some(VersionChannel::Recommended));
        assert!(req.confirm_unverified_sources && req.confirm_admin_elevation);
        assert_eq!(req.tools[1].install_options, vec!["qt-webengine"]);
    }

    #[test]
    fn deduped_tools_keeps_first_occurrence_order() {
        let req = EngineRequest::new(
            OperationKind::Install,
            vec![
                ToolRequest::id("git"),
                ToolRequest::id("node"),
                ToolRequest::id("git"),
            ],
        );
        let ids: Vec<String> = req.deduped_tools().into_iter().map(|t| t.tool_id).collect();
        assert_eq!(ids, vec!["git", "node"]);
    }

    #[test]
    fn mutation_flags_are_truthful() {
        assert!(OperationKind::Install.mutates_machine());
        assert!(OperationKind::Update.mutates_machine());
        assert!(OperationKind::RepairPath.mutates_machine());
        assert!(!OperationKind::HealthCheck.mutates_machine());
    }
}
