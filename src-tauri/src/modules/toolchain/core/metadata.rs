// ============================================================
// Локальное хранилище состояния (metadata.rs) — этап 5
// ============================================================
// Тонкий слой над ToolchainMetadata: читает/пишет
// <app_data>/toolchain/state.json.
//
// Что храним:
//   - tools   — что и когда установлено (версия, путь, каталоги PATH);
//   - prefs   — свободные пользовательские настройки.
//
// СЕКРЕТЫ здесь больше НЕ хранятся: они живут в изолированном
// хранилище core/secrets.rs (отдельный файл, DPAPI на Windows).
// Поле secrets в ToolchainMetadata осталось только для обратной
// совместимости загрузки старых state.json: при загрузке plaintext-
// секреты переносятся в SecretStore и вычищаются из state.json.
//
// Гарантии:
//   - файл отсутствует/битый → пустое состояние (не падаем);
//   - запись через временный файл + rename — не развалится на
//     полпути;
//   - наружу отдаётся только ToolchainMetadataView (без секретов).

use std::path::{Path, PathBuf};

use crate::modules::toolchain::models::{
    InstalledToolInfo, ToolchainMetadata, ToolchainMetadataView,
};

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

        // Секреты в state.json не пишутся никогда: даже унаследованное
        // поле при сохранении обнуляется (миграция в SecretStore).
        let mut snapshot = self.data.clone();
        snapshot.secrets.clear();
        let pretty = serde_json::to_string_pretty(&snapshot)
            .map_err(|e| format!("Не удалось сериализовать state.json: {e}"))?;

        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, pretty)
            .map_err(|e| format!("Не удалось записать {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("Не удалось сохранить {}: {e}", self.path.display()))?;
        Ok(())
    }

    /// Санитизированная копия данных для команд tauri: БЕЗ секретов.
    /// Это единственная форма, покидающая бэкенд через tc_get_metadata.
    pub fn view(&self) -> ToolchainMetadataView {
        ToolchainMetadataView::from(&self.data)
    }

    /// Полная копия данных (только для внутренних потребителей ядра;
    /// поле secrets после миграции всегда пустое).
    pub fn data(&self) -> &ToolchainMetadata {
        &self.data
    }

    /// Унаследованные plaintext-секреты из старого state.json (для
    /// однократной миграции в SecretStore при старте приложения).
    /// Единственный легальный способ достать legacy-секреты из стора:
    /// после миграции поле всегда пустое и в view не попадает.
    pub fn take_legacy_secrets(&mut self) -> std::collections::HashMap<String, String> {
        std::mem::take(&mut self.data.secrets)
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

    /// Отмечает время последней проверки окружения.
    pub fn touch_last_scan(&mut self, at: String) {
        self.data.last_scan = Some(at);
    }

    /// Явное «усыновление» найденной ручной установки (adopt/track).
    /// Не создаёт запись об установке StackPilot — только метку
    /// наблюдения с моментом усыновления. Состояние выдаётся наружу
    /// через ToolchainMetadataView.adopted (tc_get_metadata).
    pub fn record_adoption(&mut self, tool_id: &str, at: String) {
        self.data.adopted.insert(tool_id.to_string(), at);
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
        store.touch_last_scan("2026-08-05T12:00:00Z".to_string());
        store.save().unwrap();

        // «новый запуск приложения»
        let reloaded = MetadataStore::load(&dir);
        assert_eq!(reloaded.tool("node").unwrap().version, "v22.12.0");
        assert_eq!(
            reloaded.tool("node").unwrap().path,
            "C:\\Program Files\\nodejs"
        );
        assert_eq!(
            reloaded.data().last_scan.as_deref(),
            Some("2026-08-05T12:00:00Z")
        );
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

    /// Регрессия безопасности: секреты не должны попадать в state.json
    /// и в санитизированную выдачу tc_get_metadata.
    #[test]
    fn view_excludes_secrets_and_save_strips_them() {
        let dir = temp_dir("no-secrets");
        let mut store = MetadataStore::load(&dir);

        // Симулируем старый state.json с plaintext-секретом.
        store.data.secrets.insert(
            "postgres_password".to_string(),
            "LegacyPw123456".to_string(),
        );

        // Выдача наружу секрета не содержит.
        let view = store.view();
        let json = serde_json::to_string(&view).unwrap();
        assert!(
            !json.contains("LegacyPw123456"),
            "секрет утёк во view: {json}"
        );
        assert!(serde_json::from_str::<serde_json::Value>(&json)
            .unwrap()
            .get("secrets")
            .is_none());

        // Сохранение вычищает секрет из файла.
        store.save().unwrap();
        let raw = std::fs::read_to_string(dir.join(STATE_FILE)).unwrap();
        assert!(
            !raw.contains("LegacyPw123456"),
            "секрет остался в state.json"
        );
        assert!(
            !raw.contains("\"secrets\""),
            "пустая секция секретов не пишется"
        );

        // Миграция забирает унаследованные секреты один раз.
        let legacy = store.take_legacy_secrets();
        assert_eq!(legacy.len(), 1);
        assert!(store.take_legacy_secrets().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
