// ============================================================
// Загрузчик определений инструментов (tools.json)
// ============================================================
// tools.json компилируется прямо в бинарь через include_str!:
// файл всегда доступен, не зависит от рабочей директории запуска
// и не требует копирования рядом с exe.

use crate::modules::toolchain::models::ToolDefinition;

/// Загружает определения инструментов из tools.json.
/// Паникует только при ошибке разработчика (битый JSON), т.к.
/// это статичные данные, а не пользовательский ввод.
pub fn load_definitions() -> Vec<ToolDefinition> {
    let raw = include_str!("tools.json");
    let definitions: Vec<ToolDefinition> = serde_json::from_str(raw)
        .expect("tools.json должен быть корректным JSON и соответствовать структуре ToolDefinition");
    definitions
}

/// Базовая валидация определений. Возвращает список предупреждений:
///  - дубликаты id (сломают lookup по id);
///  - определения без правил обнаружения (никогда не найдутся);
///  - определения с версией min > recommended (ошибка в данных).
pub fn validate(definitions: &[ToolDefinition]) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();

    for def in definitions {
        if !seen.insert(def.id.as_str()) {
            warnings.push(format!("Дубликат id инструмента: {}", def.id));
        }
        let has_probe = !def.detection.version_probes.is_empty();
        let has_path = !def.detection.known_paths.is_empty();
        let has_registry = !def.detection.registry_keys.is_empty();
        if !has_probe && !has_path && !has_registry {
            warnings.push(format!(
                "{} ({}): нет ни одной правила обнаружения",
                def.id, def.display
            ));
        }
        if let (Some(min), Some(rec)) = (&def.versions.min, &def.versions.recommended) {
            if min > rec {
                warnings.push(format!(
                    "{}: минимальная версия ({}) больше рекомендуемой ({})",
                    def.id, min, rec
                ));
            }
        }
    }
    warnings
}
