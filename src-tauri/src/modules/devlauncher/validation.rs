use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use super::models::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileValidationDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    pub step_id: Option<String>,
    pub field: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileValidationResult {
    pub valid: bool,
    pub diagnostics: Vec<ProfileValidationDiagnostic>,
}

impl ProfileValidationResult {
    fn ok() -> Self {
        Self {
            valid: true,
            diagnostics: Vec::new(),
        }
    }

    fn error(code: &str, message: String) -> Self {
        Self {
            valid: false,
            diagnostics: vec![ProfileValidationDiagnostic {
                severity: DiagnosticSeverity::Error,
                code: code.to_string(),
                message,
                step_id: None,
                field: None,
            }],
        }
    }

    fn push_error(
        &mut self,
        code: &str,
        message: String,
        step_id: Option<String>,
        field: Option<String>,
    ) {
        self.diagnostics.push(ProfileValidationDiagnostic {
            severity: DiagnosticSeverity::Error,
            code: code.to_string(),
            message,
            step_id,
            field,
        });
        self.valid = false;
    }

    fn push_warning(
        &mut self,
        code: &str,
        message: String,
        step_id: Option<String>,
        field: Option<String>,
    ) {
        self.diagnostics.push(ProfileValidationDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: code.to_string(),
            message,
            step_id,
            field,
        });
    }

    fn merge(&mut self, other: ProfileValidationResult) {
        if !other.valid {
            self.valid = false;
        }
        self.diagnostics.extend(other.diagnostics);
    }
}

pub fn validate_profile_v2(profile: &LaunchProfileV2) -> ProfileValidationResult {
    let mut result = ProfileValidationResult::ok();

    if profile.name.trim().is_empty() {
        result.push_error(
            "EMPTY_PROFILE_NAME",
            "Profile name must not be empty".to_string(),
            None,
            Some("name".to_string()),
        );
    }

    if profile.steps.is_empty() {
        result.push_warning(
            "NO_STEPS",
            "Profile has no steps".to_string(),
            None,
            Some("steps".to_string()),
        );
        return result;
    }

    let mut seen_ids: HashSet<&str> = HashSet::new();
    let step_ids: HashSet<&str> = profile.steps.iter().map(|s| s.id.as_str()).collect();

    for step in &profile.steps {
        // Duplicate step IDs
        if !seen_ids.insert(&step.id) {
            result.push_error(
                "DUPLICATE_STEP_ID",
                format!("Duplicate step ID '{}'", step.id),
                Some(step.id.clone()),
                Some("id".to_string()),
            );
        }

        // Empty step ID
        if step.id.trim().is_empty() {
            result.push_error(
                "EMPTY_STEP_ID",
                "Step ID must not be empty".to_string(),
                None,
                Some("id".to_string()),
            );
        }

        // Self-dependency
        if step.depends_on.contains(&step.id) {
            result.push_error(
                "SELF_DEPENDENCY",
                format!("Step '{}' depends on itself", step.id),
                Some(step.id.clone()),
                Some("depends_on".to_string()),
            );
        }

        // Missing dependency IDs
        for dep_id in &step.depends_on {
            if !step_ids.contains(dep_id.as_str()) {
                result.push_error(
                    "MISSING_DEPENDENCY",
                    format!(
                        "Step '{}' depends on '{}' which does not exist",
                        step.id, dep_id
                    ),
                    Some(step.id.clone()),
                    Some("depends_on".to_string()),
                );
            }
        }

        // Validate step kind
        result.merge(validate_step_kind(step));

        // Validate timeout
        if let Some(timeout) = step.timeout {
            if timeout == 0 {
                result.push_error(
                    "INVALID_TIMEOUT",
                    "Timeout must be greater than 0".to_string(),
                    Some(step.id.clone()),
                    Some("timeout".to_string()),
                );
            }
        }

        // Validate working directory exists (only for absolute paths on current OS)
        if let Some(ref wd) = step.working_directory {
            let path = std::path::Path::new(wd);
            if path.is_absolute() && !path.exists() {
                result.push_warning(
                    "WORKING_DIR_NOT_FOUND",
                    format!("Working directory '{}' does not exist", wd),
                    Some(step.id.clone()),
                    Some("working_directory".to_string()),
                );
            }
        }
    }

    // Cycle detection via Kahn's algorithm
    if let Err(cycle) = detect_cycle(profile) {
        result.push_error(
            "DEPENDENCY_CYCLE",
            format!("Dependency cycle detected: {}", cycle),
            None,
            Some("depends_on".to_string()),
        );
    }

    result
}

