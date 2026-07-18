use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Emitter;

use crate::modules::workspace::models::*;

pub const PROCESS_EVENT_OUTPUT: &str = "process-output";
pub const PROCESS_EVENT_STATUS: &str = "process-status";

struct ActiveProcess {
    info: TrackedProcess,
    child: Option<Child>,
    stdout_buffer: Arc<Mutex<Vec<String>>>,
    stderr_buffer: Arc<Mutex<Vec<String>>>,
}

pub trait ProcessManager: Send + Sync {
    fn spawn_and_track(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>
    ) -> Result<TrackedProcess, String>;

    fn list(&self) -> Vec<TrackedProcess>;
    fn kill(&self, id: &str) -> Result<(), String>;
    fn refresh_status(&self, id: &str) -> Result<ProcessStatus, String>;
    fn get_logs(&self, id: &str) -> Result<ProcessLogs, String>;
}

pub struct OsProcessManager {
    processes: Arc<Mutex<Vec<ActiveProcess>>>,
    app_handle: Arc<Mutex<Option<tauri::AppHandle>>>,
}

impl OsProcessManager {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(Mutex::new(Vec::new())),
            app_handle: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        *self.app_handle.lock().expect("app_handle lock poisoned") = Some(handle);
    }

    fn spawn_reader_thread(
        stream_name: &'static str,
        reader: Box<dyn std::io::Read + Send + 'static>,
        process_id: String,
        buffer: Arc<Mutex<Vec<String>>>,
        handle: tauri::AppHandle,
    ) {
        thread::spawn(move || {
            let buf_reader = BufReader::new(reader);
            for line in buf_reader.lines() {
                match line {
                    Ok(text) => {
                        buffer.lock().expect("buffer lock poisoned").push(text.clone());
                        let _ = handle.emit(
                            PROCESS_EVENT_OUTPUT,
                            ProcessOutputEvent {
                                process_id: process_id.clone(),
                                stream: stream_name.to_string(),
                                line: text,
                            },
                        );
                    }
                    Err(_) => break,
                }
            }
        });
    }
}

impl ProcessManager for OsProcessManager {
    fn spawn_and_track(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>
    ) -> Result<TrackedProcess, String> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        }

        let mut child = cmd.spawn().map_err(|e| format!("Spawn failed: {}", e))?;
        let pid = child.id();
        let id = generate_id();
        let started_at = timestamp_now();

        let info = TrackedProcess {
            id: id.clone(),
            pid,
            label: label.to_string(),
            status: ProcessStatus::Running,
            started_at,
            duration_secs: 0,
            restarts: 0,
            last_error: None,
            session_id: session_id
        };

        let stdout_buffer: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stderr_buffer: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let handle_guard = self.app_handle.lock().expect("app_handle lock poisoned");
        let handle = handle_guard
            .clone()
            .expect("AppHandle must be set before spawning processes");

        if let Some(stdout) = child.stdout.take() {
            Self::spawn_reader_thread(
                "stdout",
                Box::new(stdout),
                id.clone(),
                stdout_buffer.clone(),
                handle.clone(),
            );
        }

        if let Some(stderr) = child.stderr.take() {
            Self::spawn_reader_thread(
                "stderr",
                Box::new(stderr),
                id.clone(),
                stderr_buffer.clone(),
                handle.clone(),
            );
        }

        let entry = ActiveProcess {
            info: info.clone(),
            child: Some(child),
            stdout_buffer,
            stderr_buffer,
        };

        self.processes
            .lock()
            .expect("processes lock poisoned")
            .push(entry);

        Ok(info)
    }

    fn list(&self) -> Vec<TrackedProcess> {
        let lock = self.processes.lock().expect("processes lock poisoned");
        lock.iter()
            .map(|p| {
                let mut info = p.info.clone();
                let started = info.started_at.parse::<u64>().unwrap_or(0);
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                info.duration_secs = now.saturating_sub(started);
                info
            })
            .collect()
    }

    fn kill(&self, id: &str) -> Result<(), String> {
        let mut lock = self.processes.lock().expect("processes lock poisoned");
        let entry = lock
            .iter_mut()
            .find(|p| p.info.id == id)
            .ok_or_else(|| format!("Process '{}' not found", id))?;

        let pid = entry.info.pid;
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .status();

        entry.child = None;
        entry.info.status = ProcessStatus::Killed;

        let handle_guard = self.app_handle.lock().expect("app_handle lock poisoned");
        if let Some(handle) = handle_guard.as_ref() {
            let _ = handle.emit(
                PROCESS_EVENT_STATUS,
                ProcessStatusEvent {
                    process_id: id.to_string(),
                    status: ProcessStatus::Killed,
                    error: None,
                },
            );
        }

        Ok(())
    }

    fn refresh_status(&self, id: &str) -> Result<ProcessStatus, String> {
        let mut lock = self.processes.lock().expect("processes lock poisoned");
        let entry = lock
            .iter_mut()
            .find(|p| p.info.id == id)
            .ok_or_else(|| format!("Process '{}' not found", id))?;

        if let Some(ref mut child) = entry.child {
            match child.try_wait() {
                Ok(None) => {
                    entry.info.status = ProcessStatus::Running;
                    Ok(ProcessStatus::Running)
                }
                Ok(Some(status)) => {
                    entry.child = None;
                    let (new_status, error_msg) = if status.success() {
                        (ProcessStatus::Exited(status.code().unwrap_or(0)), None)
                    } else {
                        let stderr = entry
                            .stderr_buffer
                            .lock()
                            .expect("stderr lock poisoned")
                            .join("\n");
                        let err_msg = if stderr.is_empty() {
                            format!("Process exited with code {}", status.code().unwrap_or(-1))
                        } else {
                            stderr
                        };
                        (ProcessStatus::Crashed, Some(err_msg))
                    };
                    entry.info.status = new_status.clone();
                    entry.info.last_error = error_msg.clone();

                    let handle_guard =
                        self.app_handle.lock().expect("app_handle lock poisoned");
                    if let Some(handle) = handle_guard.as_ref() {
                        let _ = handle.emit(
                            PROCESS_EVENT_STATUS,
                            ProcessStatusEvent {
                                process_id: id.to_string(),
                                status: new_status.clone(),
                                error: error_msg,
                            },
                        );
                    }

                    Ok(new_status)
                }
                Err(e) => Err(format!("try_wait error: {}", e)),
            }
        } else {
            Ok(entry.info.status.clone())
        }
    }

    fn get_logs(&self, id: &str) -> Result<ProcessLogs, String> {
        let lock = self.processes.lock().expect("processes lock poisoned");
        let entry = lock
            .iter()
            .find(|p| p.info.id == id)
            .ok_or_else(|| format!("Process '{}' not found", id))?;

        let stdout = entry.stdout_buffer.lock().expect("stdout lock poisoned").clone();
        let stderr = entry.stderr_buffer.lock().expect("stderr lock poisoned").clone();
        Ok(ProcessLogs {
            stdout_lines: stdout,
            stderr_lines: stderr,
        })
    }
}

fn generate_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("proc_{}", nanos)
}

fn timestamp_now() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    dur.as_secs().to_string()
}
