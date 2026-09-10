use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;

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

/// Service for browsing and editing project files.
pub trait FileExplorerService: Send + Sync {
    fn list_directory(&self, dir: &str) -> Result<Vec<FileEntry>, String>;
    fn read_file(&self, path: &str) -> Result<FileContent, String>;
    fn write_file(&self, path: &str, content: &str) -> Result<(), String>;
    fn open_in_vscode(&self, path: &str, vscode_path: Option<&str>) -> Result<(), String>;
#[allow(dead_code)]
    fn detect_language(&self, path: &str) -> String;
}

pub struct DefaultFileExplorerService;

impl DefaultFileExplorerService {
    pub fn new() -> Self {
        Self
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

impl FileExplorerService for DefaultFileExplorerService {
    fn list_directory(&self, dir: &str) -> Result<Vec<FileEntry>, String> {
        let entries =
            fs::read_dir(dir).map_err(|e| format!("Failed to read dir '{}': {}", dir, e))?;
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

            files.push(FileEntry {
                name,
                path: path.to_string_lossy().to_string(),
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
        let content =
            fs::read_to_string(path).map_err(|e| format!("Failed to read '{}': {}", path, e))?;
        let language = Self::detect_language_from_ext(path);
        Ok(FileContent { content, language })
    }

    fn write_file(&self, path: &str, content: &str) -> Result<(), String> {
        fs::write(path, content).map_err(|e| format!("Failed to write '{}': {}", path, e))
    }

    fn open_in_vscode(&self, path: &str, vscode_path: Option<&str>) -> Result<(), String> {
        // A configured-but-blank path (the settings field may be empty until
        // the user picks one) must fall back to auto-detection rather than
        // failing on an empty command.
        let configured = vscode_path.map(|s| s.trim()).filter(|s| !s.is_empty());
        let cli = configured.unwrap_or("code");
        // Resolve the CLI name to an absolute path using the shared
        // cross-platform discovery (PATH, Windows App Paths registry, macOS
        // bundles). A bare `code` fails on Windows whenever the app's PATH
        // lacks the user's shell additions вЂ” resolution fixes that.
        let resolved = crate::platform::ide::resolve_ide_executable(cli).ok_or_else(|| {
            format!(
                "Failed to open VSCode: '{}' was not found on this system. \
                     Make sure the path is correct in Settings в†’ System.",
                cli
            )
        })?;

        let result = if cfg!(target_os = "windows") {
            if crate::platform::paths::is_batch_file(&resolved) {
                // `.cmd` shims must run through cmd.exe (CreateProcess cannot
                // execute batch files directly).
                let args = crate::platform::command::batch_shim_cmd_line(&resolved, &[path]);
                Command::new("cmd").args(args).spawn()
            } else {
                Command::new(&resolved).arg(path).spawn()
            }
        } else {
            Command::new(&resolved).arg(path).spawn()
        };
        match result {
            Ok(_) => Ok(()),
            Err(e) => Err(format!(
                "Failed to open VSCode ({}): {}. Make sure the path is correct in Settings в†’ System.",
                resolved, e
            )),
        }
    }

    fn detect_language(&self, path: &str) -> String {
        Self::detect_language_from_ext(path)
    }
}
