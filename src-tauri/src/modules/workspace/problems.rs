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
                _ => false,
            };
            if is_error {
                let problem = Problem {
                    process_id: p.id.clone(),
                    process_label: p.label.clone(),
                    status: format!("{:?}", p.status),
                    error_message: p.last_error.clone(),
                    severity: ProblemSeverity::Error,
                };
                problems.push(problem.clone());
                self.storage.lock().expect("storage lock poisoned").push(problem);
            }
        }
        problems
    }

    fn add_problem(&self, problem: Problem) {
        self.storage.lock().expect("storage lock poisoned").push(problem);
    }

    fn get_all(&self) -> Vec<Problem> {
        self.storage.lock().expect("storage lock poisoned").iter().cloned().collect()
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
