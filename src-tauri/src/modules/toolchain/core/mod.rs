// ============================================================
// Ядро Toolchain Manager — бизнес-логика без привязки к ОС
// ============================================================
// Сервисы ядра:
//   - version.rs      — парсинг и сравнение версий
//   - discovery.rs    — поиск установленного ПО (PATH, пути, реестр, пробы)
//   - requirements.rs — ProjectRequirements (языки/фреймворки/тулы) → тулы
//   - check.rs        — сборка EnvironmentCheck под требования проекта
//   - planner.rs      — (этап 3) сборка плана установки
//   - installer.rs    — (этап 3) исполнение плана
//   - console.rs      — (этап 4) запуск процессов, интерактив, стриминг
//   - path_service.rs — (этап 5) PATH и переменные окружения
//   - disk.rs         — (этап 5) проверка свободного места
//   - metadata.rs     — (этап 5) локальное хранилище state.json
//   - health.rs       — (этап 6) health-отчёты
//
// Правило: core никогда не использует cfg!(target_os) напрямую.
// Всё платформенное доступно через crate::modules::toolchain::platforms.

pub mod check;
pub mod discovery;
pub mod requirements;
pub mod version;
