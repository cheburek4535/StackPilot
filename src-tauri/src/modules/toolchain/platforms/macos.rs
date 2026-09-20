// ============================================================
// macOS-адаптер
// ============================================================
// Homebrew — единственный «настоящий» менеджер пакетов macOS,
// который умеет тихую установку (этап 8). Системные сервисы
// этапа 5 (PATH, версия ОС, диск) уже работают.

use std::path::Path;

use crate::modules::toolchain::core::disk;

use super::run_command;
use super::unix_rc;
use super::PlatformAdapter;

pub struct MacosAdapter;

#[async_trait::async_trait]
impl PlatformAdapter for MacosAdapter {
    fn os_name(&self) -> String {
        "macos".to_string()
    }

    fn package_managers(&self) -> Vec<String> {
        vec!["brew".to_string()]
    }

    fn path_separator(&self) -> String {
        ":".to_string()
    }

    async fn read_user_path(&self) -> Result<Vec<String>, String> {
        Ok(unix_rc::read_rc_user_path())
    }

    /// «Системный» PATH macOS: приложение, запущенное из Finder/Dock,
    /// получает урезанный PATH (/usr/bin:/bin:/usr/sbin:/sbin), и
    /// brew-инструменты для него не существуют. Собираем честный список:
    /// /etc/paths + /etc/paths.d/* (штатный механизм macOS) плюс реально
    /// существующие каталоги Homebrew (Apple Silicon и Intel).
    async fn read_system_path(&self) -> Result<Vec<String>, String> {
        Ok(macos_system_path_dirs())
    }

    async fn write_user_path(&self, dirs: &[String]) -> Result<(), String> {
        unix_rc::write_rc_user_path(dirs)
    }

    async fn os_version(&self) -> String {
        run_command("uname", &["-sr".to_string()])
            .await
            .unwrap_or_else(|_| "macOS (версия не определена)".to_string())
    }

    async fn free_space_mb(&self, path: &Path) -> Result<u64, String> {
        let raw = run_command(
            "df",
            &["-Pk".to_string(), path.to_string_lossy().into_owned()],
        )
        .await?;
        disk::df_avail_kb(&raw)
            .map(|kb| kb / 1024)
            .ok_or_else(|| format!("Не разобрать вывод df:\n{raw}"))
    }
}

/// Существующие каталоги системного PATH macOS. /etc/paths и
/// /etc/paths.d/* — штатные файлы ОС; Homebrew добавляется отдельно,
/// потому что Finder-процесс о нём не знает. Отсутствующие каталоги
/// не выдумываются.
pub fn macos_system_path_dirs() -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();

    let mut push = |dir: &str| {
        if Path::new(dir).is_dir() && !dirs.iter().any(|d| d == dir) {
            dirs.push(dir.to_string());
        }
    };

    if let Ok(content) = std::fs::read_to_string("/etc/paths") {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            push(line);
        }
    }
    if let Ok(entries) = std::fs::read_dir("/etc/paths.d") {
        let mut files: Vec<std::path::PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        for file in files {
            if let Ok(content) = std::fs::read_to_string(&file) {
                for line in content.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    push(line);
                }
            }
        }
    }

    // Homebrew: Apple Silicon (/opt/homebrew) и Intel (/usr/local).
    for dir in [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
    ] {
        push(dir);
    }

    dirs
}
