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
        "kotlin" => &["java", "kotlin"],
        "php" => &["php"],
        "swift" => &["swift"],
        "zig" => &["zig"],
        "elixir" => &["elixir", "erlang"],
        // gleam компилируется в Erlang и требует erlc/erlang для сборки
        "gleam" => &["gleam", "erlang"],
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
///
/// Движки и SDK (unity, unreal, godot, android, qt, xcodebuild)
/// попадают сюда как manual-инструменты: проект генерируется,
/// а в отчёте проверки окружения пользователь получает честное
/// предупреждение «установите вручную» (см. manual_install в tools.json).
fn framework_extra_tools(framework: &str) -> &'static [&'static str] {
    match framework {
        // tauri-cli: генерация проекта запускает `cargo tauri`,
        // а без CLI это падает «no such command: tauri».
        "tauri" => &["rust", "node", "tauri-cli"],
        "flutter" => &["flutter"],
        // Android SDK нужен и для android, и для jetpack-compose.
        "android" | "jetpack-compose" => &["java", "android"],
        // JVM-фреймворки: spring boot и ktor требуют JDK, даже если
        // язык java не выбран в мастере явно (проект не соберётся без
        // javac — а Initializr-шаблоны его подразумевают).
        "spring-boot" | "ktor" => &["java"],
        // Игровые движки и десктоп-фреймворки: файлы проекта
        // генерируются, движок/SDK пользователь ставит сам.
        "unity" => &["unity"],
        "unreal" => &["unreal"],
        "godot" => &["godot"],
        "qt" => &["qt"],
        // SwiftUI/Vapor — только macOS: нужен Xcode (xcodebuild).
        "swiftui" | "vapor" => &["xcodebuild"],
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

/// Таблица «фреймворк → требуемые языки» из wizard_tree.json.
/// Если язык фреймворка не выбран в мастере явно, его рантайм
/// всё равно попадает в требования (django без выбранного python
/// никогда не должен молча теряться из проверки окружения).
fn framework_requires_language() -> &'static HashMap<String, Vec<String>> {
    static MAP: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
    MAP.get_or_init(|| {
        let raw = include_str!("../../project_creator/knowledge/wizard_tree.json");
        let tree: serde_json::Value = serde_json::from_str(raw)
            .expect("wizard_tree.json должен быть корректным JSON");
        tree.get("frameworks")
            .and_then(|arr| arr.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|fw| {
                        let id = fw.get("id")?.as_str()?.to_string();
                        let langs = fw
                            .get("requires_language")
                            .and_then(|l| l.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|l| l.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();
                        Some((id, langs))
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
/// Большинство совпадает (postgresql, docker, ...). Тех, кого нет
/// в каталоге toolchain (pytest, sqlalchemy, alembic, ruff, prisma,
/// drizzle, dbt), нет по дизайну: это pip/npm-пакеты или docker-образы
/// (clickhouse, airflow, opentelemetry), а не системное ПО — их
/// доставит менеджер пакетов языка или docker. docker-инструменты
/// идут вместе с самим docker (requires_docker в wizard_tree).
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
        "kafka" => Some("kafka"),
        "grafana" => Some("grafana"),
        // "terra" — старый id из мастера, оставлен для обратной
        // совместимости с сохранёнными сессиями.
        "terra" | "terraform" => Some("terraform"),
        "firebase" => Some("firebase"),
        // C# REPL: npm-пакета dotnet-cmd не существует (registry 404),
        // реальный REPL — CSharpRepl (dotnet tool install -g CSharpRepl).
        // «dotnet-cmd» замаплен на него для совместимости с мастером.
        "dotnet-cmd" | "csharprepl" => Some("csharprepl"),
        _ => None,
    }
}

// ------------------------------------------------------------
// Сборка
// ------------------------------------------------------------

/// Полный список id инструментов для проверки окружения.
/// Порядок: winget → языки → фреймворки → выбранные тулы → флаги.
/// Дубликаты убираются (HashSet-страж), первый порядок сохраняется.
///
/// winget идёт ВСЕГДА первым: это основной источник установки на
/// Windows (planner выносит его в начало плана, если он отсутствует).
pub fn resolve(requirements: &ProjectRequirements) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let mut push = |id: &str| {
        if seen.insert(id.to_string()) {
            ids.push(id.to_string());
        }
    };

    push("winget");

    for lang in &requirements.languages {
        for id in language_tools(lang) {
            push(id);
        }
    }

    for fw in &requirements.frameworks {
        for id in framework_extra_tools(fw) {
            push(id);
        }
        // Языки, которые фреймворк требует, но которые не выбраны
        // в мастере: их рантайм всё равно должен попасть в проверку
        // окружения (например nextjs без выбранного typescript).
        // requires_language — «хотя бы один из»: рантаймы добавляем
        // только когда не выбран ни один из требуемых языков.
        if let Some(langs) = framework_requires_language().get(fw) {
            if !langs.is_empty() && !langs.iter().any(|l| requirements.languages.contains(l)) {
                for lang in langs {
                    for id in language_tools(lang) {
                        push(id);
                    }
                }
            }
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
            // Инструменты, которые не могут установиться без своего рантайма:
            // firebase-tools ставится через npm, CSharpRepl — через dotnet tool.
            // Рантайм обязан попасть в требования, даже если язык не выбран.
            match id {
                "firebase" => push("node"),
                "csharprepl" => push("dotnet"),
                _ => {}
            }
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
        assert_eq!(resolve(&r), vec!["winget", "python"]);
    }

    #[test]
    fn typescript_and_javascript_use_node() {
        let mut r = req();
        r.languages = vec!["typescript".into()];
        assert_eq!(resolve(&r), vec!["winget", "node"]);

        let mut r = req();
        r.languages = vec!["javascript".into()];
        assert_eq!(resolve(&r), vec!["winget", "node"]);
    }

    #[test]
    fn elixir_brings_erlang_runtime() {
        let mut r = req();
        r.languages = vec!["elixir".into()];
        assert_eq!(resolve(&r), vec!["winget", "elixir", "erlang"]);
    }

    #[test]
    fn tauri_brings_runtime_and_framework_tools() {
        let mut r = req();
        r.frameworks = vec!["tauri".into()];
        let ids = resolve(&r);
        // рантаймы из статичной таблицы + tauri-cli для `cargo tauri`
        for expected in ["rust", "node", "tauri-cli"] {
            assert!(ids.iter().any(|i| i == expected), "нет {expected} в {ids:?}");
        }
        // тулы из wizard_tree.json (framework_tool_map: tauri → docker, npm)
        for expected in ["docker", "npm"] {
            assert!(ids.iter().any(|i| i == expected), "нет {expected} в {ids:?}");
        }
    }

    #[test]
    fn spring_boot_brings_jdk_without_language() {
        let mut r = req();
        r.frameworks = vec!["spring-boot".into()];
        let ids = resolve(&r);
        assert!(ids.iter().any(|i| i == "java"), "spring boot без JDK: {ids:?}");
        // тулы из wizard_tree.json (framework_tool_map: spring-boot → maven, gradle, ...)
        for expected in ["maven", "gradle", "postgresql", "redis", "docker", "kafka", "mongodb"] {
            assert!(ids.iter().any(|i| i == expected), "нет {expected} в {ids:?}");
        }
    }

    #[test]
    fn ktor_brings_jdk() {
        let mut r = req();
        r.frameworks = vec!["ktor".into()];
        let ids = resolve(&r);
        assert!(ids.iter().any(|i| i == "java"), "ktor без JDK: {ids:?}");
        assert!(ids.iter().any(|i| i == "kafka"), "ktor без kafka: {ids:?}");
    }

    #[test]
    fn kotlin_language_brings_kotlinc_and_jdk() {
        let mut r = req();
        r.languages = vec!["kotlin".into()];
        let ids = resolve(&r);
        assert!(ids.iter().any(|i| i == "kotlin"), "kotlin без kotlinc: {ids:?}");
        assert!(ids.iter().any(|i| i == "java"), "kotlin без JDK: {ids:?}");
    }

    #[test]
    fn airflow_grafana_and_terraform_tools_resolve() {
        let mut r = req();
        r.tools = vec!["grafana".into(), "terraform".into(), "firebase".into()];
        let ids = resolve(&r);
        for expected in ["grafana", "terraform", "firebase"] {
            assert!(ids.iter().any(|i| i == expected), "нет {expected} в {ids:?}");
        }
    }

    #[test]
    fn gleam_brings_erlang_runtime() {
        let mut r = req();
        r.languages = vec!["gleam".into()];
        let ids = resolve(&r);
        assert!(ids.iter().any(|i| i == "gleam"), "нет gleam: {ids:?}");
        assert!(ids.iter().any(|i| i == "erlang"), "gleam без erlang: {ids:?}");
    }

    #[test]
    fn firebase_brings_node_runtime() {
        let mut r = req();
        r.tools = vec!["firebase".into()];
        let ids = resolve(&r);
        assert!(ids.iter().any(|i| i == "node"), "firebase без node: {ids:?}");
    }

    #[test]
    fn csharprepl_brings_dotnet_runtime() {
        let mut r = req();
        r.tools = vec!["csharprepl".into()];
        let ids = resolve(&r);
        assert!(ids.iter().any(|i| i == "dotnet"), "csharprepl без dotnet: {ids:?}");
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
        // winget идёт первым, языки — раньше тулов фреймворка
        assert_eq!(ids[0], "winget");
        assert_eq!(ids[1], "rust");
    }

    #[test]
    fn wizard_tools_without_toolchain_entry_are_filtered() {
        assert_eq!(wizard_tool_to_toolchain("pytest"), None);
        assert_eq!(wizard_tool_to_toolchain("sqlalchemy"), None);
        assert_eq!(wizard_tool_to_toolchain("prisma"), None);
        assert_eq!(wizard_tool_to_toolchain("npm"), Some("npm"));
        assert_eq!(wizard_tool_to_toolchain("postgresql"), Some("postgresql"));
        // npm-пакета dotnet-cmd нет — реальный REPL ставится через dotnet tool
        assert_eq!(wizard_tool_to_toolchain("dotnet-cmd"), Some("csharprepl"));
        assert_eq!(wizard_tool_to_toolchain("csharprepl"), Some("csharprepl"));
    }

    #[test]
    fn engines_are_manual_install_tools() {
        // Unity/Unreal/Godot/Qt: проект генерируется, движок — вручную.
        // Требования обязаны содержать движок (для честного отчёта),
        // а не молча его терять.
        for (framework, engine) in [
            ("unity", "unity"),
            ("unreal", "unreal"),
            ("godot", "godot"),
            ("qt", "qt"),
        ] {
            let mut r = req();
            r.frameworks = vec![framework.into()];
            let ids = resolve(&r);
            assert!(
                ids.iter().any(|i| i == engine),
                "{framework} без {engine}: {ids:?}"
            );
        }
    }

    #[test]
    fn android_and_jetpack_require_android_sdk() {
        for framework in ["android", "jetpack-compose"] {
            let mut r = req();
            r.frameworks = vec![framework.into()];
            let ids = resolve(&r);
            assert!(
                ids.iter().any(|i| i == "android"),
                "{framework} без android SDK: {ids:?}"
            );
            assert!(
                ids.iter().any(|i| i == "java"),
                "{framework} без java: {ids:?}"
            );
        }
    }

    #[test]
    fn swift_ui_frameworks_require_xcode() {
        for framework in ["swiftui", "vapor"] {
            let mut r = req();
            r.frameworks = vec![framework.into()];
            let ids = resolve(&r);
            assert!(
                ids.iter().any(|i| i == "xcodebuild"),
                "{framework} без xcodebuild: {ids:?}"
            );
        }
    }

    #[test]
    fn framework_brings_required_language_runtime() {
        // nextjs требует typescript/javascript — даже без выбранного
        // языка его рантайм (node) обязан попасть в требования.
        let mut r = req();
        r.frameworks = vec!["nextjs".into()];
        let ids = resolve(&r);
        assert!(
            ids.iter().any(|i| i == "node"),
            "nextjs без node: {ids:?}"
        );
    }

    #[test]
    fn django_brings_python_runtime() {
        let mut r = req();
        r.frameworks = vec!["django".into()];
        let ids = resolve(&r);
        assert!(
            ids.iter().any(|i| i == "python"),
            "django без python: {ids:?}"
        );
    }

    #[test]
    fn empty_requirements_resolve_to_winget_only() {
        // winget — обязательный базовый инструмент даже при пустом запросе
        assert_eq!(resolve(&req()), vec!["winget"]);
    }

    /// Гарантия отсутствия молчаливых дыр: каждый id, который resolve()
    /// может вернуть для ЛЮБОГО языка/фреймворка/тула/флага из мастера,
    /// обязан существовать в tools.json. Иначе check.rs просто молча
    /// пропустит неизвестный id, и требование потеряется из отчёта.
    #[test]
    fn every_resolved_id_exists_in_tools_json() {
        let raw = include_str!("../tools.json");
        let defs: serde_json::Value = serde_json::from_str(raw).expect("tools.json");
        let tool_ids: HashSet<String> = defs
            .as_array()
            .expect("tools.json — массив")
            .iter()
            .filter_map(|d| d.get("id").and_then(|i| i.as_str()).map(String::from))
            .collect();

        let tree_raw = include_str!("../../project_creator/knowledge/wizard_tree.json");
        let tree: serde_json::Value =
            serde_json::from_str(tree_raw).expect("wizard_tree.json должен быть корректным JSON");

        let collect = |arr: Option<&serde_json::Value>| -> Vec<String> {
            arr.and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.get("id").and_then(|i| i.as_str()).map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };

        let languages = collect(tree.get("languages"));
        let frameworks = collect(tree.get("frameworks"));
        let wizard_tools = collect(tree.get("tools"));

        let mut checked = 0usize;
        let mut missing: Vec<String> = Vec::new();

        let mut check_ids = |r: &ProjectRequirements, origin: &str, out: &mut Vec<String>| {
            for id in resolve(r) {
                if !tool_ids.contains(&id) {
                    out.push(format!("{origin} → {id}"));
                }
            }
        };

        for lang in &languages {
            let mut r = req();
            r.languages = vec![lang.clone()];
            check_ids(&r, &format!("язык {lang}"), &mut missing);
            checked += 1;
        }
        for fw in &frameworks {
            let mut r = req();
            r.frameworks = vec![fw.clone()];
            check_ids(&r, &format!("фреймворк {fw}"), &mut missing);
            checked += 1;
        }
        for tool in &wizard_tools {
            let mut r = req();
            r.tools = vec![tool.clone()];
            check_ids(&r, &format!("тул {tool}"), &mut missing);
            checked += 1;
        }
        {
            let mut r = req();
            r.git_init = true;
            r.vscode_config = true;
            r.docker = true;
            check_ids(&r, "флаги", &mut missing);
        }

        assert!(
            missing.is_empty(),
            "resolve() вернул id, которых нет в tools.json ({missing:?}); проверено {checked} сценариев"
        );
    }

    /// Обратная гарантия: каждый фреймворк мастера обязан дать хоть одно
    /// требование (плюс всегда winget) — иначе фреймворк молча выпадает
    /// из проверки окружения, как было с unity/unreal/godot/qt до фикса.
    #[test]
    fn every_framework_produces_at_least_one_requirement() {
        let tree_raw = include_str!("../../project_creator/knowledge/wizard_tree.json");
        let tree: serde_json::Value =
            serde_json::from_str(tree_raw).expect("wizard_tree.json должен быть корректным JSON");
        let frameworks: Vec<String> = tree
            .get("frameworks")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.get("id").and_then(|i| i.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        for fw in &frameworks {
            let mut r = req();
            r.frameworks = vec![fw.clone()];
            let ids = resolve(&r);
            assert!(
                ids.len() > 1,
                "фреймворк {fw} не даёт ни одного требования (только winget): {ids:?}"
            );
        }
    }
}
