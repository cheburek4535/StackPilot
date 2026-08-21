// ============================================================
// Каноническая нормализация контекста (normalize.rs)
// ============================================================
//
// ЕДИНСТВЕННАЯ точка, которая приводит произвольный WizardContext
// (из мастера, старых сессий, пресетов или прямых вызовов движка) к
// каноническому виду, в котором живут plan()/compose_recipe()/
// validate_stack():
//   - стабильная дедупликация всех списков (порядок первого вхождения);
//   - вывод сторон для языков с однозначной category (backend → backend,
//     frontend/static → frontend), когда явного назначения нет;
//   - languages = объединение языков всех сторон (порядок пользователя
//     сохраняется);
//   - docker активируется автоматически выбранным инструментом с
//     requires_docker (docker-флаг — производное состояние, а не
//     самостоятельная фича);
//   - фичи-флаги (docker/testing/git_init/vscode_config/ci) выводятся из
//     features, когда пользователь явно их выбрал;
//   - консистентность: один и тот же язык может стоять на обеих сторонах
//     (full-stack TS: nest + react) — сторона решается фреймворками;
//   - неизвестные id (язык/фреймворк/инструмент) — ошибки с тем же текстом,
//     что у compose_recipe ("not supported"), чтобы план и валидация
//     говорили одно и то же.
//
// Функция возвращает список ошибок и НЕ паникует: вызывающие решают, как
// превратить их в результат (plan() → Err, команды → StackIssue).

use super::models::{WizardContext, WizardTreeData};

/// Привести контекст к каноническому виду. Возвращает ошибки консистентности
/// (неизвестные id, язык на обеих сторонах); пустой список — контекст
/// нормален. Контекст меняется in-place: дедупликация, вывод сторон,
/// пересчёт флагов. Вызов идемпотентен.
pub fn normalize_context(tree: &WizardTreeData, ctx: &mut WizardContext) -> Vec<String> {
    let mut errors: Vec<String> = Vec::new();

    // 1. Стабильная дедупликация (первое вхождение сохраняет позицию).
    dedup_stable(&mut ctx.languages);
    dedup_stable(&mut ctx.backend_languages);
    dedup_stable(&mut ctx.frontend_languages);
    dedup_stable(&mut ctx.frameworks);
    dedup_stable(&mut ctx.tools);
    dedup_stable(&mut ctx.local_infra_tools);
    dedup_stable(&mut ctx.features);
    dedup_stable(&mut ctx.infrastructure);

    // 2. Один язык на двух сторонах — штатный full-stack стек (TypeScript на
    //    бэкенде и фронтенде: nest + react). Раскладка решается фреймворками
    //    (backend/-frontend/), а не языком — движок это поддерживает.

    // 3. Неизвестные id — те же формулировки, что в compose_recipe:
    //    "… is not supported: no implementation is available in this build".
    for l in &ctx.languages {
        if !tree.languages.iter().any(|d| &d.id == l) {
            errors.push(format!(
                "Language '{l}' is not supported: no implementation is available in this build"
            ));
        }
    }
    for l in ctx
        .backend_languages
        .iter()
        .chain(ctx.frontend_languages.iter())
    {
        if !ctx.languages.contains(l) && !tree.languages.iter().any(|d| &d.id == l) {
            errors.push(format!(
                "Language '{l}' is not supported: no implementation is available in this build"
            ));
        }
    }
    for f in &ctx.frameworks {
        if !tree.frameworks.iter().any(|d| &d.id == f) {
            errors.push(format!(
                "Framework '{f}' is not supported: no implementation is available in this build"
            ));
        }
    }
    for t in &ctx.tools {
        if !tree.tools.iter().any(|d| &d.id == t) {
            errors.push(format!(
                "Tool '{t}' is not supported: no implementation is available in this build"
            ));
        }
    }

    // 4. Вывод сторон для языков без явного назначения. Только однозначные
    //    category (backend/frontend/static); "both"-языки (csharp, dart,
    //    kotlin...) остаются без стороны — их сторона решается пользователем
    //    (см. flutter+maui: dart может быть и фронтендом).
    let lang_def = |id: &str| tree.languages.iter().find(|d| d.id == id);
    for l in &ctx.languages {
        if ctx.backend_languages.contains(l) || ctx.frontend_languages.contains(l) {
            continue;
        }
        match lang_def(l).and_then(|d| d.category.as_deref()) {
            Some("backend") => ctx.backend_languages.push(l.clone()),
            Some("frontend") | Some("static") => ctx.frontend_languages.push(l.clone()),
            _ => {}
        }
    }

    // 5. languages — объединение всех сторон (порядок пользователя первый).
    for l in ctx
        .backend_languages
        .iter()
        .chain(ctx.frontend_languages.iter())
    {
        if !ctx.languages.contains(l) {
            ctx.languages.push(l.clone());
        }
    }

    // 6. Фичи-флаги: явный выбор в features имеет приоритет; docker, кроме
    //    того, активируется любым выбранным инструментом с requires_docker —
    //    такой инструмент локально не ставится, развернуть его можно только
    //    контейнером (см. steps_for_docker).
    if ctx.features.iter().any(|f| f == "docker") {
        ctx.docker = true;
    }
    if ctx.features.iter().any(|f| f == "testing") {
        ctx.testing = true;
    }
    if ctx.features.iter().any(|f| f == "git" || f == "git_init") {
        ctx.git_init = true;
    }
    if ctx
        .features
        .iter()
        .any(|f| f == "vscode" || f == "vscode_config")
    {
        ctx.vscode_config = true;
    }
    if ctx.features.iter().any(|f| f == "ci") {
        ctx.ci = true;
    }
    if !ctx.docker
        && ctx
            .tools
            .iter()
            .any(|t| tree.tools.iter().any(|d| &d.id == t && d.requires_docker))
    {
        ctx.docker = true;
    }

    errors
}

