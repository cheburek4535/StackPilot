use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::modules::workspace::models::*;

struct ActiveProcess {
    info: TrackedProcess,
    child: Option<Child>,
}

pub trait ProcessManager: Send + Sync {
    fn spawn_and_track(
        &self,
        command: &str,
        args: &[String],
        working_dir: Option<&str>,
        label: &str,
    ) -> Result<TrackedProcess, String>;

    fn list(&self) -> Vec<TrackedProcess>;
    fn kill(&self, id: &str) -> Result<(), String>;
    fn refresh_status(&self, id: &str) -> Result<ProcessStatus, String>;
}

pub struct OsProcessManager {
    processes: Arc<Mutex<HashMap<String, ActiveProcess>>>,
}

impl OsProcessManager {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl ProcessManager for OsProcessManager {
    fn spawn_and_track(
        &self,
        command: &str,
        args: &[String],
        working_dir: Option<&str>,
        label: &str,
    ) -> Result<TrackedProcess, String> {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C")
            .arg(command)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        }

        let child = cmd.spawn().map_err(|e| format!("Spawn failed: {}", e))?;
        let pid = child.id();
        let id = generate_id();
        let started_at = timestamp_now();

        let info = TrackedProcess {
            id: id.clone(),
            pid,
            label: label.to_string(),
            status: ProcessStatus::Running,
            started_at,
        };

        let entry = ActiveProcess {
            info: info.clone(),
            child: Some(child),
        };

        self.processes
            .lock()
            .expect("processes lock poisoned")
            .insert(id, entry);

        Ok(info)
    }

    fn list(&self) -> Vec<TrackedProcess> {
        let lock = self.processes.lock().expect("processes lock poisoned");
        lock.values().map(|p| p.info.clone()).collect()
    }

    fn kill(&self, id: &str) -> Result<(), String> {
        let mut lock = self.processes.lock().expect("processes lock poisoned");
        let entry = lock
            .get_mut(id)
            .ok_or_else(|| format!("Process '{}' not found", id))?;

        let _ = Command::new("taskkill")
            .args(["/F", "/PID", &entry.info.pid.to_string()])
            .status();

        entry.child = None;
        entry.info.status = ProcessStatus::Killed;
        Ok(())
    }

    fn refresh_status(&self, id: &str) -> Result<ProcessStatus, String> {
        let mut lock = self.processes.lock().expect("processes lock poisoned");
        let entry = lock
            .get_mut(id)
            .ok_or_else(|| format!("Process '{}' not found", id))?;

        if let Some(ref mut child) = entry.child {
            match child.try_wait() {
                Ok(None) => {
                    entry.info.status = ProcessStatus::Running;
                    Ok(ProcessStatus::Running)
                }
                Ok(Some(status)) => {
                    entry.child = None;
                    let new_status = if status.success() {
                        ProcessStatus::Exited(status.code().unwrap_or(0))
                    } else {
                        ProcessStatus::Crashed
                    };
                    entry.info.status = new_status.clone();
                    Ok(new_status)
                }
                Err(e) => Err(format!("try_wait error: {}", e)),
            }
        } else {
            Ok(entry.info.status.clone())
        }
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
