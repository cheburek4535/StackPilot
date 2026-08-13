// ============================================================
// Требования проекта → инструменты (requirements.rs)
// ============================================================
// Превращает ProjectRequirements (что выбрал пользователь в мастере)
// в упорядоченный список id инструментов из tools.json.
//
// Правило независимости: toolchain НЕ зависит от project_creator
// как от кода — на вход приходит простой JSON-контракт. Но чтобы
// маппинг «фреймворк → тулы» не расходился с мастером, таблицы
// читаются из wizard_tree.json (данные, не код).
//
// ВАЖНО: framework_tool_map из wizard_tree.json — это НЕ список
// обязательных инструментов, а набор, который мастер ПРЕДЛАГАЕТ
// выбрать к фреймворку (БД, кеши и т.п.). Что пользователь реально
// выбрал, приходит в requirements.tools. Безусловно обязательными
// считаются только required_tools фреймворка (npm для JS/TS,
// maven/gradle для JVM-сборки) — иначе aspnetcore начал бы требовать
// mongodb/sqlite только за то, что они упомянуты в каталоге.
//
// Источники требований:
//   1. языки       — статичная таблица language_tools (рантайм языка);
//   2. фреймворки  — framework_extra_tools (рантайм, которого нет
//                    в wizard_tree) + required_tools из wizard_tree.json
//                    (обязательные инструменты сборки) + requires_language;
//   3. выбранные тулы — wizard_tool_to_toolchain (явный выбор мастера);
//                      «двойные» docker-инструменты (requires_docker +
//                      локальные источники в tools.json: postgresql, redis,
//                      mongodb, kafka, grafana, mysql) попадают в требования
//                      ТОЛЬКО когда пользователь выбрал их локальную
//                      установку (requirements.local_infra_tools); чисто
//                      docker-инструменты (clickhouse, airflow, mailpit)
//                      сюда НЕ попадают никогда — их разворачивает project
//                      creator в docker-compose;
//   4. флаги       — git_init → git, vscode_config → vscode, docker → docker.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::modules::toolchain::models::{ProjectRequirements, ToolDefinition, ToolRequirement, ToolStatus};

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
        "elixir" => &["erlang", "elixir"],
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
        // Генерация Tauri-проекта идёт через `npx create-tauri-app`
        // (CLI-first, см. engine/mod.rs) — cargo-подкоманда tauri-cli
        // больше не нужна, хватает rust + node/npm.
        "tauri" => &["rust", "node"],
        "flutter" => &["flutter"],
        // PHP-фреймворки: генерация проекта идёт через composer
        // (`composer create-project symfony/skeleton`, `laravel/laravel`),
        // поэтому php и composer обязаны попасть в требования даже
        // если язык php не выбран в мастере явно (symfony/laravel —
        // standalone-фреймворки со своим scaffold'ом).
        "symfony" | "laravel" => &["php", "composer"],
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

