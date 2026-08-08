// ============================================================
// Валидация стека (validate.rs)
// ============================================================
// Проверяет выбранный пользователем стек до начала генерации:
//   - фреймворк доступен на текущей ОС (platforms);
//   - лимиты выбора: максимум 1 backend-фреймворк и максимум 1
//     прикладной (frontend/mobile/desktop/extension/bot/game);
//   - взаимные конфликты из wizard_tree (conflicts);
//   - у фреймворка выбран требуемый язык (requires_language).
//
// Фронтенд отражает эти же правила для UX, но источник истины —
// эта функция: start_project_execution отказывается выполнять
// невалидный стек.

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

/// Проверяет стек и возвращает найденные проблемы (порядок значимый).
pub fn validate_stack(
    tree: &WizardTreeData,
    languages: &[String],
    frameworks: &[String],
    os: &str,
) -> Vec<StackIssue> {
    let mut issues: Vec<StackIssue> = Vec::new();

    let selected: Vec<&FrameworkDef> = frameworks
        .iter()
        .filter_map(|id| tree.frameworks.iter().find(|f| &f.id == id))
        .collect();

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

    let backend: Vec<&str> = selected
        .iter()
        .filter(|f| f.kind.as_deref() == Some("backend"))
        .map(|f| f.label.as_str())
        .collect();
    if backend.len() > 1 {
        issues.push(StackIssue {
            severity: StackSeverity::Error,
            message: format!(
                "Можно выбрать не более одного backend-фреймворка, выбрано: {}.",
                backend.join(", ")
            ),
        });
    }

    let app: Vec<&str> = selected
        .iter()
        .filter(|f| f.kind.as_deref() != Some("backend"))
        .map(|f| f.label.as_str())
        .collect();
    if app.len() > 1 {
        issues.push(StackIssue {
            severity: StackSeverity::Error,
            message: format!(
                "Можно выбрать не более одного прикладного фреймворка (frontend/mobile/desktop/extension/bot/game), выбрано: {}.",
                app.join(", ")
            ),
        });
    }

    for a in &selected {
        for b in &selected {
            if a.id == b.id {
                continue;
            }
            if a.conflicts.contains(&b.id) {
                issues.push(StackIssue {
                    severity: StackSeverity::Error,
                    message: format!(
                        "«{}» несовместим с «{}».",
                        a.label, b.label
                    ),
                });
            }
        }
    }

    for fw in &selected {
        // requires_language — «хотя бы один из этих языков»: достаточно,
        // чтобы хотя бы один требуемый язык присутствовал в стеке.
        if !fw.requires_language.is_empty()
            && !fw.requires_language.iter().any(|l| languages.contains(l))
        {
            issues.push(StackIssue {
                severity: StackSeverity::Error,
                message: format!(
                    "Фреймворк «{}» требует один из языков: {}.",
                    fw.label,
                    fw.requires_language.join(", ")
                ),
            });
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

    #[test]
    fn macos_only_blocked_on_windows() {
        let t = tree();
        let issues = validate_stack(&t, &["swift".into()], &["swiftui".into()], "windows");
        assert!(
            issues.iter().any(|i| i.message.contains("SwiftUI") && i.message.contains("macos")),
            "swiftui не заблокирован на windows: {issues:?}"
        );
    }

    #[test]
    fn macos_only_allowed_on_macos() {
        let t = tree();
        let issues = validate_stack(&t, &["swift".into()], &["swiftui".into()], "macos");
        assert!(!issues.iter().any(|i| i.message.contains("SwiftUI")), "{issues:?}");
    }

    #[test]
    fn two_backend_frameworks_blocked() {
        let t = tree();
        let issues = validate_stack(
            &t,
            &["python".into(), "typescript".into()],
            &["fastapi".into(), "nest".into()],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("не более одного backend")),
            "{issues:?}"
        );
    }

    #[test]
    fn next_plus_expo_plus_unity_blocked() {
        let t = tree();
        // nextjs (frontend) + expo (mobile) + unity (game) — 3 прикладных
        let issues = validate_stack(
            &t,
            &["typescript".into(), "csharp".into()],
            &["nextjs".into(), "expo".into(), "unity".into()],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("не более одного прикладного")),
            "{issues:?}"
        );
    }

    #[test]
    fn backend_plus_app_allowed() {
        let t = tree();
        // nest (backend) + nextjs (frontend) — допустимый полный стек
        let issues = validate_stack(
            &t,
            &["typescript".into()],
            &["nest".into(), "nextjs".into()],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn conflicts_still_enforced() {
        let t = tree();
        let issues = validate_stack(
            &t,
            &["typescript".into()],
            &["nest".into(), "express".into()],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
            "{issues:?}"
        );
    }

    #[test]
    fn framework_requires_language() {
        let t = tree();
        let issues = validate_stack(&t, &["cpp".into()], &["django".into()], "windows");
        assert!(
            issues.iter().any(|i| i.message.contains("требует один из языков: python")),
            "{issues:?}"
        );
    }

    #[test]
    fn all_frameworks_have_kind_and_require_language() {
        let t = tree();
        for f in &t.frameworks {
            assert!(
                f.kind.is_some(),
                "у фреймворка {} нет kind — валидация лимитов сломается",
                f.id
            );
            assert!(
                !f.requires_language.is_empty(),
                "у фреймворка {} нет requires_language",
                f.id
            );
        }
    }
}
