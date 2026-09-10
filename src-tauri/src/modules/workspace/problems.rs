use crate::modules::workspace::models::{ProcessStatus, TrackedProcess};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Problem {
    pub process_id: String,
    pub process_label: String,
    pub status: String,
    pub error_message: Option<String>,
    pub severity: ProblemSeverity,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ProblemSeverity {
    Error,
    Warning,
    Info,
}

/// Service that aggregates problems from various sources:
/// - Crashed/failed processes
/// - Missing dependencies
/// - Configuration issues
/// - Any module can push problems here
pub trait ProblemsService: Send + Sync {
    fn collect_from_processes(&self, processes: &[TrackedProcess]) -> Vec<Problem>;
#[allow(dead_code)]
    fn add_problem(&self, problem: Problem);
    fn get_all(&self) -> Vec<Problem>;
    fn clear(&self);
}

pub struct DefaultProblemsService {
    storage: Arc<Mutex<Vec<Problem>>>,
}

impl ProblemsService for DefaultProblemsService {
    fn collect_from_processes(&self, processes: &[TrackedProcess]) -> Vec<Problem> {
        let mut problems = Vec::new();
        for p in processes {
            let is_error = match &p.status {
                ProcessStatus::Crashed => true,
                ProcessStatus::Exited(code) if *code != 0 => true,
                ProcessStatus::ExitedWithError(_) => true,
                ProcessStatus::Killed => true,
                ProcessStatus::TimedOut => true,
                _ => false,
            };
            if is_error {
                let status_str = format!("{:?}", p.status);
                let problem = Problem {
                    process_id: p.id.clone(),
                    process_label: p.label.clone(),
                    status: status_str.clone(),
                    error_message: p.last_error.clone(),
                    severity: ProblemSeverity::Error,
                };

                // Deduplicate: skip if a problem with the same process_id and status already exists
                let mut storage = self.storage.lock().expect("storage lock poisoned");
                let already_exists = storage.iter().any(|existing| {
                    existing.process_id == problem.process_id && existing.status == problem.status
                });
                if !already_exists {
                    problems.push(problem.clone());
                    storage.push(problem);
                }
            }
        }
        problems
    }

    fn add_problem(&self, problem: Problem) {
        let mut storage = self.storage.lock().expect("storage lock poisoned");
        // Deduplicate by process_id + status
        let already_exists = storage.iter().any(|existing| {
            existing.process_id == problem.process_id && existing.status == problem.status
        });
        if !already_exists {
            storage.push(problem);
        }
    }

    fn get_all(&self) -> Vec<Problem> {
        self.storage
            .lock()
            .expect("storage lock poisoned")
            .iter()
            .cloned()
            .collect()
    }

    fn clear(&self) {
        self.storage.lock().expect("storage lock poisoned").clear();
    }
}

impl DefaultProblemsService {
    pub fn new() -> Self {
        Self {
            storage: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::workspace::models::TrackedProcess;

    fn make_running_process(id: &str) -> TrackedProcess {
        TrackedProcess {
            id: id.to_string(),
            pid: 1000,
            label: format!("proc-{}", id),
            status: ProcessStatus::Running,
            started_at: "2026-01-01T00:00:00Z".to_string(),
            duration_secs: 0,
            restarts: 0,
            last_error: None,
            session_id: None,
            visible: false,
            run_id: None,
            step_id: None,
            command: None,
            working_dir: None,
            tracking_quality: None,
        }
    }

    fn make_crashed_process(id: &str) -> TrackedProcess {
        TrackedProcess {
            id: id.to_string(),
            pid: 1000,
            label: format!("proc-{}", id),
            status: ProcessStatus::Crashed,
            started_at: "2026-01-01T00:00:00Z".to_string(),
            duration_secs: 0,
            restarts: 0,
            last_error: Some("segfault".to_string()),
            session_id: None,
            visible: false,
            run_id: None,
            step_id: None,
            command: None,
            working_dir: None,
            tracking_quality: None,
        }
    }

    fn make_error_process(id: &str, code: i32) -> TrackedProcess {
        TrackedProcess {
            id: id.to_string(),
            pid: 1000,
            label: format!("proc-{}", id),
            status: ProcessStatus::ExitedWithError(code),
            started_at: "2026-01-01T00:00:00Z".to_string(),
            duration_secs: 0,
            restarts: 0,
            last_error: Some(format!("exit code {}", code)),
            session_id: None,
            visible: false,
            run_id: None,
            step_id: None,
            command: None,
            working_dir: None,
            tracking_quality: None,
        }
    }

    #[test]
    fn test_collect_from_processes_no_duplicates_on_repeated_call() {
        let service = DefaultProblemsService::new();
        let processes = vec![make_crashed_process("p1")];

        // First call вЂ” should create 1 problem
        let problems = service.collect_from_processes(&processes);
        assert_eq!(problems.len(), 1);

        // Second call with same process вЂ” should NOT create duplicates
        let problems = service.collect_from_processes(&processes);
        assert_eq!(problems.len(), 0);

        // Storage should have exactly 1 problem
        assert_eq!(service.get_all().len(), 1);
    }

    #[test]
    fn test_different_processes_are_not_deduplicated() {
        let service = DefaultProblemsService::new();
        let processes = vec![make_crashed_process("p1"), make_crashed_process("p2")];

        let problems = service.collect_from_processes(&processes);
        assert_eq!(problems.len(), 2);
        assert_eq!(service.get_all().len(), 2);
    }

    #[test]
    fn test_same_process_different_status_not_deduplicated() {
        let service = DefaultProblemsService::new();

        // Create a process that changes status
        let mut proc1 = make_crashed_process("p1");
        let problems1 = service.collect_from_processes(&[proc1.clone()]);
        assert_eq!(problems1.len(), 1);

        // Same process but different status (e.g. restarted then crashed again with different status)
        proc1.status = ProcessStatus::ExitedWithError(1);
        proc1.last_error = Some("exit code 1".to_string());
        let problems2 = service.collect_from_processes(&[proc1]);
        // Different status вЂ” should create a new problem
        assert_eq!(problems2.len(), 1);
        assert_eq!(service.get_all().len(), 2);
    }

    #[test]
    fn test_add_problem_deduplication() {
        let service = DefaultProblemsService::new();
        let problem = Problem {
            process_id: "p1".to_string(),
            process_label: "test".to_string(),
            status: "Crashed".to_string(),
            error_message: Some("segfault".to_string()),
            severity: ProblemSeverity::Error,
        };

        service.add_problem(problem.clone());
        assert_eq!(service.get_all().len(), 1);

        // Add same problem again вЂ” should be deduplicated
        service.add_problem(problem);
        assert_eq!(service.get_all().len(), 1);
    }

    #[test]
    fn test_clear_removes_all_problems() {
        let service = DefaultProblemsService::new();
        let processes = vec![make_crashed_process("p1"), make_error_process("p2", 1)];
        service.collect_from_processes(&processes);
        assert_eq!(service.get_all().len(), 2);

        service.clear();
        assert!(service.get_all().is_empty());
    }

    #[test]
    fn test_running_processes_not_added_as_problems() {
        let service = DefaultProblemsService::new();
        let processes = vec![make_running_process("p1")];

        let problems = service.collect_from_processes(&processes);
        assert!(problems.is_empty());
        assert!(service.get_all().is_empty());
    }
}
