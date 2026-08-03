// ============================================================
// Требования проекта → инструменты (requirements.rs)
// ============================================================
// Превращает ProjectRequirements (что выбрал пользователь в мастере)
// в упорядоченный список id инструментов из tools.json.
//
// Правило независимости: toolchain НЕ зависит от project_creator
// как от кода — на вход приходит простой JSON-контракт. Но чтобы
// маппинг «фреймворк → тулы» не расходился с мастером, таблица
// framework_tool_map читается из wizard_tree.json (данные, не код).
//
// Источники требований:
//   1. языки       — статичная таблица language_tools (рантайм языка);
//   2. фреймворки  — framework_extra_tools (рантайм, которого нет
//                    в wizard_tree) + framework_tool_map из wizard_tree.json;
//   3. выбранные тулы — wizard_tool_to_toolchain;
//   4. флаги       — git_init → git, vscode_config → vscode, docker → docker.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::modules::toolchain::models::ProjectRequirements;

// ------------------------------------------------------------
// 1. Языки → инструменты
// ------------------------------------------------------------

/// Язык из мастера → id инструментов в tools.json.
/// Некоторые языки тянут рантаймы: elixir работает поверх erlang,
/// kotlin — на JVM (java), typescript/javascript исполняются в node.
fn language_tools(lang: &str) -> &'static [&'static str] {
    match lang {
        "python" => &["python"],
        "rust" => &["rust", "msvc-build-tools"],
        "go" => &["go"],
        "typescript" | "javascript" => &["node"],
        "java" => &["java"],
        "csharp" => &["dotnet"],
        "cpp" => &["msvc-build-tools"],
        "dart" => &["dart"],
        "kotlin" => &["java"],
        "php" => &["php"],
        "swift" => &["swift"],
        "zig" => &["zig"],
        "elixir" => &["elixir", "erlang"],
        "gleam" => &["gleam"],
        // html — статика, отдельного рантайма нет
        _ => &[],
    }
}

// ------------------------------------------------------------
// 2. Фреймворки → дополнительные инструменты
// ------------------------------------------------------------

/// Рантаймы, которые фреймворк требует, но которых нет в его
/// framework_tool_map из wizard_tree.json (там только «тулы»,
/// языки не фигурируют). tauri собран на rust, flutter — свой SDK.
fn framework_extra_tools(framework: &str) -> &'static [&'static str] {
    match framework {
        "tauri" => &["rust", "node"],
        "flutter" => &["flutter"],
        "android" | "jetpack-compose" => &["java"],
        _ => &[],
    }
}

