use crate::modules::workspace::models::ProjectContext;

#[derive(Debug, Clone)]
pub struct ProjectInfo {
    pub profile_name: String,
    pub project_path: Option<String>,
    pub description: String,
    pub stack: Vec<String>,
    pub opened_at: String,
    pub file_count: Option<usize>,
    pub git_branch: Option<String>,
}

/// Service for project metadata.
/// Aggregates info from ProjectContext + file system + git.
pub trait InfoService: Send + Sync {
    fn get_info(&self, ctx: &ProjectContext) -> ProjectInfo;
    fn get_readme(&self, path: &str) -> Option<String>;
}

pub struct DefaultInfoService;

impl InfoService for DefaultInfoService {
    fn get_info(&self, ctx: &ProjectContext) -> ProjectInfo {
        ProjectInfo {
            profile_name: ctx.profile_name.clone(),
            project_path: ctx.project_path.clone(),
            description: ctx.description.clone(),
            stack: ctx.stack.clone(),
            opened_at: ctx.opened_at.clone(),
            file_count: None,
            git_branch: None,
        }
    }

    fn get_readme(&self, _path: &str) -> Option<String> {
        None
    }
}

impl DefaultInfoService {
    pub fn new() -> Self {
        Self
    }
}
