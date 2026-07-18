use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use crate::modules::workspace::models::ProjectContext;

pub struct ProjectState {
    current: Arc<Mutex<Option<ProjectContext>>>,
}

impl ProjectState {
    pub fn new() -> Self {
        Self {
            current: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set(&self, ctx: ProjectContext) {
        *self.current.lock().expect("project state lock poisoned") = Some(ctx);
    }

    pub fn get(&self) -> Option<ProjectContext> {
        self.current.lock().expect("project state lock poisoned").clone()
    }

    pub fn clear(&self) {
        *self.current.lock().expect("project state lock poisoned") = None;
    }
}

pub fn timestamp_now() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    dur.as_secs().to_string()
}
