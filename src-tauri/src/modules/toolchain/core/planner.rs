// ============================================================
// Составление плана установки (planner.rs)
// ============================================================
// Этап 3: по отчёту проверки (EnvironmentCheck) строит InstallPlan —
// список задач, которые нужно выполнить, чтобы окружение стало
// готовым.
//
// Правила:
//   - задача создаётся только для «не готовых» требований
//     (Missing или UpdateAvailable — устаревшие идут на апгрейд);
//   - Installed тулы в план не попадают (работать уже можно);
//   - ManualInstall (движки, SDK) в план не попадают НИКОГДА —
//     их установка выполняется вручную;
//   - winget ставится ПЕРВЫМ: пока его нет, все остальные пакеты
//     на Windows поставить нечем.

use std::collections::{HashMap, HashSet};

use crate::modules::toolchain::models::*;

// ------------------------------------------------------------
// Канонизация плана (бэкенд — авторитет)
// ------------------------------------------------------------

/// Пересобирает план из КАТАЛОГА: фронтенд присылает только список
/// tool_id (и опции Qt), всё остальное — display, размер, admin-флаг,
/// описание источника — берётся из tools.json бэкенда. Подделать
/// источник установки/URL/аргументы через payload невозможно.
///
/// Правила:
///   - неизвестный id → ошибка (план отклоняется целиком);
///   - дубликаты id схлопываются;
///   - manual_install-инструменты не ставятся никогда;
///   - инструменты без источников для текущей ОС отбрасываются;
///   - os берётся у платформенного слоя, не из payload.
pub fn canonicalize_plan(
    definitions: &[ToolDefinition],
    requested_ids: &[String],
    install_options: &HashMap<String, Vec<String>>,
) -> Result<InstallPlan, String> {
    let os = crate::modules::toolchain::platforms::current_platform().os_name();
    let mut tasks: Vec<InstallTask> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut total_size_mb: u64 = 0;

    for id in requested_ids {
        if !seen.insert(id.clone()) {
            continue; // дубликат
        }
        let Some(def) = definitions.iter().find(|d| d.id == *id) else {
            return Err(format!("Неизвестный инструмент в плане: {id}"));
        };
        // Ручная установка не автоматизируется никогда.
        if def.manual_install.is_some() {
            continue;
        }
        let os_sources: &[InstallSource] = match os.as_str() {
            "windows" => def.sources.windows.as_slice(),
            "linux" => def.sources.linux.as_slice(),
            "macos" => def.sources.macos.as_slice(),
            _ => &[],
        };
        // Нет источников на этой ОС — задача бессмысленна.
        if os_sources.is_empty() {
            continue;
        }

        total_size_mb += def.size_mb as u64;
        tasks.push(InstallTask {
            task_id: def.id.clone(),
            tool_id: def.id.clone(),
            display: def.display.clone(),
            icon: def.icon.clone(),
            size_mb: def.size_mb,
            needs_admin: def.needs_admin,
            source_description: source_description_for(def, &os),
            install_options: install_options.get(&def.id).cloned().unwrap_or_default(),
            state: TaskState::Pending,
        });
    }

    // winget всегда первым (как и в build_plan).
    if let Some(i) = tasks.iter().position(|t| t.tool_id == "winget") {
        if i != 0 {
            let winget = tasks.remove(i);
            tasks.insert(0, winget);
        }
    }

    // Замыкание зависимостей (bundled_with + extended.dependencies):
    // composer без php невозможен («Не удалось запустить php») — недостающие
    // зависимости добавляются задачами и упорядочиваются ПЕРЕД зависимыми.
    add_missing_dependencies(&mut tasks, definitions, &os, &mut total_size_mb);

    Ok(InstallPlan {
        tasks,
        total_size_mb,
        os,
        session_id: String::new(),
    })
}

