use crate::modules::workspace::models::{ProjectContext, SessionInfo};
use crate::modules::workspace::project::ProjectService;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Persistent per-project time ledger. This is the "mini-DB" that lets the
/// user track how much time they spend on a project across sessions: when a
/// session ends the elapsed seconds are folded into `total_secs` and written
/// to disk, and when a project is reopened the timer resumes from that value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionStatsFile {
    #[serde(default)]
    pub projects: HashMap<String, ProjectStat>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ProjectStat {
    #[serde(default)]
    pub total_secs: u64,
}

pub trait SessionService: Send + Sync {
    fn start_session(&self, ctx: &ProjectContext);
    fn end_session(&self);
    fn get_session(&self) -> Option<SessionInfo>;
    fn link_process(&self, process_id: &str);
#[allow(dead_code)]
    fn get_linked_processes(&self) -> Vec<String>;
    fn increment_errors(&self);
}

struct ActiveSession {
#[allow(dead_code)]
    context: ProjectContext,
    key: String,
    started_at: u64,
    baseline_secs: u64,
    process_ids: Vec<String>,
    error_count: usize,
}

pub struct DefaultSessionService {
    current: Arc<Mutex<Option<ActiveSession>>>,
    stats: Arc<Mutex<SessionStatsFile>>,
    stats_path: Option<PathBuf>,
    /// Last time the stats file was written, used to throttle disk writes
    /// while a session is live (polled every few seconds).
    last_write: Arc<Mutex<u64>>,
}

/// Cooldown between persisted writes while a session is active.
const WRITE_COOLDOWN_SECS: u64 = 10;

/// Project identity used as the ledger key. Prefers the on-disk path so two
/// distinct folders with the same display name never share accumulated time.
fn project_key(ctx: &ProjectContext) -> String {
    ctx.project_path
        .as_deref()
        .filter(|p| !p.is_empty())
        .unwrap_or(&ctx.profile_name)
        .to_string()
}

impl DefaultSessionService {
    pub fn new(data_dir: Option<PathBuf>) -> Self {
        let (stats, stats_path) = match &data_dir {
            Some(dir) => {
                let path = dir.join("session_stats.json");
                let loaded = fs::read_to_string(&path)
                    .ok()
                    .and_then(|raw| serde_json::from_str::<SessionStatsFile>(&raw).ok())
                    .unwrap_or_default();
                (loaded, Some(path))
            }
            None => (SessionStatsFile::default(), None),
        };
        Self {
            current: Arc::new(Mutex::new(None)),
            stats: Arc::new(Mutex::new(stats)),
            stats_path,
            last_write: Arc::new(Mutex::new(0)),
        }
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Write the ledger to disk (atomic temp-file + rename). Failures are
    /// logged to stderr only вЂ” time tracking must never break the workspace.
    fn persist(&self, force: bool) {
        let path = match &self.stats_path {
            Some(p) => p.clone(),
            None => return,
        };
        let now = Self::now_secs();
        {
            let mut last = self.last_write.lock().expect("write lock poisoned");
            if !force && now.saturating_sub(*last) < WRITE_COOLDOWN_SECS {
                return;
            }
            *last = now;
        }
        let snapshot = self.stats.lock().expect("stats lock poisoned").clone();
        match serde_json::to_string_pretty(&snapshot) {
            Ok(content) => {
                if let Some(parent) = path.parent() {
                    let tmp = parent.join("session_stats.json.tmp");
                    if fs::write(&tmp, &content).is_ok() {
                        let _ = fs::rename(&tmp, &path);
                    }
                }
            }
            Err(e) => log::warn!("[session] failed to serialize stats: {e}"),
        }
    }

    /// Fold the elapsed seconds of a finished session into the project's
    /// accumulated total and persist it.
    fn accumulate_and_clear(&self, session: &ActiveSession) {
        let elapsed = Self::now_secs().saturating_sub(session.started_at);
        let added = session.baseline_secs.saturating_add(elapsed);
        if let Ok(mut stats) = self.stats.lock() {
            let entry = stats.projects.entry(session.key.clone()).or_default();
            entry.total_secs = entry.total_secs.max(added);
        }
        self.persist(true);
    }
}

impl SessionService for DefaultSessionService {
    fn start_session(&self, ctx: &ProjectContext) {
        // Fold any active session into its project's total before switching
        // (e.g. opening a different project without explicitly closing).
        let previous = self.current.lock().expect("session lock poisoned").take();
        if let Some(s) = previous {
            self.accumulate_and_clear(&s);
        }
        let key = project_key(ctx);
        let baseline = self
            .stats
            .lock()
            .expect("stats lock poisoned")
            .projects
            .get(&key)
            .map(|s| s.total_secs)
            .unwrap_or(0);
        let session = ActiveSession {
            context: ctx.clone(),
            key,
            started_at: Self::now_secs(),
            baseline_secs: baseline,
            process_ids: Vec::new(),
            error_count: 0,
        };
        *self.current.lock().expect("session lock poisoned") = Some(session);
    }

    fn end_session(&self) {
        let finished = {
            let mut guard = self.current.lock().expect("session lock poisoned");
            guard.take()
        };
        if let Some(session) = finished {
            self.accumulate_and_clear(&session);
        }
    }

