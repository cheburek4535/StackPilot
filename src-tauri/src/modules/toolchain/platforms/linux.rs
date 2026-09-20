// ============================================================
// Linux-адаптер
// ============================================================
// Приоритет менеджеров: apt (Debian/Ubuntu), dnf (Fedora/RHEL),
// pacman (Arch), zypper (openSUSE). Установка исполняется
// core/installer (build_linux_pkg_command) под pkexec/sudo; этот
// адаптер отвечает за PATH, версию ОС, диск и дистрибутивные имена
// пакетов (package_alias).

use std::path::Path;

use crate::modules::toolchain::core::disk;

use super::run_command;
use super::unix_rc;
use super::PlatformAdapter;

pub struct LinuxAdapter;

/// Best-effort сопоставление id пакетов каталога (написан под
/// Debian/Ubuntu) именам в других менеджерах. Только высокоуверенные
/// пары: неизвестный id проходит без изменений (ошибка «пакет не
/// найден» честнее выдуманного имени). Возвращается имя, которое
/// реально существует в репозиториях целевого дистрибутива.
pub fn package_alias(manager: &str, package: &str) -> String {
    let mapped = match (manager, package) {
        // Fedora / RHEL-совместимые (dnf).
        ("dnf", "openjdk-21-jdk") => "java-21-openjdk-devel",
        ("dnf", "redis-server") => "redis",
        ("dnf", "docker.io") => "moby-engine",
        ("dnf", "dart") => "dart-sdk",
        ("dnf", "postgresql") => "postgresql-server",
        // Arch Linux (pacman).
        ("pacman", "python3") => "python",
        ("pacman", "openjdk-21-jdk") => "jdk-openjdk",
        ("pacman", "redis-server") => "redis",
        ("pacman", "mysql-server") => "mariadb",
        ("pacman", "docker.io") => "docker",
        ("pacman", "dotnet-sdk-10.0") => "dotnet-sdk",
        // openSUSE (zypper).
        ("zypper", "openjdk-21-jdk") => "java-21-openjdk-devel",
        ("zypper", "redis-server") => "redis",
        ("zypper", "docker.io") => "docker",
        ("zypper", "postgresql") => "postgresql-server",
        ("zypper", "mysql-server") => "mariadb",
        ("zypper", "composer") => "php-composer",
        _ => return package.to_string(),
    };
    mapped.to_string()
}

/// Стандартные системные каталоги, которые обязаны быть в PATH
/// (существующие — отсутствующие не выдумываем). Нужны для GUI-запуска:
/// .desktop-процесс может получить урезанный PATH, и apt-поставленный
/// инструмент «не найдётся» при живом /usr/bin.
pub fn system_path_dirs() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for dir in [
        "/usr/local/sbin",
        "/usr/local/bin",
        "/usr/sbin",
        "/usr/bin",
        "/sbin",
        "/bin",
        "/snap/bin",
    ] {
        if Path::new(dir).is_dir() {
            out.push(dir.to_string());
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let local_bin = format!("{home}/.local/bin");
        if Path::new(&local_bin).is_dir() {
            out.push(local_bin);
        }
    }
    out
}

#[async_trait::async_trait]
impl PlatformAdapter for LinuxAdapter {
    fn os_name(&self) -> String {
        "linux".to_string()
    }

    fn package_managers(&self) -> Vec<String> {
        vec![
            "apt".to_string(),
            "dnf".to_string(),
            "pacman".to_string(),
            "zypper".to_string(),
        ]
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
        // Дополнительно отдаём реально существующие стандартные каталоги:
        // GUI-запуск (.desktop) может получить урезанный PATH, и
        // установленный пакет «не найдётся» при живом /usr/bin.
        Ok(system_path_dirs())
    }

    async fn write_user_path(&self, dirs: &[String]) -> Result<(), String> {
        unix_rc::write_rc_user_path(dirs)
    }

    async fn os_version(&self) -> String {
        run_command("uname", &["-sr".to_string()])
            .await
            .unwrap_or_else(|_| "Linux (версия не определена)".to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_alias_maps_known_distro_names() {
        // Fedora: JDK называется java-21-openjdk-devel, redis — redis.
        assert_eq!(
            package_alias("dnf", "openjdk-21-jdk"),
            "java-21-openjdk-devel"
        );
        assert_eq!(package_alias("dnf", "redis-server"), "redis");
        assert_eq!(package_alias("dnf", "docker.io"), "moby-engine");
        // Arch: python3 → python, mysql-server → mariadb.
        assert_eq!(package_alias("pacman", "python3"), "python");
        assert_eq!(package_alias("pacman", "mysql-server"), "mariadb");
        assert_eq!(package_alias("pacman", "dotnet-sdk-10.0"), "dotnet-sdk");
        // openSUSE.
        assert_eq!(package_alias("zypper", "redis-server"), "redis");
        assert_eq!(package_alias("zypper", "docker.io"), "docker");
        // apt (Debian/Ubuntu) — каталог уже написан под эти имена.
        assert_eq!(package_alias("apt-get", "php"), "php");
        // Неизвестный пакет проходит без изменений.
        assert_eq!(package_alias("dnf", "unknown-pkg-xyz"), "unknown-pkg-xyz");
    }

    #[test]
    fn system_path_dirs_are_existing_and_absolute() {
        for dir in system_path_dirs() {
            assert!(dir.starts_with('/'), "не абсолютный: {dir}");
            assert!(Path::new(&dir).is_dir(), "не существует: {dir}");
        }
    }
}