/// Добавляет недостающие зависимости (bundled_with / extended.dependencies)
/// в конец плана и стабильно упорядочивает задачи: зависимые всегда после
/// своих зависимостей (порядок остальных задач сохраняется).
fn add_missing_dependencies(
    tasks: &mut Vec<InstallTask>,
    definitions: &[ToolDefinition],
    os: &str,
    total_size_mb: &mut u64,
) {
    // 1. Недостающие зависимости — задачами (по одному проходу; цикл
    //    повторяется, пока замыкание не сойдётся: у php может быть
    //    своя зависимость и т.п.).
    loop {
        let mut added = false;
        let mut extra: Vec<InstallTask> = Vec::new();
        for task in tasks.iter() {
            let Some(def) = definitions.iter().find(|d| d.id == task.tool_id) else {
                continue;
            };
            for dep in declared_dependency_ids(def) {
                if tasks.iter().any(|t| t.tool_id == dep)
                    || extra.iter().any(|t| t.tool_id == dep)
                {
                    continue;
                }
                let Some(dep_def) = definitions.iter().find(|d| d.id == dep) else {
                    continue;
                };
                if dep_def.manual_install.is_some() {
                    continue;
                }
                let os_sources: &[InstallSource] = match os {
                    "windows" => dep_def.sources.windows.as_slice(),
                    "linux" => dep_def.sources.linux.as_slice(),
                    "macos" => dep_def.sources.macos.as_slice(),
                    _ => &[],
                };
                if os_sources.is_empty() {
                    continue;
                }
                *total_size_mb += dep_def.size_mb as u64;
                extra.push(InstallTask {
                    task_id: dep_def.id.clone(),
                    tool_id: dep_def.id.clone(),
                    display: dep_def.display.clone(),
                    icon: dep_def.icon.clone(),
                    size_mb: dep_def.size_mb,
                    needs_admin: dep_def.needs_admin,
                    source_description: source_description_for(dep_def, os),
                    install_options: Vec::new(),
                    state: TaskState::Pending,
                });
                added = true;
            }
        }
        if extra.is_empty() {
            break;
        }
        tasks.extend(extra);
        if !added {
            break;
        }
    }

    // 2. Стабильная топологическая сортировка: задача не встаёт раньше
    //    своей (всё ещё не размещённой) зависимости. Циклы невозможны
    //    (планировщик не создаёт обратных рёбер), но страховка есть:
    //    остаток дописывается в исходном порядке, без потери задач.
    let mut placed: HashSet<String> = HashSet::new();
    let mut ordered: Vec<InstallTask> = Vec::with_capacity(tasks.len());
    let mut remaining = std::mem::take(tasks);
    while !remaining.is_empty() {
        let idx = remaining.iter().position(|t| {
            let Some(def) = definitions.iter().find(|d| d.id == t.tool_id) else {
                return true;
            };
            declared_dependency_ids(def).iter().all(|dep| {
                !remaining.iter().any(|o| &o.tool_id == dep) || placed.contains(dep)
            })
        });
        let Some(idx) = idx else {
            ordered.extend(remaining);
            *tasks = ordered;
            return;
        };
        let task = remaining.remove(idx);
        placed.insert(task.tool_id.clone());
        ordered.push(task);
    }
    *tasks = ordered;
}

/// Все объявленные зависимости инструмента: bundled-хост (npm→node)
/// и явный список extended.dependencies (composer→php).
fn declared_dependency_ids(def: &ToolDefinition) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(host) = def.bundled_with.as_deref() {
        ids.push(host.to_string());
    }
    for dep in def.extended.dependencies.iter() {
        if !ids.contains(dep) {
            ids.push(dep.clone());
        }
    }
    ids
}

