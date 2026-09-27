use crate::modules::workspace::project::ProjectService;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContent {
    pub content: String,
    pub language: String,
}

/// Service for browsing and editing project files within a sandboxed project workspace.
pub trait FileExplorerService: Send + Sync {
    fn list_directory(&self, dir: &str) -> Result<Vec<FileEntry>, String>;
    fn read_file(&self, path: &str) -> Result<FileContent, String>;
    fn write_file(&self, path: &str, content: &str) -> Result<(), String>;
    fn open_in_vscode(&self, path: &str, vscode_path: Option<&str>) -> Result<(), String>;
    #[allow(dead_code)]
    fn detect_language(&self, path: &str) -> String;
}

/// Normalizes Windows verbatim UNC paths (`\\?\C:\...` or `\\?\UNC\...`) to standard paths
/// for display and frontend compatibility, while retaining portability across operating systems.
pub fn strip_unc_prefix(path: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let s = path.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{}", stripped));
        }
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(stripped);
        }
    }
    path.to_path_buf()
}

/// Resolves a requested user path relative to `project_root` and strictly verifies that
/// the canonical target lies within the canonical project root (Path Sandboxing).
///
/// Prevents:
/// - Directory traversal via `..` (e.g. `../../Windows/System32` or `sub/../../../../etc/passwd`)
/// - Arbitrary absolute path access outside the project root (e.g. `C:\Users\...\.ssh\id_rsa`)
/// - Prefix collision attacks (e.g. `/home/user/project_fake` when root is `/home/user/project`)
/// - Traversal attempts targeting non-existent files for write/creation
/// - Symlink/junction escapes outside the project root
pub fn resolve_and_verify_path(project_root: &Path, user_path: &str) -> Result<PathBuf, String> {
    let trimmed = user_path.trim().trim_matches('"');

    // Canonicalize project root to resolve symlinks and get absolute device path
    let canonical_root = project_root
        .canonicalize()
        .map_err(|e| format!("Invalid project root '{}': {}", project_root.display(), e))?;

    if !canonical_root.is_dir() {
        return Err(format!(
            "Project root '{}' is not a directory",
            project_root.display()
        ));
    }

    if trimmed.is_empty() || trimmed == "." {
        return Ok(strip_unc_prefix(&canonical_root));
    }

    let target_path = Path::new(trimmed);
    let target = if target_path.is_relative() {
        project_root.join(target_path)
    } else {
        target_path.to_path_buf()
    };

    if target.exists() {
        let canonical_target = target
            .canonicalize()
            .map_err(|e| format!("Invalid path '{}': {}", target.display(), e))?;

        if !canonical_target.starts_with(&canonical_root) {
            return Err("Access Denied: Path Traversal detected".into());
        }

        Ok(strip_unc_prefix(&canonical_target))
    } else {
        // Target does not exist yet (e.g. creating/writing a new file).
        // Trace up to find the deepest existing ancestor directory.
        let mut existing = target.clone();
        let mut missing_segments: Vec<std::ffi::OsString> = Vec::new();

        while !existing.exists() {
            if let Some(file_name) = existing.file_name() {
                missing_segments.push(file_name.to_os_string());
                match existing.parent() {
                    Some(parent) if parent != existing => {
                        existing = parent.to_path_buf();
                    }
                    _ => break,
                }
            } else {
                break;
            }
        }

        if !existing.exists() {
            return Err(format!(
                "Invalid path: ancestor of '{}' does not exist",
                target.display()
            ));
        }

        let mut canonical_ancestor = existing
            .canonicalize()
            .map_err(|e| format!("Invalid path '{}': {}", existing.display(), e))?;

        if !canonical_ancestor.starts_with(&canonical_root) {
            return Err("Access Denied: Path Traversal detected".into());
        }

        missing_segments.reverse();
        for segment in missing_segments {
            let s = segment.to_string_lossy();
            if s == ".." || s == "." || s.contains('/') || s.contains('\\') || s.contains('\0') {
                return Err("Access Denied: Path Traversal detected".into());
            }
            canonical_ancestor.push(segment);
        }

        if !canonical_ancestor.starts_with(&canonical_root) {
            return Err("Access Denied: Path Traversal detected".into());
        }

        Ok(strip_unc_prefix(&canonical_ancestor))
    }
}

