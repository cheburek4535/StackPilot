// ============================================================
// Windows-адаптер
// ============================================================
// Первичный менеджер пакетов — winget. Если его нет на машине,
// Toolchain Manager сначала ставит сам winget (см. tools.json,
// инструмент "winget"), а уже потом использует его.
// Второстепенный — choco (если вдруг установлен).

use super::PlatformAdapter;

pub struct WindowsAdapter;

impl PlatformAdapter for WindowsAdapter {
    fn os_name(&self) -> String {
        "windows".to_string()
    }

    fn package_managers(&self) -> Vec<String> {
        vec!["winget".to_string(), "choco".to_string()]
    }
}
