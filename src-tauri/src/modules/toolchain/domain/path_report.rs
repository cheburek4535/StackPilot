// ============================================================
// Диагностика PATH (domain/path_report.rs)
// ============================================================
// Строго читающий анализ PATH: дубликаты, «мертвые» записи,
// нераскрытые переменные, непроверяемые записи и расхождения
// между найденными установками и видимостью в PATH.
//
// НИКАКИХ мутаций PATH здесь нет и не может быть: запись в PATH —
// только явное утверждённое задание установки (path_service).

use std::collections::HashSet;
use std::path::PathBuf;

use crate::modules::toolchain::core::discovery;
use crate::modules::toolchain::core::path_service;
use crate::modules::toolchain::models::ToolDefinition;

use super::models::DetectedInstall;
use super::models::{PathEntryReport, PathFinding, PathFindingKind, PathReport};

/// Нормализация записи для сравнения: единый разделитель, без хвостового
/// слеша; на Windows — без учёта регистра. Содержимое записей не меняется.
pub fn normalize_entry(entry: &str) -> String {
    let mut s = entry.trim().replace('/', "\\");
    while s.ends_with('\\') && s.len() > 1 {
        s.pop();
    }
    if cfg!(target_os = "windows") {
        s.to_ascii_lowercase()
    } else {
        s
    }
}

/// Есть ли в записи нераскрытые переменные вида %NAME%.
pub fn has_unexpanded_vars(entry: &str) -> bool {
    let mut rest = entry;
    while let Some(start) = rest.find('%') {
        if let Some(end_rel) = rest[start + 1..].find('%') {
            let name = &rest[start + 1..start + 1 + end_rel];
            if !name.is_empty() {
                return true;
            }
            rest = &rest[start + 2 + end_rel..];
        } else {
            return false;
        }
    }
    false
}

/// Разбирает список записей PATH и строит отчёт по каждой:
/// существование, дубликаты (точные и нормализованные), раскрытие,
/// проверяемость. Порядок записей сохраняется.
pub fn analyze_entries(entries: &[String]) -> Vec<PathEntryReport> {
    let mut reports = Vec::with_capacity(entries.len());
    for (index, raw) in entries.iter().enumerate() {
        let trimmed = raw.trim();
        let verifiable = !trimmed.is_empty();
        let expanded = path_service::expand_env_vars(trimmed);
        let requires_expansion = has_unexpanded_vars(trimmed);
        // Существование проверяем только по раскрытому пути без glob:
        // glob-записи (`PostgreSQL/*/bin`) резолвятся через glob_first.
        let exists = if verifiable && expanded.contains('*') {
            discovery::glob_first(&PathBuf::from(&expanded)).is_some()
        } else {
            verifiable && std::path::Path::new(&expanded).is_dir()
        };

        let mut duplicate_of = None;
        let mut case_duplicate_of = None;
        for (prev_index, prev) in entries.iter().enumerate().take(index) {
            if duplicate_of.is_none() && prev.trim() == trimmed {
                duplicate_of = Some(prev_index);
            }
            if case_duplicate_of.is_none() && normalize_entry(prev) == normalize_entry(trimmed) {
                case_duplicate_of = Some(prev_index);
            }
        }

        reports.push(PathEntryReport {
            raw: raw.clone(),
            expanded,
            exists,
            verifiable,
            duplicate_of,
            case_duplicate_of,
            requires_expansion,
        });
    }
    reports
}

/// Глобальные находки по разобранным записям (stale/дубликаты/...).
pub fn findings_from_reports(reports: &[PathEntryReport]) -> Vec<PathFinding> {
    let mut findings = Vec::new();
    let mut reported_case_dupes: HashSet<usize> = HashSet::new();

    for (_index, report) in reports.iter().enumerate() {
        if !report.verifiable {
            findings.push(PathFinding {
                kind: PathFindingKind::UnverifiableEntry,
                entry: report.raw.clone(),
                detail: "Запись пустая или не может быть проверена".to_string(),
            });
            continue;
        }
        if report.requires_expansion {
            findings.push(PathFinding {
                kind: PathFindingKind::RequiresExpansion,
                entry: report.raw.clone(),
                detail: format!(
                    "Запись содержит переменные, требующие раскрытия: {}",
                    report.expanded
                ),
            });
        }
        if let Some(first) = report.duplicate_of {
            findings.push(PathFinding {
                kind: PathFindingKind::DuplicateEntry,
                entry: report.raw.clone(),
                detail: format!("Точный дубликат записи #{first}"),
            });
        } else if let Some(first) = report.case_duplicate_of {
            // Одна находка на пару: помечаем вторую, первую запоминаем.
            if reported_case_dupes.insert(first) || !reported_case_dupes.contains(&first) {
                findings.push(PathFinding {
                    kind: PathFindingKind::CaseDuplicateEntry,
                    entry: report.raw.clone(),
                    detail: format!("Дубликат с точностью до нормализации записи #{first}"),
                });
            }
        } else if !report.exists {
            findings.push(PathFinding {
                kind: PathFindingKind::StaleEntry,
                entry: report.raw.clone(),
                detail: format!("Каталог не существует: {}", report.expanded),
            });
        }
    }
    findings
}