pub struct DefaultFileExplorerService {
    project: Option<Arc<dyn ProjectService>>,
}

impl DefaultFileExplorerService {
    pub fn new(project: Arc<dyn ProjectService>) -> Self {
        Self {
            project: Some(project),
        }
    }

    /// Standalone instance without an attached project service.
    /// All file operations will return access denied.
    pub fn standalone() -> Self {
        Self { project: None }
    }

    fn get_project_root(&self) -> Result<PathBuf, String> {
        let project = self
            .project
            .as_ref()
            .ok_or_else(|| "No active project: access denied".to_string())?;
        let ctx = project
            .get_current()
            .ok_or_else(|| "No active project: access denied".to_string())?;
        let root_str = ctx
            .project_path
            .ok_or_else(|| "Project path is not set: access denied".to_string())?;
        if root_str.trim().is_empty() {
            return Err("Project path is empty: access denied".to_string());
        }
        Ok(PathBuf::from(root_str))
    }

    fn detect_language_from_ext(path: &str) -> String {
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        match ext.to_lowercase().as_str() {
            "js" | "mjs" | "cjs" => "javascript".into(),
            "ts" | "tsx" | "mts" | "cts" => "typescript".into(),
            "jsx" => "jsx".into(),
            "rs" => "rust".into(),
            "py" => "python".into(),
            "json" => "json".into(),
            "html" | "htm" => "html".into(),
            "css" | "scss" | "less" => "css".into(),
            "md" | "markdown" => "markdown".into(),
            "yml" | "yaml" => "yaml".into(),
            "toml" => "toml".into(),
            "xml" | "svg" => "xml".into(),
            "sh" | "bash" | "zsh" => "bash".into(),
            "ps1" => "powershell".into(),
            "go" => "go".into(),
            "java" => "java".into(),
            "rb" => "ruby".into(),
            "php" => "php".into(),
            "sql" => "sql".into(),
            "dockerfile" => "dockerfile".into(),
            "gitignore" => "ignore".into(),
            "env" => "env".into(),
            "lock" | "log" | "gitkeep" => "text".into(),
            _ => "text".into(),
        }
    }
}

impl Default for DefaultFileExplorerService {
    fn default() -> Self {
        Self::standalone()
    }
}

impl FileExplorerService for DefaultFileExplorerService {
    fn list_directory(&self, dir: &str) -> Result<Vec<FileEntry>, String> {
        let root = self.get_project_root()?;
        let verified_dir = resolve_and_verify_path(&root, dir)?;

        let entries = fs::read_dir(&verified_dir)
            .map_err(|e| format!("Failed to read dir '{}': {}", verified_dir.display(), e))?;
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();
            let name = entry.file_name().to_str().unwrap_or("?").to_string();

            // Skip hidden files/dirs
            if name.starts_with('.') && name != ".env" {
                continue;
            }

            let is_dir = path.is_dir();
            let size = if is_dir {
                0u64
            } else {
                fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
            };

            let clean_path = strip_unc_prefix(&path);

            files.push(FileEntry {
                name,
                path: clean_path.to_string_lossy().to_string(),
                is_dir,
                size,
            });
        }

