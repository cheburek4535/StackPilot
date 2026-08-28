use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectContext {
    pub profile_name: String,
    pub project_path: Option<String>,
    pub description: String,
    pub stack: Vec<String>,
    pub opened_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub started_at: String,
    pub duration_secs: u64,
    pub process_count: usize,
    pub error_count: usize,
}

// ---------------------------------------------------------------------------
// ProcessStatus — expanded to match contract requirements
// ---------------------------------------------------------------------------

/// Lifecycle status of a managed process.
///
/// The states distinguish between different ways a process can terminate,
/// preserving exit codes where available, and distinguishing user-initiated
/// cancellation from crashes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    /// Process is being spawned (fork/exec not yet complete).
    Starting,
    /// Process is running.
    Running,
    /// Process is running and has passed a readiness check (port open, etc.).
    Ready,
    /// Process exited with code 0.
    Exited(i32),
    /// Process exited with a non-zero exit code. The i32 is the exit code.
    ExitedWithError(i32),
    /// Process terminated abnormally (signal, segfault, etc.).
    Crashed,
    /// Process was forcibly terminated by the supervisor (SIGKILL / taskkill /F).
    Killed,
    /// Process exceeded its timeout and was killed.
    TimedOut,
    /// Process was cancelled by user action (SIGTERM then SIGKILL after grace).
    Cancelled,
    /// Process was launched in detached/external mode; no direct tracking.
    ExternalLaunchAccepted,
    /// Process status is unknown (could not be determined).
    Unknown,
}

impl ProcessStatus {
    /// Returns true if the process is in a terminal (non-running) state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ProcessStatus::Exited(_)
                | ProcessStatus::ExitedWithError(_)
                | ProcessStatus::Crashed
                | ProcessStatus::Killed
                | ProcessStatus::TimedOut
                | ProcessStatus::Cancelled
                | ProcessStatus::ExternalLaunchAccepted
                | ProcessStatus::Unknown
        )
    }

    /// Returns true if the process terminated due to an error condition.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            ProcessStatus::ExitedWithError(_)
                | ProcessStatus::Crashed
                | ProcessStatus::Killed
                | ProcessStatus::TimedOut
        )
    }

    /// Extract the exit code if available.
    pub fn exit_code(&self) -> Option<i32> {
        match self {
            ProcessStatus::Exited(code) => Some(*code),
            ProcessStatus::ExitedWithError(code) => Some(*code),
            _ => None,
        }
    }

    /// Returns a human-readable label for the status.
    pub fn label(&self) -> &'static str {
        match self {
            ProcessStatus::Starting => "Starting",
            ProcessStatus::Running => "Running",
            ProcessStatus::Ready => "Ready",
            ProcessStatus::Exited(0) => "Exited Successfully",
            ProcessStatus::Exited(_) => "Exited",
            ProcessStatus::ExitedWithError(_) => "Exited With Error",
            ProcessStatus::Crashed => "Crashed",
            ProcessStatus::Killed => "Killed",
            ProcessStatus::TimedOut => "Timed Out",
            ProcessStatus::Cancelled => "Cancelled",
            ProcessStatus::ExternalLaunchAccepted => "External Launch",
            ProcessStatus::Unknown => "Unknown",
        }
    }
}

// ---------------------------------------------------------------------------
// ProcessTrackingQuality — how accurately the tracked PID represents the
// real command process (contract §A.6)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessTrackingQuality {
    /// PID points to the actual command process.
    Exact,
    /// PID points to a terminal wrapper (cmd.exe, osascript, xterm).
    TerminalWrapper,
    /// PID was the process but it has since been reaped / replaced.
    Approximate,
    /// Process was launched detached; no PID tracking.
    Detached,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedProcess {
    pub id: String,
    pub pid: u32,
    pub label: String,
    pub status: ProcessStatus,
    pub started_at: String,
    pub duration_secs: u64,
    pub restarts: u32,
    pub last_error: Option<String>,
    pub session_id: Option<String>,
    /// True if the process was spawned in its own native terminal window.
    #[serde(default)]
    pub visible: bool,
    /// The orchestrator run this process belongs to (V2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// The orchestrator step that spawned this process (V2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    /// The full command line this process was launched with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The working directory the process was launched in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// How accurately the tracked PID represents the real command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking_quality: Option<ProcessTrackingQuality>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessLogs {
    pub stdout_lines: Vec<String>,
    pub stderr_lines: Vec<String>,
}

// ---------------------------------------------------------------------------
// ProcessOutputEvent — enhanced with run/step ownership and sequencing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessOutputEvent {
    pub process_id: String,
    pub stream: String,
    pub line: String,
    /// Sequence number within this process's output stream.
    #[serde(default)]
    pub sequence: Option<u64>,
    /// Timestamp in ISO 8601 format if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// The run this process belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// The step within the run that spawned this process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessStatusEvent {
    pub process_id: String,
    pub status: ProcessStatus,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Bounded log storage metadata