/// Дедупликация с сохранением порядка первого вхождения.
fn dedup_stable(items: &mut Vec<String>) {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    items.retain(|i| seen.insert(i.clone()));
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

    fn ctx() -> WizardContext {
        WizardContext {
            project_name: Some("myapp".into()),
            project_path: Some("C:\\dev\\myapp".into()),
            ..Default::default()
        }
    }

    #[test]
    fn unassigned_languages_get_sides_by_category() {
        let t = tree();
        let mut c = ctx();
        c.languages = vec!["python".into(), "typescript".into()];
        let errors = normalize_context(&t, &mut c);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(c.backend_languages, vec!["python"]);
        assert_eq!(c.frontend_languages, vec!["typescript"]);
        assert_eq!(c.languages, vec!["python", "typescript"]);
    }

    #[test]
    fn explicit_side_wins_over_category() {
        let t = tree();
        let mut c = ctx();
        c.languages = vec!["typescript".into()];
        c.backend_languages = vec!["typescript".into()];
        let errors = normalize_context(&t, &mut c);
        assert!(errors.is_empty(), "{errors:?}");
        // typescript на бэкенде — явный выбор, category не перебивает.
        assert_eq!(c.backend_languages, vec!["typescript"]);
        assert!(c.frontend_languages.is_empty());
    }

    #[test]
    fn both_category_language_stays_unassigned() {
        let t = tree();
        let mut c = ctx();
        c.languages = vec!["csharp".into()];
        let errors = normalize_context(&t, &mut c);
        assert!(errors.is_empty(), "{errors:?}");
        // "both"-язык без явного назначения стороны не занимает — сторона
        // решается пользователем (maui может быть и фронтендом).
        assert!(c.backend_languages.is_empty());
        assert!(c.frontend_languages.is_empty());
        assert_eq!(c.languages, vec!["csharp"]);
    }

    #[test]
    fn same_language_on_both_sides_is_allowed() {
        let t = tree();
        let mut c = ctx();
        c.languages = vec!["typescript".into()];
        c.backend_languages = vec!["typescript".into()];
        c.frontend_languages = vec!["typescript".into()];
        let errors = normalize_context(&t, &mut c);
        assert!(errors.is_empty(), "{errors:?}");
        // TypeScript на обеих сторонах — штатный full-stack стек (nest + react):
        // сторона языка остаётся явной на каждой стороне, ошибки нет.
        assert_eq!(c.backend_languages, vec!["typescript"]);
        assert_eq!(c.frontend_languages, vec!["typescript"]);
        assert_eq!(c.languages, vec!["typescript"]);
    }

    #[test]
    fn unknown_ids_reported_with_compose_recipe_message() {
        let t = tree();
        let mut c = ctx();
        c.languages = vec!["brainfuck".into()];
        c.frameworks = vec!["not-a-real-framework".into()];
        c.tools = vec!["not-a-real-tool".into()];
        let errors = normalize_context(&t, &mut c);
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors[0].contains("not supported"), "{errors:?}");
        assert!(errors[1].contains("not-a-real-framework"), "{errors:?}");
        assert!(errors[2].contains("not-a-real-tool"), "{errors:?}");
    }

    #[test]
    fn docker_tool_activates_docker_flag() {
        let t = tree();
        let mut c = ctx();
        c.tools = vec!["postgresql".into()];
        let errors = normalize_context(&t, &mut c);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(
            c.docker,
            "requires_docker-инструмент обязан включить docker"
        );
    }

    #[test]
    fn non_docker_tool_keeps_docker_flag_off() {
        let t = tree();
        let mut c = ctx();
        c.tools = vec!["alembic".into()];
        let errors = normalize_context(&t, &mut c);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(
            !c.docker,
            "без requires_docker-инструментов docker не включается"
        );
    }

    #[test]
    fn features_map_to_flags() {
        let t = tree();
        let mut c = ctx();
        c.features = vec![
            "docker".into(),
            "testing".into(),
            "git".into(),
            "vscode".into(),
            "ci".into(),
        ];
        let errors = normalize_context(&t, &mut c);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(c.docker && c.testing && c.git_init && c.vscode_config && c.ci);
    }

    #[test]
    fn dedup_is_stable_and_side_languages_merge_into_languages() {
        let t = tree();
        let mut c = ctx();
        c.languages = vec!["python".into(), "typescript".into(), "python".into()];
        c.backend_languages = vec!["python".into()];
        c.frontend_languages = vec!["typescript".into()];
        let errors = normalize_context(&t, &mut c);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(c.languages, vec!["python", "typescript"]);
        // Язык только в side-списке тоже попадает в languages.
        c.backend_languages.push("go".into());
        let errors = normalize_context(&t, &mut c);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(c.languages.contains(&"go".to_string()), "{:?}", c.languages);
    }

    #[test]
    fn normalize_is_idempotent() {
        let t = tree();
        let mut c = ctx();
        c.languages = vec!["python".into(), "typescript".into()];
        c.tools = vec!["postgresql".into()];
        let first = normalize_context(&t, &mut c);
        assert!(first.is_empty(), "{first:?}");
        let snapshot = c.clone();
        let second = normalize_context(&t, &mut c);
        assert!(second.is_empty(), "{second:?}");
        assert_eq!(format!("{c:?}"), format!("{snapshot:?}"));
    }
}
