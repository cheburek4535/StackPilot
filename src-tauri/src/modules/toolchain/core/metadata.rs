// ============================================================
// Локальное хранилище состояния (metadata.rs) — этап 5
// ============================================================
// Тонкий слой над ToolchainMetadata: читает/пишет
// <app_data>/toolchain/state.json.
//
// Что храним:
//   - tools   — что и когда установлено (версия, путь, каталоги PATH);
//   - secrets — пароль PostgreSQL и прочие секреты, сгенерированные
//               при установке. В проект никогда не попадают;
//   - prefs   — свободные пользовательские настройки.
//
// Гарантии:
//   - файл отсутствует/битый → пустое состояние (не падаем);
//   - запись через временный файл + rename — не развалится на
//     полпути (и app не потеряет данные при сбое);
//   - работаем только с app_data-каталогом приложения — файл
//     принадлежит пользователю, поэтому секреты здесь хранимы,
//     но ничего не отправляется наружу.

use std::path::{Path, PathBuf};

use crate::modules::toolchain::models::{InstalledToolInfo, ToolchainMetadata};

/// Путь к файлу состояния относительно корня данных.
const STATE_FILE: &str = "state.json";

pub struct MetadataStore {
    path: PathBuf,
    data: ToolchainMetadata,
}

impl MetadataStore {
    /// Загружает состояние из `dir` (создаёт каталог, если нет).
    /// Битый или отсутствующий файл — не ошибка: начинаем с чистого.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(STATE_FILE);
        let data = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        Self { path, data }
    }

    /// Атомарная запись на диск (temp-файл + rename).
    pub fn save(&self) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| format!("Нет родительского каталога для {}", self.path.display()))?;
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Не удалось создать {}: {e}", parent.display()))?;

        let pretty = serde_json::to_string_pretty(&self.data)
            .map_err(|e| format!("Не удалось сериализовать state.json: {e}"))?;

        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, pretty)
            .map_err(|e| format!("Не удалось записать {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("Не удалось сохранить {}: {e}", self.path.display()))?;
        Ok(())
    }

    /// Полная копия данных (для команд tauri и отладки).
    pub fn data(&self) -> &ToolchainMetadata {
        &self.data
    }

    /// Записывает факт установки инструмента (перезаписывает прежнюю).
    pub fn record_tool_installed(&mut self, tool_id: &str, info: InstalledToolInfo) {
        self.data.tools.insert(tool_id.to_string(), info);
    }

    /// Информация об установленном инструменте, если известна.
    /// Покрыто тестами; станет основным читателем на этапе 8
    /// (подстановка версий/путей при генерации проекта).
    #[allow(dead_code)]
    pub fn tool(&self, tool_id: &str) -> Option<&InstalledToolInfo> {
        self.data.tools.get(tool_id)
    }

    /// Сохраняет секрет (например пароль PostgreSQL).
    pub fn set_secret(&mut self, key: &str, value: &str) {
        self.data.secrets.insert(key.to_string(), value.to_string());
    }

    /// Покрыто тестами; этап 8 — пароль PostgreSQL для генерации БД.
    #[allow(dead_code)]
    pub fn get_secret(&self, key: &str) -> Option<&str> {
        self.data.secrets.get(key).map(|s| s.as_str())
    }

    /// Все секреты — для передачи в сессию установки.
    /// Покрыто тестами; понадобится странице окружения.
    #[allow(dead_code)]
    pub fn secrets(&self) -> &std::collections::HashMap<String, String> {
        &self.data.secrets
    }

    /// Отмечает время последней проверки окружения.
    pub fn touch_last_scan(&mut self, at: String) {
        self.data.last_scan = Some(at);
    }
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::toolchain::core::console::timestamp;

    /// Уникальный временный каталог под тест (без внешних крейтов).
    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tc-meta-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_info() -> InstalledToolInfo {
        InstalledToolInfo {
            path: "C:\\Program Files\\nodejs".to_string(),
            version: "v22.12.0".to_string(),
            installed_at: timestamp(),
            path_entries: vec!["C:\\Program Files\\nodejs".to_string()],
        }
    }

    #[test]
    fn load_missing_file_gives_default() {
        let store = MetadataStore::load(&temp_dir("missing"));
        assert!(store.data().tools.is_empty());
        assert!(store.data().secrets.is_empty());
        assert!(store.data().last_scan.is_none());
    }

    #[test]
    fn load_broken_file_gives_default() {
        let dir = temp_dir("broken");
        std::fs::write(dir.join(STATE_FILE), "{ не json").unwrap();
        let store = MetadataStore::load(&dir);
        assert!(store.data().tools.is_empty());
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let dir = temp_dir("roundtrip");
        let mut store = MetadataStore::load(&dir);

        store.record_tool_installed("node", sample_info());
        store.set_secret("postgres_password", "0123456789abcdef");
        store.touch_last_scan("2026-08-05T12:00:00Z".to_string());
        store.save().unwrap();

        // «новый запуск приложения»
        let reloaded = MetadataStore::load(&dir);
        assert_eq!(reloaded.tool("node").unwrap().version, "v22.12.0");
        assert_eq!(reloaded.tool("node").unwrap().path, "C:\\Program Files\\nodejs");
        assert_eq!(reloaded.get_secret("postgres_password"), Some("0123456789abcdef"));
        assert_eq!(reloaded.data().last_scan.as_deref(), Some("2026-08-05T12:00:00Z"));
    }

    #[test]
    fn record_overwrites_previous() {
        let dir = temp_dir("overwrite");
        let mut store = MetadataStore::load(&dir);
        store.record_tool_installed("node", sample_info());
        let mut updated = sample_info();
        updated.version = "v23.0.0".to_string();
        store.record_tool_installed("node", updated);

        assert_eq!(store.tool("node").unwrap().version, "v23.0.0");
        assert_eq!(store.tool("node").unwrap().path_entries.len(), 1);
    }
}