        // Sort: directories first, then by name
        files.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        Ok(files)
    }

    fn read_file(&self, path: &str) -> Result<FileContent, String> {
        let root = self.get_project_root()?;
        let verified = resolve_and_verify_path(&root, path)?;
        let content = fs::read_to_string(&verified)
            .map_err(|e| format!("Failed to read '{}': {}", verified.display(), e))?;
        let language = Self::detect_language_from_ext(path);
        Ok(FileContent { content, language })
    }

    fn write_file(&self, path: &str, content: &str) -> Result<(), String> {
        let root = self.get_project_root()?;
        let verified = resolve_and_verify_path(&root, path)?;

        if let Some(parent) = verified.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "Failed to create parent directory '{}': {}",
                        parent.display(),
                        e
                    )
                })?;
            }
        }

        fs::write(&verified, content)
            .map_err(|e| format!("Failed to write '{}': {}", verified.display(), e))
    }

    fn open_in_vscode(&self, path: &str, vscode_path: Option<&str>) -> Result<(), String> {
        let path_to_open = if let Ok(root) = self.get_project_root() {
            let verified = resolve_and_verify_path(&root, path)?;
            if !verified.exists() {
                return Err(format!("Path does not exist: {}", verified.display()));
            }
            verified.to_string_lossy().to_string()
        } else {
            let p = Path::new(path);
            if !p.exists() {
                return Err(format!("Path does not exist: {}", path));
            }
            path.to_string()
        };

        let configured = vscode_path.map(|s| s.trim()).filter(|s| !s.is_empty());
        let cli = configured.unwrap_or("code");
        let resolved = crate::platform::ide::resolve_ide_executable(cli).ok_or_else(|| {
            format!(
                "Failed to open VSCode: '{}' was not found on this system. \
                     Make sure the path is correct in Settings -> System.",
                cli
            )
        })?;

        let result = if cfg!(target_os = "windows") {
            if crate::platform::paths::is_batch_file(&resolved) {
                let args = crate::platform::command::batch_shim_cmd_line(&resolved, &[&path_to_open]);
                let mut cmd = Command::new("cmd");
                cmd.args(args);
                #[cfg(target_os = "windows")]
                crate::platform::suppress_child_console(&mut cmd);
                cmd.spawn()
            } else {
                Command::new(&resolved).arg(&path_to_open).spawn()
            }
        } else {
            let (program, mut launcher_args) =
                crate::platform::app_launcher::split_launcher_string(&resolved);
            launcher_args.push(path_to_open);
            Command::new(&program).args(&launcher_args).spawn()
        };
        match result {
            Ok(_) => Ok(()),
            Err(e) => Err(format!(
                "Failed to open VSCode ({}): {}. Make sure the path is correct in Settings -> System.",
                resolved, e
            )),
        }
    }

    fn detect_language(&self, path: &str) -> String {
        Self::detect_language_from_ext(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::workspace::models::ProjectContext;
    use crate::modules::workspace::project::DefaultProjectService;
    use std::fs::File;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(1);

    struct TempTestDir {
        path: PathBuf,
    }

    impl TempTestDir {
        fn new(name: &str) -> Self {
            let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!("sp_fe_audit_test_{}_{}_{}", std::process::id(), name, id));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("Failed to create test temp dir");
            Self { path }
        }
    }

    impl Drop for TempTestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_resolve_and_verify_path_valid_relative_and_dot() {
        let temp = TempTestDir::new("valid_rel");
        let src_dir = temp.path.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        let main_file = src_dir.join("main.rs");
        File::create(&main_file).unwrap().write_all(b"fn main() {}").unwrap();

        // Dot / empty resolves to root
        let root_res = resolve_and_verify_path(&temp.path, ".").unwrap();
        assert_eq!(root_res, strip_unc_prefix(&temp.path.canonicalize().unwrap()));

        let empty_res = resolve_and_verify_path(&temp.path, "").unwrap();
        assert_eq!(empty_res, strip_unc_prefix(&temp.path.canonicalize().unwrap()));

        // Relative path inside root
        let resolved = resolve_and_verify_path(&temp.path, "src/main.rs").unwrap();
        assert_eq!(resolved, strip_unc_prefix(&main_file.canonicalize().unwrap()));

        // Relative with dot-slash
        let resolved_dot = resolve_and_verify_path(&temp.path, "./src/main.rs").unwrap();
        assert_eq!(resolved_dot, strip_unc_prefix(&main_file.canonicalize().unwrap()));
    }

    #[test]
    fn test_resolve_and_verify_path_valid_absolute_inside_root() {
        let temp = TempTestDir::new("valid_abs");
        let test_file = temp.path.join("config.json");
        File::create(&test_file).unwrap().write_all(b"{}").unwrap();

        let abs_str = test_file.to_string_lossy().to_string();
        let resolved = resolve_and_verify_path(&temp.path, &abs_str).unwrap();
        assert_eq!(resolved, strip_unc_prefix(&test_file.canonicalize().unwrap()));
    }

    #[test]
    fn test_resolve_and_verify_path_blocks_dot_dot_traversal() {
        let temp = TempTestDir::new("traversal_dotdot");
        let secret = temp.path.join("secret.txt");
        File::create(&secret).unwrap().write_all(b"supersecret").unwrap();

        let sub_project = temp.path.join("project_root");
        fs::create_dir_all(&sub_project).unwrap();

        // Attempting to step out of sub_project to secret.txt
        let err = resolve_and_verify_path(&sub_project, "../secret.txt").unwrap_err();
        assert!(err.contains("Access Denied: Path Traversal detected"), "Unexpected error: {err}");

        // Attempting multiple steps
        let err2 = resolve_and_verify_path(&sub_project, "../../secret.txt").unwrap_err();
        assert!(err2.contains("Access Denied: Path Traversal detected"), "Unexpected error: {err2}");
    }

    #[test]
    fn test_resolve_and_verify_path_blocks_nested_traversal_outside() {
        let temp = TempTestDir::new("nested_traversal");
        let project = temp.path.join("proj");
        let sub = project.join("a").join("b");
        fs::create_dir_all(&sub).unwrap();

        // sub/../../../../outside
        let err = resolve_and_verify_path(&project, "a/b/../../../../Windows/System32").unwrap_err();
        assert!(err.contains("Access Denied: Path Traversal detected"), "Unexpected error: {err}");
    }

    #[test]
    fn test_resolve_and_verify_path_blocks_absolute_outside_root() {
        let temp1 = TempTestDir::new("abs_out_1");
        let temp2 = TempTestDir::new("abs_out_2");

        let file2 = temp2.path.join("other.txt");
        File::create(&file2).unwrap().write_all(b"data").unwrap();

        let err = resolve_and_verify_path(&temp1.path, &file2.to_string_lossy()).unwrap_err();
        assert!(err.contains("Access Denied: Path Traversal detected"), "Unexpected error: {err}");

        #[cfg(target_os = "windows")]
        {
            let win_err = resolve_and_verify_path(&temp1.path, "C:\\Windows\\System32").unwrap_err();
            assert!(win_err.contains("Access Denied: Path Traversal detected"), "Unexpected error: {win_err}");
        }
    }

    #[test]
    fn test_resolve_and_verify_path_blocks_prefix_collision() {
        let temp = TempTestDir::new("prefix_base");
        let real_project = temp.path.join("app");
        let fake_project = temp.path.join("app_evil");

        fs::create_dir_all(&real_project).unwrap();
        fs::create_dir_all(&fake_project).unwrap();

        let evil_file = fake_project.join("payload.bat");
        File::create(&evil_file).unwrap().write_all(b"evil").unwrap();

        let err = resolve_and_verify_path(&real_project, &evil_file.to_string_lossy()).unwrap_err();
        assert!(err.contains("Access Denied: Path Traversal detected"), "Prefix collision attack was not blocked: {err}");
    }

    #[test]
    fn test_resolve_and_verify_path_nonexistent_file_inside_root() {
        let temp = TempTestDir::new("nonexistent_valid");
        let resolved = resolve_and_verify_path(&temp.path, "new_file.txt").unwrap();
        let expected = strip_unc_prefix(&temp.path.canonicalize().unwrap()).join("new_file.txt");
        assert_eq!(resolved, expected);

        let resolved_sub = resolve_and_verify_path(&temp.path, "subdir1/subdir2/new_file.txt").unwrap();
        let expected_sub = strip_unc_prefix(&temp.path.canonicalize().unwrap())
            .join("subdir1")
            .join("subdir2")
            .join("new_file.txt");
        assert_eq!(resolved_sub, expected_sub);
    }

    #[test]
    fn test_resolve_and_verify_path_blocks_nonexistent_traversal_outside() {
        let temp = TempTestDir::new("nonexistent_traversal");
        let project = temp.path.join("proj");
        fs::create_dir_all(&project).unwrap();

        let err = resolve_and_verify_path(&project, "../../nonexistent_dir/malware.bat").unwrap_err();
        assert!(err.contains("Access Denied: Path Traversal detected"), "Unexpected error: {err}");

        let err2 = resolve_and_verify_path(&project, "sub/../../../../nonexistent_dir/malware.bat").unwrap_err();
        assert!(err2.contains("Access Denied: Path Traversal detected"), "Unexpected error: {err2}");
    }

    #[test]
    fn test_file_explorer_denies_all_operations_without_active_project() {
        let explorer = DefaultFileExplorerService::standalone();

        let list_err = explorer.list_directory(".").unwrap_err();
        assert!(list_err.contains("No active project: access denied"));

        let read_err = explorer.read_file("some_file.txt").unwrap_err();
        assert!(read_err.contains("No active project: access denied"));

        let write_err = explorer.write_file("some_file.txt", "content").unwrap_err();
        assert!(write_err.contains("No active project: access denied"));
    }

    #[test]
    fn test_file_explorer_denies_operations_when_project_path_is_none() {
        let project_service = Arc::new(DefaultProjectService::new());
        project_service.set_current(ProjectContext {
            profile_name: "NoPath".into(),
            project_path: None,
            description: "".into(),
            stack: vec![],
            opened_at: "0".into(),
        });

        let explorer = DefaultFileExplorerService::new(project_service);

        let list_err = explorer.list_directory(".").unwrap_err();
        assert!(list_err.contains("Project path is not set: access denied"));

        let read_err = explorer.read_file("file.txt").unwrap_err();
        assert!(read_err.contains("Project path is not set: access denied"));

        let write_err = explorer.write_file("file.txt", "abc").unwrap_err();
        assert!(write_err.contains("Project path is not set: access denied"));
    }

    #[test]
    fn test_file_explorer_service_crud_lifecycle_and_traversal_rejection() {
        let temp = TempTestDir::new("fe_crud");
        let project_service = Arc::new(DefaultProjectService::new());
        project_service.set_current(ProjectContext {
            profile_name: "ActiveTest".into(),
            project_path: Some(temp.path.to_string_lossy().to_string()),
            description: "".into(),
            stack: vec![],
            opened_at: "100".into(),
        });

        let explorer = DefaultFileExplorerService::new(project_service.clone());

        // 1. Initial list directory is empty
        let entries = explorer.list_directory(".").unwrap();
        assert!(entries.is_empty());

        // 2. Write file inside project
        explorer.write_file("nested/hello.txt", "Hello Secure World!").unwrap();

        // 3. Read file inside project
        let content = explorer.read_file("nested/hello.txt").unwrap();
        assert_eq!(content.content, "Hello Secure World!");
        assert_eq!(content.language, "text");

        // 4. List directory shows nested
        let entries2 = explorer.list_directory(".").unwrap();
        assert_eq!(entries2.len(), 1);
        assert_eq!(entries2[0].name, "nested");
        assert!(entries2[0].is_dir);

        // 5. Attempt traversal read
        let read_traversal = explorer.read_file("../../secret.txt").unwrap_err();
        assert!(read_traversal.contains("Access Denied: Path Traversal detected"));

        // 6. Attempt traversal write
        let write_traversal = explorer.write_file("../../evil.bat", "payload").unwrap_err();
        assert!(write_traversal.contains("Access Denied: Path Traversal detected"));

        // 7. Attempt traversal list
        let list_traversal = explorer.list_directory("../..").unwrap_err();
        assert!(list_traversal.contains("Access Denied: Path Traversal detected"));

        // 8. Clear project -> immediate access denied
        project_service.clear_current();
        let denied = explorer.read_file("nested/hello.txt").unwrap_err();
        assert!(denied.contains("No active project: access denied"));
    }
}
