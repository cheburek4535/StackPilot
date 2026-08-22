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
    let definitions: Vec<ToolDefinition> = serde_json::from_str(raw).expect(
        "tools.json должен быть корректным JSON и соответствовать структуре ToolDefinition",
    );
    definitions
}

/// Легаси-каталог совместимости Project Creator (unity/unreal/godot).
///
/// Эти движки УДАЛЕНЫ из standalone-каталога tools.json и не участвуют
/// ни в сканах tcx_*, ни в планах канонического движка, ни в каталоге
/// standalone UI. Определения сохранены ЗДЕСЬ исключительно для
/// легаси-данных Project Creator: wizard_tree.json ссылается на
/// фреймворки unity/unreal/godot, и мастер обязан получать честный
/// отчёт «ставится вручную» вместо молчаливой потери требования
/// (контракт §7: легаси-совместимость только там, где она нужна).
pub fn load_legacy_definitions() -> Vec<ToolDefinition> {
    let raw = include_str!("legacy_compat_tools.json");
    serde_json::from_str(raw).expect(
        "legacy_compat_tools.json должен быть корректным JSON и соответствовать ToolDefinition",
    )
}

/// Объединённый каталог для ЛЕГАСИ-команд (tc_*): standalone-каталог +
/// легаси-совместимость. Порядок: сначала основной каталог.
/// Дубликаты id между файлами — ошибка разработчика (паника при старте).
pub fn load_merged_definitions() -> Vec<ToolDefinition> {
    let mut all = load_definitions();
    for legacy in load_legacy_definitions() {
        assert!(
            !all.iter().any(|d| d.id == legacy.id),
            "легаси-id {} пересекается со standalone-каталогом",
            legacy.id
        );
        all.push(legacy);
    }
    all
}

/// Базовая валидация определений. Возвращает список предупреждений:
///  - дубликаты id (сломают lookup по id);
///  - определения без правил обнаружения (никогда не найдутся);
///  - определения с версией min > recommended (ошибка в данных);
///  - PkgManager-источники без инструмента `winget` в каталоге
///    (бутстрап-зависимость: пакетные установки молча останутся без неё).
pub fn validate(definitions: &[ToolDefinition]) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let has_winget = definitions.iter().any(|d| d.id == "winget");

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
        if !has_winget
            && def.sources.windows.iter().any(|s| {
                matches!(
                    s.kind,
                    crate::modules::toolchain::models::InstallSourceKind::PkgManager
                )
            })
        {
            warnings.push(format!(
                "{} ({}): PkgManager-источник есть, но инструмент winget отсутствует в каталоге — бутстрап пакетных установок не сработает",
                def.id, def.display
            ));
        }
    }
    warnings
}

