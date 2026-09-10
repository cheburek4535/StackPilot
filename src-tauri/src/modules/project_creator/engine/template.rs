use std::collections::HashMap;

/// TemplateEngine — заменяет {{ variable }} в строках.
///
/// В будущем можно подключить Handlebars / MiniJinja / Tera,
/// но для начала хватит простого подстановочного движка.
pub struct TemplateEngine;

impl TemplateEngine {
    pub fn new() -> Self {
        Self
    }

    /// Рендерит строку template, подставляя значения из context.
    /// Синтаксис: {{ key }} — заменится на context["key"].
    /// Если ключ не найден, плейсхолдер остаётся в тексте как есть —
    /// шаблон не ломается и видно, чего не хватило.
    pub fn render(&self, template: &str, context: &HashMap<String, String>) -> String {
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            match after.find("}}") {
                Some(end) => {
                    let key = after[..end].trim();
                    match context.get(key) {
                        Some(value) => out.push_str(value),
                        None => {
                            // Неизвестный ключ оставляем как есть.
                            out.push_str("{{");
                            out.push_str(&after[..end]);
                            out.push_str("}}");
                        }
                    }
                    rest = &after[end + 2..];
                }
                None => {
                    // Незакрытый плейсхолдер — оставляем остаток как есть.
                    out.push_str(&rest[start..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// Загрузить шаблон из встроенной строки (для использования в Recipe).
    /// В будущем может загружать из файлов.
#[allow(dead_code)]
    pub fn load_template(&self, name: &str, content: &str) -> String {
        // TBD: кэширование, валидация
        let _ = name;
        content.to_string()
    }

    // =========================================================================
    // Qt WebEngine: расширенные шаблоны для стека qt-webengine + веб-фреймворк
    // (react/vue/svelte). Используются генератором qt_steps_webengine.
    // =========================================================================

    /// main.cpp для QWebEngineView: минимальный бойлерплейт, загружающий
    /// собранную веб-часть из ресурсов (qrc:/web/index.html) с фолбэком на
    /// локальный dev-сервер фронтенда.
    pub fn qt_webengine_main_cpp(&self, project_name: &str) -> String {
        let mut ctx = HashMap::new();
        ctx.insert("project_name".to_string(), project_name.to_string());
        self.render(
            r#"#include <QApplication>
#include <QUrl>
#include <QWebEngineView>

int main(int argc, char *argv[])
{
    QApplication app(argc, argv);
    QWebEngineView view;
    view.setWindowTitle("{{ project_name }}");
    view.resize(1000, 700);

    // Собранное веб-приложение (npm run build внутри веб-фреймворка)
    // встраивается в ресурсы через qt_add_resources — см. CMakeLists.txt.
    view.load(QUrl(QStringLiteral("qrc:/web/index.html")));
    view.show();
    return app.exec();
}
"#,
            &ctx,
        )
    }

    /// CMakeLists.txt для Qt WebEngine: find_package(WebEngineWidgets),
    /// линковка Qt6::WebEngineWidgets и встраивание веб-dist в ресурсы.
    pub fn qt_webengine_cmake(&self, project_name: &str) -> String {
        let mut ctx = HashMap::new();
        ctx.insert("project_name".to_string(), project_name.to_string());
        self.render(
            r#"cmake_minimum_required(VERSION 3.16)
project({{ project_name }})

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 REQUIRED COMPONENTS WebEngineWidgets)

add_executable({{ project_name }} src/main.cpp)

# Собранная веб-часть (npm run build внутри веб-приложения) встраивается
# в ресурсы приложения. Веб-приложение может лежать рядом, внутри подпапки
# проекта или в соседнем сегменте (frontend/) — ищем все варианты.
if(EXISTS ${CMAKE_CURRENT_SOURCE_DIR}/dist/index.html)
    set(WEB_DIST ${CMAKE_CURRENT_SOURCE_DIR}/dist)
elseif(EXISTS ${CMAKE_CURRENT_SOURCE_DIR}/${PROJECT_NAME}/dist/index.html)
    set(WEB_DIST ${CMAKE_CURRENT_SOURCE_DIR}/${PROJECT_NAME}/dist)
elseif(EXISTS ${CMAKE_CURRENT_SOURCE_DIR}/../frontend/${PROJECT_NAME}/dist/index.html)
    set(WEB_DIST ${CMAKE_CURRENT_SOURCE_DIR}/../frontend/${PROJECT_NAME}/dist)
endif()

if(WEB_DIST)
    qt_add_resources({{ project_name }} "web"
        PREFIX "/web"
        BASE ${WEB_DIST}
        FILES ${WEB_DIST}/index.html
    )
endif()

target_link_libraries({{ project_name }} Qt6::WebEngineWidgets)
"#,
            &ctx,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_substitutes_known_keys() {
        let engine = TemplateEngine::new();
        let mut ctx = HashMap::new();
        ctx.insert("name".to_string(), "myapp".to_string());
        ctx.insert("language".to_string(), "Rust".to_string());
        assert_eq!(
            engine.render("project {{ name }} uses {{ language }}", &ctx),
            "project myapp uses Rust"
        );
    }

    #[test]
    fn render_keeps_unknown_keys_untouched() {
        let engine = TemplateEngine::new();
        let ctx = HashMap::new();
        assert_eq!(
            engine.render("{{ missing }} stays", &ctx),
            "{{ missing }} stays"
        );
    }

    #[test]
    fn render_handles_unclosed_placeholder() {
        let engine = TemplateEngine::new();
        let mut ctx = HashMap::new();
        ctx.insert("a".to_string(), "1".to_string());
        assert_eq!(engine.render("{{ a }} and {{ oops", &ctx), "1 and {{ oops");
    }

    #[test]
    fn qt_webengine_templates_render_project_name() {
        let engine = TemplateEngine::new();
        let main_cpp = engine.qt_webengine_main_cpp("myapp");
        assert!(main_cpp.contains("#include <QApplication>"));
        assert!(main_cpp.contains("#include <QWebEngineView>"));
        assert!(main_cpp.contains("QWebEngineView view;"));
        assert!(main_cpp.contains("qrc:/web/index.html"));
        assert!(
            !main_cpp.contains("{{"),
            "плейсхолдеры не должны остаться: {main_cpp}"
        );

        let cmake = engine.qt_webengine_cmake("myapp");
        assert!(cmake.contains("find_package(Qt6 REQUIRED COMPONENTS WebEngineWidgets)"));
        assert!(cmake.contains("target_link_libraries(myapp Qt6::WebEngineWidgets)"));
        assert!(cmake.contains("add_executable(myapp src/main.cpp)"));
        assert!(
            !cmake.contains("{{"),
            "плейсхолдеры не должны остаться: {cmake}"
        );
    }
}
