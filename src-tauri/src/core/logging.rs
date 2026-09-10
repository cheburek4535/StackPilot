//! Лёгкий логгер приложения: дублирует записи в stderr (для разработки) и в
//! файл `<app_data>/logs/stackpilot.log` (для диагностики у пользователей).
//! Зависит только от `log` — внешний крейт-логгер не нужен.

use log::{LevelFilter, Log, Metadata, Record};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

struct StackPilotLogger {
    file: Mutex<Option<std::fs::File>>,
}

impl Log for StackPilotLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} [{:<5}] {}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            record.level(),
            record.args()
        );
        eprint!("{line}");
        if let Ok(mut guard) = self.file.lock() {
            if let Some(f) = guard.as_mut() {
                let _ = f.write_all(line.as_bytes());
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut guard) = self.file.lock() {
            if let Some(f) = guard.as_mut() {
                let _ = f.flush();
            }
        }
    }
}

static LOGGER: StackPilotLogger = StackPilotLogger {
    file: Mutex::new(None),
};

/// Инициализировать глобальный логгер. Файл логов — `<data_dir>/logs/stackpilot.log`
/// (создаётся при первом запуске). Повторные вызовы безопасны: каталог и файл
/// переоткрываются, старые записи дописываются.
pub fn init(data_dir: &Path) {
    let log_dir = data_dir.join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let path = log_dir.join("stackpilot.log");
    let file = OpenOptions::new().create(true).append(true).open(&path).ok();
    *LOGGER
        .file
        .lock()
        .expect("logger mutex poisoned; no code inside can panic") = file;
    let _ = log::set_logger(&LOGGER);
    let level = if cfg!(debug_assertions) {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    log::set_max_level(level);
    log::info!("logging initialized: {}", path.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_writes_log_file() {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_log_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        init(&dir);
        log::info!("logging smoke test marker");
        log::logger().flush();

        let path = dir.join("logs").join("stackpilot.log");
        let content = std::fs::read_to_string(&path)
            .expect("лог-файл обязан создаться после init");
        assert!(
            content.contains("logging smoke test marker"),
            "маркер обязан попасть в файл: {content}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}