fn validate_step_kind(step: &LaunchStep) -> ProfileValidationResult {
    let mut result = ProfileValidationResult::ok();

    match &step.kind {
        StepKind::RunCommand {
            command,
            command_spec,
        } => {
            if command.trim().is_empty() {
                result.push_error(
                    "EMPTY_COMMAND",
                    "RunCommand command must not be empty".to_string(),
                    Some(step.id.clone()),
                    Some("kind.command".to_string()),
                );
            }
            if let Some(spec) = command_spec {
                if spec.program.trim().is_empty() {
                    result.push_error(
                        "EMPTY_COMMAND_SPEC_PROGRAM",
                        "CommandSpec program must not be empty".to_string(),
                        Some(step.id.clone()),
                        Some("kind.command_spec.program".to_string()),
                    );
                }
            }
            // Validate shell if specified in command_spec
            if let Some(spec) = command_spec {
                if let Some(ref shell) = spec.shell {
                    if let Err(e) = super::models::validate_shell_for_os(shell) {
                        result.push_warning(
                            "UNSUPPORTED_SHELL",
                            e,
                            Some(step.id.clone()),
                            Some("kind.command_spec.shell".to_string()),
                        );
                    }
                }
            }
        }
        StepKind::RunScript { script, shell } => {
            if script.trim().is_empty() {
                result.push_error(
                    "EMPTY_SCRIPT",
                    "RunScript script must not be empty".to_string(),
                    Some(step.id.clone()),
                    Some("kind.script".to_string()),
                );
            }
            if let Some(ref shell) = shell {
                if let Err(e) = super::models::validate_shell_for_os(shell) {
                    result.push_warning(
                        "UNSUPPORTED_SHELL",
                        e,
                        Some(step.id.clone()),
                        Some("kind.shell".to_string()),
                    );
                }
            }
        }
        StepKind::OpenApplication { path, .. } => {
            if path.trim().is_empty() {
                result.push_error(
                    "EMPTY_APPLICATION_PATH",
                    "OpenApplication path must not be empty".to_string(),
                    Some(step.id.clone()),
                    Some("kind.path".to_string()),
                );
            }
        }
        StepKind::OpenUrl { url } => {
            if url.trim().is_empty() {
                result.push_error(
                    "EMPTY_URL",
                    "OpenUrl url must not be empty".to_string(),
                    Some(step.id.clone()),
                    Some("kind.url".to_string()),
                );
            }
        }
        StepKind::WaitForPort { host, port } => {
            if host.trim().is_empty() {
                result.push_error(
                    "EMPTY_HOST",
                    "WaitForPort host must not be empty".to_string(),
                    Some(step.id.clone()),
                    Some("kind.host".to_string()),
                );
            }
            if *port == 0 {
                result.push_error(
                    "INVALID_PORT",
                    "WaitForPort port must be greater than 0".to_string(),
                    Some(step.id.clone()),
                    Some("kind.port".to_string()),
                );
            }
        }
        StepKind::WaitForUrl { url } => {
            if url.trim().is_empty() {
                result.push_error(
                    "EMPTY_URL",
                    "WaitForUrl url must not be empty".to_string(),
                    Some(step.id.clone()),
                    Some("kind.url".to_string()),
                );
            }
        }
        StepKind::WaitForDocker {} => {
            // Timeout (if any) is validated separately on the step.
        }
        StepKind::Delay { seconds } => {
            if *seconds == 0 {
                result.push_warning(
                    "ZERO_DELAY",
                    "Delay of 0 seconds is a no-op".to_string(),
                    Some(step.id.clone()),
                    Some("kind.seconds".to_string()),
                );
            }
        }
        StepKind::OpenTerminal { command } => {
            if command.trim().is_empty() {
                result.push_warning(
                    "EMPTY_TERMINAL_COMMAND",
                    "OpenTerminal with an empty command opens a plain terminal".to_string(),
                    Some(step.id.clone()),
                    Some("kind.command".to_string()),
                );
            }
        }
        StepKind::OpenFolder { path } => {
            if path.trim().is_empty() {
                result.push_error(
                    "EMPTY_PATH",
                    "OpenFolder path must not be empty".to_string(),
                    Some(step.id.clone()),
                    Some("kind.path".to_string()),
                );
            }
        }
    }

    result
}

