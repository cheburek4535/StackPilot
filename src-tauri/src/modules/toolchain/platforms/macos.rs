// ============================================================
// macOS-адаптер (заглушка — реализация на этапе 8)
// ============================================================
// Homebrew — единственный «настоящий» менеджер пакетов macOS,
// который умеет тихую установку. Реализация — этап 8.

use super::PlatformAdapter;

pub struct MacosAdapter;

impl PlatformAdapter for MacosAdapter {
    fn os_name(&self) -> String {
        "macos".to_string()
    }

    fn package_managers(&self) -> Vec<String> {
        vec!["brew".to_string()]
    }
}
