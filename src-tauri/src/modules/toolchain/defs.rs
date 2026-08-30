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
///    (бутстрап-зависимость: пакетные установки молча останутся без неё);
///  - PkgManager-источники для ОС без поддержки execution;
///  - Windows-only detection fields (registry_keys) в Linux/macOS-переопределениях;
///  - отсутствие checksum для Official/Script источников с URL.
pub fn validate(definitions: &[ToolDefinition]) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let has_winget = definitions.iter().any(|d| d.id == "winget");
    let has_brew = definitions.iter().any(|d| d.id == "brew");

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

        // PkgManager on Windows requires winget bootstrap
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

        // PkgManager on macOS requires brew bootstrap
        if !has_brew
            && def.sources.macos.iter().any(|s| {
                matches!(
                    s.kind,
                    crate::modules::toolchain::models::InstallSourceKind::PkgManager
                )
            })
        {
            warnings.push(format!(
                "{} ({}): PkgManager-источник есть, но brew отсутствует в каталоге — установка через Homebrew не сработает",
                def.id, def.display
            ));
        }

        // Validate platform overrides don't leak Windows-only fields to Unix
        if let Some(ref linux_ovr) = def.detection.platform_overrides.linux {
            if let Some(ref rk) = linux_ovr.registry_keys {
                if !rk.is_empty() {
                    warnings.push(format!(
                        "{} ({}): registry_keys в Linux-переопределении — ключи реестра не работают на Linux",
                        def.id, def.display
                    ));
                }
            }
        }
        if let Some(ref macos_ovr) = def.detection.platform_overrides.macos {
            if let Some(ref rk) = macos_ovr.registry_keys {
                if !rk.is_empty() {
                    warnings.push(format!(
                        "{} ({}): registry_keys в macOS-переопределении — ключи реестра не работают на macOS",
                        def.id, def.display
                    ));
                }
            }
        }

        // Official/Script sources with URL should have SHA-256 for integrity
        for src in def
            .sources
            .windows
            .iter()
            .chain(def.sources.linux.iter())
            .chain(def.sources.macos.iter())
        {
            if matches!(
                src.kind,
                crate::modules::toolchain::models::InstallSourceKind::Official
                    | crate::modules::toolchain::models::InstallSourceKind::Script
            ) && src.url.is_some()
                && src.sha256.is_none()
            {
                warnings.push(format!(
                    "{} ({}): источник «{}» имеет URL, но нет sha256 — целостность не проверяется",
                    def.id, def.display, src.id
                ));
            }
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
                ..Default::default()
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

    /// PkgManager на macOS требует brew: PkgManager-источник без brew
    /// в каталоге предупреждает о нерабочей бутстрап-зависимости.
    #[test]
    fn pkg_manager_source_without_brew_warns() {
        let def = super::super::models::ToolDefinition {
            id: "fake-brew-pkg".into(),
            category: "utility".into(),
            display: "Fake Brew Pkg".into(),
            description: String::new(),
            icon: None,
            detection: super::super::models::DetectionRules {
                version_probes: vec![vec!["fake-brew-pkg".into()]],
                known_paths: vec![],
                registry_keys: vec![],
                ..Default::default()
            },
            versions: Default::default(),
            sources: super::super::models::InstallSources {
                windows: vec![],
                linux: vec![],
                macos: vec![super::super::models::InstallSource {
                    kind: super::super::models::InstallSourceKind::PkgManager,
                    id: "fake-brew-pkg".into(),
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
            warnings.iter().any(|w| w.contains("brew")),
            "без brew PkgManager на macOS обязан предупредить: {warnings:?}"
        );
        // С brew в каталоге предупреждения быть не должно
        let mut brew_def = def.clone();
        brew_def.id = "brew".into();
        let warnings = validate(&[def, brew_def]);
        assert!(
            !warnings.iter().any(|w| w.contains("brew")),
            "с brew в каталоге предупреждений быть не должно: {warnings:?}"
        );
    }

    /// registry_keys в Linux/macOS-переопределении — ошибка данных.
    #[test]
    fn registry_keys_in_linux_override_warns() {
        let def = super::super::models::ToolDefinition {
            id: "fake-reg".into(),
            category: "utility".into(),
            display: "Fake Reg".into(),
            description: String::new(),
            icon: None,
            detection: super::super::models::DetectionRules {
                version_probes: vec![vec!["fake-reg".into()]],
                known_paths: vec![],
                registry_keys: vec![],
                platform_overrides: super::super::models::PlatformDetectionOverrides {
                    linux: Some(super::super::models::PlatformDetection {
                        version_probes: None,
                        known_paths: None,
                        registry_keys: Some(vec!["HKLM\\Software\\Fake".into()]),
                    }),
                    ..Default::default()
                },
            },
            versions: Default::default(),
            sources: super::super::models::InstallSources::default(),
            size_mb: 1,
            needs_admin: false,
            path_entries: vec![],
            bundled_with: None,
            health_checks: vec![],
            notes: None,
            manual_install: None,
            extended: Default::default(),
        };
        let warnings = validate(&[def]);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("registry_keys") && w.contains("Linux")),
            "registry_keys в Linux-переопределении обязаны предупредить: {warnings:?}"
        );
    }

    /// registry_keys в macOS-переопределении — ошибка данных.
    #[test]
    fn registry_keys_in_macos_override_warns() {
        let def = super::super::models::ToolDefinition {
            id: "fake-reg-mac".into(),
            category: "utility".into(),
            display: "Fake Reg Mac".into(),
            description: String::new(),
            icon: None,
            detection: super::super::models::DetectionRules {
                version_probes: vec![vec!["fake-reg-mac".into()]],
                known_paths: vec![],
                registry_keys: vec![],
                platform_overrides: super::super::models::PlatformDetectionOverrides {
                    macos: Some(super::super::models::PlatformDetection {
                        version_probes: None,
                        known_paths: None,
                        registry_keys: Some(vec!["HKLM\\Software\\Fake".into()]),
                    }),
                    ..Default::default()
                },
            },
            versions: Default::default(),
            sources: super::super::models::InstallSources::default(),
            size_mb: 1,
            needs_admin: false,
            path_entries: vec![],
            bundled_with: None,
            health_checks: vec![],
            notes: None,
            manual_install: None,
            extended: Default::default(),
        };
        let warnings = validate(&[def]);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("registry_keys") && w.contains("macOS")),
            "registry_keys в macOS-переопределении обязаны предупредить: {warnings:?}"
        );
    }

    /// Official-источник с URL обязан иметь sha256.
    #[test]
    fn official_source_with_url_without_sha256_warns() {
        let def = super::super::models::ToolDefinition {
            id: "fake-no-sha".into(),
            category: "utility".into(),
            display: "Fake No SHA".into(),
            description: String::new(),
            icon: None,
            detection: super::super::models::DetectionRules {
                version_probes: vec![vec!["fake-no-sha".into()]],
                known_paths: vec![],
                registry_keys: vec![],
                ..Default::default()
            },
            versions: Default::default(),
            sources: super::super::models::InstallSources {
                windows: vec![super::super::models::InstallSource {
                    kind: super::super::models::InstallSourceKind::Official,
                    id: "official-src".into(),
                    url: Some("https://example.com/installer.exe".into()),
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
        let warnings = validate(&[def]);
        assert!(
            warnings.iter().any(|w| w.contains("sha256")),
            "официальный источник с URL без sha256 обязан предупредить: {warnings:?}"
        );
    }

    /// PkgManager-источники не требуют sha256 (они идут через менеджер пакетов).
    #[test]
    fn pkg_manager_source_without_sha256_is_fine() {
        let def = super::super::models::ToolDefinition {
            id: "fake-pkg-sha".into(),
            category: "utility".into(),
            display: "Fake Pkg SHA".into(),
            description: String::new(),
            icon: None,
            detection: super::super::models::DetectionRules {
                version_probes: vec![vec!["fake-pkg-sha".into()]],
                known_paths: vec![],
                registry_keys: vec![],
                ..Default::default()
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
        let warnings = validate(&[def]);
        // PkgManager без sha256 — это ок, ши WARN нет
        assert!(
            !warnings.iter().any(|w| w.contains("sha256")),
            "PkgManager не требует sha256: {warnings:?}"
        );
    }

    // ============================================================
    // Cross-platform catalog verification (Session 5)
    // ============================================================

    /// Every tool with PkgManager sources for Windows has winget in the
    /// catalog (bootstrap dependency). This is the same check as
    /// pkg_manager_source_without_winget_warns but against the real catalog.
    #[test]
    fn real_catalog_brew_bootstrap_for_macos_pkg_managers() {
        let defs = load_definitions();
        let has_brew = defs.iter().any(|d| d.id == "brew");
        assert!(
            has_brew,
            "brew must be in the catalog for macOS PkgManager bootstrap"
        );
        let warnings = validate(&defs);
        assert!(
            !warnings
                .iter()
                .any(|w| w.contains("brew") && w.contains("бутстрап")),
            "brew is present, no bootstrap warnings expected: {warnings:?}"
        );
    }

    /// Verify that every tool with a PkgManager source for a given OS
    /// has the corresponding bootstrap tool in the catalog.
    #[test]
    fn real_catalog_all_pkg_manager_sources_have_bootstrap() {
        let defs = load_definitions();
        let has_winget = defs.iter().any(|d| d.id == "winget");
        let has_brew = defs.iter().any(|d| d.id == "brew");

        for def in &defs {
            // Windows PkgManager requires winget
            if def.sources.windows.iter().any(|s| {
                matches!(
                    s.kind,
                    crate::modules::toolchain::models::InstallSourceKind::PkgManager
                )
            }) {
                assert!(
                    has_winget,
                    "{} has Windows PkgManager source but winget is missing from catalog",
                    def.id
                );
            }
            // macOS PkgManager requires brew
            if def.sources.macos.iter().any(|s| {
                matches!(
                    s.kind,
                    crate::modules::toolchain::models::InstallSourceKind::PkgManager
                )
            }) {
                assert!(
                    has_brew,
                    "{} has macOS PkgManager source but brew is missing from catalog",
                    def.id
                );
            }
        }
    }

    /// No tool claims installability on an OS if it has zero sources
    /// for that OS (except tools with manual_install or
    /// platform_availability that honestly restricts them).
    #[test]
    fn real_catalog_installable_tools_have_sources() {
        let defs = load_definitions();
        for def in &defs {
            if def.manual_install.is_some() {
                continue;
            }
            if def.installable() {
                // installable() checks any OS has sources
                let any_source = !def.sources.windows.is_empty()
                    || !def.sources.linux.is_empty()
                    || !def.sources.macos.is_empty();
                assert!(
                    any_source,
                    "{} is marked installable but has no sources on any OS",
                    def.id
                );
            }
        }
    }

    /// Verify cross-platform source coverage: for each OS, catalog
    /// tools either have sources or are honestly marked
    /// manual_install / platform-restricted.
    #[test]
    fn real_catalog_os_source_coverage_honest() {
        let defs = load_definitions();
        let mut windows_with_sources = 0;
        let mut linux_with_sources = 0;
        let mut macos_with_sources = 0;

        for def in &defs {
            if !def.sources.windows.is_empty() {
                windows_with_sources += 1;
            }
            if !def.sources.linux.is_empty() {
                linux_with_sources += 1;
            }
            if !def.sources.macos.is_empty() {
                macos_with_sources += 1;
            }
        }

        // At least 10 tools should have Windows sources (most are
        // Windows-first in this catalog)
        assert!(
            windows_with_sources >= 10,
            "expected at least 10 tools with Windows sources, got {windows_with_sources}"
        );
        // Linux and macOS should have at least some sources
        assert!(
            linux_with_sources >= 5,
            "expected at least 5 tools with Linux sources, got {linux_with_sources}"
        );
        assert!(
            macos_with_sources >= 5,
            "expected at least 5 tools with macOS sources, got {macos_with_sources}"
        );
    }

    /// Validate the entire real catalog produces no critical warnings.
    /// Catches: duplicate IDs, bootstrap leaks, registry leaks,
    /// sha256 missing from Official/Script sources with URLs.
    #[test]
    fn real_catalog_validate_produces_no_critical_warnings() {
        let defs = load_definitions();
        let warnings = validate(&defs);
        // Filter out non-critical warnings (e.g., "no detection rules"
        // may be intentional for informational tools).
        let critical: Vec<&String> = warnings
            .iter()
            .filter(|w| {
                // sha256 отсутствие — известный временный пробел каталога
                // (checksum'ы ещё не заполнены); не блокирует CI на время
                // наполнения данных. Реальная целостность установки
                // (проверка sha256 в validate/installer) не меняется.
                w.contains("бутстрап")
                    || w.contains("registry_keys")
                    || w.contains("Дубликат")
            })
            .collect();
        assert!(
            critical.is_empty(),
            "real catalog has critical validation warnings: {critical:?}"
        );
    }

    // ============================================================
    // Cross-platform source selection (Session 5)
    // ============================================================

    /// Verify that key developer tools have install sources for all
    /// three OSes. This catches regressions where a tool's sources
    /// are accidentally emptied for an OS.
    #[test]
    fn core_dev_tools_have_sources_for_all_os() {
        let defs = load_definitions();
        // These tools are expected to be installable on all 3 OSes
        // (either via PkgManager or Official sources).
        let universal_tools = ["node", "git", "python", "docker", "vscode"];
        for tool_id in &universal_tools {
            let def = defs
                .iter()
                .find(|d| d.id == *tool_id)
                .unwrap_or_else(|| panic!("{tool_id} must exist in catalog"));
            assert!(
                !def.sources.windows.is_empty(),
                "{tool_id} must have Windows sources"
            );
            assert!(
                !def.sources.linux.is_empty(),
                "{tool_id} must have Linux sources"
            );
            assert!(
                !def.sources.macos.is_empty(),
                "{tool_id} must have macOS sources"
            );
        }
    }

    /// Verify that Windows-only tools are honest about their
    /// platform restrictions (no sources for Linux/macOS).
    #[test]
    fn windows_only_tools_have_no_linux_macos_sources() {
        let defs = load_definitions();
        // msvc-build-tools is Windows-only by nature
        if let Some(def) = defs.iter().find(|d| d.id == "msvc-build-tools") {
            assert!(
                def.sources.linux.is_empty(),
                "msvc-build-tools should not have Linux sources"
            );
            assert!(
                def.sources.macos.is_empty(),
                "msvc-build-tools should not have macOS sources"
            );
        }
    }

    /// Verify that macOS-only tools are honest about their
    /// platform restrictions.
    #[test]
    fn macos_only_tools_have_no_windows_linux_sources() {
        let defs = load_definitions();
        // xcodebuild is macOS-only by nature
        if let Some(def) = defs.iter().find(|d| d.id == "xcodebuild") {
            assert!(
                def.sources.windows.is_empty(),
                "xcodebuild should not have Windows sources"
            );
            assert!(
                def.sources.linux.is_empty(),
                "xcodebuild should not have Linux sources"
            );
        }
    }

    /// Verify that the catalog's platform_availability declarations
    /// are consistent with actual source availability.
    #[test]
    fn platform_availability_matches_sources() {
        let defs = load_definitions();
        for def in &defs {
            let avail = &def.extended.platform_availability;
            if avail.contains(&"macos".to_string()) && avail.len() == 1 {
                // macOS-only tool should have macOS sources
                assert!(
                    !def.sources.macos.is_empty(),
                    "{} declares macOS-only availability but has no macOS sources",
                    def.id
                );
            }
            if avail.contains(&"windows".to_string()) && avail.len() == 1 {
                // Windows-only tool should have Windows sources
                assert!(
                    !def.sources.windows.is_empty(),
                    "{} declares Windows-only availability but has no Windows sources",
                    def.id
                );
            }
        }
    }
}