/// Таблица «фреймворк → тулы» из wizard_tree.json.
/// Кэшируется в OnceLock: файл компилируется в бинарь (include_str!),
/// парсится один раз при первом обращении.
fn framework_tool_map() -> &'static HashMap<String, Vec<String>> {
    static MAP: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
    MAP.get_or_init(|| {
        let raw = include_str!("../../project_creator/knowledge/wizard_tree.json");
        let tree: serde_json::Value = serde_json::from_str(raw)
            .expect("wizard_tree.json должен быть корректным JSON");
        tree.get("framework_tool_map")
            .and_then(|m| m.as_object())
            .map(|obj| {
                obj.iter()
                    .map(|(fw, tools)| {
                        let list = tools
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|t| t.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();
                        (fw.clone(), list)
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}

// ------------------------------------------------------------
// 3. Тулы мастера → тулы toolchain
// ------------------------------------------------------------

/// id тула из wizard_tree.json → id в tools.json.
/// Многие совпадают (postgresql, docker, ...). Те, которых нет
/// в каталоге toolchain (pytest, sqlalchemy, prisma, firebase...),
/// не проверяются: это библиотеки/сервисы, а не системное ПО.
pub fn wizard_tool_to_toolchain(wizard_id: &str) -> Option<&'static str> {
    // id, совпадающие с каталогом toolchain, возвращаются строковыми
    // литералами, а не заимствованным входным &str: это даёт
    match wizard_id {
        "npm" => Some("npm"),
        "docker" => Some("docker"),
        "postgresql" => Some("postgresql"),
        "redis" => Some("redis"),
        "mongodb" => Some("mongodb"),
        "sqlite" => Some("sqlite"),
        "maven" => Some("maven"),
        "gradle" => Some("gradle"),
        _ => None,
    }
}

// ------------------------------------------------------------
// Сборка
// ------------------------------------------------------------

/// Полный список id инструментов для проверки окружения.
/// Порядок: языки → фреймворки → выбранные тулы → флаги.
/// Дубликаты убираются (HashSet-страж), первый порядок сохраняется.
pub fn resolve(requirements: &ProjectRequirements) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let mut push = |id: &str| {
        if seen.insert(id.to_string()) {
            ids.push(id.to_string());
        }
    };

    for lang in &requirements.languages {
        for id in language_tools(lang) {
            push(id);
        }
    }

    for fw in &requirements.frameworks {
        for id in framework_extra_tools(fw) {
            push(id);
        }
        if let Some(tools) = framework_tool_map().get(fw) {
            for wizard_tool in tools {
                if let Some(id) = wizard_tool_to_toolchain(wizard_tool) {
                    push(id);
                }
            }
        }
    }

    for tool in &requirements.tools {
        if let Some(id) = wizard_tool_to_toolchain(tool) {
            push(id);
        }
    }

    if requirements.git_init {
        push("git");
    }
    if requirements.vscode_config {
        push("vscode");
    }
    if requirements.docker {
        push("docker");
    }

    ids
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> ProjectRequirements {
        ProjectRequirements::default()
    }

    #[test]
    fn language_python_resolves_to_python() {
        let mut r = req();
        r.languages = vec!["python".into()];
        assert_eq!(resolve(&r), vec!["python"]);
    }

    #[test]
    fn typescript_and_javascript_use_node() {
        let mut r = req();
        r.languages = vec!["typescript".into()];
        assert_eq!(resolve(&r), vec!["node"]);

        let mut r = req();
        r.languages = vec!["javascript".into()];
        assert_eq!(resolve(&r), vec!["node"]);
    }

    #[test]
    fn elixir_brings_erlang_runtime() {
        let mut r = req();
        r.languages = vec!["elixir".into()];
        assert_eq!(resolve(&r), vec!["elixir", "erlang"]);
    }

    #[test]
    fn tauri_brings_runtime_and_framework_tools() {
        let mut r = req();
        r.frameworks = vec!["tauri".into()];
        let ids = resolve(&r);
        // рантаймы из статичной таблицы
        for expected in ["rust", "node"] {
            assert!(ids.iter().any(|i| i == expected), "нет {expected} в {ids:?}");
        }
        // тулы из wizard_tree.json (framework_tool_map: tauri → docker, npm)
        for expected in ["docker", "npm"] {
            assert!(ids.iter().any(|i| i == expected), "нет {expected} в {ids:?}");
        }
    }

    #[test]
    fn flags_add_git_vscode_docker() {
        let mut r = req();
        r.git_init = true;
        r.vscode_config = true;
        r.docker = true;
        let ids = resolve(&r);
        for expected in ["git", "vscode", "docker"] {
            assert!(ids.iter().any(|i| i == expected), "нет {expected} в {ids:?}");
        }
    }

    #[test]
    fn dedup_and_language_first() {
        let mut r = req();
        r.frameworks = vec!["tauri".into()];
        r.languages = vec!["rust".into()];
        r.tools = vec!["docker".into()];
        let ids = resolve(&r);

        let mut seen = std::collections::HashSet::new();
        for id in &ids {
            assert!(seen.insert(id), "дубликат {id} в {ids:?}");
        }
        // языки идут раньше тулов фреймворка
        assert_eq!(ids[0], "rust");
    }

    #[test]
    fn wizard_tools_without_toolchain_entry_are_filtered() {
        assert_eq!(wizard_tool_to_toolchain("pytest"), None);
        assert_eq!(wizard_tool_to_toolchain("sqlalchemy"), None);
        assert_eq!(wizard_tool_to_toolchain("prisma"), None);
        assert_eq!(wizard_tool_to_toolchain("npm"), Some("npm"));
        assert_eq!(wizard_tool_to_toolchain("postgresql"), Some("postgresql"));
    }

    #[test]
    fn empty_requirements_resolve_to_empty() {
        assert!(resolve(&req()).is_empty());
    }
}
