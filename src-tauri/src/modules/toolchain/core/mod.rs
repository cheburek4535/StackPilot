// ============================================================
// Ядро Toolchain Manager — бизнес-логика без привязки к ОС
// ============================================================
// Сервисы ядра (жирным — уже реализованы, остальное — очередь):
//   - version.rs      — парсинг и сравнение версий ✔
//   - discovery.rs    — поиск установленного ПО (PATH, пути, реестр, пробы) ✔
//   - requirements.rs — ProjectRequirements (языки/фреймворки/тулы) → тулы ✔
//   - check.rs        — сборка EnvironmentCheck под требования проекта ✔
//   - planner.rs      — сборка плана установки ✔
//   - installer.rs    — исполнение плана (скачивание, стриминг, verify) ✔
//   - console.rs      — запуск процессов, отмена, прогресс, UAC-элевация ✔
//   - path_service.rs — PATH и переменные окружения ✔
//   - disk.rs         — проверка свободного места ✔
//   - metadata.rs     — локальное хранилище state.json ✔
//   - health.rs       — health-отчёты ✔
//
// Правило: core никогда не использует cfg!(target_os) напрямую.
// Всё платформенное доступно через crate::modules::toolchain::platforms.

pub mod check;
pub mod console;
pub mod discovery;
pub mod disk;
pub mod health;
pub mod installer;
pub mod metadata;
pub mod path_service;
pub mod planner;
pub mod requirements;
pub mod version;