// ============================================================
// Тесты целостности каталогов
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Игровые движки удалены из STANDALONE-каталога: они не появляются
    /// ни в каталоге UI, ни в сканах tcx_*, ни в планах планировщика.
    #[test]
    fn legacy_engines_are_absent_from_standalone_catalog() {
        let defs = load_definitions();
        for legacy_id in ["unity", "unreal", "godot"] {
            assert!(
                !defs.iter().any(|d| d.id == legacy_id),
                "{legacy_id} не должен входить в tools.json"
            );
        }
    }

    /// Легаси-совместимость Project Creator сохранена отдельно и ТОЛЬКО
    /// там: определения движков живут в legacy_compat_tools.json.
    #[test]
    fn legacy_engines_live_only_in_legacy_catalog() {
        let legacy = load_legacy_definitions();
        let ids: Vec<&str> = legacy.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids.len(), 3);
        for expected in ["unity", "unreal", "godot"] {
            assert!(ids.contains(&expected), "нет {expected}: {ids:?}");
        }
        // Ни один легаси-id не пересекается со standalone-каталогом —
        // иначе merged-каталог молча дублировал бы определения.
        let standalone = load_definitions();
        for d in &legacy {
            assert!(
                !standalone.iter().any(|s| s.id == d.id),
                "легаси-id {} пересекается со standalone",
                d.id
            );
        }
    }

    /// Swift на Windows: официальный дистрибутив существует, источник —
    /// download.swift.org, целостность ЯВНАЯ (sha256 из официального
    /// манифеста winget). Фейковый URL или источник без контрольной
    /// суммы ломают этот тест.
    #[test]
    fn swift_windows_official_source_carries_explicit_integrity() {
        let defs = load_definitions();
        let swift = defs
            .iter()
            .find(|d| d.id == "swift")
            .expect("swift в каталоге");
        assert!(
            !swift.sources.windows.is_empty(),
            "у Swift есть локальный источник для Windows"
        );
        assert!(
            swift.manual_install.is_none(),
            "Swift больше не manual-only: официальный дистрибутив существует"
        );
        let official = swift.sources.windows.iter().find(|s| {
            s.url
                .as_deref()
                .map(|u| u.starts_with("https://download.swift.org/"))
                .unwrap_or(false)
        });
        let src = official.expect("официальный источник download.swift.org для Windows");
        let sha = src.sha256.as_deref().expect("sha256 обязателен");
        assert_eq!(sha.len(), 64, "sha256 в hex");
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// Xcode объявлен доступным только на macOS: классификация не станет
    /// «ручной установкой» на чужих ОС (см. classify_applicability).
    #[test]
    fn xcode_declares_macos_only_availability() {
        let defs = load_definitions();
        let xcode = defs
            .iter()
            .find(|d| d.id == "xcodebuild")
            .expect("xcodebuild в каталоге");
        assert_eq!(xcode.extended.platform_availability, vec!["macos"]);
    }

    /// Docker-related инструменты БД/наблюдаемости заявлены локально
    /// устанавливаемыми на текущую платформу-разработчика (Windows) и
    /// НЕСУТ docker только как метаданные-альтернативу.
    #[test]
    fn docker_related_tools_are_locally_installable_with_docker_metadata() {
        let defs = load_definitions();
        for id in [
            "postgresql",
            "redis",
            "mongodb",
            "kafka",
            "grafana",
            "mysql",
        ] {
            let def = defs
                .iter()
                .find(|d| d.id == id)
                .unwrap_or_else(|| panic!("{id} в каталоге"));
            assert!(def.installable(), "{id} должен иметь источники установки");
            assert!(
                !def.sources.windows.is_empty(),
                "{id}: есть источник для Windows"
            );
            assert!(
                def.extended.docker.is_some(),
                "{id}: docker заявлен как альтернатива (метаданные)"
            );
        }
    }

    /// Бутстрап-зависимость пакетных установок: в standalone-каталоге
    /// обязан быть сам winget, иначе PkgManager-задачи молча останутся
    /// без зависимости.
    #[test]
    fn real_catalog_has_winget_bootstrap_and_no_winget_warning() {
        let defs = load_definitions();
        assert!(
            defs.iter().any(|d| d.id == "winget"),
            "winget обязан быть в каталоге для PkgManager-бутстрапа"
        );
        let warnings = validate(&defs);
        assert!(
            !warnings.iter().any(|w| w.contains("бутстрап")),
            "при наличии winget предупреждений о бутстрапе быть не должно: {warnings:?}"
        );
    }

    /// Если каталог когда-нибудь потеряет winget, PkgManager-источники
    /// обязаны честно предупредить разработчика при валидации.
    #[test]
    fn pkg_manager_source_without_winget_warns() {
        let mut def = super::super::models::ToolDefinition {
            id: "fake-pkg".into(),
            category: "utility".into(),
            display: "Fake Pkg".into(),
            description: String::new(),
            icon: None,
            detection: super::super::models::DetectionRules {
                version_probes: vec![vec!["fake-pkg".into()]],
                known_paths: vec![],
                registry_keys: vec![],
            },
            versions: Default::default(),
            sources: super::super::models::InstallSources {
                windows: vec![super::super::models::InstallSource {
                    kind: super::super::models::InstallSourceKind::PkgManager,
                    id: "Fake.Pkg".into(),
                    url: None,
                    args: vec![],
                    extra_args: vec![],
                    dynamic_args: false,
                    install_dir: None,
                    needs_admin: None,
                    file_name: None,
                    execution: None,
                    sha256: None,
                }],
                linux: vec![],
                macos: vec![],
            },
            size_mb: 1,
            needs_admin: false,
            path_entries: vec![],
            bundled_with: None,
            health_checks: vec![],
            notes: None,
            manual_install: None,
            extended: Default::default(),
        };
        let warnings = validate(&[def.clone()]);
        assert!(
            warnings.iter().any(|w| w.contains("бутстрап")),
            "без winget PkgManager-источник обязан предупредить: {warnings:?}"
        );
        def.id = "winget".into();
        let warnings = validate(&[def]);
        assert!(
            !warnings.iter().any(|w| w.contains("бутстрап")),
            "с самим winget предупреждения быть не должно: {warnings:?}"
        );
    }
}
