use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use crate::modules::workspace::models::ProjectContext;

pub trait ProjectService: Send + Sync {
    fn set_current(&self, ctx: ProjectContext);
    fn get_current(&self) -> Option<ProjectContext>;
    fn clear_current(&self);
    fn open_project(&self, path: &str, profile_name: &str, description: &str, stack: &[String]) -> ProjectContext;
}

pub struct DefaultProjectService {
    current: Arc<Mutex<Option<ProjectContext>>>,
}

impl DefaultProjectService {
    pub fn new() -> Self {
        Self {
            current: Arc::new(Mutex::new(None)),
        }
    }
}

impl ProjectService for DefaultProjectService {
    fn set_current(&self, ctx: ProjectContext) {
        *self.current.lock().expect("project state lock poisoned") = Some(ctx);
    }

    fn get_current(&self) -> Option<ProjectContext> {
        self.current.lock().expect("project state lock poisoned").clone()
    }

    fn clear_current(&self) {
        *self.current.lock().expect("project state lock poisoned") = None;
    }

    fn open_project(&self, path: &str, profile_name: &str, description: &str, stack: &[String]) -> ProjectContext {
        let ctx = ProjectContext {
            profile_name: profile_name.to_string(),
            project_path: Some(path.to_string()),
            description: description.to_string(),
            stack: stack.to_vec(),
            opened_at: timestamp_now(),
        };
        self.set_current(ctx.clone());
        ctx
    }
}

pub fn timestamp_now() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    dur.as_secs().to_string()
}
