// ============================================================
// Проверка диска (disk.rs) — этап 5
// ============================================================
// Свободное место на диске, куда ставятся инструменты.
//
// Сам запрос к диску OS-специфичен и живёт в платформенном слое:
//   - Windows: [System.IO.DriveInfo] (быстро, без админских прав);
//   - Linux/macOS: df -Pk (POSIX-гарантии формата: одна строка
//     на файловую систему, без переносов).
//
// Здесь — фасад core (free_space_mb), точка старта для проверки
// (install_root — диск, на котором лежит exe: туда же ставятся
// инструменты в подавляющем числе случаев) и чистые парсеры,
// которые покрыты тестами независимо от ОС.

use std::path::{Path, PathBuf};

use crate::modules::toolchain::platforms;

/// Диск, с которого работает приложение: корень пути exe
/// (Windows — «C:\», Unix — «/»). Это лучшая эвристика «куда
/// поставятся инструменты» без лишних запросов к системе.
pub fn install_root() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.components()
                .next()
                .map(|c| PathBuf::from(c.as_os_str()))
        })
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Свободное место на диске, содержащем `path`, в МБ.
pub async fn free_space_mb(path: &Path) -> Result<u64, String> {
    platforms::current_platform().free_space_mb(path).await
}

/// Парсит число байт из вывода [System.IO.DriveInfo].AvailableFreeSpace.
/// PowerShell печатает long без тысяч-разделителей.
pub fn drive_bytes_to_mb(raw: &str) -> Option<u64> {
    let bytes: u128 = raw.trim().parse().ok()?;
    Some((bytes / (1024 * 1024)) as u64)
}

/// Парсит свободное место в 1K-блоках из вывода `df -Pk`.
/// POSIX-формат: заголовок + по одной строке на ФС, доступно — 4-е поле:
///   Filesystem 1024-blocks Used Available Capacity Mounted on
///   /dev/sda1      999320 123456    876543      12% /
/// Возвращает килобайты (или None — df не опознан).
/// На Windows не используется (там DriveInfo) — отсюда и ветка allow.
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub fn df_avail_kb(raw: &str) -> Option<u64> {
    let data_line = raw.lines().find(|l| {
        let mut fields = l.split_whitespace();
        let first = fields.next();
        // строка данных начинается с пути ФС ("/" или "/dev/...")
        first.map_or(false, |f| f.starts_with('/')) && fields.count() >= 4
    })?;
    let fields: Vec<&str> = data_line.split_whitespace().collect();
    fields.get(3)?.parse().ok()
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_to_mb_rounds_down() {
        assert_eq!(drive_bytes_to_mb("1048576"), Some(1));
        assert_eq!(drive_bytes_to_mb("500000000000"), Some(476837));
        assert_eq!(drive_bytes_to_mb("junk"), None);
        assert_eq!(drive_bytes_to_mb(""), None);
    }

    #[test]
    fn df_parse_standard_output() {
        let raw = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n\
                   /dev/sda1        999320    123456    876543      12% /\n";
        assert_eq!(df_avail_kb(raw), Some(876543));
    }

    #[test]
    fn df_parse_tolerates_whitespace() {
        let raw = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                   /dev/nvme0n1p2  10485760 2097152 8388608 20% /\n";
        assert_eq!(df_avail_kb(raw), Some(8388608));
    }

    #[test]
    fn df_parse_rejects_junk() {
        assert_eq!(df_avail_kb("not df output"), None);
        assert_eq!(df_avail_kb(""), None);
    }

    #[test]
    fn df_parse_handles_multiple_fs() {
        // df <путь> печатает только ФС пути, но парсер всё равно
        // устойчив к лишним строкам — берёт первую строку данных.
        let raw = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                   /dev/sda1       100000 50000     50000  50% /boot\n\
                   /dev/sda2     10000000 4000000   6000000 40% /\n";
        assert_eq!(df_avail_kb(raw), Some(50000));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn install_root_is_drive_prefix() {
        let root = install_root();
        let s = root.to_string_lossy();
        assert!(
            s.len() == 2 && s.ends_with(':') || s.len() == 3 && s.ends_with(":\\"),
            "ожидали корень диска, получили {s}"
        );
    }
}