/// Полный отчёт по списку записей (разбор + находки).
pub fn build_report(entries: &[String]) -> PathReport {
    let reports = analyze_entries(entries);
    let findings = findings_from_reports(&reports);
    PathReport {
        entries: reports,
        findings,
    }
}

/// Находки PATH для одного инструмента:
///   - BinaryNotOnPath: установка отвечает, но её каталога нет в PATH;
///   - EntryMissing: каталоги из path_entries каталога отсутствуют в PATH.
pub fn tool_path_findings(
    def: &ToolDefinition,
    installs: &[DetectedInstall],
    process_entries: &[String],
) -> Vec<PathFinding> {
    let normalized_process: HashSet<String> =
        process_entries.iter().map(|e| normalize_entry(e)).collect();

    let mut findings = Vec::new();

    // Установка найдена вне PATH (пробой из known_paths), а каталога
    // в PATH нет — честная находка «бинарь есть, PATH молчит».
    for install in installs {
        if install.evidence == super::models::EvidenceKind::KnownPath
            && !install.reachable_via_path
            && !install.location.is_empty()
        {
            if let Some(parent) = std::path::Path::new(&install.location).parent() {
                let dir = parent.to_string_lossy().into_owned();
                if !normalized_process.contains(&normalize_entry(&dir)) {
                    findings.push(PathFinding {
                        kind: PathFindingKind::BinaryNotOnPath,
                        entry: dir.clone(),
                        detail: format!(
                            "{} найден, но каталог {} отсутствует в PATH",
                            def.display, dir
                        ),
                    });
                }
            }
        }
    }

    // Ожидаемые каталогом записи PATH: раскрываем %VAR% и glob
    // (`PostgreSQL/*/bin`) и проверяем присутствие в PATH процесса.
    if installs.is_empty() {
        return findings;
    }
    for entry in &def.path_entries {
        let expanded = path_service::expand_env_vars(entry);
        let resolved = if expanded.contains('*') {
            match discovery::glob_first(&PathBuf::from(&expanded)) {
                Some(found) => found.to_string_lossy().into_owned(),
                None => continue, // glob ничего не нашёл — установки и так нет
            }
        } else {
            expanded
        };
        if !normalized_process.contains(&normalize_entry(&resolved)) {
            findings.push(PathFinding {
                kind: PathFindingKind::EntryMissing,
                entry: resolved.clone(),
                detail: format!("Каталог установки {} не добавлен в PATH", resolved),
            });
        }
    }
    findings
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_duplicates_detected() {
        let entries = vec![
            "C:\\tools\\bin".to_string(),
            "C:\\Windows".to_string(),
            "C:\\tools\\bin".to_string(),
        ];
        let reports = analyze_entries(&entries);
        assert!(reports[0].duplicate_of.is_none());
        assert_eq!(reports[2].duplicate_of, Some(0));
        assert!(reports[0].exists || !std::path::Path::new("C:\\tools\\bin").is_dir());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn case_and_slash_duplicates_detected_on_windows() {
        let entries = vec![
            "C:\\Program Files\\nodejs".to_string(),
            "c:/program files/NODEJS\\".to_string(),
        ];
        let reports = analyze_entries(&entries);
        assert!(
            reports[1].duplicate_of.is_none(),
            "регистр/слеши не дают точного дубликата"
        );
        assert_eq!(reports[1].case_duplicate_of, Some(0));
    }

    #[test]
    fn stale_entry_reported() {
        let missing = if cfg!(target_os = "windows") {
            "Z:\\definitely-not-exists-xyz\\bin"
        } else {
            "/nonexistent-definitely-xyz/bin"
        };
        let reports = analyze_entries(&[missing.to_string()]);
        assert!(!reports[0].exists);
        let findings = findings_from_reports(&reports);
        assert!(findings
            .iter()
            .any(|f| f.kind == PathFindingKind::StaleEntry));
    }

    #[test]
    fn unexpanded_var_flagged() {
        assert!(has_unexpanded_vars("%ProgramFiles%\\nodejs"));
        assert!(has_unexpanded_vars("C:\\%UNKNOWN_VAR%\\x"));
        assert!(!has_unexpanded_vars("C:\\plain\\path"));
        assert!(!has_unexpanded_vars("100% done")); // % без пары — не переменная

        let reports = analyze_entries(&["%SOME_DIR_XYZ%\\bin".to_string()]);
        assert!(reports[0].requires_expansion);
    }

    #[test]
    fn empty_entry_is_unverifiable() {
        let reports = analyze_entries(&["   ".to_string(), "".to_string()]);
        assert!(!reports[0].verifiable);
        let findings = findings_from_reports(&reports);
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.kind == PathFindingKind::UnverifiableEntry)
                .count(),
            2
        );
    }

    #[test]
    fn normalize_matches_mixed_separators() {
        if cfg!(target_os = "windows") {
            assert_eq!(normalize_entry("C:/a/b/"), normalize_entry("c:\\a\\b"));
        }
    }

    #[test]
    fn build_report_keeps_order_and_fills_findings() {
        let missing_dir = if cfg!(target_os = "windows") {
            "Q:\\gone\\bin"
        } else {
            "/gone/bin"
        };
        let entries = vec![
            missing_dir.to_string(),
            "C:\\dup-test-xyz".to_string(),
            "C:\\dup-test-xyz".to_string(),
        ];
        let report = build_report(&entries);
        assert_eq!(report.entries.len(), 3);
        assert!(report
            .findings
            .iter()
            .any(|f| f.kind == PathFindingKind::StaleEntry));
        assert!(report
            .findings
            .iter()
            .any(|f| f.kind == PathFindingKind::DuplicateEntry));
    }

    /// Инструмент найден known_path-пробой вне PATH → BinaryNotOnPath;
    /// ожидаемая path_entries запись отсутствует → EntryMissing.
    #[test]
    fn tool_findings_cover_not_on_path_and_missing_entry() {
        let mut def = crate::modules::toolchain::defs::load_definitions()
            .into_iter()
            .next()
            .unwrap();
        def.path_entries = vec![if cfg!(target_os = "windows") {
            "Z:\\no-such-dir-xyz\\bin".to_string()
        } else {
            "/no-such-dir-xyz/bin".to_string()
        }];

        let installs = vec![DetectedInstall {
            raw_version: "psql (PostgreSQL) 17.2".to_string(),
            parsed_version: Some("17.2".to_string()),
            location: if cfg!(target_os = "windows") {
                "C:\\Program Files\\PostgreSQL\\17\\bin\\psql.exe".to_string()
            } else {
                "/usr/lib/postgresql/17/bin/psql".to_string()
            },
            evidence: super::super::models::EvidenceKind::KnownPath,
            reachable_via_path: false,
            path_scope: None,
            probe_log: None,
        }];

        let findings = tool_path_findings(&def, &installs, &[]);
        assert!(findings
            .iter()
            .any(|f| f.kind == PathFindingKind::BinaryNotOnPath));
        assert!(findings
            .iter()
            .any(|f| f.kind == PathFindingKind::EntryMissing));
    }

    /// Установка, чей каталог уже в PATH, не порождает ложных находок.
    #[test]
    fn on_path_install_produces_no_binary_finding() {
        let def = crate::modules::toolchain::defs::load_definitions()
            .into_iter()
            .next()
            .unwrap();

        let dir = std::env::temp_dir().join(format!("tc-on-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("probe.exe");
        std::fs::write(&bin, b"mz").unwrap();

        let installs = vec![DetectedInstall {
            raw_version: "1.0".to_string(),
            parsed_version: Some("1.0".to_string()),
            location: bin.to_string_lossy().into_owned(),
            evidence: super::super::models::EvidenceKind::KnownPath,
            reachable_via_path: false,
            path_scope: None,
            probe_log: None,
        }];
        // Каталог родитель передан как запись PATH процесса — находки нет.
        let process_entries = vec![dir.to_string_lossy().into_owned()];
        let findings = tool_path_findings(&def, &installs, &process_entries);
        assert!(
            findings.is_empty(),
            "ожидали чистый результат: {findings:?}"
        );

        let _ = std::fs::remove_file(&bin);
        let _ = std::fs::remove_dir(&dir);
    }
}