/// Человекочитаемое описание первого источника на указанной ОС
/// (дублирует логику check::source_description, но работает с явной ОС).
fn source_description_for(def: &ToolDefinition, os: &str) -> String {
    if def.manual_install.is_some() {
        return "Устанавливается вручную".to_string();
    }
    if !def.installable() {
        return "Отдельная установка не требуется".to_string();
    }
    let sources: &[InstallSource] = match os {
        "windows" => def.sources.windows.as_slice(),
        "linux" => def.sources.linux.as_slice(),
        "macos" => def.sources.macos.as_slice(),
        _ => &[],
    };
    let Some(first) = sources.first() else {
        return "Установка на этой ОС не предусмотрена".to_string();
    };
    match first.kind {
        InstallSourceKind::PkgManager => format!("Менеджер пакетов: {}", first.id),
        InstallSourceKind::Official => format!("Официальный установщик: {}", first.id),
        InstallSourceKind::Script => format!("Скрипт установки: {}", first.id),
        InstallSourceKind::QtOnline => "Официальный репозиторий Qt (online)".to_string(),
    }
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------
    // Канонизация (защита от подделки плана фронтендом)
    //
    // Семантика планирования («пропущены установленные», «winget первый»,
    // «manual/docker не ставятся») покрыта тестами канонического
    // планировщика engine/planner.rs — здесь проверяется только
    // пересборка содержимого задач из каталога.
    // ------------------------------------------------------------

    /// Официальный источник, одинаковый для всех ОС: канонизация берёт
    /// источники слота текущей ОС, тесты не должны зависеть от платформы.
    fn catalog_source() -> InstallSource {
        InstallSource {
            kind: InstallSourceKind::Official,
            id: "src".to_string(),
            url: Some("https://example.com/x.exe".to_string()),
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            file_name: None,
            execution: None,
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
        }
    }

    fn catalog_def(id: &str) -> ToolDefinition {
        ToolDefinition {
            id: id.to_string(),
            category: "utility".to_string(),
            display: format!("Display {id}"),
            description: String::new(),
            icon: None,
            detection: DetectionRules {
                version_probes: vec![],
                known_paths: vec![],
                registry_keys: vec![],
                ..Default::default()
            },
            versions: Default::default(),
            sources: InstallSources {
                windows: vec![catalog_source()],
                linux: vec![catalog_source()],
                macos: vec![catalog_source()],
            },
            size_mb: 42,
            needs_admin: true,
            path_entries: vec![],
            bundled_with: None,
            health_checks: vec![],
            notes: None,
            manual_install: None,
            extended: Default::default(),
        }
    }

    #[test]
    fn canonical_plan_rebuilds_content_from_catalog() {
        let defs = vec![catalog_def("node"), catalog_def("winget")];
        let plan = canonicalize_plan(
            &defs,
            &["node".to_string(), "winget".to_string()],
            &HashMap::new(),
        )
        .unwrap();

        // winget вынесен вперёд, содержимое задач — из каталога.
        assert_eq!(plan.tasks[0].tool_id, "winget");
        assert_eq!(plan.tasks[1].tool_id, "node");
        assert_eq!(plan.tasks[1].display, "Display node");
        assert_eq!(plan.tasks[1].size_mb, 42);
        assert!(plan.tasks[1].needs_admin);
        assert_eq!(plan.total_size_mb, 84);
        // os берётся у платформенного слоя, не из payload.
        assert_eq!(
            plan.os,
            crate::modules::toolchain::platforms::current_platform().os_name()
        );
    }

    #[test]
    fn canonical_plan_rejects_unknown_tool() {
        let defs = vec![catalog_def("node")];
        let err = canonicalize_plan(
            &defs,
            &["node".to_string(), "evil-tool".to_string()],
            &HashMap::new(),
        )
        .unwrap_err();
        assert!(
            err.contains("evil-tool"),
            "неизвестный id отклоняется: {err}"
        );
    }

    #[test]
    fn canonical_plan_dedupes_and_skips_manual_and_unsupported() {
        let mut manual = catalog_def("unity");
        manual.manual_install = Some("ставится вручную".to_string());
        let mut no_sources = catalog_def("xcodebuild");
        no_sources.sources = InstallSources::default();
        let defs = vec![catalog_def("node"), manual, no_sources];

        let plan = canonicalize_plan(
            &defs,
            &[
                "node".to_string(),
                "node".to_string(),       // дубликат
                "unity".to_string(),      // manual — мимо
                "xcodebuild".to_string(), // нет источников на ОС — мимо
            ],
            &HashMap::new(),
        )
        .unwrap();

        let ids: Vec<&str> = plan.tasks.iter().map(|t| t.tool_id.as_str()).collect();
        assert_eq!(ids, vec!["node"]);
    }

    #[test]
    fn canonical_plan_carries_install_options_for_requested_tool_only() {
        let defs = vec![catalog_def("qt"), catalog_def("node")];
        let mut options = HashMap::new();
        options.insert("qt".to_string(), vec!["qt-webengine".to_string()]);
        let plan =
            canonicalize_plan(&defs, &["qt".to_string(), "node".to_string()], &options).unwrap();
        let qt = plan.tasks.iter().find(|t| t.tool_id == "qt").unwrap();
        let node = plan.tasks.iter().find(|t| t.tool_id == "node").unwrap();
        assert_eq!(qt.install_options, vec!["qt-webengine".to_string()]);
        assert!(node.install_options.is_empty());
    }

    /// Объявленная зависимость (extended.dependencies: composer→php)
    /// добавляется в план ПЕРЕД зависимым, даже если пользователь выбрал
    /// только composer. Регрессия: composer без php падает на обоих
    /// источниках («Не удалось запустить php»).
    #[test]
    fn canonical_plan_adds_declared_dependency_first() {
        let mut composer = catalog_def("composer");
        composer.extended.dependencies = vec!["php".to_string()];
        let defs = vec![catalog_def("php"), composer];

        let plan = canonicalize_plan(&defs, &["composer".to_string()], &HashMap::new()).unwrap();
        let ids: Vec<&str> = plan.tasks.iter().map(|t| t.tool_id.as_str()).collect();
        assert_eq!(ids, vec!["php", "composer"], "php обязан идти раньше");
        assert_eq!(plan.total_size_mb, 84, "размер зависимости учтён");
    }

    /// Зависимость уже в списке запрошенных — дубликата нет, порядок
    /// зависимость-первой сохраняется.
    #[test]
    fn canonical_plan_does_not_duplicate_requested_dependency() {
        let mut composer = catalog_def("composer");
        composer.extended.dependencies = vec!["php".to_string()];
        let defs = vec![catalog_def("php"), composer];

        let plan = canonicalize_plan(
            &defs,
            &["composer".to_string(), "php".to_string()],
            &HashMap::new(),
        )
        .unwrap();
        let php_count = plan.tasks.iter().filter(|t| t.tool_id == "php").count();
        assert_eq!(php_count, 1, "дубликат зависимости: {:?}", plan.tasks);
        assert_eq!(plan.tasks[0].tool_id, "php");
        assert_eq!(plan.tasks[1].tool_id, "composer");
    }
}
