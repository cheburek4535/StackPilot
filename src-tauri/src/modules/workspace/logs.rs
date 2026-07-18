use crate::modules::workspace::models::ProcessLogs;

/// Service for aggregating and filtering process logs.
/// Wraps ProcessManager's get_logs and can add search/filter capabilities.
pub trait LogsService: Send + Sync {
    fn filter_logs(&self, logs: &ProcessLogs, query: &str, stream: Option<&str>) -> ProcessLogs;
    fn merge_logs(&self, all_logs: Vec<(&str, ProcessLogs)>) -> MergedLog;
}

#[derive(Debug, Clone)]
pub struct MergedLog {
    pub entries: Vec<LogEntry>,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub process_id: String,
    pub stream: String,
    pub line: String,
}

pub struct DefaultLogsService;

impl LogsService for DefaultLogsService {
    fn filter_logs(&self, logs: &ProcessLogs, query: &str, stream: Option<&str>) -> ProcessLogs {
        let filter_stdout = stream.map_or(true, |s| s == "stdout");
        let filter_stderr = stream.map_or(true, |s| s == "stderr");

        let stdout: Vec<String> = if filter_stdout && !query.is_empty() {
            logs.stdout_lines.iter().filter(|l| l.contains(query)).cloned().collect()
        } else if filter_stdout {
            logs.stdout_lines.clone()
        } else {
            Vec::new()
        };

        let stderr: Vec<String> = if filter_stderr && !query.is_empty() {
            logs.stderr_lines.iter().filter(|l| l.contains(query)).cloned().collect()
        } else if filter_stderr {
            logs.stderr_lines.clone()
        } else {
            Vec::new()
        };

        ProcessLogs { stdout_lines: stdout, stderr_lines: stderr }
    }

    fn merge_logs(&self, all_logs: Vec<(&str, ProcessLogs)>) -> MergedLog {
        let mut entries = Vec::new();
        for (pid, logs) in all_logs {
            for line in &logs.stdout_lines {
                entries.push(LogEntry {
                    process_id: pid.to_string(),
                    stream: "stdout".into(),
                    line: line.clone(),
                });
            }
            for line in &logs.stderr_lines {
                entries.push(LogEntry {
                    process_id: pid.to_string(),
                    stream: "stderr".into(),
                    line: line.clone(),
                });
            }
        }
        MergedLog { entries }
    }
}

impl DefaultLogsService {
    pub fn new() -> Self {
        Self
    }
}
