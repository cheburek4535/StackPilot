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
#[cfg(unix)]
pub mod unix_rc;

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

/// Единый интерфейс платформенного адаптера.
#[async_trait::async_trait]
pub trait PlatformAdapter: Send + Sync {
    /// Имя ОС для отчётов ("windows", "linux", "macos")
    fn os_name(&self) -> String;
    /// Доступные менеджеры пакетов в порядке приоритета.
    /// Это список «известных» менеджеров платформы; реальное наличие
    /// каждого на конкретной машине проверяет DiscoveryService.
    fn package_managers(&self) -> Vec<String>;
    /// Разделитель записей в PATH (";" на Windows, ":" на Unix).
    fn path_separator(&self) -> String;
    /// PATH пользователя (Windows: HKCU\Environment; Unix: rc-файл с
    /// маркером StackPilot). Записи могут содержать %VAR% — их
    /// раскрывает path_service::expand_env_vars.
    async fn read_user_path(&self) -> Result<Vec<String>, String>;
    /// Системный PATH (Windows: HKLM; Unix: неразличим — пустой список,
    /// базовым считаем текущий PATH процесса).
    async fn read_system_path(&self) -> Result<Vec<String>, String>;
    /// Полностью перезаписывает PATH пользователя переданными записями.
    async fn write_user_path(&self, dirs: &[String]) -> Result<(), String>;
    /// Версия ОС для отчётов, например "Microsoft Windows 11 Pro (10.0.22631)".
    async fn os_version(&self) -> String;
    /// Свободное место на диске, содержащем путь, в МБ.
    async fn free_space_mb(&self, path: &Path) -> Result<u64, String>;
}

/// Общий запуск команды с таймаутом (для быстрых проб платформы:
/// PowerShell/df/uname). Возвращает stdout (trim) или понятную ошибку.
/// Повторяет паттерн discovery::run_capture, но с именами аргументов
/// String — так удобнее собирать PowerShell-скрипты.
pub(crate) async fn run_command(program: &str, args: &[String]) -> Result<String, String> {
    let output = timeout(
        Duration::from_secs(30),
        TokioCommand::new(program).args(args).output(),
    )
    .await
    .map_err(|_| format!("Таймаут команды {program}"))?
    .map_err(|e| format!("Не удалось запустить {program}: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "{program} завершился с кодом {}",
            output.status.code().unwrap_or(-1)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Адаптер для неподдерживаемых ОС — ничего не умеет, но не падает.
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
pub struct UnsupportedAdapter;

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
#[async_trait::async_trait]
impl PlatformAdapter for UnsupportedAdapter {
    fn os_name(&self) -> String {
        std::env::consts::OS.to_string()
    }
    fn package_managers(&self) -> Vec<String> {
        Vec::new()
    }
    fn path_separator(&self) -> String {
        ".".to_string()
    }
    async fn read_user_path(&self) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
    async fn read_system_path(&self) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
    async fn write_user_path(&self, _dirs: &[String]) -> Result<(), String> {
        Err("PATH не поддерживается на этой ОС".to_string())
    }
    async fn os_version(&self) -> String {
        std::env::consts::OS.to_string()
    }
    async fn free_space_mb(&self, _path: &Path) -> Result<u64, String> {
        Err("Проверка диска не поддерживается на этой ОС".to_string())
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
