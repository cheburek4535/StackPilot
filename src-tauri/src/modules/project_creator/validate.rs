// ============================================================
// Каноническая валидация стека (validate.rs)
// ============================================================
// Единственный источник правды о том, какие комбинации можно
// собирать. Фронтенд-зеркало (rules.ts) предсказывает эти ошибки
// для UX, но финальный барьер — эта функция: start_project_execution
// отказывается выполнять невалидный стек.
//
// Правила (все — severity=Error):
//   1. Фреймворк доступен на текущей ОС (platforms).
//   2. Взаимные конфликты из wizard_tree (conflicts).
//   3. Тип проекта разрешает фреймворк (project_types).
//   4. На каждую сторону — не более одного «главного» фреймворка
//      (kind="app", side != "either"). Универсальные (tauri, qt) и
//      побочные (aiogram, telegraf) этим правилом не ограничены —
//      только явными конфликтами.
//   5. Язык стороны совместим с фреймворком (side + languages).

use super::models::{FrameworkDef, WizardTreeData};

/// Одна найденная проблема стека.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StackIssue {
    /// Error — генерация блокируется, Warning — только предупреждение
    pub severity: StackSeverity,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum StackSeverity {
    Error,
    Warning,
}

/// ОС, на которой сейчас работает приложение ("windows", "macos", "linux").
pub fn current_os() -> &'static str {
    std::env::consts::OS
}

/// Фреймворк доступен на текущей ОС?
fn platform_ok(fw: &FrameworkDef, os: &str) -> bool {
    fw.platforms.is_empty() || fw.platforms.iter().any(|p| p == os)
}

/// На какой стороне живёт фреймворк в текущем выборе:
/// "backend", "frontend" или None (side="either" и язык не выбран).
fn resolved_side(
    fw: &FrameworkDef,
    backend_lang: Option<&str>,
    frontend_lang: Option<&str>,
) -> Option<&'static str> {
    match fw.side.as_str() {
        "backend" => Some("backend"),
        "frontend" => Some("frontend"),
        _ => {
            // "either": сторона языка, который фреймворк требует.
            if let Some(l) = backend_lang {
                if fw.languages.iter().any(|x| x == l) {
                    return Some("backend");
                }
            }
            if let Some(l) = frontend_lang {
                if fw.languages.iter().any(|x| x == l) {
                    return Some("frontend");
                }
            }
            None
        }
    }
}

