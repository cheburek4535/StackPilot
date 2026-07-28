use std::collections::HashMap;

/// TemplateEngine — заменяет {{ variable }} в строках.
///
/// В будущем можно подключить Handlebars / MiniJinja / Tera,
/// но для начала хватит простого regex-замены.
pub struct TemplateEngine;


impl TemplateEngine {
    pub fn new() -> Self {
        Self
    }

    /// Рендерит строку template, подставляя значения из context.
    /// Синтаксис: {{ key }} — заменится на context["key"].
    /// Если ключ не найден, оставляет как есть (или заменяет на "" — TBD).
    pub fn render(&self, template: &str, context: &HashMap<String, String>) -> String {
        // TZ Task 5: реализовать замену {{ key }} через regex
        //
        // Алгоритм:
        //   1. Найти все вхождения {{ ... }} в template
        //   2. Для каждого — извлечь имя ключа (trim)
        //   3. Если ключ есть в context → подставить значение
        //   4. Если ключа нет → оставить {{ key }} или заменить на ""
        //
        // Пример:
        //   template: "project {{ name }} uses {{ language }}"
        //   context:  {"name": "myapp", "language": "Rust"}
        //   result:   "project myapp uses Rust"
        //
        // Пока возвращаем template без изменений.
        template.to_string()
    }

    /// Загрузить шаблон из встроенной строки (для использования в Recipe).
    /// В будущем может загружать из файлов.
    pub fn load_template(&self, name: &str, content: &str) -> String {
        // TBD: кэширование, валидация
        content.to_string()
    }
}