// ---------------------------------------------------------------------------

/// Metadata about log truncation applied to a set of log lines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogTruncation {
    /// Total number of lines ever written.
    pub total_lines: u64,
    /// Total bytes ever written.
    pub total_bytes: u64,
    /// Number of lines currently retained (after truncation).
    pub retained_lines: usize,
    /// Number of lines dropped from the beginning.
    pub dropped_lines: u64,
    /// Whether truncation was applied.
    pub truncated: bool,
}

/// Bounded log buffer that retains recent lines and discards old ones.
///
/// Configurable maximum lines and bytes. Avoids unbounded memory growth.
/// Retains stderr separately. Never blocks indefinitely on locks.
pub struct BoundedLogBuffer {
    stdout: std::sync::Mutex<Vec<String>>,
    stderr: std::sync::Mutex<Vec<String>>,
    stdout_meta: std::sync::Mutex<LogTruncation>,
    stderr_meta: std::sync::Mutex<LogTruncation>,
    max_lines: usize,
    max_bytes: usize,
}

impl BoundedLogBuffer {
    /// Create a new bounded buffer with the given limits.
    /// `max_lines`: maximum lines to retain per stream (0 = unlimited).
    /// `max_bytes`: maximum total bytes to retain per stream (0 = unlimited).
    pub fn new(max_lines: usize, max_bytes: usize) -> Self {
        Self {
            stdout: std::sync::Mutex::new(Vec::new()),
            stderr: std::sync::Mutex::new(Vec::new()),
            stdout_meta: std::sync::Mutex::new(LogTruncation {
                total_lines: 0,
                total_bytes: 0,
                retained_lines: 0,
                dropped_lines: 0,
                truncated: false,
            }),
            stderr_meta: std::sync::Mutex::new(LogTruncation {
                total_lines: 0,
                total_bytes: 0,
                retained_lines: 0,
                dropped_lines: 0,
                truncated: false,
            }),
            max_lines,
            max_bytes,
        }
    }

    /// Push a line to stdout. Returns immediately; does not block.
    pub fn push_stdout(&self, line: String) {
        self.push_line(&self.stdout, &self.stdout_meta, line);
    }

    /// Push a line to stderr. Returns immediately; does not block.
    pub fn push_stderr(&self, line: String) {
        self.push_line(&self.stderr, &self.stderr_meta, line);
    }

    fn push_line(
        &self,
        buffer: &std::sync::Mutex<Vec<String>>,
        meta: &std::sync::Mutex<LogTruncation>,
        line: String,
    ) {
        let line_bytes = line.len() as u64;

        if let Ok(mut buf) = buffer.try_lock() {
            if let Ok(mut m) = meta.try_lock() {
                buf.push(line);
                m.total_lines += 1;
                m.total_bytes += line_bytes;
                m.retained_lines = buf.len();

                // Enforce line limit
                if self.max_lines > 0 && buf.len() > self.max_lines {
                    let excess = buf.len() - self.max_lines;
                    buf.drain(..excess);
                    m.dropped_lines += excess as u64;
                    m.retained_lines = buf.len();
                    m.truncated = true;
                }

                // Enforce byte limit
                if self.max_bytes > 0 {
                    let total: usize = buf.iter().map(|l| l.len()).sum();
                    if total > self.max_bytes {
                        let mut removed = 0;
                        let mut bytes_removed = 0usize;
                        for (i, line) in buf.iter().enumerate() {
                            bytes_removed += line.len() + 1; // +1 for newline
                            removed = i + 1;
                            if total - bytes_removed <= self.max_bytes {
                                break;
                            }
                        }
                        if removed > 0 {
                            buf.drain(..removed);
                            m.dropped_lines += removed as u64;
                            m.retained_lines = buf.len();
                            m.truncated = true;
                        }
                    }
                }
            }
        }
    }

    /// Get stdout lines (snapshot).
    pub fn stdout_lines(&self) -> Vec<String> {
        self.stdout
            .try_lock()
            .map(|buf| buf.clone())
            .unwrap_or_default()
    }

    /// Get stderr lines (snapshot).
    pub fn stderr_lines(&self) -> Vec<String> {
        self.stderr
            .try_lock()
            .map(|buf| buf.clone())
            .unwrap_or_default()
    }

    /// Get truncation metadata for stdout.
    pub fn stdout_truncation(&self) -> LogTruncation {
        self.stdout_meta
            .try_lock()
            .map(|m| m.clone())
            .unwrap_or(LogTruncation {
                total_lines: 0,
                total_bytes: 0,
                retained_lines: 0,
                dropped_lines: 0,
                truncated: false,
            })
    }

