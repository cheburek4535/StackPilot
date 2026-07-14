// ============================================================
// ProfileManager — управление профилями запуска.
//
// # Архитектура
//
// ProfileManager — это трейт (интерфейс). Любая реализация
// этого трейта может быть подставлена в приложение. Это
// позволяет легко:
//   • переключаться между разными способами хранения
//   • писать тесты с mock-реализациями
//
// JsonProfileManager — конкретная реализация, хранящая
// профили как JSON-файлы в指定нной папке.
// ============================================================

use std::path::PathBuf;
use std::fs;

use crate::models::*;

// --------------------------------------------------
// Трейт ProfileManager
// --------------------------------------------------
pub trait ProfileManager: Send + Sync {
    /// Вернуть список всех сохранённых профилей
    fn list_profiles(&self) -> Result<Vec<LaunchProfile>, String>;

    /// Вернуть профиль по имени
    fn get_profile(&self, name: &str) -> Result<LaunchProfile, String>;

    /// Сохранить профиль (создать или перезаписать)
    fn save_profile(&self, profile: &LaunchProfile) -> Result<(), String>;

    /// Удалить профиль по имени
    fn delete_profile(&self, name: &str) -> Result<(), String>;
}

// --------------------------------------------------
// JsonProfileManager — профили в JSON-файлах
//
// Каждый профиль = отдельный .json файл.
// Имя файла = имя профиля + ".json"
// --------------------------------------------------
pub struct JsonProfileManager {
    profiles_dir: PathBuf,
}

impl JsonProfileManager {
    pub fn new(profiles_dir: PathBuf) -> Self {
        Self { profiles_dir }
    }

    /// Построить путь к файлу профиля
    fn profile_path(&self, name: &str) -> PathBuf {
        self.profiles_dir.join(format!("{}.json", name))
    }
}

impl ProfileManager for JsonProfileManager {
    fn list_profiles(&self) -> Result<Vec<LaunchProfile>, String> {
        let mut profiles = Vec::new();

        // Читаем содержимое папки профилей
        let entries = fs::read_dir(&self.profiles_dir)
            .map_err(|e| format!("Не удалось прочитать папку профилей: {}", e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Ошибка чтения записи: {}", e))?;
            let path = entry.path();

            // Нас интересуют только .json файлы
            if path.extension().map_or(false, |ext| ext == "json") {
                let content = fs::read_to_string(&path)
                    .map_err(|e| format!("Не удалось прочитать {}: {}", path.display(), e))?;
                let profile: LaunchProfile = serde_json::from_str(&content)
                    .map_err(|e| format!("Ошибка парсинга {}: {}", path.display(), e))?;
                profiles.push(profile);
            }
        }

        Ok(profiles)
    }

    fn get_profile(&self, name: &str) -> Result<LaunchProfile, String> {
        let path = self.profile_path(name);
        let content = fs::read_to_string(&path)
            .map_err(|e| format!("Не удалось прочитать профиль '{}': {}", name, e))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Ошибка парсинга профиля '{}': {}", name, e))
    }

    fn save_profile(&self, profile: &LaunchProfile) -> Result<(), String> {
        let path = self.profile_path(&profile.name);
        let content = serde_json::to_string_pretty(profile)
            .map_err(|e| format!("Ошибка сериализации профиля: {}", e))?;
        fs::write(&path, content)
            .map_err(|e| format!("Не удалось записать профиль '{}': {}", profile.name, e))
    }

    fn delete_profile(&self, name: &str) -> Result<(), String> {
        let path = self.profile_path(name);
        fs::remove_file(&path)
            .map_err(|e| format!("Не удалось удалить профиль '{}': {}", name, e))
    }
}