/// Проверяет стек и возвращает найденные проблемы (порядок значимый).
pub fn validate_stack(
    tree: &WizardTreeData,
    project_type: Option<&str>,
    backend_lang: Option<&str>,
    frontend_lang: Option<&str>,
    frameworks: &[String],
    os: &str,
) -> Vec<StackIssue> {
    let mut issues: Vec<StackIssue> = Vec::new();

    let selected: Vec<&FrameworkDef> = frameworks
        .iter()
        .filter_map(|id| tree.frameworks.iter().find(|f| &f.id == id))
        .collect();

    // 1. Платформа
    for fw in &selected {
        if !platform_ok(fw, os) {
            issues.push(StackIssue {
                severity: StackSeverity::Error,
                message: format!(
                    "«{}» недоступен на этой ОС (требуется: {}).",
                    fw.label,
                    fw.platforms.join(", ")
                ),
            });
        }
    }

    // 2. Взаимные конфликты
    for a in &selected {
        for b in &selected {
            if a.id == b.id {
                continue;
            }
            if a.conflicts.contains(&b.id) {
                issues.push(StackIssue {
                    severity: StackSeverity::Error,
                    message: format!("«{}» несовместим с «{}».", a.label, b.label),
                });
            }
        }
    }

    // 3. Тип проекта
    if let Some(pt) = project_type {
        for fw in &selected {
            if !fw.project_types.is_empty() && !fw.project_types.iter().any(|p| p == pt) {
                issues.push(StackIssue {
                    severity: StackSeverity::Error,
                    message: format!(
                        "«{}» не подходит для проекта «{}». Выберите другой тип или снимите фреймворк.",
                        fw.label, pt
                    ),
                });
            }
        }
    }

    // 4. Не более одного «главного» (kind="app") фреймворка на сторону.
    //    side="either" — универсальные (tauri, qt): в лимит сторон не входят,
    //    ограничены только явными conflicts. Побочные (kind="side":
    //    aiogram, telegraf) могут соседствовать с любым числом других.
    let mut by_side: Vec<(&str, &FrameworkDef)> = Vec::new();
    for fw in &selected {
        if fw.kind == "app" && fw.side != "either" {
            by_side.push((fw.side.as_str(), fw));
        }
    }
    for (i, (side, a)) in by_side.iter().enumerate() {
        for (other_side, b) in by_side.iter().skip(i + 1) {
            if side == other_side {
                issues.push(StackIssue {
                    severity: StackSeverity::Error,
                    message: format!(
                        "«{}» и «{}» — оба главные фреймворки {}. На сторону можно выбрать только один главный фреймворк.",
                        a.label, b.label, side
                    ),
                });
            }
        }
    }

    // 5. Язык стороны должен подходить фреймворку
    for fw in &selected {
        match fw.side.as_str() {
            "backend" => {
                if !fw.languages.iter().any(|l| Some(l.as_str()) == backend_lang) {
                    issues.push(StackIssue {
                        severity: StackSeverity::Error,
                        message: format!(
                            "«{}» работает на бэкенде и требует один из языков: {}. Замените бэкенд-язык на «{}».",
                            fw.label,
                            fw.languages.join(", "),
                            fw.recommended_language
                        ),
                    });
                }
            }
            "frontend" => {
                if !fw.languages.iter().any(|l| Some(l.as_str()) == frontend_lang) {
                    issues.push(StackIssue {
                        severity: StackSeverity::Error,
                        message: format!(
                            "«{}» работает на фронтенде и требует один из языков: {}. Замените фронтенд-язык на «{}».",
                            fw.label,
                            fw.languages.join(", "),
                            fw.recommended_language
                        ),
                    });
                }
            }
            _ => {
                let backend_ok = backend_lang.is_some_and(|l| fw.languages.iter().any(|x| x == l));
                let frontend_ok =
                    frontend_lang.is_some_and(|l| fw.languages.iter().any(|x| x == l));
                if !backend_ok && !frontend_ok {
                    issues.push(StackIssue {
                        severity: StackSeverity::Error,
                        message: format!(
                            "«{}» требует один из языков: {} (на любой стороне).",
                            fw.label,
                            fw.languages.join(", ")
                        ),
                    });
                }
            }
        }
    }

    issues
}

