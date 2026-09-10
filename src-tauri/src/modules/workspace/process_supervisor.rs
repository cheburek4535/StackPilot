use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;
use tokio::sync::Notify;

use super::models::*;
use crate::modules::devlauncher::models::ProcessTrackingQuality;
use crate::platform::host::{current_os, HostOs};

// ---------------------------------------------------------------------------
// ProcessHandle — opaque handle for an active process tracked by the supervisor
// ---------------------------------------------------------------------------

/// Opaque handle representing a process tracked by the supervisor.
///
/// Contains the OS PID, tracking quality, ownership info, and a reference
/// to the bounded log buffer. The handle is cloneable and thread-safe.
#[derive(Clone)]
#[allow(dead_code)]
pub struct ProcessHandle {
    pub id: String,
    pub pid: u32,
    pub run_id: Option<String>,
    pub step_id: Option<String>,
    pub label: String,
    pub tracking_quality: ProcessTrackingQuality,
    pub log_buffer: Arc<BoundedLogBuffer>,
    pub started_at: Instant,
    pub stdout_seq: Arc<AtomicU64>,
    pub stderr_seq: Arc<AtomicU64>,
}

impl ProcessHandle {
    pub fn elapsed_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    #[allow(dead_code)]
    pub fn next_stdout_seq(&self) -> u64 {
        self.stdout_seq.fetch_add(1, Ordering::Relaxed)
    }