/// Таблица «фреймворк → обязательные системные инструменты» из wizard_tree.json.
/// В отличие от framework_tool_map (тулы, которые мастер лишь предлагает
/// выбрать пользователю), эти инструменты нужны фреймворку всегда — без них
/// проект не собрать (npm для JS/TS, maven/gradle для JVM). Кэшируется
/// в OnceLock: файл компилируется в бинарь (include_str!), парсится один раз.
fn framework_required_tools() -> &'static HashMap<String, Vec<String>> {
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
                        let tools = fw
                            .get("required_tools")
                            .and_then(|t| t.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|t| t.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();
                        Some((id, tools))
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
                            .get("languages")
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
///
/// «Двойные» docker-инструменты (requires_docker: true в wizard_tree.json
/// И локальные источники в tools.json: postgresql, redis, mongodb, kafka,
/// grafana, mysql) замаплены здесь: наличие маппинга НЕ значит, что тул
/// требуется локально — resolve() добавляет их только когда пользователь
/// выбрал локальную установку (requirements.local_infra_tools). По умолчанию
/// их разворачивает project creator в docker-compose.yaml.
///
/// Чисто docker-инструменты без локального источника (clickhouse, airflow,
/// mailpit) отсутствуют в таблице — в локальную проверку не попадают
/// никогда. По той же причине отсутствуют pip/npm-пакеты и docker-образы
/// (pytest, sqlalchemy, alembic, prisma, drizzle, dbt, opentelemetry):
/// их доставит менеджер пакетов языка или docker.
pub fn wizard_tool_to_toolchain(wizard_id: &str) -> Option<&'static str> {
    // id, совпадающие с каталогом toolchain, возвращаются строковыми
    // литералами, а не заимствованным входным &str: это даёт
    match wizard_id {
        "npm" => Some("npm"),
        "docker" => Some("docker"),
        "sqlite" => Some("sqlite"),
        "maven" => Some("maven"),
        "gradle" => Some("gradle"),
        // Terra — старый id из мастера, оставлен для обратной
        // совместимости с сохранёнными сессиями.
        "terra" | "terraform" => Some("terraform"),
        "firebase" => Some("firebase"),
        // «Двойные» docker-инструменты: локальная установка — только
        // по явному выбору пользователя (см. resolve и is_dual_tool).
        "postgresql" => Some("postgresql"),
        "redis" => Some("redis"),
        "mongodb" => Some("mongodb"),
        "kafka" => Some("kafka"),
        "grafana" => Some("grafana"),
        "mysql" => Some("mysql"),
        // C# REPL: npm-пакета dotnet-cmd не существует (registry 404),
        // реальный REPL — CSharpRepl (dotnet tool install -g CSharpRepl).
        // «dotnet-cmd» замаплен на него для совместимости с мастером.
        "dotnet-cmd" | "csharprepl" => Some("csharprepl"),
        _ => None,
    }
}

// ------------------------------------------------------------
// Docker-инструменты мастера и «двойные» инструменты
// ------------------------------------------------------------

/// Все id тулов мастера с requires_docker: true (из wizard_tree.json).
/// Кэшируется в OnceLock: файл компилируется в бинарь, парсится один раз.
fn docker_managed_tools() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        let raw = include_str!("../../project_creator/knowledge/wizard_tree.json");
        let tree: serde_json::Value = serde_json::from_str(raw)
            .expect("wizard_tree.json должен быть корректным JSON");
        tree.get("tools")
            .and_then(|arr| arr.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|t| {
                        t.get("requires_docker")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                    })
                    .filter_map(|t| t.get("id").and_then(|i| i.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// «Двойной» инструмент: разворачивается docker-compose.yaml проекта,
/// НО при желании ставится локально (есть установочные источники
/// в tools.json). Примеры: postgresql, redis, mongodb, kafka, grafana, mysql.
/// Чисто docker-инструменты (clickhouse, airflow, mailpit) в tools.json
/// отсутствуют и «двойными» не являются.
pub fn is_dual_tool(id: &str, definitions: &[ToolDefinition]) -> bool {
    docker_managed_tools().contains(id)
        && definitions
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.installable())
            .unwrap_or(false)
}

/// Docker-инструменты из requirements.tools, НЕ выбранные локально
/// (отсутствуют в local_infra_tools). Из них команда tc_check_environment
/// соберёт optional_requirements — опциональную секцию экрана окружения.
pub fn docker_optional_tool_ids(requirements: &ProjectRequirements) -> Vec<String> {
    requirements
        .tools
        .iter()
        .filter(|t| docker_managed_tools().contains(*t) && !requirements.local_infra_tools.contains(*t))
        .cloned()
        .collect()
}

/// Опциональные требования для экрана окружения: docker-инструменты
/// мастера, которые по умолчанию развернутся контейнерами. Статус
/// RunInDocker — «в Docker, не требуется локально». Пользователь может
/// переключить такой инструмент на локальную установку, дописав его
/// в local_infra_tools и перезапустив проверку.
pub fn docker_optional_requirements(
    requirements: &ProjectRequirements,
    definitions: &[ToolDefinition],
) -> Vec<ToolRequirement> {
    docker_optional_tool_ids(requirements)
        .into_iter()
        .filter_map(|id| {
            let def = definitions.iter().find(|d| d.id == id)?;
            // Чисто docker-инструменты (нет локальных источников) в
            // опциональную секцию не попадают: выбирать локальную
            // установку для них нечего.
            if !def.installable() {
                return None;
            }
            Some(ToolRequirement {
                tool_id: def.id.clone(),
                display: def.display.clone(),
                category: def.category.clone(),
                icon: def.icon.clone(),
                status: ToolStatus::RunInDocker,
                size_mb: 0,
                needs_admin: false,
                source_description: "Docker (docker-compose.yaml)".to_string(),
                install_options: Vec::new(),
            })
        })
        .collect()
}

// ------------------------------------------------------------
// Сборка
// ------------------------------------------------------------

/// UI-вариант фреймворка (qt → qt-qml/qt-widgets/qt-webengine/qt-kirigami)
/// → модуль установки Qt. Варианты приходят из мастера как «подфреймворки»
/// (wizard_tree.json: qt_ui_options), а в toolchain превращаются в
/// install_options задачи установки.
fn qt_ui_module(framework: &str) -> Option<&'static str> {
    match framework {
        "qt-qml" => Some("qt-qml"),
        "qt-widgets" => Some("qt-widgets"),
        "qt-webengine" => Some("qt-webengine"),
        "qt-kirigami" => Some("qt-kirigami"),
        _ => None,
    }
}

