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

    async fn read_system_path(&self) -> Result<Vec<String>, String> {
        // На Unix системный и пользовательский PATH неразличимы —
        // базой считаем текущий PATH процесса (его берёт path_service).
        Ok(Vec::new())
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
        let raw = run_command("df", &["-Pk".to_string(), path.to_string_lossy().into_owned()])
            .await?;
        disk::df_avail_kb(&raw)
            .map(|kb| kb / 1024)
            .ok_or_else(|| format!("Не разобрать вывод df:\n{raw}"))
    }
}
