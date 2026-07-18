use crate::modules::workspace::models::{ProcessStatus, TrackedProcess};

#[derive(Debug, Clone)]
pub struct Problem {
    pub process_id: String,
    pub process_label: String,
    pub status: String,
    pub error_message: Option<String>,
    pub severity: ProblemSeverity,
}

#[derive(Debug, Clone, PartialEq)]
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

pub struct DefaultProblemsService;

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
                problems.push(Problem {
                    process_id: p.id.clone(),
                    process_label: p.label.clone(),
                    status: format!("{:?}", p.status),
                    error_message: p.last_error.clone(),
                    severity: ProblemSeverity::Error,
                });
            }
        }
        problems
    }

    fn add_problem(&self, _problem: Problem) {
        // TODO: store in-memory problem list for external problem sources
    }

    fn get_all(&self) -> Vec<Problem> {
        Vec::new()
    }

    fn clear(&self) {
        // TODO: clear stored problems
    }
}

impl DefaultProblemsService {
    pub fn new() -> Self {
        Self
    }
}