    #[allow(dead_code)]
    pub fn next_stderr_seq(&self) -> u64 {
        self.stderr_seq.fetch_add(1, Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------
// TrackedProcessEntry — internal entry with child handle
// ---------------------------------------------------------------------------

struct TrackedProcessEntry {
    handle: ProcessHandle,
    child: Option<std::process::Child>,
    status: ProcessStatus,
    last_check: Instant,
    terminated: bool,
}

// ---------------------------------------------------------------------------
// SupervisorEvent — emitted when a process terminates
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ProcessTerminatedEvent {
    pub process_id: String,
    pub run_id: Option<String>,
    pub step_id: Option<String>,
    pub status: ProcessStatus,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub stderr_tail: String,
    pub duration_secs: u64,
}

// ---------------------------------------------------------------------------
// ProcessSupervisor — background monitor for child processes
// ---------------------------------------------------------------------------

pub struct ProcessSupervisor {
    processes: Arc<RwLock<HashMap<String, TrackedProcessEntry>>>,
    app_handle: Arc<RwLock<Option<tauri::AppHandle>>>,
    shutdown: Arc<AtomicBool>,
    notify: Arc<Notify>,
    poll_interval_ms: u64,
}

impl ProcessSupervisor {
    /// Create a new supervisor. Does not start the background monitor yet.
    pub fn new() -> Self {
        Self {
            processes: Arc::new(RwLock::new(HashMap::new())),
            app_handle: Arc::new(RwLock::new(None)),
            shutdown: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
            poll_interval_ms: 500,
        }
    }

    /// Set the Tauri AppHandle for event emission.
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        if let Ok(mut h) = self.app_handle.write() {
            *h = Some(handle);
        }
    }

    /// Start the background supervisor task. Returns immediately.
    /// The task runs until `shutdown()` is called.
    pub fn start(self: &Arc<Self>) {
        let supervisor = self.clone();
        tauri::async_runtime::spawn(async move {
            supervisor.monitor_loop().await;
        });
    }

    /// Signal the supervisor to shut down cleanly.
    #[allow(dead_code)]
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Register a new process with the supervisor.
    ///
    /// Returns a `ProcessHandle` that can be used to emit output events,
    /// check status, and retrieve logs.
    #[allow(dead_code)]
    pub fn register_process(
        &self,
        id: String,
        pid: u32,
        child: Option<std::process::Child>,
        run_id: Option<String>,
        step_id: Option<String>,
        label: String,
        tracking_quality: ProcessTrackingQuality,
        log_buffer: Arc<BoundedLogBuffer>,
    ) -> ProcessHandle {
        let handle = ProcessHandle {
            id: id.clone(),
            pid,
            run_id: run_id.clone(),
            step_id: step_id.clone(),
            label,
            tracking_quality,
            log_buffer,
            started_at: Instant::now(),
            stdout_seq: Arc::new(AtomicU64::new(0)),
            stderr_seq: Arc::new(AtomicU64::new(0)),
        };

        let entry = TrackedProcessEntry {
            handle: handle.clone(),
            child,
            status: ProcessStatus::Running,
            last_check: Instant::now(),
            terminated: false,
        };

        if let Ok(mut procs) = self.processes.write() {
            procs.insert(id, entry);
        }

        handle
    }

    /// Emit a process-output event for a specific process.
    #[allow(dead_code)]
    pub fn emit_output(&self, process_id: &str, stream: &str, line: &str) {
        let (run_id, step_id, seq) = if let Ok(procs) = self.processes.read() {
            if let Some(entry) = procs.get(process_id) {
                let seq = if stream == "stdout" {
                    entry.handle.next_stdout_seq()
                } else {
                    entry.handle.next_stderr_seq()
                };
                (
                    entry.handle.run_id.clone(),
                    entry.handle.step_id.clone(),
                    seq,
                )
            } else {
                (None, None, 0)
            }
        } else {
            (None, None, 0)
        };

        let ts = default_now_iso();

        // Push to bounded buffer
        if let Ok(procs) = self.processes.read() {
            if let Some(entry) = procs.get(process_id) {
                if stream == "stderr" {
                    entry.handle.log_buffer.push_stderr(line.to_string());
                } else {
                    entry.handle.log_buffer.push_stdout(line.to_string());
                }
            }
        }

        // Emit Tauri event
        if let Ok(handle_guard) = self.app_handle.read() {
            if let Some(h) = handle_guard.as_ref() {
                let _ = h.emit(
                    crate::modules::workspace::process_manager::PROCESS_EVENT_OUTPUT,
                    ProcessOutputEvent {
                        process_id: process_id.to_string(),
                        stream: stream.to_string(),
                        line: line.to_string(),
                        sequence: Some(seq),
                        timestamp: Some(ts),
                        run_id,
                        step_id,
                    },
                );
            }
        }
    }

    /// Get a snapshot of a process handle.
    #[allow(dead_code)]
    pub fn get_handle(&self, process_id: &str) -> Option<ProcessHandle> {
        self.processes
            .read()
            .ok()
            .and_then(|procs| procs.get(process_id).map(|e| e.handle.clone()))
    }

    /// Get the current status of a tracked process.
    #[allow(dead_code)]
    pub fn get_status(&self, process_id: &str) -> Option<ProcessStatus> {
        self.processes
            .read()
            .ok()
            .and_then(|procs| procs.get(process_id).map(|e| e.status.clone()))
    }

    /// Kill a tracked process tree. Returns the outcome.
    pub fn kill(&self, process_id: &str) -> Result<ProcessStatus, String> {
        // Снимок под замком, сам kill — вне него: блокирующее завершение
        // дерева (Unix: SIGTERM + 2s grace + SIGKILL) не должно держать
        // write-lock — иначе стриминг логов ВСЕХ процессов встанет на это
        // время (emit_output ждёт read-lock).
        let (pid, mut child) = {
            let mut procs = self
                .processes
                .write()
                .map_err(|e| format!("Lock poisoned: {}", e))?;

            let entry = procs
                .get_mut(process_id)
                .ok_or_else(|| format!("Process '{}' not found", process_id))?;

            if entry.terminated {
                return Ok(entry.status.clone());
            }

            (entry.handle.pid, entry.child.take())
        };

        // Try to kill the process tree
        let kill_result = if let Some(ref mut child) = child {
            kill_process_tree(child, pid)
        } else {
            Ok(false)
        };

        // Reap
        if let Some(ref mut child) = child {
            let _ = child.wait();
        }

        // Update the entry under the lock
        let mut procs = self
            .processes
            .write()
            .map_err(|e| format!("Lock poisoned: {}", e))?;
        let entry = procs
            .get_mut(process_id)
            .ok_or_else(|| format!("Process '{}' not found", process_id))?;

        match kill_result {
            Ok(was_running) => {
                if was_running {
                    entry.status = ProcessStatus::Killed;
                    entry.terminated = true;
                }
                Ok(entry.status.clone())
            }
            Err(e) => {
                entry
                    .handle
                    .log_buffer
                    .push_stderr(format!("Kill failed: {}", e));
                Err(e)
            }
        }
    }

    /// Remove a process from tracking (e.g., after it has been reaped).
    #[allow(dead_code)]
    pub fn remove(&self, process_id: &str) {
        if let Ok(mut procs) = self.processes.write() {
            procs.remove(process_id);
        }
    }

    /// Get the number of active (non-terminated) processes.
    #[allow(dead_code)]
    pub fn active_count(&self) -> usize {
        self.processes
            .read()
            .map(|procs| procs.values().filter(|e| !e.terminated).count())
            .unwrap_or(0)
    }

    /// Get all tracked process IDs.
    #[allow(dead_code)]
    pub fn list_process_ids(&self) -> Vec<String> {
        self.processes
            .read()
            .map(|procs| procs.keys().cloned().collect())
            .unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // Background monitor loop
    // -----------------------------------------------------------------------

    async fn monitor_loop(self: &Arc<Self>) {
        loop {
            if self.shutdown.load(Ordering::SeqCst) {
                break;
            }

            self.check_processes();

            // Wait with cancellation check
            let _ = tokio::time::timeout(
                Duration::from_millis(self.poll_interval_ms),
                self.notify.notified(),
            )
            .await;
        }

        // Final cleanup
        self.terminate_all();
    }

    /// Check all tracked processes for termination.
    /// Does NOT hold the lock for extended periods — snapshots the list first.
    fn check_processes(&self) {
        // Phase 1: snapshot the process IDs to check
        let ids: Vec<String> = self
            .processes
            .read()
            .map(|procs| procs.keys().cloned().collect())
            .unwrap_or_default();

        for id in ids {
            self.check_single_process(&id);
        }
    }

    /// Check a single process for termination. Non-blocking.
    fn check_single_process(&self, process_id: &str) {
        let mut procs = match self.processes.write() {
            Ok(p) => p,
            Err(_) => return,
        };

        let entry = match procs.get_mut(process_id) {
            Some(e) => e,
            None => return,
        };

        if entry.terminated {
            return;
        }

        // Rate-limit checks: don't poll faster than every 200ms per process
        if entry.last_check.elapsed() < Duration::from_millis(200) {
            return;
        }
        entry.last_check = Instant::now();

        // Try to check process status
        if let Some(ref mut child) = entry.child {
            match child.try_wait() {
                Ok(None) => {
                    // Still running
                    entry.status = ProcessStatus::Running;
                }
                Ok(Some(exit_status)) => {
                    // Process terminated
                    entry.child = None;
                    entry.terminated = true;

                    let code = exit_status.code().unwrap_or(-1);
                    let stderr_tail = entry
                        .handle
                        .log_buffer
                        .stderr_lines()
                        .last()
                        .cloned()
                        .unwrap_or_default();

                    entry.status = if exit_status.success() {
                        ProcessStatus::Exited(code)
                    } else {
                        // Distinguish between killed by signal vs normal error
                        #[cfg(unix)]
                        {
                            use std::os::unix::process::ExitStatusExt;
                            if exit_status.signal().is_some() {
                                ProcessStatus::Crashed
                            } else {
                                ProcessStatus::ExitedWithError(code)
                            }
                        }
                        #[cfg(not(unix))]
                        {
                            ProcessStatus::ExitedWithError(code)
                        }
                    };

                    let event = ProcessTerminatedEvent {
                        process_id: process_id.to_string(),
                        run_id: entry.handle.run_id.clone(),
                        step_id: entry.handle.step_id.clone(),
                        status: entry.status.clone(),
                        exit_code: entry.status.exit_code(),
                        error: if entry.status.is_error() {
                            Some(format!("Process exited with code {}", code))
                        } else {
                            None
                        },
                        stderr_tail,
                        duration_secs: entry.handle.elapsed_secs(),
                    };

                    // Emit event outside the lock
                    drop(procs);
                    self.emit_terminated(&event);
                    return;
                }
                Err(_) => {
                    // try_wait error — assume process is gone
                    entry.child = None;
                    entry.terminated = true;
                    entry.status = ProcessStatus::Unknown;
                }
            }
        } else {
            // No child handle — process was already reaped or is external
            if entry.status == ProcessStatus::Running {
                entry.status = ProcessStatus::Exited(0);
                entry.terminated = true;
            }
        }
    }

    fn emit_terminated(&self, event: &ProcessTerminatedEvent) {
        if let Ok(handle_guard) = self.app_handle.read() {
            if let Some(h) = handle_guard.as_ref() {
                let _ = h.emit("devlauncher:process-terminated", event);
            }
        }

        // Also emit the legacy process-status event
        if let Ok(handle_guard) = self.app_handle.read() {
            if let Some(h) = handle_guard.as_ref() {
                let _ = h.emit(
                    crate::modules::workspace::process_manager::PROCESS_EVENT_STATUS,
                    ProcessStatusEvent {
                        process_id: event.process_id.clone(),
                        status: event.status.clone(),
                        error: event.error.clone(),
                    },
                );
            }
        }
    }

    /// Terminate all tracked processes (called during shutdown).
    fn terminate_all(&self) {
        let ids: Vec<String> = self
            .processes
            .read()
            .map(|procs| {
                procs
                    .iter()
                    .filter(|(_, e)| !e.terminated)
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default();

        for id in &ids {
            let _ = self.kill(id);
        }
    }
}

impl Default for ProcessSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Platform-specific process tree termination
// ---------------------------------------------------------------------------

/// Terminate a process tree on the current platform.
/// Returns `Ok(true)` if the process was running and we terminated it,
/// `Ok(false)` if it had already exited, or `Err` if termination failed.
fn kill_process_tree(child: &mut std::process::Child, pid: u32) -> Result<bool, String> {
    match current_os() {
        HostOs::Windows => kill_windows_tree(pid),
        HostOs::Linux | HostOs::Macos => kill_unix_group(child, pid),
    }
}

/// Kill a process tree on Windows using `taskkill /F /T /PID`.
fn kill_windows_tree(pid: u32) -> Result<bool, String> {
    let mut cmd = std::process::Command::new("taskkill");
    cmd.args(["/F", "/T", "/PID", &pid.to_string()]);
    #[cfg(target_os = "windows")]
    crate::platform::suppress_child_console(&mut cmd);
    let status = cmd.status();

    match status {
        Ok(s) if s.success() => Ok(true),
        Ok(s) => {
            let code = s.code().unwrap_or(-1);
            if code >= 128 || code == 0 {
                Ok(false)
            } else {
                Err(format!(
                    "taskkill /F /T /PID {} failed with exit code {}",
                    pid, code
                ))
            }
        }
        Err(e) => Err(format!("Failed to run taskkill: {}", e)),
    }
}

/// Kill a process group on Unix: SIGTERM → grace → SIGKILL.
#[cfg(unix)]
fn kill_unix_group(child: &mut std::process::Child, pid: u32) -> Result<bool, String> {
    use std::os::unix::process::ExitStatusExt;

    // Check if already exited
    match child.try_wait() {
        Ok(Some(_)) => {
            let _ = child.wait();
            return Ok(false);
        }
        Ok(None) => {}
        Err(e) => {
            return Err(format!("Failed to check process status: {}", e));
        }
    }

    let pgid = pid as i32;

    // SIGTERM to the entire process group
    unsafe {
        libc::kill(-pgid, libc::SIGTERM);
    }

    // Wait up to 2 seconds for the child to exit
    let grace = Duration::from_millis(2000);
    let deadline = Instant::now() + grace;

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let _ = child.wait();
                return Ok(true);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                return Err(format!("Failed to check process status: {}", e));
            }
        }
    }

    // SIGKILL to the entire process group
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }

    // Brief wait for SIGKILL to take effect
    std::thread::sleep(Duration::from_millis(200));
    match child.try_wait() {
        Ok(Some(_)) => {
            let _ = child.wait();
            Ok(true)
        }
        Ok(None) => {
            let _ = child.wait();
            Ok(true)
        }
        Err(e) => Err(format!(
            "Failed to check process status after SIGKILL: {}",
            e
        )),
    }
}

#[cfg(not(unix))]
fn kill_unix_group(_child: &mut std::process::Child, _pid: u32) -> Result<bool, String> {
    Err("Unix process group kill is not available on this platform".to_string())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_handle_tracks_sequences() {
        let handle = ProcessHandle {
            id: "test".into(),
            pid: 1234,
            run_id: None,
            step_id: None,
            label: "test".into(),
            tracking_quality: ProcessTrackingQuality::Exact,
            log_buffer: Arc::new(BoundedLogBuffer::new(100, 1024 * 1024)),
            started_at: Instant::now(),
            stdout_seq: Arc::new(AtomicU64::new(0)),
            stderr_seq: Arc::new(AtomicU64::new(0)),
        };

        assert_eq!(handle.next_stdout_seq(), 0);
        assert_eq!(handle.next_stdout_seq(), 1);
        assert_eq!(handle.next_stderr_seq(), 0);
        assert_eq!(handle.next_stderr_seq(), 1);
    }

    #[test]
    fn supervisor_register_and_status() {
        let supervisor = ProcessSupervisor::new();
        let buffer = Arc::new(BoundedLogBuffer::new(100, 1024 * 1024));

        let handle = supervisor.register_process(
            "p1".into(),
            1234,
            None,
            Some("run1".into()),
            Some("step1".into()),
            "test process".into(),
            ProcessTrackingQuality::Exact,
            buffer,
        );

        assert_eq!(handle.id, "p1");
        assert_eq!(handle.pid, 1234);
        assert_eq!(handle.run_id, Some("run1".into()));
        assert_eq!(handle.step_id, Some("step1".into()));

        let status = supervisor.get_status("p1");
        assert_eq!(status, Some(ProcessStatus::Running));

        assert_eq!(supervisor.active_count(), 1);
        assert_eq!(supervisor.list_process_ids(), vec!["p1"]);
    }

    #[test]
    fn supervisor_kill_nonexistent() {
        let supervisor = ProcessSupervisor::new();
        let result = supervisor.kill("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn supervisor_remove() {
        let supervisor = ProcessSupervisor::new();
        let buffer = Arc::new(BoundedLogBuffer::new(100, 1024 * 1024));
        supervisor.register_process(
            "p1".into(),
            1234,
            None,
            None,
            None,
            "test".into(),
            ProcessTrackingQuality::Exact,
            buffer,
        );
        assert_eq!(supervisor.active_count(), 1);
        supervisor.remove("p1");
        assert_eq!(supervisor.active_count(), 0);
    }

    #[test]
    fn process_terminated_event_serialize() {
        let event = ProcessTerminatedEvent {
            process_id: "p1".into(),
            run_id: Some("r1".into()),
            step_id: Some("s1".into()),
            status: ProcessStatus::Exited(0),
            exit_code: Some(0),
            error: None,
            stderr_tail: String::new(),
            duration_secs: 5,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("p1"));
        assert!(json.contains("r1"));
    }
}