/// Дополнительные опции установки по инструментам (tool_id → модули).
/// Сейчас единственный потребитель — Qt: мастерийские UI-варианты
/// (qt-webengine и т.п.) превращаются в модули, которые установщик
/// превратит в конкретные пакеты репозитория.
pub fn resolve_install_options(requirements: &ProjectRequirements) -> std::collections::HashMap<String, Vec<String>> {
    let mut options: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for fw in &requirements.frameworks {
        if let Some(module) = qt_ui_module(fw) {
            options
                .entry("qt".to_string())
                .or_default()
                .push(module.to_string());
        }
    }
    options
}

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
        // Безусловно обязательные инструменты сборки (npm, maven, gradle).
        // Инфраструктуру (БД, кеши, docker) сюда НЕ включаем: она приходит
        // только через явный выбор пользователя в requirements.tools.
        if let Some(tools) = framework_required_tools().get(fw) {
            for wizard_tool in tools {
                if let Some(id) = wizard_tool_to_toolchain(wizard_tool) {
                    push(id);
                }
            }
        }
    }

    for tool in &requirements.tools {
        // «Двойные» docker-инструменты (postgresql, redis, mongodb, kafka,
        // grafana, mysql) локально требуются только по явному выбору:
        // по умолчанию их разворачивает project creator в docker-compose.
        // Чисто docker-инструменты (clickhouse, airflow, mailpit) здесь же
        // отсекаются — маппинга в tools.json у них нет.
        if docker_managed_tools().contains(tool) && !requirements.local_infra_tools.contains(tool) {
            continue;
        }
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
        // erlang идёт ПЕРВЫМ: elixir.bat запускает erl, без рантайма
        // на PATH проверка установленного elixir не сработает.
        assert_eq!(resolve(&r), vec!["winget", "erlang", "elixir"]);
    }

    #[test]
    fn symfony_and_laravel_bring_php_and_composer() {
        // Standalone-фреймворки: scaffold идёт через `composer create-project`,
        // поэтому php и composer обязаны попасть в check_environment, даже
        // если язык php в мастере не выбран явно.
        for fw in ["symfony", "laravel"] {
            let mut r = req();
            r.frameworks = vec![fw.into()];
            let ids = resolve(&r);
            for expected in ["php", "composer"] {
                assert!(
                    ids.iter().any(|i| i == expected),
                    "{fw} без {expected}: {ids:?}"
                );
            }
            // composer ставится ПОСЛЕ php (команда php нужна его установщику)
            let php_pos = ids.iter().position(|i| i == "php").unwrap();
            let composer_pos = ids.iter().position(|i| i == "composer").unwrap();
            assert!(
                php_pos < composer_pos,
                "php должен идти раньше composer: {ids:?}"
            );
        }
    }

    #[test]
    fn tauri_brings_runtime_and_framework_tools() {
        let mut r = req();
        r.frameworks = vec!["tauri".into()];
        let ids = resolve(&r);
        // рантаймы из статичной таблицы (tauri-cli больше не нужен —
        // генерация идёт через npx create-tauri-app)
        for expected in ["rust", "node", "msvc-build-tools"] {
            assert!(ids.iter().any(|i| i == expected), "нет {expected} в {ids:?}");
        }
        // инфраструктура из framework_tool_map (docker, npm) НЕ является
        // обязательным требованием — она приходит только явным выбором
        assert!(!ids.contains(&"docker".to_string()), "tauri не должен требовать docker сам по себе: {ids:?}");
    }

    #[test]
    fn spring_boot_brings_jdk_and_build_tools() {
        let mut r = req();
        r.frameworks = vec!["spring-boot".into()];
        let ids = resolve(&r);
        assert!(ids.iter().any(|i| i == "java"), "spring boot без JDK: {ids:?}");
        // обязательные тулы сборки из required_tools
        for expected in ["maven", "gradle"] {
            assert!(ids.iter().any(|i| i == expected), "нет {expected} в {ids:?}");
        }
        // БД/кеши/очереди из framework_tool_map — только по явному выбору
        for unexpected in ["postgresql", "redis", "docker", "kafka", "mongodb"] {
            assert!(!ids.contains(&unexpected.to_string()), "{unexpected} не должен требоваться без выбора: {ids:?}");
        }
    }

    #[test]
    fn aspnetcore_does_not_require_infra_without_user_choice() {
        // Регрессия из репорта: стек c# + aspnetcore + express без выбора
        // mongodb/sqlite не должен тянуть их в требования окружения.
        let mut r = req();
        r.languages = vec!["csharp".into(), "javascript".into()];
        r.frameworks = vec!["aspnetcore".into(), "express".into()];
        let ids = resolve(&r);

        for unexpected in ["mongodb", "sqlite"] {
            assert!(
                !ids.contains(&unexpected.to_string()),
                "aspnetcore не должен требовать {unexpected} сам по себе: {ids:?}"
            );
        }
        // рантаймы и обязательные тулы на месте
        for expected in ["dotnet", "node", "npm"] {
            assert!(ids.iter().any(|i| i == expected), "нет {expected} в {ids:?}");
        }
        // инфраструктура появляется ТОЛЬКО явным выбором, причём
        // docker-инструменты (postgresql, redis) по умолчанию локально
        // не требуются — их разворачивает docker-compose из project creator.
        let mut r2 = req();
        r2.languages = vec!["csharp".into(), "javascript".into()];
        r2.frameworks = vec!["aspnetcore".into(), "express".into()];
        r2.tools = vec!["postgresql".into(), "redis".into(), "docker".into()];
        r2.git_init = true;
        r2.vscode_config = true;
        let ids2 = resolve(&r2);
        for expected in ["docker", "git", "vscode"] {
            assert!(ids2.iter().any(|i| i == expected), "нет {expected} в {ids2:?}");
        }
        for unexpected in ["postgresql", "redis", "mongodb", "sqlite"] {
            assert!(!ids2.contains(&unexpected.to_string()), "{unexpected} в требованиях: {ids2:?}");
        }
        // локальная установка docker-инструментов — только по явному выбору:
        // postgresql и redis в local_infra_tools обязаны появиться в требованиях
        let mut r3 = r2.clone();
        r3.local_infra_tools = vec!["postgresql".into(), "redis".into()];
        let ids3 = resolve(&r3);
        for expected in ["postgresql", "redis"] {
            assert!(ids3.iter().any(|i| i == expected), "нет {expected} после локального выбора в {ids3:?}");
        }
    }

    #[test]
    fn ktor_brings_jdk_and_gradle() {
        let mut r = req();
        r.frameworks = vec!["ktor".into()];
        let ids = resolve(&r);
        assert!(ids.iter().any(|i| i == "java"), "ktor без JDK: {ids:?}");
        assert!(ids.iter().any(|i| i == "gradle"), "ktor без gradle: {ids:?}");
        assert!(!ids.contains(&"kafka".to_string()), "kafka не обязательна для ktor: {ids:?}");
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
    fn terraform_and_firebase_tools_resolve() {
        let mut r = req();
        r.tools = vec!["grafana".into(), "terraform".into(), "firebase".into()];
        let ids = resolve(&r);
        for expected in ["terraform", "firebase"] {
            assert!(ids.iter().any(|i| i == expected), "нет {expected} в {ids:?}");
        }
        // grafana — «двойной» docker-инструмент: без локального выбора
        // его разворачивает docker-compose, локально он не нужен
        assert!(
            !ids.contains(&"grafana".to_string()),
            "grafana не должен требоваться локально без выбора: {ids:?}"
        );
        // явный локальный выбор — grafana попадает в требования
        let mut r2 = r.clone();
        r2.local_infra_tools = vec!["grafana".into()];
        let ids2 = resolve(&r2);
        assert!(
            ids2.iter().any(|i| i == "grafana"),
            "grafana должен появиться после локального выбора: {ids2:?}"
        );
    }

    #[test]
    fn dockerized_tools_require_only_docker() {
        // Docker-инструменты мастера (requires_docker: true) по умолчанию
        // не попадают в локальную проверку: их разворачивает docker-compose.
        // В требованиях остаётся только сам docker.
        let mut r = req();
        r.tools = vec![
            "postgresql".into(),
            "redis".into(),
            "mongodb".into(),
            "kafka".into(),
            "grafana".into(),
            "mysql".into(),
            "clickhouse".into(),
            "airflow".into(),
            "mailpit".into(),
            "docker".into(),
        ];
        assert_eq!(resolve(&r), vec!["winget", "docker"]);

        // попытка выбрать «локально» чисто docker-инструмент (нет источников
        // в tools.json) ничего не даёт: clickhouse/airflow/mailpit не замаплены
        let mut r2 = r.clone();
        r2.local_infra_tools = vec!["clickhouse".into(), "mailpit".into()];
        assert_eq!(resolve(&r2), vec!["winget", "docker"]);
    }

    #[test]
    fn local_opt_in_brings_dual_tools_to_requirements() {
        // «Двойные» инструменты (docker + локальная установка): выбор
        // local_infra_tools переносит их из docker-compose в требования.
        let mut r = req();
        r.tools = vec![
            "postgresql".into(),
            "redis".into(),
            "mongodb".into(),
            "kafka".into(),
            "grafana".into(),
            "mysql".into(),
        ];
        r.local_infra_tools = r.tools.clone();
        let ids = resolve(&r);
        for expected in ["postgresql", "redis", "mongodb", "kafka", "grafana", "mysql"] {
            assert!(
                ids.iter().any(|i| i == expected),
                "нет {expected} после локального выбора в {ids:?}"
            );
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
        // «Двойные» docker-инструменты замаплены: resolve() решает, нужны ли
        // они локально (только по выбору пользователя в local_infra_tools).
        assert_eq!(wizard_tool_to_toolchain("postgresql"), Some("postgresql"));
        assert_eq!(wizard_tool_to_toolchain("redis"), Some("redis"));
        assert_eq!(wizard_tool_to_toolchain("mongodb"), Some("mongodb"));
        assert_eq!(wizard_tool_to_toolchain("kafka"), Some("kafka"));
        assert_eq!(wizard_tool_to_toolchain("grafana"), Some("grafana"));
        assert_eq!(wizard_tool_to_toolchain("mysql"), Some("mysql"));
        // чисто docker-инструменты (нет локальных источников) не замаплены
        assert_eq!(wizard_tool_to_toolchain("clickhouse"), None);
        assert_eq!(wizard_tool_to_toolchain("airflow"), None);
        assert_eq!(wizard_tool_to_toolchain("mailpit"), None);
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

        let check_ids = |r: &ProjectRequirements, origin: &str, out: &mut Vec<String>| {
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
            // Локальная установка «двойных» docker-инструментов: resolve()
            // должен вернуть только id, существующие в tools.json
            // (postgresql/redis/mongodb/kafka/grafana/mysql).
            let mut r2 = req();
            r2.tools = vec![tool.clone()];
            r2.local_infra_tools = vec![tool.clone()];
            check_ids(&r2, &format!("тул {tool} (локально)"), &mut missing);
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
