// ============================================================
// Linux-адаптер (заглушка — реализация на этапе 8)
// ============================================================
// Приоритет менеджеров: apt (Debian/Ubuntu), dnf (Fedora/RHEL),
// pacman (Arch). Реальная реализация установки — этап 8.

use super::PlatformAdapter;

pub struct LinuxAdapter;

impl PlatformAdapter for LinuxAdapter {
    fn os_name(&self) -> String {
        "linux".to_string()
    }

    fn package_managers(&self) -> Vec<String> {
        vec![
            "apt".to_string(),
            "dnf".to_string(),
            "pacman".to_string(),
        ]
    }
}