    fn get_session(&self) -> Option<SessionInfo> {
        let guard = self.current.lock().expect("session lock poisoned");
        let info = guard.as_ref().map(|s| {
            let now = Self::now_secs();
            let session_secs = now.saturating_sub(s.started_at);
            SessionInfo {
                started_at: s.started_at.to_string(),
                duration_secs: session_secs,
                total_duration_secs: s.baseline_secs.saturating_add(session_secs),
                process_count: s.process_ids.len(),
                error_count: s.error_count,
            }
        });
        // Persist opportunistically (throttled) so a crash/kill never loses
        // more than the cooldown window of accumulated time.
        drop(guard);
        if info.is_some() {
            self.persist(false);
        }
        info
    }

    fn link_process(&self, process_id: &str) {
        if let Some(ref mut session) = *self.current.lock().expect("session lock poisoned") {
            session.process_ids.push(process_id.to_string());
        }
    }

    fn get_linked_processes(&self) -> Vec<String> {
        self.current
            .lock()
            .expect("session lock poisoned")
            .as_ref()
            .map(|s| s.process_ids.clone())
            .unwrap_or_default()
    }
    fn increment_errors(&self) {
        if let Some(ref mut session) = *self.current.lock().expect("session lock poisoned") {
            session.error_count += 1;
        }
    }
}

/// Ensures an active session exists before a process is about to start.
///
/// If a project is bound we (re)start the session so the spawned process is
/// linked to it (the timer resumes from the project's persisted total).
/// Returns the current session start timestamp (if any) for linkage.
pub fn ensure_session(state: &crate::modules::workspace::WorkspaceState) -> Option<String> {
    if state.session.get_session().is_none() {
        if let Some(ctx) = state.project.get_current() {
            state.session.start_session(&ctx);
        } else {
            return None;
        }
    }
    state.session.get_session().map(|s| s.started_at.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(name: &str, path: Option<&str>) -> ProjectContext {
        ProjectContext {
            profile_name: name.to_string(),
            project_path: path.map(|p| p.to_string()),
            description: String::new(),
            stack: vec![],
            opened_at: "0".into(),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sp_session_test_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn accumulates_total_across_sessions() {
        let dir = temp_dir("accumulate");
        let svc = DefaultSessionService::new(Some(dir.clone()));

        svc.start_session(&ctx("app", Some("C:/dev/app")));
        // Manually rewind the start time so elapsed is non-zero.
        {
            let mut guard = svc.current.lock().unwrap();
            let s = guard.as_mut().unwrap();
            s.started_at = s.started_at.saturating_sub(120);
        }
        let info = svc.get_session().unwrap();
        assert!(info.duration_secs >= 120);
        assert_eq!(info.total_duration_secs, info.duration_secs);
        svc.end_session();

        // Reopen the same project: the total continues from the persisted value.
        svc.start_session(&ctx("app", Some("C:/dev/app")));
        let reopened = svc.get_session().unwrap();
        assert!(reopened.total_duration_secs >= 120);
        assert!(reopened.total_duration_secs >= reopened.duration_secs);

        let stats = svc.stats.lock().unwrap();
        assert!(stats.projects.get("C:/dev/app").unwrap().total_secs >= 120);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn different_projects_are_tracked_separately() {
        let dir = temp_dir("separate");
        let svc = DefaultSessionService::new(Some(dir.clone()));

        svc.start_session(&ctx("a", Some("C:/dev/a")));
        {
            let mut guard = svc.current.lock().unwrap();
            guard.as_mut().unwrap().started_at =
                guard.as_mut().unwrap().started_at.saturating_sub(60);
        }
        svc.end_session();

        svc.start_session(&ctx("b", Some("C:/dev/b")));
        {
            let mut guard = svc.current.lock().unwrap();
            guard.as_mut().unwrap().started_at =
                guard.as_mut().unwrap().started_at.saturating_sub(10);
        }
        let info = svc.get_session().unwrap();
        assert!(
            info.total_duration_secs < 30,
            "project b must not inherit project a time"
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn persisted_total_survives_service_restart() {
        let dir = temp_dir("restart");
        {
            let svc = DefaultSessionService::new(Some(dir.clone()));
            svc.start_session(&ctx("app", Some("C:/dev/app")));
            {
                let mut guard = svc.current.lock().unwrap();
                guard.as_mut().unwrap().started_at =
                    guard.as_mut().unwrap().started_at.saturating_sub(45);
            }
            svc.end_session();
        }
        // New service instance loads the file from disk.
        let svc2 = DefaultSessionService::new(Some(dir.clone()));
        svc2.start_session(&ctx("app", Some("C:/dev/app")));
        let info = svc2.get_session().unwrap();
        assert!(info.total_duration_secs >= 45);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn switching_projects_folds_previous_elapsed() {
        let dir = temp_dir("switch");
        let svc = DefaultSessionService::new(Some(dir.clone()));

        svc.start_session(&ctx("a", Some("C:/dev/a")));
        {
            let mut guard = svc.current.lock().unwrap();
            guard.as_mut().unwrap().started_at =
                guard.as_mut().unwrap().started_at.saturating_sub(90);
        }
        // Switch directly to another project вЂ” old elapsed must be persisted.
        svc.start_session(&ctx("b", Some("C:/dev/b")));

        // Reopen project A: its total reflects the folded 90s.
        svc.start_session(&ctx("a", Some("C:/dev/a")));
        let a = svc.get_session().unwrap();
        assert!(a.total_duration_secs >= 90);

        let stats = svc.stats.lock().unwrap();
        assert!(stats.projects.get("C:/dev/a").unwrap().total_secs >= 90);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn end_session_without_start_is_noop() {
        let dir = temp_dir("noop");
        let svc = DefaultSessionService::new(Some(dir.clone()));
        svc.end_session();
        assert!(svc.get_session().is_none());
        fs::remove_dir_all(dir).ok();
    }
}
