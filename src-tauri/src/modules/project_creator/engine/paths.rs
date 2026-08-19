//! Безопасное разрешение путей проекта.
//!
//! Все пути шагов рецепта (write_file, working_dir команд, условия
//! FileExists/FileNotExists, expected_outputs генераторов) трактуются как
//! ОТНОСИТЕЛЬНЫЕ пути проекта и обязаны оставаться ВНУТРИ корня проекта:
//!   - обратные слеши нормализуются в прямые (условие `backend\package.json`
//!     работает на любой платформе);
//!   - пути, выходящие за корень (`..`) или абсолютные (включая диски
//!     `C:\...`), отвергаются — вернуть значение за пределы проекта нельзя;
//!   - пробелы и прочие символы пути сохраняются как есть (PathBuf не
//!     разбивает строки).

use std::path::{Path, PathBuf};

/// Нормализовать относительный путь проекта: `\` → `/`, схлопнуть пустые и
/// `.`-сегменты. Возвращает `None`, если путь пустой, абсолютный
/// (включая диски вида `C:`/`C:/...`), или содержит `..` (выход за корень).
pub fn normalize_rel_path(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    // Ведущий разделитель (/etc, \etc) — абсолютный путь Unix/Windows.
    if path.starts_with('/') || path.starts_with('\\') {
        return None;
    }
    // Диск-префикс (`C:`, `C:\...`) — абсолютный путь Windows; на Unix
    // `C:/x` не распознался бы std::path как абсолютный, поэтому проверяем
    // префикс явно.
    let head = path.chars().take(2).collect::<String>();
    if head.len() == 2
        && head.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && head.chars().nth(1) == Some(':')
    {
        return None;
    }
    let mut normalized: Vec<String> = Vec::new();
    for raw_segment in path.split(['/', '\\']) {
        match raw_segment {
            "" | "." => {}
            ".." => return None,
            segment => normalized.push(segment.to_string()),
        }
    }
    if normalized.is_empty() {
        return None;
    }
    let joined = normalized.join("/");
    let candidate = Path::new(&joined);
    // Начальный `/` (Unix-абсолютный) или `\` на любой платформе.
    if candidate.is_absolute() || joined.starts_with('/') || joined.starts_with('\\') {
        return None;
    }
    Some(joined)
}

/// Разрешить относительный путь строго внутри `root` (безопасно против
/// выхода за корень и абсолютных путей). `None` — путь недопустим.
pub fn resolve_in_root(root: &Path, rel: &str) -> Option<PathBuf> {
    let normalized = normalize_rel_path(rel)?;
    Some(root.join(normalized))
}

/// Разрешить рабочую директорию команды относительно корня проекта.
/// Абсолютные пути (рецепты используют `project_path.to_string()` и
/// `project_path + "/segment"`) допускаются ТОЛЬКО внутри корня проекта;
/// относительные — через `resolve_in_root` (выход за корень отклоняется).
pub fn resolve_working_dir(root: &Path, wd: &str) -> Result<PathBuf, String> {
    if wd.is_empty() || wd == "." {
        return Ok(root.to_path_buf());
    }
    let wd_path = Path::new(wd);
    let drive_letter =
        wd.len() >= 2 && wd.as_bytes()[0].is_ascii_alphabetic() && wd.as_bytes()[1] == b':';
    let wd_abs = wd_path.is_absolute() || drive_letter;
    if wd_abs {
        if root.is_relative() {
            return Err(format!(
                "working directory '{}' is absolute but the project root '{}' is relative",
                wd,
                root.display()
            ));
        }
        let wd_norm = wd.replace('\\', "/");
        let root_norm = root.to_string_lossy().replace('\\', "/");
        let (wd_check, root_check) = if cfg!(windows) {
            (wd_norm.to_lowercase(), root_norm.to_lowercase())
        } else {
            (wd_norm, root_norm)
        };
        if wd_check == root_check || wd_check.starts_with(&format!("{}/", root_check)) {
            return Ok(wd_path.to_path_buf());
        }
        return Err(format!(
            "working directory '{}' is outside the project root '{}'",
            wd,
            root.display()
        ));
    }
    resolve_in_root(root, wd)
        .ok_or_else(|| format!("working directory '{}' escapes the project root", wd))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_backslashes_and_dots() {
        assert_eq!(normalize_rel_path("a\\b\\c"), Some("a/b/c".into()));
        assert_eq!(normalize_rel_path("a/./b"), Some("a/b".into()));
        assert_eq!(normalize_rel_path("a//b///c"), Some("a/b/c".into()));
        assert_eq!(
            normalize_rel_path("./frontend/package.json"),
            Some("frontend/package.json".into())
        );
    }

    #[test]
    fn normalize_preserves_spaces() {
        assert_eq!(
            normalize_rel_path("my folder/sub/file.json"),
            Some("my folder/sub/file.json".into())
        );
        assert_eq!(
            normalize_rel_path("folder with spaces\\file.txt"),
            Some("folder with spaces/file.txt".into())
        );
    }

    #[test]
    fn normalize_rejects_traversal() {
        assert_eq!(normalize_rel_path(".."), None);
        assert_eq!(normalize_rel_path("../escape.txt"), None);
        assert_eq!(normalize_rel_path("a/../b"), None);
        assert_eq!(normalize_rel_path("a\\..\\b"), None);
        assert_eq!(normalize_rel_path("..\\..\\Windows\\win.ini"), None);
    }

    #[test]
    fn normalize_rejects_absolute_and_empty() {
        assert_eq!(normalize_rel_path(""), None);
        assert_eq!(normalize_rel_path("/etc/passwd"), None);
        assert_eq!(normalize_rel_path("\\etc"), None);
        assert_eq!(normalize_rel_path("C:\\Windows\\x"), None);
        assert_eq!(normalize_rel_path("C:/Windows/x"), None);
        assert_eq!(normalize_rel_path("D:"), None);
    }

    #[test]
    fn resolve_in_root_stays_inside_root() {
        let root = Path::new("C:\\dev\\my app");
        assert_eq!(
            resolve_in_root(root, "frontend\\package.json"),
            Some(PathBuf::from("C:\\dev\\my app\\frontend/package.json"))
        );
        assert!(resolve_in_root(root, "../outside").is_none());
        assert!(resolve_in_root(root, "C:\\Windows").is_none());
        assert!(resolve_in_root(root, "").is_none());
    }

    #[test]
    fn resolve_working_dir_accepts_absolute_inside_root() {
        let root = Path::new("C:\\dev\\myapp");
        assert_eq!(
            resolve_working_dir(root, "C:\\dev\\myapp").unwrap(),
            root.to_path_buf()
        );
        assert_eq!(
            resolve_working_dir(root, "C:\\dev\\myapp/frontend").unwrap(),
            PathBuf::from("C:\\dev\\myapp/frontend")
        );
        assert_eq!(resolve_working_dir(root, ".").unwrap(), root.to_path_buf());
        assert_eq!(
            resolve_working_dir(root, "frontend").unwrap(),
            PathBuf::from("C:\\dev\\myapp\\frontend")
        );
    }

    #[test]
    fn resolve_working_dir_rejects_escaping() {
        let root = Path::new("C:\\dev\\myapp");
        assert!(resolve_working_dir(root, "C:\\dev\\other").is_err());
        assert!(resolve_working_dir(root, "..\\other").is_err());
        assert!(resolve_working_dir(root, "C:\\").is_err());
    }
}
