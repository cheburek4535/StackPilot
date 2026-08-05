// ============================================================
// Windows-адаптер
// ============================================================
// Первичный менеджер пакетов — winget. Если его нет на машине,
// Toolchain Manager сначала ставит сам winget (см. tools.json,
// инструмент "winget"), а уже потом использует его.
// Второстепенный — choco (если вдруг установлен).
//
// Этап 5 — системные сервисы:
//   - PATH: читаем/пишем пользовательский PATH через [Environment]
//     (.NET корректно работает с реестром HKCU\Environment и сам
//     рассылает WM_SETTINGCHANGE — Explorer узнает об изменении);
//   - версия ОС: Get-CimInstance Win32_OperatingSystem;
//   - свободное место: [System.IO.DriveInfo].
//
// Все системные операции асинхронные (tokio + таймаут) — UI не моргает.

use std::path::Path;

use crate::modules::toolchain::core::console::ps_quote;
use crate::modules::toolchain::core::disk;

use super::run_command;
use super::PlatformAdapter;

pub struct WindowsAdapter;

/// Скрипт-«полуфабрикат»: читает переменную окружения в нужной сфере.
fn read_env_script(scope: &str) -> String {
    format!("[Environment]::GetEnvironmentVariable('Path','{scope}')")
}

/// Скрипт: полностью заменяет PATH пользователя на переданный список.
/// Значения экранируются ps_quote (одинарные кавычки), %VAR% при этом
/// не раскрываются — реестр хранит их как есть, раскрывает сам Windows.
fn write_user_path_script(dirs: &[String]) -> String {
    let quoted: Vec<String> = dirs.iter().map(|d| ps_quote(d)).collect();
    format!(
        "[Environment]::SetEnvironmentVariable('Path', ({} -join ';'), 'User')",
        quoted.join(",")
    )
}

#[async_trait::async_trait]
impl PlatformAdapter for WindowsAdapter {
    fn os_name(&self) -> String {
        "windows".to_string()
    }

    fn package_managers(&self) -> Vec<String> {
        vec!["winget".to_string(), "choco".to_string()]
    }

    fn path_separator(&self) -> String {
        ";".to_string()
    }

    async fn read_user_path(&self) -> Result<Vec<String>, String> {
        let raw = run_command(
            "powershell",
            &[
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                read_env_script("User"),
            ],
        )
        .await?;
        Ok(split_path(&raw))
    }

    async fn read_system_path(&self) -> Result<Vec<String>, String> {
        let raw = run_command(
            "powershell",
            &[
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                read_env_script("Machine"),
            ],
        )
        .await?;
        Ok(split_path(&raw))
    }

    async fn write_user_path(&self, dirs: &[String]) -> Result<(), String> {
        run_command(
            "powershell",
            &[
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                write_user_path_script(dirs),
            ],
        )
        .await
        .map(|_| ())
    }

    async fn os_version(&self) -> String {
        let script = "$o = Get-CimInstance Win32_OperatingSystem; '{0} ({1})' -f $o.Caption, $o.Version"
            .to_string();
        run_command(
            "powershell",
            &[
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                script,
            ],
        )
        .await
        .unwrap_or_else(|_| "Windows (версия не определена)".to_string())
    }

    async fn free_space_mb(&self, path: &Path) -> Result<u64, String> {
        // Из пути берём корень диска: "C:\foo" → "C:\".
        let mut root = path
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .ok_or_else(|| format!("Нет корня диска в пути {}", path.display()))?;
        if root.ends_with(':') {
            root.push('\\');
        }

        let script = format!("[System.IO.DriveInfo]::new({}).AvailableFreeSpace", ps_quote(&root));
        let raw = run_command(
            "powershell",
            &[
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                script,
            ],
        )
        .await?;
        disk::drive_bytes_to_mb(&raw).ok_or_else(|| format!("Не разобрать вывод DriveInfo: {raw}"))
    }
}

/// Делит строку PATH по ';', убирает пустые и пробелы по краям.
fn split_path(raw: &str) -> Vec<String> {
    raw.split(';')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_path_skips_empties() {
        assert_eq!(
            split_path("C:\\a;C:\\b;; C:\\c ;"),
            vec!["C:\\a".to_string(), "C:\\b".to_string(), "C:\\c".to_string()]
        );
    }

    #[test]
    fn user_path_script_quotes_dirs() {
        let script = write_user_path_script(&[
            "C:\\Program Files\\nodejs".to_string(),
            "O'Brien".to_string(),
        ]);
        assert_eq!(
            script,
            "[Environment]::SetEnvironmentVariable('Path', ('C:\\Program Files\\nodejs','O''Brien' -join ';'), 'User')"
        );
    }
}