/// Первая ошибка (для короткого сообщения пользователю).
pub fn first_error(issues: &[StackIssue]) -> Option<String> {
    issues
        .iter()
        .find(|i| matches!(i.severity, StackSeverity::Error))
        .map(|i| i.message.clone())
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> WizardTreeData {
        let raw = include_str!("knowledge/wizard_tree.json");
        serde_json::from_str(raw).expect("wizard_tree.json должен парситься")
    }

    fn fw(tree: &WizardTreeData, id: &str) -> FrameworkDef {
        tree.frameworks
            .iter()
            .find(|f| f.id == id)
            .unwrap_or_else(|| panic!("фреймворк {id} не найден"))
            .clone()
    }

    fn validate(
        tree: &WizardTreeData,
        pt: Option<&str>,
        b: Option<&str>,
        f: Option<&str>,
        fws: &[&str],
        os: &str,
    ) -> Vec<StackIssue> {
        let fws: Vec<String> = fws.iter().map(|s| s.to_string()).collect();
        validate_stack(tree, pt, b, f, &fws, os)
    }

    // ----------------------------------------------------------
    // Каноничность данных
    // ----------------------------------------------------------

    #[test]
    fn all_frameworks_wellformed() {
        let t = tree();
        for f in &t.frameworks {
            assert!(
                f.class == "standalone" || f.class == "inplace",
                "у фреймворка {} class не распознан: {:?}",
                f.id,
                f.class
            );
            assert!(
                f.kind == "app" || f.kind == "side",
                "у фреймворка {} kind не распознан: {:?}",
                f.id,
                f.kind
            );
            assert!(
                f.side == "backend" || f.side == "frontend" || f.side == "either",
                "у фреймворка {} side не распознан: {:?}",
                f.id,
                f.side
            );
            assert!(!f.languages.is_empty(), "у фреймворка {} нет languages", f.id);
            assert!(
                f.languages.contains(&f.recommended_language),
                "у фреймворка {} recommended_language не входит в languages",
                f.id
            );
            for r in &f.recommends {
                assert!(
                    t.frameworks.iter().any(|x| &x.id == &r.framework),
                    "у фреймворка {} recommends указывает на несуществующий {}",
                    f.id,
                    r.framework
                );
            }
        }
    }

    #[test]
    fn same_side_apps_declare_conflicts() {
        let t = tree();
        // Любые два app-фреймворка с одинаковой стороной, общим языком и
        // общим типом проекта обязаны быть в явном конфликте — иначе их
        // можно случайно совместить и сломать структуру проекта.
        for (i, a) in t.frameworks.iter().enumerate() {
            if a.kind != "app" || a.side == "either" {
                continue;
            }
            for b in t.frameworks.iter().skip(i + 1) {
                if b.kind != "app" || b.side == "either" {
                    continue;
                }
                if a.side != b.side {
                    continue;
                }
                let share_lang = a.languages.iter().any(|l| b.languages.contains(l));
                let share_type = a.project_types.is_empty()
                    || b.project_types.is_empty()
                    || a.project_types.iter().any(|p| b.project_types.contains(p));
                if share_lang && share_type {
                    assert!(
                        a.conflicts.contains(&b.id) || b.conflicts.contains(&a.id),
                        "app-фреймворки «{}» и «{}» на стороне {} не объявляют конфликт",
                        a.id,
                        b.id,
                        a.side
                    );
                }
            }
        }
    }

    #[test]
    fn no_game_engines_in_tree() {
        let t = tree();
        assert!(
            !t.frameworks.iter().any(|f| ["unity", "unreal", "godot"].contains(&f.id.as_str())),
            "игровые движки должны быть удалены из wizard_tree"
        );
        assert!(!t.project_types.iter().any(|p| p.id == "game"));
    }

    #[test]
    fn all_presets_valid() {
        let t = tree();
        assert!(!t.presets.is_empty(), "пресеты должны существовать");
        for p in &t.presets {
            let s = &p.stack;
            let issues = validate(
                &t,
                Some(&s.project_type),
                s.backend_lang.as_deref(),
                s.frontend_lang.as_deref(),
                &s.frameworks.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                "windows",
            );
            assert!(
                issues.is_empty(),
                "пресет {} невалиден: {:?}",
                p.id,
                issues
                    .iter()
                    .map(|i| i.message.clone())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn every_framework_has_side_consistent_with_engine() {
        let t = tree();
        // Каждый фреймворк с scaffold root/subdir обязан быть standalone —
        // иначе он не создаёт каркас сам.
        for f in &t.frameworks {
            if f.scaffold.is_some() {
                assert_eq!(f.class, "standalone", "scaffold-фреймворк {} должен быть standalone", f.id);
            }
        }
    }

    // ----------------------------------------------------------
    // Правило: сторона и язык
    // ----------------------------------------------------------

    #[test]
    fn nest_blocked_when_backend_is_cpp() {
        let t = tree();
        // Классический кейс из бага: cpp-бэкенд + ts-фронтенд + nest.
        let issues = validate(&t, Some("rest-api"), Some("cpp"), Some("typescript"), &["nest"], "windows");
        assert!(
            issues.iter().any(|i| i.message.contains("требует один из языков")),
            "{issues:?}"
        );
    }

    #[test]
    fn telegraf_blocked_when_backend_is_cpp() {
        let t = tree();
        let issues = validate(&t, Some("telegram-bot"), Some("cpp"), Some("typescript"), &["telegraf"], "windows");
        assert!(
            issues.iter().any(|i| i.message.contains("требует один из языков")),
            "{issues:?}"
        );
    }

    #[test]
    fn qt_and_plasmo_ok_with_cpp_backend() {
        let t = tree();
        // qt (either, cpp) и plasmo (frontend, ts) совместимы с cpp+ts.
        let issues = validate(
            &t,
            Some("custom"),
            Some("cpp"),
            Some("typescript"),
            &["qt", "plasmo"],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn framework_requires_language_side() {
        let t = tree();
        // express требует язык именно на бэкенде: ts только на фронте — ошибка.
        let issues = validate(&t, Some("rest-api"), None, Some("typescript"), &["express"], "windows");
        assert!(
            issues.iter().any(|i| i.message.contains("требует один из языков")),
            "{issues:?}"
        );
    }

    #[test]
    fn framework_ok_with_language_on_its_side() {
        let t = tree();
        let issues = validate(&t, Some("rest-api"), Some("typescript"), None, &["express"], "windows");
        assert!(issues.is_empty(), "{issues:?}");
    }

    // ----------------------------------------------------------
    // Правило: один главный фреймворк на сторону
    // ----------------------------------------------------------

    #[test]
    fn two_backend_apps_blocked() {
        let t = tree();
        // django + fastapi — оба главные бэкенд-фреймворки.
        let issues = validate(&t, Some("web-app"), Some("python"), None, &["django", "fastapi"], "windows");
        assert!(
            issues.iter().any(|i| i.message.contains("оба главные фреймворки")),
            "{issues:?}"
        );
    }

    #[test]
    fn two_frontend_apps_blocked() {
        let t = tree();
        let issues = validate(&t, Some("web-app"), None, Some("typescript"), &["nextjs", "sveltekit"], "windows");
        assert!(
            issues.iter().any(|i| i.message.contains("оба главные фреймворки")),
            "{issues:?}"
        );
    }

    #[test]
    fn django_plus_aiogram_allowed() {
        let t = tree();
        // Главный (django) + побочный (aiogram) на одном бэкенде — ок.
        let issues = validate(&t, Some("web-app"), Some("python"), None, &["django", "aiogram"], "windows");
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn side_framework_alone_allowed() {
        let t = tree();
        // aiogram сам по себе — валидный телеграм-бот.
        let issues = validate(&t, Some("telegram-bot"), Some("python"), None, &["aiogram"], "windows");
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn universal_app_not_counted_per_side() {
        let t = tree();
        // tauri (either) + react (frontend) — универсальный фреймворк не
        // занимает сторону: react владеет фронтендом, tauri — оболочка.
        let issues = validate(
            &t,
            Some("custom"),
            Some("rust"),
            Some("typescript"),
            &["tauri", "react"],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
        // А с конфликтующим главным фронтендом (nextjs) — блок.
        let issues = validate(
            &t,
            Some("custom"),
            Some("rust"),
            Some("typescript"),
            &["tauri", "nextjs"],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
            "{issues:?}"
        );
    }

    #[test]
    fn express_and_telegraf_blocked_by_conflict() {
        let t = tree();
        // express (app) + telegraf (side) — не side-лимит, а явный конфликт.
        let issues = validate(&t, Some("rest-api"), Some("typescript"), None, &["express", "telegraf"], "windows");
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
            "{issues:?}"
        );
    }

    #[test]
    fn backend_plus_frontend_allowed() {
        let t = tree();
        // nest (бэкенд) + nextjs (фронтенд) — легальный полный стек.
        let issues = validate(&t, Some("web-app"), Some("typescript"), Some("typescript"), &["nest", "nextjs"], "windows");
        assert!(issues.is_empty(), "{issues:?}");
    }

    // ----------------------------------------------------------
    // Правило: тип проекта
    // ----------------------------------------------------------

    #[test]
    fn project_type_filters_frameworks() {
        let t = tree();
        // telegraf не подходит для embedded.
        let issues = validate(&t, Some("embedded"), Some("cpp"), None, &["telegraf"], "windows");
        assert!(
            issues.iter().any(|i| i.message.contains("не подходит для проекта")),
            "{issues:?}"
        );
    }

    #[test]
    fn cli_frameworks_allowed_for_cli_tool() {
        let t = tree();
        let issues = validate(&t, Some("cli-tool"), Some("rust"), None, &["clap"], "windows");
        assert!(issues.is_empty(), "{issues:?}");
    }

    // ----------------------------------------------------------
    // Старые правила, которые остались
    // ----------------------------------------------------------

    #[test]
    fn macos_only_blocked_on_windows() {
        let t = tree();
        let issues = validate(&t, Some("mobile-app"), None, Some("swift"), &["swiftui"], "windows");
        assert!(
            issues.iter().any(|i| i.message.contains("SwiftUI") && i.message.contains("macos")),
            "{issues:?}"
        );
    }

    #[test]
    fn macos_only_allowed_on_macos() {
        let t = tree();
        let issues = validate(&t, Some("mobile-app"), None, Some("swift"), &["swiftui"], "macos");
        assert!(!issues.iter().any(|i| i.message.contains("SwiftUI")), "{issues:?}");
    }

    #[test]
    fn conflicts_still_enforced() {
        let t = tree();
        // nest + nextjs конфликтуют явно (tauri-ветки), но здесь — fallback-слой:
        // даже без side-правил список conflicts обязан работать.
        let issues = validate(&t, Some("web-app"), Some("typescript"), None, &["nest", "express"], "windows");
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
            "{issues:?}"
        );
    }
}