    /// Get truncation metadata for stderr.
    pub fn stderr_truncation(&self) -> LogTruncation {
        self.stderr_meta
            .try_lock()
            .map(|m| m.clone())
            .unwrap_or(LogTruncation {
                total_lines: 0,
                total_bytes: 0,
                retained_lines: 0,
                dropped_lines: 0,
                truncated: false,
            })
    }

    /// Clear all log lines.
    pub fn clear(&self) {
        if let Ok(mut buf) = self.stdout.try_lock() {
            buf.clear();
        }
        if let Ok(mut buf) = self.stderr.try_lock() {
            buf.clear();
        }
        if let Ok(mut m) = self.stdout_meta.try_lock() {
            m.retained_lines = 0;
        }
        if let Ok(mut m) = self.stderr_meta.try_lock() {
            m.retained_lines = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_status_is_terminal() {
        assert!(ProcessStatus::Exited(0).is_terminal());
        assert!(ProcessStatus::ExitedWithError(1).is_terminal());
        assert!(ProcessStatus::Crashed.is_terminal());
        assert!(ProcessStatus::Killed.is_terminal());
        assert!(ProcessStatus::TimedOut.is_terminal());
        assert!(ProcessStatus::Cancelled.is_terminal());
        assert!(!ProcessStatus::Running.is_terminal());
        assert!(!ProcessStatus::Starting.is_terminal());
        assert!(!ProcessStatus::Ready.is_terminal());
    }

    #[test]
    fn process_status_is_error() {
        assert!(ProcessStatus::ExitedWithError(1).is_error());
        assert!(ProcessStatus::Crashed.is_error());
        assert!(ProcessStatus::Killed.is_error());
        assert!(ProcessStatus::TimedOut.is_error());
        assert!(!ProcessStatus::Exited(0).is_error());
        assert!(!ProcessStatus::Exited(1).is_error());
        assert!(!ProcessStatus::Running.is_error());
    }

    #[test]
    fn process_status_exit_code() {
        assert_eq!(ProcessStatus::Exited(0).exit_code(), Some(0));
        assert_eq!(ProcessStatus::ExitedWithError(1).exit_code(), Some(1));
        assert_eq!(ProcessStatus::Crashed.exit_code(), None);
    }

    #[test]
    fn process_status_label() {
        assert_eq!(ProcessStatus::Starting.label(), "Starting");
        assert_eq!(ProcessStatus::Running.label(), "Running");
        assert_eq!(ProcessStatus::Exited(0).label(), "Exited Successfully");
        assert_eq!(ProcessStatus::Exited(1).label(), "Exited");
        assert_eq!(
            ProcessStatus::ExitedWithError(1).label(),
            "Exited With Error"
        );
    }

    #[test]
    fn bounded_log_buffer_line_limit() {
        let buf = BoundedLogBuffer::new(3, 0);
        buf.push_stdout("line1".into());
        buf.push_stdout("line2".into());
        buf.push_stdout("line3".into());
        buf.push_stdout("line4".into());
        buf.push_stdout("line5".into());
        let lines = buf.stdout_lines();
        assert_eq!(lines, vec!["line3", "line4", "line5"]);
        let meta = buf.stdout_truncation();
        assert!(meta.truncated);
        assert_eq!(meta.dropped_lines, 2);
        assert_eq!(meta.total_lines, 5);
    }

    #[test]
    fn bounded_log_buffer_byte_limit() {
        let buf = BoundedLogBuffer::new(0, 20);
        buf.push_stdout("short".into()); // 5 bytes
        buf.push_stdout("another line".into()); // 11 bytes
        buf.push_stdout("x".into()); // 1 byte
        buf.push_stdout("final long line here".into()); // 19 bytes
        let meta = buf.stdout_truncation();
        // Should have dropped some lines to stay under 20 bytes
        assert!(meta.truncated);
        let lines = buf.stdout_lines();
        let total: usize = lines.iter().map(|l| l.len()).sum();
        assert!(total <= 20, "total bytes {} should be <= 20", total);
    }

    #[test]
    fn bounded_log_buffer_stderr_separate() {
        let buf = BoundedLogBuffer::new(2, 0);
        buf.push_stdout("out1".into());
        buf.push_stderr("err1".into());
        buf.push_stdout("out2".into());
        buf.push_stderr("err2".into());
        buf.push_stdout("out3".into());
        buf.push_stderr("err3".into());
        assert_eq!(buf.stdout_lines(), vec!["out2", "out3"]);
        assert_eq!(buf.stderr_lines(), vec!["err2", "err3"]);
    }

    #[test]
    fn bounded_log_buffer_clear() {
        let buf = BoundedLogBuffer::new(10, 0);
        buf.push_stdout("a".into());
        buf.push_stderr("b".into());
        buf.clear();
        assert!(buf.stdout_lines().is_empty());
        assert!(buf.stderr_lines().is_empty());
    }
}
