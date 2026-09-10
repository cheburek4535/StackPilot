use crate::modules::workspace::models::ProjectContext;
use std::fs;
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct ProjectInfo {
    pub profile_name: String,
    pub project_path: Option<String>,
    pub description: String,
    pub stack: Vec<String>,
    pub opened_at: String,
    pub file_count: usize,
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
        let (file_count, git_branch) = if let Some(path_str) = ctx.project_path.as_deref() {
            (self.count_files(path_str), self.get_git_branch(path_str))
        } else {
            (0, None)
        };
        ProjectInfo {
            profile_name: ctx.profile_name.clone(),
            project_path: ctx.project_path.clone(),
            description: ctx.description.clone(),
            stack: ctx.stack.clone(),
            opened_at: ctx.opened_at.clone(),
            file_count,
            git_branch,
        }
    }

    fn get_readme(&self, path: &str) -> Option<String> {
        let candidates = ["readme.md", "readme.txt", "readme"];
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if candidates.contains(&name.as_str()) {
                    return fs::read_to_string(entry.path()).ok();
                }
            }
        }
        None
    }
}

impl DefaultInfoService {
    pub fn new() -> Self {
        Self
    }

    fn get_git_branch(&self, path: &str) -> Option<String> {
        let mut cmd = std::process::Command::new("git");
        cmd.args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(path);
        #[cfg(target_os = "windows")]
        crate::platform::suppress_child_console(&mut cmd);
        let output = cmd.output().ok()?;
        if output.status.success() {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            None
        }
    }

    fn count_files(&self, path: &str) -> usize {
        let excluded = [
            "node_modules",
            ".git",
            "target",
            ".venv",
            "__pycache__",
            ".next",
        ];
        WalkDir::new(path)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy().to_lowercase();
                !excluded.contains(&name.as_ref())
            })
            .flatten()
            .filter(|e| e.file_type().is_file())
            .count()
    }
}