/// Detect dependency cycles using Kahn's algorithm (topological sort).
/// Returns Ok(topological_order) or Err(description_of_cycle).
fn detect_cycle(profile: &LaunchProfileV2) -> Result<Vec<String>, String> {
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for step in &profile.steps {
        in_degree.entry(&step.id).or_insert(0);
        for dep_id in &step.depends_on {
            dependents
                .entry(dep_id.as_str())
                .or_default()
                .push(&step.id);
            *in_degree.entry(&step.id).or_insert(0) += 1;
        }
    }

    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();
    queue.sort(); // deterministic ordering

    let mut sorted: Vec<String> = Vec::new();

    while let Some(id) = queue.pop() {
        sorted.push(id.to_string());
        if let Some(deps) = dependents.get(id) {
            let mut new_ready: Vec<&str> = Vec::new();
            for &dep_id in deps {
                let degree = in_degree.get_mut(dep_id).unwrap();
                *degree -= 1;
                if *degree == 0 {
                    new_ready.push(dep_id);
                }
            }
            new_ready.sort();
            queue.extend(new_ready);
        }
    }

    if sorted.len() == profile.steps.len() {
        Ok(sorted)
    } else {
        // Find the nodes still in the cycle
        let in_cycle: Vec<&str> = in_degree
            .iter()
            .filter(|(_, &deg)| deg > 0)
            .map(|(&id, _)| id)
            .collect();
        Err(format!(
            "Steps in cycle: {}",
            in_cycle
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

/// Compute topological order of enabled steps (for execution scheduling).
/// Skips disabled steps and their dependents.
pub fn topological_order(profile: &LaunchProfileV2) -> Vec<String> {
    let enabled_ids: HashSet<&str> = profile
        .steps
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.id.as_str())
        .collect();

    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for step in &profile.steps {
        if !enabled_ids.contains(step.id.as_str()) {
            continue;
        }
        in_degree.entry(&step.id).or_insert(0);
        for dep_id in &step.depends_on {
            if !enabled_ids.contains(dep_id.as_str()) {
                // Disabled dependency is considered satisfied
                continue;
            }
            dependents
                .entry(dep_id.as_str())
                .or_default()
                .push(&step.id);
            *in_degree.entry(&step.id).or_insert(0) += 1;
        }
    }

    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();
    queue.sort();

    let mut sorted: Vec<String> = Vec::new();

    while let Some(id) = queue.pop() {
        sorted.push(id.to_string());
        if let Some(deps) = dependents.get(id) {
            let mut new_ready: Vec<&str> = Vec::new();
            for &dep_id in deps {
                let degree = in_degree.get_mut(dep_id).unwrap();
                *degree -= 1;
                if *degree == 0 {
                    new_ready.push(dep_id);
                }
            }
            new_ready.sort();
            queue.extend(new_ready);
        }
    }

    sorted
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    fn make_step(id: &str, deps: Vec<&str>) -> LaunchStep {
        LaunchStep {
            id: id.to_string(),
            label: id.to_string(),
            enabled: true,
            kind: StepKind::RunCommand {
                command: "echo test".to_string(),
                command_spec: None,
            },
            depends_on: deps.into_iter().map(String::from).collect(),
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
        }
    }

    fn make_disabled_step(id: &str) -> LaunchStep {
        LaunchStep {
            id: id.to_string(),
            label: id.to_string(),
            enabled: false,
            kind: StepKind::RunCommand {
                command: "echo test".to_string(),
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
        }
    }

    fn valid_profile(steps: Vec<LaunchStep>) -> LaunchProfileV2 {
        LaunchProfileV2 {
            schema_version: "2".to_string(),
            id: "test".to_string(),
            name: "Test".to_string(),
            description: "desc".to_string(),
            project_root: None,
            steps,
            environment_binding_id: None,
            preferred_ide: None,
            default_execution_mode: None,
            extra: Map::new(),
        }
    }

    #[test]
    fn valid_graph_passes() {
        let profile = valid_profile(vec![
            make_step("a", vec![]),
            make_step("b", vec!["a"]),
            make_step("c", vec!["a"]),
            make_step("d", vec!["b", "c"]),
        ]);
        let result = validate_profile_v2(&profile);
        assert!(
            result.valid,
            "Expected valid, got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn duplicate_step_ids_detected() {
        let profile = valid_profile(vec![
            make_step("a", vec![]),
            LaunchStep {
                id: "a".to_string(),
                label: "a2".to_string(),
                enabled: true,
                kind: StepKind::RunCommand {
                    command: "echo".to_string(),
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
            },
        ]);
        let result = validate_profile_v2(&profile);
        assert!(!result.valid);
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == "DUPLICATE_STEP_ID"));
    }

    #[test]
    fn missing_dependency_detected() {
        let profile = valid_profile(vec![make_step("a", vec!["nonexistent"])]);
        let result = validate_profile_v2(&profile);
        assert!(!result.valid);
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == "MISSING_DEPENDENCY"));
    }

    #[test]
    fn self_dependency_detected() {
        let profile = valid_profile(vec![make_step("a", vec!["a"])]);
        let result = validate_profile_v2(&profile);
        assert!(!result.valid);
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == "SELF_DEPENDENCY"));
    }

    #[test]
    fn cycle_detected() {
        let profile = valid_profile(vec![
            make_step("a", vec!["c"]),
            make_step("b", vec!["a"]),
            make_step("c", vec!["b"]),
        ]);
        let result = validate_profile_v2(&profile);
        assert!(!result.valid);
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == "DEPENDENCY_CYCLE"));
    }

    #[test]
    fn disabled_step_skipped_in_topo_sort() {
        let profile = valid_profile(vec![
            make_step("a", vec![]),
            make_disabled_step("b"),
            make_step("c", vec!["b"]),
        ]);
        let order = topological_order(&profile);
        assert_eq!(order, vec!["a", "c"]);
    }

    #[test]
    fn parallel_independent_steps() {
        let profile = valid_profile(vec![
            make_step("a", vec![]),
            make_step("b", vec![]),
            make_step("c", vec![]),
        ]);
        let order = topological_order(&profile);
        assert_eq!(order.len(), 3);
        // All should be roots (in_degree 0) — any order is valid but deterministic
    }

    #[test]
    fn dependency_barrier() {
        let profile = valid_profile(vec![
            make_step("a", vec![]),
            make_step("b", vec!["a"]),
            make_step("c", vec!["a"]),
            make_step("d", vec!["b", "c"]),
        ]);
        let order = topological_order(&profile);
        let pos_a = order.iter().position(|x| x == "a").unwrap();
        let pos_b = order.iter().position(|x| x == "b").unwrap();
        let pos_c = order.iter().position(|x| x == "c").unwrap();
        let pos_d = order.iter().position(|x| x == "d").unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_a < pos_c);
        assert!(pos_b < pos_d);
        assert!(pos_c < pos_d);
    }

    #[test]
    fn empty_command_detected() {
        let profile = valid_profile(vec![LaunchStep {
            id: "s1".to_string(),
            label: "s1".to_string(),
            enabled: true,
            kind: StepKind::RunCommand {
                command: "   ".to_string(),
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
        }]);
        let result = validate_profile_v2(&profile);
        assert!(!result.valid);
        assert!(result.diagnostics.iter().any(|d| d.code == "EMPTY_COMMAND"));
    }

    #[test]
    fn invalid_port_detected() {
        let profile = valid_profile(vec![LaunchStep {
            id: "s1".to_string(),
            label: "s1".to_string(),
            enabled: true,
            kind: StepKind::WaitForPort {
                host: "localhost".to_string(),
                port: 0,
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
        }]);
        let result = validate_profile_v2(&profile);
        assert!(!result.valid);
        assert!(result.diagnostics.iter().any(|d| d.code == "INVALID_PORT"));
    }

    #[test]
    fn zero_timeout_detected() {
        let profile = valid_profile(vec![LaunchStep {
            id: "s1".to_string(),
            label: "s1".to_string(),
            enabled: true,
            kind: StepKind::RunCommand {
                command: "echo".to_string(),
                command_spec: None,
            },
            depends_on: vec![],
            working_directory: None,
            environment: None,
            visibility: None,
            execution_mode: None,
            completion: None,
            timeout: Some(0),
            failure_policy: None,
            retry_policy: None,
            metadata: None,
            extra: Map::new(),
        }]);
        let result = validate_profile_v2(&profile);
        assert!(!result.valid);
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == "INVALID_TIMEOUT"));
    }

    #[test]
    fn empty_step_id_detected() {
        let profile = valid_profile(vec![LaunchStep {
            id: "   ".to_string(),
            label: "s1".to_string(),
            enabled: true,
            kind: StepKind::RunCommand {
                command: "echo".to_string(),
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
        }]);
        let result = validate_profile_v2(&profile);
        assert!(!result.valid);
        assert!(result.diagnostics.iter().any(|d| d.code == "EMPTY_STEP_ID"));
    }

    #[test]
    fn empty_script_detected() {
        let profile = valid_profile(vec![LaunchStep {
            id: "s1".to_string(),
            label: "s1".to_string(),
            enabled: true,
            kind: StepKind::RunScript {
                script: "  ".to_string(),
                shell: None,
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
        }]);
        let result = validate_profile_v2(&profile);
        assert!(!result.valid);
        assert!(result.diagnostics.iter().any(|d| d.code == "EMPTY_SCRIPT"));
    }

    #[test]
    fn empty_application_path_detected() {
        let profile = valid_profile(vec![LaunchStep {
            id: "s1".to_string(),
            label: "s1".to_string(),
            enabled: true,
            kind: StepKind::OpenApplication {
                path: "".to_string(),
                args: None,
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
        }]);
        let result = validate_profile_v2(&profile);
        assert!(!result.valid);
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == "EMPTY_APPLICATION_PATH"));
    }

    #[test]
    fn empty_url_detected() {
        let profile = valid_profile(vec![LaunchStep {
            id: "s1".to_string(),
            label: "s1".to_string(),
            enabled: true,
            kind: StepKind::OpenUrl {
                url: "  ".to_string(),
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
        }]);
        let result = validate_profile_v2(&profile);
        assert!(!result.valid);
        assert!(result.diagnostics.iter().any(|d| d.code == "EMPTY_URL"));
    }

    #[test]
    fn empty_wait_for_url_detected() {
        let profile = valid_profile(vec![LaunchStep {
            id: "s1".to_string(),
            label: "s1".to_string(),
            enabled: true,
            kind: StepKind::WaitForUrl {
                url: "".to_string(),
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
        }]);
        let result = validate_profile_v2(&profile);
        assert!(!result.valid);
        assert!(result.diagnostics.iter().any(|d| d.code == "EMPTY_URL"));
    }

    #[test]
    fn zero_delay_warning() {
        let profile = valid_profile(vec![LaunchStep {
            id: "s1".to_string(),
            label: "s1".to_string(),
            enabled: true,
            kind: StepKind::Delay { seconds: 0 },
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
        }]);
        let result = validate_profile_v2(&profile);
        assert!(result.valid);
        assert!(result.diagnostics.iter().any(|d| d.code == "ZERO_DELAY"));
    }

    #[test]
    fn empty_wait_for_port_host_detected() {
        let profile = valid_profile(vec![LaunchStep {
            id: "s1".to_string(),
            label: "s1".to_string(),
            enabled: true,
            kind: StepKind::WaitForPort {
                host: "  ".to_string(),
                port: 3000,
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
        }]);
        let result = validate_profile_v2(&profile);
        assert!(!result.valid);
        assert!(result.diagnostics.iter().any(|d| d.code == "EMPTY_HOST"));
    }

    #[test]
    fn empty_terminal_command_detected() {
        let profile = valid_profile(vec![LaunchStep {
            id: "s1".to_string(),
            label: "s1".to_string(),
            enabled: true,
            kind: StepKind::OpenTerminal {
                command: "".to_string(),
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
        }]);
        let result = validate_profile_v2(&profile);
        // Empty command is a plain terminal: valid, but warned.
        assert!(result.valid);
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == "EMPTY_TERMINAL_COMMAND"));
    }

    #[test]
    fn empty_folder_path_detected() {
        let profile = valid_profile(vec![LaunchStep {
            id: "s1".to_string(),
            label: "s1".to_string(),
            enabled: true,
            kind: StepKind::OpenFolder {
                path: "  ".to_string(),
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
        }]);
        let result = validate_profile_v2(&profile);
        assert!(!result.valid);
        assert!(result.diagnostics.iter().any(|d| d.code == "EMPTY_PATH"));
    }

    #[test]
    fn empty_command_spec_program_detected() {
        let profile = valid_profile(vec![LaunchStep {
            id: "s1".to_string(),
            label: "s1".to_string(),
            enabled: true,
            kind: StepKind::RunCommand {
                command: "echo".to_string(),
                command_spec: Some(CommandSpec {
                    program: "".to_string(),
                    args: vec![],
                    shell: None,
                    script: None,
                    env: None,
                    cwd: None,
                }),
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
        }]);
        let result = validate_profile_v2(&profile);
        assert!(!result.valid);
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == "EMPTY_COMMAND_SPEC_PROGRAM"));
    }
}
