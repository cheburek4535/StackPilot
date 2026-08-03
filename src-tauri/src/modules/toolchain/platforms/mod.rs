// ============================================================
// Платформенный слой Toolchain Manager
// ============================================================
// Правило архитектуры из манифеста: core/ не содержит
// OS-специфичного кода. Всё, что зависит от ОС, живёт здесь
// и наружу не торчит: core и commands работают через PlatformAdapter.
//
// Добавление новой ОС = новый адаптер в этой папке,
// бизнес-логика при этом не меняется.

#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;

use std::sync::OnceLock;

/// Единый интерфейс платформенного адаптера.
pub trait PlatformAdapter: Send + Sync {
    /// Имя ОС для отчётов ("windows", "linux", "macos")
    fn os_name(&self) -> String;
    /// Доступные менеджеры пакетов в порядке приоритета.
    /// Это список «известных» менеджеров платформы; реальное наличие
    /// каждого на конкретной машине проверяет DiscoveryService.
    fn package_managers(&self) -> Vec<String>;
}

/// Адаптер для неподдерживаемых ОС — ничего не умеет, но не падает.
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
pub struct UnsupportedAdapter;

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
impl PlatformAdapter for UnsupportedAdapter {
    fn os_name(&self) -> String {
        std::env::consts::OS.to_string()
    }
    fn package_managers(&self) -> Vec<String> {
        Vec::new()
    }
}

static CURRENT_PLATFORM: OnceLock<&'static dyn PlatformAdapter> = OnceLock::new();

/// Активный адаптер текущей ОС. Выбирается один раз при первом обращении
/// (OnceLock) и кэшируется на всё время работы приложения.
pub fn current_platform() -> &'static dyn PlatformAdapter {
    *CURRENT_PLATFORM.get_or_init(|| {
        #[cfg(target_os = "windows")]
        {
            &windows::WindowsAdapter
        }
        #[cfg(target_os = "linux")]
        {
            &linux::LinuxAdapter
        }
        #[cfg(target_os = "macos")]
        {
            &macos::MacosAdapter
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        {
            &UnsupportedAdapter
        }
    })
}
