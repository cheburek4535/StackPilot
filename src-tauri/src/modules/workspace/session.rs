use crate::modules::workspace::models::{ProjectContext, SessionInfo};
use crate::modules::workspace::project::ProjectService;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub trait SessionService: Send + Sync {
    fn start_session(&self, ctx: &ProjectContext);
    fn end_session(&self);
    fn get_session(&self) -> Option<SessionInfo>;
    fn link_process(&self, process_id: &str);
    fn get_linked_processes(&self) -> Vec<String>;
    fn increment_errors(&self);
}

struct ActiveSession {
    context: ProjectContext,
    started_at: u64,
    process_ids: Vec<String>,
    error_count: usize,
}

pub struct DefaultSessionService {
    current: Arc<Mutex<Option<ActiveSession>>>,
}

impl DefaultSessionService {
    pub fn new() -> Self {
        Self {
            current: Arc::new(Mutex::new(None)),
        }
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

impl SessionService for DefaultSessionService {
    fn start_session(&self, ctx: &ProjectContext) {        let session = ActiveSession {
            context: ctx.clone(),
            started_at: Self::now_secs(),
            process_ids: Vec::new(),
            error_count: 0,
        };
        *self.current.lock().expect("session lock poisoned") = Some(session);
    }

    fn end_session(&self) {
        *self.current.lock().expect("session lock poisoned") = None;
    }

    fn get_session(&self) -> Option<SessionInfo> {
        let guard = self.current.lock().expect("session lock poisoned");
        guard.as_ref().map(|s| {
            let now = Self::now_secs();
            SessionInfo {
                started_at: s.started_at.to_string(),
                duration_secs: now.saturating_sub(s.started_at),
                process_count: s.process_ids.len(),
                error_count: s.error_count,
            }
        })
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
/// A session auto-ends when the workspace is open but idle (nothing running).
/// When the user then starts a launch without re-binding the project, the
/// session would otherwise stay `None` and the fresh process would not be
/// tracked by the running timer. If a project is bound we (re)start the
/// session so the spawned process is linked to it. Returns the current
/// session start timestamp (if any) for linkage.
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
