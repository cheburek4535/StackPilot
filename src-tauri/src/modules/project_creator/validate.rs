// ============================================================
// Валидация стека (validate.rs)
// ============================================================
// Проверяет выбранный пользователем стек до начала генерации:
//   - фреймворк доступен на текущей ОС (platforms);
//   - взаимные конфликты из wizard_tree (conflicts);
//   - у фреймворка выбран требуемый язык (requires_language).
//
// Жёстких лимитов «1 backend / 1 frontend» нет: выбор ограничен
// только явной таблицей conflicts (что физически не сможет
// существовать вместе) — остальное на усмотрение пользователя.
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
    fn cross_language_backends_allowed() {
        let t = tree();
        // express (JS) + fastapi (Python): файлы генерации не пересекаются,
        // свобода выбора не ограничивается — это осознанный выбор пользователя.
        let issues = validate_stack(
            &t,
            &["typescript".into(), "python".into()],
            &["express".into(), "fastapi".into()],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn bot_plus_frontend_allowed() {
        let t = tree();
        // aiogram (бот) + nuxt (фронтенд): веб-панель для бота — адекватный сценарий.
        let issues = validate_stack(
            &t,
            &["python".into(), "typescript".into()],
            &["aiogram".into(), "nuxt".into()],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn two_frontends_blocked_by_conflicts() {
        let t = tree();
        let issues = validate_stack(
            &t,
            &["typescript".into()],
            &["nextjs".into(), "sveltekit".into()],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
            "{issues:?}"
        );
    }

    #[test]
    fn game_engines_block_each_other() {
        let t = tree();
        // unity + unreal: оба — игровые движки, в одной папке не сосуществуют.
        let issues = validate_stack(
            &t,
            &["csharp".into(), "cpp".into()],
            &["unity".into(), "unreal".into()],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
            "{issues:?}"
        );
    }

    #[test]
    fn mobile_frameworks_block_each_other() {
        let t = tree();
        let issues = validate_stack(
            &t,
            &["dart".into(), "typescript".into()],
            &["flutter".into(), "expo".into()],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
            "{issues:?}"
        );
    }

    #[test]
    fn same_root_manifest_files_blocked() {
        let t = tree();
        // telegraf и express оба пишут package.json в корень — второй шаг
        // скипнется и проект выйдет сломанным, поэтому пара заблокирована.
        let issues = validate_stack(
            &t,
            &["typescript".into()],
            &["telegraf".into(), "express".into()],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
            "{issues:?}"
        );
        // flask и aiogram оба пишут requirements.txt в корень — та же механика.
        let issues = validate_stack(
            &t,
            &["python".into()],
            &["flask".into(), "aiogram".into()],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
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
