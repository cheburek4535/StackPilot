pub mod models;
pub mod process_manager;
pub mod commands;

use std::sync::Arc;
use crate::modules::workspace::process_manager::ProcessManager;

pub struct WorkspaceState {
    pub process_manager: Arc<dyn ProcessManager>,
}

impl WorkspaceState {
    pub fn new(process_manager: Arc<dyn ProcessManager>) -> Self {
        Self { process_manager }
    }
}
