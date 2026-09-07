// ============================================================
// Каноническая валидация стека (validate.rs)
// ============================================================
// Единственный источник правды о том, какие комбинации можно
// собирать. Фронтенд-зеркало (rules.ts) предсказывает эти ошибки
// для UX, но финальный барьер — эта функция: start_project_execution
// отказывается выполнять невалидный стек.
//
// Правила (все — severity=Error, кроме 6 — Warning):
//   1. Фреймворк доступен на текущей ОС (platforms).
//   2. Взаимные конфликты из wizard_tree (conflicts + conflict_notes).
//   3. Тип проекта разрешает фреймворк (project_types).
//   4. На каждую сторону — не более одного «главного» фреймворка
//      (kind="app", side != "either"). Исключения — data-driven:
//      allowed_main_pairs (легальные связки: gin+cobra, axum+clap,
//      android+jetpack-compose, electron+react/vue/svelte) и
//      main_limit_exempt (zig-cli). Универсальные (tauri, qt) и
//      побочные (aiogram, telegraf) этим правилом не ограничены —
//      только явными конфликтами.
//   5. Язык стороны совместим с фреймворком (side + languages).
//   6. Предупреждения из warning_pairs (Phoenix LiveView + SPA, два
//      full-stack фреймворка, backend + Electron) — не блокируют
//      генерацию, только поясняют и советуют альтернативу.
//   Инструменты (Session 2, только добавляют проверки, ничего не меняют):
//     • зависимость (tool.requires) не выбрана — Error;
//     • несовпадение языков (tool.for_languages) — Error
//       (пустой список = универсальный инструмент);
//     • пересечение ответственностей (responsibility + alternative_policy):
//       exclusive — Error, warn — Warning, allow — допустимо; при
//       расхождении политик действует более строгая из двух;
//     • связки фреймворк↔инструмент: tool_conflicts — Error,
//       tool_warnings — Warning (причина + рекомендация).

use super::engine::duplicate_framework_write_paths;
use super::models::{AlternativePolicy, FrameworkDef, ToolDef, WizardContext, WizardTreeData};
use super::normalize::normalize_context;

/// Одна найденная проблема стека.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StackIssue {
    /// Error — генерация блокируется, Warning — только предупреждение
    pub severity: StackSeverity,
    /// Человекочитаемый текст (используется тестами и как fallback).
    pub message: String,
    /// i18n-ключ сообщения для фронтенда (если задан — фронтенд переводит
    /// его через i18n.t(message_key, args) вместо показа message).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_key: Option<String>,
    /// Параметры подстановки для message_key (label'ы и т.п.).
    #[serde(default)]
    pub args: std::collections::HashMap<String, String>,
}

impl StackIssue {
    pub fn keyed(
        severity: StackSeverity,
        message: String,
        message_key: impl Into<String>,
        args: std::collections::HashMap<String, String>,
    ) -> Self {
        StackIssue {
            severity,
            message,
            message_key: Some(message_key.into()),
            args,
        }
    }

    /// Проблема без i18n-ключа (технические сообщения: неизвестные id,
    /// конфликты путей генерации). Фронтенд показывает message как есть.
    pub fn plain(severity: StackSeverity, message: String) -> Self {
        StackIssue {
            severity,
            message,
            message_key: None,
            args: std::collections::HashMap::new(),
        }
    }
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

/// Пара фреймворков объявлена легальной связкой в wizard_tree.json
/// (allowed_main_pairs). Порядок пары не важен.
fn is_allowed_pair(tree: &WizardTreeData, a: &str, b: &str) -> bool {
    tree.allowed_main_pairs
        .iter()
        .any(|p| p.len() == 2 && ((p[0] == a && p[1] == b) || (p[0] == b && p[1] == a)))
}

/// Фреймворк не занимает лимит «одного главного на сторону»
/// (main_limit_exempt: zig-cli — std-CLI, не каркас приложения).
fn is_main_limit_exempt(tree: &WizardTreeData, id: &str) -> bool {
    tree.main_limit_exempt.iter().any(|x| x == id)
}

/// Человеческое объяснение конфликта (conflict_notes) в обе стороны.
fn conflict_note<'a>(fw: &'a FrameworkDef, other: &'a FrameworkDef) -> Option<&'a str> {
    fw.conflict_notes
        .get(&other.id)
        .or_else(|| other.conflict_notes.get(&fw.id))
        .map(String::as_str)
}

/// Сторона языка, СОГЛАСОВАННАЯ С ДВИЖКОМ (ProjectLayout::side_for_language):
///   1. явное назначение мастера (backend_languages/frontend_languages);
///   2. язык, требуемый фреймворком РОВНО одной стороны, — но только для
///      ВЫБРАННЫХ и НЕ назначенных языков (zig+dart+flutter: dart выбран и
///      не назначен — нужен только flutter → frontend; nest+cpp+ts: ts
///      назначен на фронтенд, а невыбранный js не может «спасти» nest);
///   3. вывод по category ("backend"/"both" → backend, "frontend"/"static" →
///      frontend; неизвестный язык → None).
fn language_side(
    tree: &WizardTreeData,
    backend_langs: &[String],
    frontend_langs: &[String],
    selected_langs: &[String],
    selected_frameworks: &[&FrameworkDef],
    lang: &str,
) -> Option<&'static str> {
    if backend_langs.iter().any(|l| l == lang) {
        return Some("backend");
    }
    if frontend_langs.iter().any(|l| l == lang) {
        return Some("frontend");
    }
    if selected_langs.iter().any(|l| l == lang) {
        let mut required_by: Vec<&'static str> = Vec::new();
        for fw in selected_frameworks {
            if !fw.languages.iter().any(|l| l == lang) {
                continue;
            }
            match fw.side.as_str() {
                "backend" if !required_by.contains(&"backend") => required_by.push("backend"),
                "frontend" if !required_by.contains(&"frontend") => required_by.push("frontend"),
                _ => {}
            }
        }
        if required_by.len() == 1 {
            return Some(required_by[0]);
        }
    }
    // Только ВЫБРАННЫЙ язык может внести свою сторону (category). Невыбранный
    // язык (например, javascript при выбранных cpp+typescript) не может
    // «спасти» фреймворк: категория стороны действует лишь для тех языков,
    // которые реально в стеке (nest+cpp+ts: невыбранный js не даёт nest бэкенд).
    if selected_langs.iter().any(|l| l == lang) {
        match tree
            .languages
            .iter()
            .find(|l| &l.id == lang)
            .and_then(|l| l.category.as_deref())
        {
            Some("backend") | Some("both") => return Some("backend"),
            Some("frontend") | Some("static") => return Some("frontend"),
            _ => {}
        }
    }
    None
}
/// ЕДИНСТВЕННЫЙ полный барьер валидации контекста. Три слоя в одном вызове:
///   1. каноническая нормализация (normalize_context): неизвестные id,
///      язык на обеих сторонах, вывод сторон, пересчёт docker;
///   2. правила стека (validate_stack): платформы, конфликты, типы проектов,
///      лимиты главных фреймворков, языки по сторонам, warning_pairs;
///   3. целостность генерации (duplicate_framework_write_paths): два
///      фреймворка, пишущие один файл/каталог, сломают выполнение.
/// Контекст нормализуется in-place: после вызова у вызывающего — КАНОНИЧЕСКИЙ
/// вид, тот же, что видит движок (plan). Все потребители (tauri-команды,
/// пресеты, plan) идут только через эту функцию — валидация и планирование
/// всегда говорят одно и то же.
pub fn validate_context(
    tree: &WizardTreeData,
    context: &mut WizardContext,
    os: &str,
) -> Vec<StackIssue> {
    let mut issues: Vec<StackIssue> = normalize_context(tree, context)
        .into_iter()
        .map(|message| StackIssue::plain(StackSeverity::Error, message))
        .collect();
    issues.extend(validate_stack(
        tree,
        context.project_type.as_deref(),
        &context.languages,
        &context.backend_languages,
        &context.frontend_languages,
        &context.frameworks,
        &context.tools,
        os,
    ));
    issues.extend(
        duplicate_framework_write_paths(context)
            .into_iter()
            .map(|message| StackIssue::plain(StackSeverity::Error, message)),
    );
    issues
}

/// Проверяет стек и возвращает найденные проблемы (порядок значимый).
pub fn validate_stack(
    tree: &WizardTreeData,
    project_type: Option<&str>,
    languages: &[String],
    backend_langs: &[String],
    frontend_langs: &[String],
    frameworks: &[String],
    tools: &[String],
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
            let mut args = std::collections::HashMap::new();
            args.insert("a".to_string(), fw.label.clone());
            args.insert("list".to_string(), fw.platforms.join(", "));
            issues.push(StackIssue::keyed(
                StackSeverity::Error,
                format!(
                    "«{}» недоступен на этой ОС (требуется: {}).",
                    fw.label,
                    fw.platforms.join(", ")
                ),
                "stack.platform",
                args,
            ));
        }
    }

    // 2. Взаимные конфликты
    for a in &selected {
        for b in &selected {
            if a.id == b.id {
                continue;
            }
            if a.conflicts.contains(&b.id) {
                let mut message = format!("«{}» несовместим с «{}».", a.label, b.label);
                if let Some(note) = conflict_note(a, b) {
                    message.push_str(&format!(" {}", note));
                }
                let mut args = std::collections::HashMap::new();
                args.insert("a".to_string(), a.label.clone());
                args.insert("b".to_string(), b.label.clone());
                issues.push(StackIssue::keyed(
                    StackSeverity::Error,
                    message,
                    "stack.conflict",
                    args,
                ));
            }
        }
    }

    // 3. Тип проекта
    if let Some(pt) = project_type {
        for fw in &selected {
            if !fw.project_types.is_empty() && !fw.project_types.iter().any(|p| p == pt) {
                let mut args = std::collections::HashMap::new();
                args.insert("a".to_string(), fw.label.clone());
                args.insert("pt".to_string(), pt.to_string());
                issues.push(StackIssue::keyed(
                    StackSeverity::Error,
                    format!(
                        "«{}» не подходит для проекта «{}». Выберите другой тип или снимите фреймворк.",
                        fw.label, pt
                    ),
                    "stack.project_type",
                    args,
                ));
            }
        }
    }

    // 4. Не более одного «главного» (kind="app") фреймворка на сторону.
    //    Исключения: allowed_main_pairs (легальные связки — gin+cobra,
    //    axum+clap, android+jetpack-compose, electron+react/vue/svelte) и
    //    main_limit_exempt (zig-cli). side="either" — универсальные
    //    (tauri, qt): в лимит сторон не входят, ограничены только явными
    //    conflicts. Побочные (kind="side": aiogram, telegraf) могут
    //    соседствовать с любым числом других.
    let mut by_side: Vec<(&str, &FrameworkDef)> = Vec::new();
    for fw in &selected {
        if fw.kind == "app" && fw.side != "either" && !is_main_limit_exempt(tree, &fw.id) {
            by_side.push((fw.side.as_str(), fw));
        }
    }
    for (i, (side, a)) in by_side.iter().enumerate() {
        for (other_side, b) in by_side.iter().skip(i + 1) {
            if side == other_side {
                let legal_pair = is_allowed_pair(tree, &a.id, &b.id);
                if legal_pair {
                    continue;
                }
                let mut args = std::collections::HashMap::new();
                args.insert("a".to_string(), a.label.clone());
                args.insert("b".to_string(), b.label.clone());
                args.insert("side".to_string(), side.to_string());
                issues.push(StackIssue::keyed(
                    StackSeverity::Error,
                    format!(
                        "«{}» и «{}» — оба главные фреймворки {}. На сторону можно выбрать только один главный фреймворк.",
                        a.label, b.label, side
                    ),
                    "stack.two_main",
                    args,
                ));
            }
        }
    }

    // 6. Предупреждения из warning_pairs (Phoenix LiveView + тяжёлый SPA,
    //    два full-stack фреймворка, backend + Electron...): не блокируют
    //    генерацию, но объясняют концептуальный конфликт и советуют
    //    альтернативу. Плейсхолдеры {a}/{b}/{a_lang} заменяются label'ами.
    for wp in &tree.warning_pairs {
        let a = selected.iter().find(|f| f.id == wp.a);
        let b = selected.iter().find(|f| f.id == wp.b);
        if let (Some(a), Some(b)) = (a, b) {
            let a_lang_label = tree
                .languages
                .iter()
                .find(|l| l.id == a.recommended_language)
                .map(|l| l.label.clone())
                .unwrap_or_else(|| a.recommended_language.clone());
            let resolve = |text: &str| -> String {
                text.replace("{a_lang}", &a_lang_label)
                    .replace("{a}", &a.label)
                    .replace("{b}", &b.label)
            };
            let mut message = format!(
                "«{}» и «{}» — спорная связка. {}",
                a.label,
                b.label,
                resolve(&wp.reason)
            );
            if !wp.alternative.is_empty() {
                message.push_str(&format!(" Альтернатива: {}.", resolve(&wp.alternative)));
            }
            issues.push(StackIssue::plain(StackSeverity::Warning, message));
        }
    }

    // 5. Язык(и) стороны должны подходить фреймворку. Сторона языка
    //    разрешается ТАК ЖЕ, как в движке (ProjectLayout::side_for_language):
    //    явное назначение мастера > язык, требуемый фреймворком ровно одной
    //    стороны (только выбранный и не назначенный) > category. Валидация и
    //    планирование классифицируют язык ОДИНАКОВО (иначе: zig+dart+flutter
    //    без явных сторон валиден для движка, но отклонялся валидацией —
    //    и наоборот).
    for fw in &selected {
        let side_ok = |lang: &str, side: &str| {
            // Прямая проверка явных списков сторон: язык может стоять на
            // обеих сторонах одновременно (full-stack TS: nest + react, или
            // C#: MAUI на фронте + ASP.NET Core на бэке). language_side()
            // возвращает "backend" при первом совпадении, что ломает
            // валидацию MAUI (csharp в обоих списках → "backend" → MAUI
            // не видит csharp на фронтенде).
            let explicit = match side {
                "backend" => backend_langs.iter().any(|l| l == lang),
                "frontend" => frontend_langs.iter().any(|l| l == lang),
                _ => false,
            };
            explicit
                || language_side(
                    tree,
                    backend_langs,
                    frontend_langs,
                    languages,
                    &selected,
                    lang,
                ) == Some(side)
        };
        match fw.side.as_str() {
            "backend" => {
                if !fw.languages.iter().any(|l| side_ok(l, "backend")) {
                    let mut args = std::collections::HashMap::new();
                    args.insert("a".to_string(), fw.label.clone());
                    args.insert("list".to_string(), fw.languages.join(", "));
                    args.insert("rec".to_string(), fw.recommended_language.clone());
                    issues.push(StackIssue::keyed(
                        StackSeverity::Error,
                        format!(
                            "«{}» работает на бэкенде и требует один из языков: {}. Замените бэкенд-язык на «{}».",
                            fw.label,
                            fw.languages.join(", "),
                            fw.recommended_language
                        ),
                        "stack.backend_lang",
                        args,
                    ));
                }
            }
            "frontend" => {
                if !fw.languages.iter().any(|l| side_ok(l, "frontend")) {
                    let mut args = std::collections::HashMap::new();
                    args.insert("a".to_string(), fw.label.clone());
                    args.insert("list".to_string(), fw.languages.join(", "));
                    args.insert("rec".to_string(), fw.recommended_language.clone());
                    issues.push(StackIssue::keyed(
                        StackSeverity::Error,
                        format!(
                            "«{}» работает на фронтенде и требует один из языков: {}. Замените фронтенд-язык на «{}».",
                            fw.label,
                            fw.languages.join(", "),
                            fw.recommended_language
                        ),
                        "stack.frontend_lang",
                        args,
                    ));
                }
            }
            _ => {
                let any_ok = fw.languages.iter().any(|x| {
                    matches!(
                        language_side(tree, backend_langs, frontend_langs, languages, &selected, x),
                        Some("backend") | Some("frontend")
                    )
                });
                if !any_ok {
                    let mut args = std::collections::HashMap::new();
                    args.insert("a".to_string(), fw.label.clone());
                    args.insert("list".to_string(), fw.languages.join(", "));
                    issues.push(StackIssue::keyed(
                        StackSeverity::Error,
                        format!(
                            "«{}» требует один из языков: {} (на любой стороне).",
                            fw.label,
                            fw.languages.join(", ")
                        ),
                        "stack.either_lang",
                        args,
                    ));
                }
            }
        }
    }

    // 7. UI-варианты фреймворка (qt-qml/qt-widgets/qt-webengine/qt-kirigami)
    //    не могут существовать без своего владельца (qt): они дописывают
    //    файлы к его каркасу и в мастере выбираются только в его попапе.
    for fw in &selected {
        let owner = tree
            .frameworks
            .iter()
            .find(|f| f.qt_ui_options.iter().any(|m| m.id == fw.id));
        if let Some(owner) = owner {
            if !frameworks.iter().any(|id| id == &owner.id) {
                let mut args = std::collections::HashMap::new();
                args.insert("a".to_string(), fw.label.clone());
                args.insert("b".to_string(), owner.label.clone());
                issues.push(StackIssue::keyed(
                    StackSeverity::Error,
                    format!(
                        "«{}» — UI-вариант «{}» и не может быть выбран без него. Снимите «{}» или добавьте «{}».",
                        fw.label, owner.label, fw.label, owner.label
                    ),
                    "stack.ui_owner",
                    args,
                ));
            }
        }
    }

    // 8–11. Инструменты (Session 2): зависимости, языковая совместимость,
    //    пересечение ответственностей и связки фреймворк↔инструмент.
    //    Проверки не меняют поведение фреймворк-правил выше — только
    //    добавляют новые проблемы в конец списка.
    validate_tool_dependencies(tree, tools, &mut issues);
    validate_tool_language_compatibility(tree, tools, languages, &mut issues);
    validate_tool_responsibility_overlap(tree, tools, &mut issues);
    validate_framework_tool_warnings(tree, frameworks, tools, &mut issues);

    issues
}

// ============================================================
// Tool-валидация (Session 2)
// ============================================================

/// Зависимости инструментов (tool.requires[]): alembic требует sqlalchemy.
/// Если требуемый инструмент не выбран — Error: конфигурация окружения
/// такого стека не собирается. Неизвестный инструмент пропускается молча;
/// циклы зависимостей не проверяются (данные wizard_tree каноничны).
///
/// # Аргументы
/// * `tree` — дерево мастера со всеми определениями инструментов
/// * `tools` — список выбранных id инструментов
/// * `issues` — вектор, в который добавляются найденные проблемы
///
/// # Генерируемые проблемы
/// * Error с ключом "stack.tool.missing_dependency", когда требуемый
///   инструмент не выбран
fn validate_tool_dependencies(
    tree: &WizardTreeData,
    tools: &[String],
    issues: &mut Vec<StackIssue>,
) {
    let selected_ids: std::collections::HashSet<&str> = tools.iter().map(String::as_str).collect();

    for tool_id in tools {
        let tool = match tree.tools.iter().find(|t| &t.id == tool_id) {
            Some(t) => t,
            None => continue,
        };
        for required_id in &tool.requires {
            if selected_ids.contains(required_id.as_str()) {
                continue;
            }
            let required_label = tree
                .tools
                .iter()
                .find(|t| &t.id == required_id)
                .map(|t| t.label.as_str())
                .unwrap_or(required_id);
            let mut args = std::collections::HashMap::new();
            args.insert("tool".to_string(), tool.label.clone());
            args.insert("required".to_string(), required_label.to_string());
            issues.push(StackIssue::keyed(
                StackSeverity::Error,
                format!(
                    "«{}» требует «{}», но этот инструмент не выбран. Добавьте «{}» или снимите «{}».",
                    tool.label, required_label, required_label, tool.label
                ),
                "stack.tool.missing_dependency",
                args,
            ));
        }
    }
}

/// Языковая совместимость инструмента (tool.for_languages[]). Пустой
/// список — универсальный инструмент (postgresql, docker), подходит любому
/// стеку. Иначе хотя бы один выбранный язык обязан совпасть (npm — только
/// javascript/typescript); несовпадение — Error.
///
/// # Аргументы
/// * `tree` — дерево мастера со всеми определениями инструментов
/// * `tools` — список выбранных id инструментов
/// * `languages` — список выбранных языков
/// * `issues` — вектор, в который добавляются найденные проблемы
///
/// # Генерируемые проблемы
/// * Error с ключом "stack.tool.language_mismatch", когда ни один из
///   for_languages инструмента не выбран
fn validate_tool_language_compatibility(
    tree: &WizardTreeData,
    tools: &[String],
    languages: &[String],
    issues: &mut Vec<StackIssue>,
) {
    for tool_id in tools {
        let tool = match tree.tools.iter().find(|t| &t.id == tool_id) {
            Some(t) => t,
            None => continue,
        };
        if tool.for_languages.is_empty() {
            continue;
        }
        let compatible = tool
            .for_languages
            .iter()
            .any(|lang| languages.contains(lang));
        if compatible {
            continue;
        }
        let mut args = std::collections::HashMap::new();
        args.insert("tool".to_string(), tool.label.clone());
        args.insert("languages".to_string(), tool.for_languages.join(", "));
        issues.push(StackIssue::keyed(
            StackSeverity::Error,
            format!(
                "«{}» требует один из языков: {}. Ни один из них не выбран — добавьте язык или снимите инструмент.",
                tool.label,
                tool.for_languages.join(", ")
            ),
            "stack.tool.language_mismatch",
            args,
        ));
    }
}

/// Инструменты с одинаковой ответственностью (tool.responsibility,
/// например "orm" у sqlalchemy/prisma/drizzle) дублируют друг друга.
/// Судьбу пары решает alternative_policy: allow — допустимо, warn —
/// Warning, exclusive — Error. При расхождении политик действует более
/// строгая из двух. Инструмент без responsibility в проверке не участвует.
///
/// # Аргументы
/// * `tree` — дерево мастера со всеми определениями инструментов
/// * `tools` — список выбранных id инструментов
/// * `issues` — вектор, в который добавляются найденные проблемы
///
/// # Генерируемые проблемы
/// * Error с ключом "stack.tool.exclusive_alternatives" для пары с
///   политикой exclusive
/// * Warning с ключом "stack.tool.overlapping_responsibility" для пары
///   с политикой warn
fn validate_tool_responsibility_overlap(
    tree: &WizardTreeData,
    tools: &[String],
    issues: &mut Vec<StackIssue>,
) {
    let selected: Vec<&ToolDef> = tools
        .iter()
        .filter_map(|id| tree.tools.iter().find(|t| &t.id == id))
        .collect();

    // Группировка по ответственности.
    let mut by_responsibility: std::collections::HashMap<&str, Vec<&ToolDef>> =
        std::collections::HashMap::new();
    for tool in &selected {
        if let Some(resp) = &tool.responsibility {
            by_responsibility
                .entry(resp.as_str())
                .or_default()
                .push(tool);
        }
    }

    // Проверка каждой пары внутри одной группы.
    for (responsibility, tools_in_group) in by_responsibility {
        if tools_in_group.len() <= 1 {
            continue;
        }
        for (i, tool_a) in tools_in_group.iter().enumerate() {
            for tool_b in tools_in_group.iter().skip(i + 1) {
                // Политика пары: более строгая из двух заявленных.
                let policy_a = &tool_a.alternative_policy;
                let policy_b = &tool_b.alternative_policy;
                let effective_policy = match (policy_a, policy_b) {
                    (AlternativePolicy::Exclusive, _) | (_, AlternativePolicy::Exclusive) => {
                        AlternativePolicy::Exclusive
                    }
                    (AlternativePolicy::Warn, _) | (_, AlternativePolicy::Warn) => {
                        AlternativePolicy::Warn
                    }
                    _ => AlternativePolicy::Allow,
                };

                match effective_policy {
                    AlternativePolicy::Exclusive => {
                        let mut args = std::collections::HashMap::new();
                        args.insert("a".to_string(), tool_a.label.clone());
                        args.insert("b".to_string(), tool_b.label.clone());
                        args.insert("responsibility".to_string(), responsibility.to_string());
                        issues.push(StackIssue::keyed(
                            StackSeverity::Error,
                            format!(
                                "«{}» и «{}» — взаимоисключающие инструменты: оба выполняют роль {} и не могут быть выбраны вместе.",
                                tool_a.label, tool_b.label, responsibility
                            ),
                            "stack.tool.exclusive_alternatives",
                            args,
                        ));
                    }
                    AlternativePolicy::Warn => {
                        let mut args = std::collections::HashMap::new();
                        args.insert("a".to_string(), tool_a.label.clone());
                        args.insert("b".to_string(), tool_b.label.clone());
                        args.insert("responsibility".to_string(), responsibility.to_string());
                        issues.push(StackIssue::keyed(
                            StackSeverity::Warning,
                            format!(
                                "«{}» и «{}» оба выполняют роль {}. Оставьте один из них, если нет особой архитектурной причины.",
                                tool_a.label, tool_b.label, responsibility
                            ),
                            "stack.tool.overlapping_responsibility",
                            args,
                        ));
                    }
                    AlternativePolicy::Allow => {}
                }
            }
        }
    }
}

/// Связки фреймворк↔инструмент из данных фреймворка: tool_conflicts —
/// жёсткая несовместимость (Error, генерация блокируется); tool_warnings —
/// нежелательное, но допустимое сочетание (Warning: причина + рекомендация).
///
/// # Аргументы
/// * `tree` — дерево мастера со всеми определениями инструментов
/// * `frameworks` — список выбранных id фреймворков
/// * `tools` — список выбранных id инструментов
/// * `issues` — вектор, в который добавляются найденные проблемы
///
/// # Генерируемые проблемы
/// * Error с ключом "stack.framework_tool_conflict", когда инструмент
///   входит в tool_conflicts фреймворка
/// * Warning с ключом "stack.framework_tool_warning", когда инструмент
///   входит в tool_warnings фреймворка (причина + рекомендация)
fn validate_framework_tool_warnings(
    tree: &WizardTreeData,
    frameworks: &[String],
    tools: &[String],
    issues: &mut Vec<StackIssue>,
) {
    let selected_fws: Vec<&FrameworkDef> = frameworks
        .iter()
        .filter_map(|id| tree.frameworks.iter().find(|f| &f.id == id))
        .collect();

    for fw in selected_fws {
        for tool_id in tools {
            // Жёсткий конфликт: фреймворк запрещает инструмент.
            if fw.tool_conflicts.contains(tool_id) {
                let tool_label = tree
                    .tools
                    .iter()
                    .find(|t| &t.id == tool_id)
                    .map(|t| t.label.as_str())
                    .unwrap_or(tool_id);
                let mut args = std::collections::HashMap::new();
                args.insert("framework".to_string(), fw.label.clone());
                args.insert("tool".to_string(), tool_label.to_string());
                issues.push(StackIssue::keyed(
                    StackSeverity::Error,
                    format!(
                        "«{}» несовместим с инструментом «{}».",
                        fw.label, tool_label
                    ),
                    "stack.framework_tool_conflict",
                    args,
                ));
            }

            // Warning: нежелательное сочетание с причиной и рекомендацией.
            if let Some(warning) = fw.tool_warnings.get(tool_id) {
                let tool_label = tree
                    .tools
                    .iter()
                    .find(|t| &t.id == tool_id)
                    .map(|t| t.label.as_str())
                    .unwrap_or(tool_id);
                let mut message = format!(
                    "«{}» и «{}» — спорная связка. {}",
                    fw.label, tool_label, warning.reason
                );
                if !warning.recommendation.is_empty() {
                    message.push_str(&format!(" Рекомендация: {}.", warning.recommendation));
                }
                let mut args = std::collections::HashMap::new();
                args.insert("framework".to_string(), fw.label.clone());
                args.insert("tool".to_string(), tool_label.to_string());
                args.insert("reason".to_string(), warning.reason.clone());
                args.insert("recommendation".to_string(), warning.recommendation.clone());
                issues.push(StackIssue::keyed(
                    StackSeverity::Warning,
                    message,
                    "stack.framework_tool_warning",
                    args,
                ));
            }
        }
    }
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
        let b_langs: Vec<String> = b.map(|s| s.to_string()).into_iter().collect();
        let f_langs: Vec<String> = f.map(|s| s.to_string()).into_iter().collect();
        // Выбранные языки = объединение сторон (тестовый помощник).
        let mut langs = b_langs.clone();
        langs.extend(f_langs.iter().cloned());
        // Инструменты: legacy-тесты инструментов не выбирают — пустой
        // список проходит tool-валидацию молча (новые тесты — ниже,
        // mod tool_validation_tests).
        validate_stack(tree, pt, &langs, &b_langs, &f_langs, &fws, &[], os)
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
            assert!(
                !f.languages.is_empty(),
                "у фреймворка {} нет languages",
                f.id
            );
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
        // Исключение — только пары из allowed_main_pairs (легальные связки,
        // генерация которых гарантированно не пересекается) и фреймворки
        // из main_limit_exempt (не занимают лимит стороны).
        for (i, a) in t.frameworks.iter().enumerate() {
            if a.kind != "app" || a.side == "either" {
                continue;
            }
            if is_main_limit_exempt(&t, &a.id) {
                continue;
            }
            for b in t.frameworks.iter().skip(i + 1) {
                if b.kind != "app" || b.side == "either" {
                    continue;
                }
                if is_main_limit_exempt(&t, &b.id) {
                    continue;
                }
                if a.side != b.side {
                    continue;
                }
                if is_allowed_pair(&t, &a.id, &b.id) {
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
    fn allowed_main_pairs_are_consistent() {
        let t = tree();
        // Каждая легальная связка обязана:
        //   1. существовать в дереве;
        //   2. быть app-фреймворками одной стороны;
        //   3. НЕ иметь взаимных conflicts (иначе конфликт победит связку);
        //   4. не содержать фреймворки из main_limit_exempt (избыточно, но
        //      допустимо как явное документирование связки — см. zig-cli+zap).
        for p in &t.allowed_main_pairs {
            assert_eq!(p.len(), 2, "пара должна содержать ровно два id: {p:?}");
            let a = fw(&t, &p[0]);
            let b = fw(&t, &p[1]);
            assert_eq!(
                a.kind, "app",
                "«{}» из allowed_main_pairs не kind=app",
                a.id
            );
            assert_eq!(
                b.kind, "app",
                "«{}» из allowed_main_pairs не kind=app",
                b.id
            );
            assert_eq!(
                a.side, b.side,
                "«{}» и «{}» из allowed_main_pairs — разные стороны",
                a.id, b.id
            );
            assert!(
                !a.conflicts.contains(&b.id) && !b.conflicts.contains(&a.id),
                "«{}» и «{}» — легальная связка, но объявляет conflicts",
                a.id,
                b.id
            );
        }
        // Компаньоны (tauri/electron → UI) обязаны быть легальными связками
        // и не конфликтовать с владельцем. Для side="either" владельцев
        // (tauri) правило 4 не действует, поэтому пара не обязательна —
        // компаньон просто добавляется к стек-выбору UI.
        for f in &t.frameworks {
            for c in &f.companions {
                if f.side != "either" {
                    assert!(
                        is_allowed_pair(&t, &f.id, c),
                        "«{}» объявляет компаньона «{}», но пары нет в allowed_main_pairs",
                        f.id,
                        c
                    );
                }
                assert!(
                    !f.conflicts.contains(c),
                    "«{}» объявляет компаньона «{}», но конфликтует с ним",
                    f.id,
                    c
                );
                let cdef = fw(&t, c);
                if f.qt_ui_options.iter().any(|m| m.id == *c) {
                    // UI-вариант (qt → qt-qml/qt-widgets/...): собственная
                    // технология фреймворка, живёт в его попапе, сторона
                    // может быть "either".
                } else {
                    assert_eq!(
                        cdef.side, "frontend",
                        "компаньон «{}» должен быть frontend",
                        c
                    );
                }
            }
            // Каждый qt_ui_options обязан указывать на существующий
            // фреймворк-вариант kind=side и быть в companions владельца.
            for m in &f.qt_ui_options {
                let vdef = fw(&t, &m.id);
                assert_eq!(
                    vdef.kind, "side",
                    "UI-вариант «{}» должен быть kind=side",
                    m.id
                );
                assert!(
                    f.companions.contains(&m.id),
                    "«{}» заявляет qt_ui_options «{}», но нет в companions",
                    f.id,
                    m.id
                );
            }
        }
        assert!(
            !t.allowed_main_pairs.is_empty() && !t.warning_pairs.is_empty(),
            "allowed_main_pairs и warning_pairs должны быть заполнены"
        );
    }

    #[test]
    fn no_game_engines_in_tree() {
        let t = tree();
        assert!(
            !t.frameworks
                .iter()
                .any(|f| ["unity", "unreal", "godot"].contains(&f.id.as_str())),
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
                issues.iter().map(|i| i.message.clone()).collect::<Vec<_>>()
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
                assert_eq!(
                    f.class, "standalone",
                    "scaffold-фреймворк {} должен быть standalone",
                    f.id
                );
            }
        }
    }

    #[test]
    fn client_shell_allows_non_rest_backend() {
        let t = tree();
        assert!(
            !t.client_shell_frameworks.is_empty(),
            "client_shell_frameworks должны быть заданы"
        );
        // Expo (клиентская оболочка) + zig-cli (не REST API) — больше не
        // блокируется: мон-репо с клиентом и CLI/ботом — легальная структура,
        // движок раскладывает их по frontend//backend/ (ShellClientApi).
        let issues = validate(
            &t,
            Some("custom"),
            Some("zig"),
            Some("typescript"),
            &["expo", "zig-cli"],
            "windows",
        );
        assert!(
            !issues
                .iter()
                .any(|i| matches!(i.severity, StackSeverity::Error)),
            "expo + zig-cli не должны блокироваться: {:?}",
            issues
        );
        // Electron + Django (не-JS бэкенд, REST API) — не блокируется,
        // но обязана появиться спорная связка (Warning из warning_pairs)
        let issues = validate(
            &t,
            Some("custom"),
            Some("python"),
            Some("typescript"),
            &["electron", "django"],
            "windows",
        );
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.severity, StackSeverity::Warning)),
            "electron + django должны дать Warning: {:?}",
            issues
        );
    }

    // ----------------------------------------------------------
    // Правило: сторона и язык
    // ----------------------------------------------------------

    #[test]
    fn nest_blocked_when_backend_is_cpp() {
        let t = tree();
        // Классический кейс из бага: cpp-бэкенд + ts-фронтенд + nest.
        let issues = validate(
            &t,
            Some("rest-api"),
            Some("cpp"),
            Some("typescript"),
            &["nest"],
            "windows",
        );
        assert!(
            issues
                .iter()
                .any(|i| i.message.contains("требует один из языков")),
            "{issues:?}"
        );
    }

    #[test]
    fn telegraf_blocked_when_backend_is_cpp() {
        let t = tree();
        let issues = validate(
            &t,
            Some("telegram-bot"),
            Some("cpp"),
            Some("typescript"),
            &["telegraf"],
            "windows",
        );
        assert!(
            issues
                .iter()
                .any(|i| i.message.contains("требует один из языков")),
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
        let issues = validate(
            &t,
            Some("rest-api"),
            None,
            Some("typescript"),
            &["express"],
            "windows",
        );
        assert!(
            issues
                .iter()
                .any(|i| i.message.contains("требует один из языков")),
            "{issues:?}"
        );
    }

    #[test]
    fn framework_ok_with_language_on_its_side() {
        let t = tree();
        let issues = validate(
            &t,
            Some("rest-api"),
            Some("typescript"),
            None,
            &["express"],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    // ----------------------------------------------------------
    // Правило: один главный фреймворк на сторону
    // ----------------------------------------------------------

    #[test]
    fn two_backend_apps_blocked() {
        let t = tree();
        // django + fastapi — оба главные бэкенд-фреймворки.
        let issues = validate(
            &t,
            Some("web-app"),
            Some("python"),
            None,
            &["django", "fastapi"],
            "windows",
        );
        assert!(
            issues
                .iter()
                .any(|i| i.message.contains("оба главные фреймворки")),
            "{issues:?}"
        );
    }

    #[test]
    fn two_frontend_apps_blocked() {
        let t = tree();
        let issues = validate(
            &t,
            Some("web-app"),
            None,
            Some("typescript"),
            &["nextjs", "sveltekit"],
            "windows",
        );
        assert!(
            issues
                .iter()
                .any(|i| i.message.contains("оба главные фреймворки")),
            "{issues:?}"
        );
    }

    #[test]
    fn django_plus_aiogram_allowed() {
        let t = tree();
        // Главный (django) + побочный (aiogram) на одном бэкенде — ок.
        let issues = validate(
            &t,
            Some("web-app"),
            Some("python"),
            None,
            &["django", "aiogram"],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn side_framework_alone_allowed() {
        let t = tree();
        // aiogram сам по себе — валидный телеграм-бот.
        let issues = validate(
            &t,
            Some("telegram-bot"),
            Some("python"),
            None,
            &["aiogram"],
            "windows",
        );
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
        let issues = validate(
            &t,
            Some("rest-api"),
            Some("typescript"),
            None,
            &["express", "telegraf"],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
            "{issues:?}"
        );
    }

    #[test]
    fn backend_plus_frontend_allowed() {
        let t = tree();
        // nest (бэкенд) + nextjs (фронтенд) — легальный полный стек.
        let issues = validate(
            &t,
            Some("web-app"),
            Some("typescript"),
            Some("typescript"),
            &["nest", "nextjs"],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    // ----------------------------------------------------------
    // Правило: легальные связки (allowed_main_pairs / main_limit_exempt)
    // ----------------------------------------------------------

    #[test]
    fn gin_and_cobra_allowed_together() {
        let t = tree();
        // Веб-сервер (gin) + CLI (cobra) делят обязанности — связка легальна.
        let issues = validate(
            &t,
            Some("custom"),
            Some("go"),
            None,
            &["gin", "cobra"],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn axum_and_clap_allowed_together() {
        let t = tree();
        let issues = validate(
            &t,
            Some("custom"),
            Some("rust"),
            None,
            &["axum", "clap"],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn android_and_jetpack_compose_allowed_together() {
        let t = tree();
        // Нативный Kotlin-стек: Android SDK + Jetpack Compose (часть Android).
        let issues = validate(
            &t,
            Some("mobile-app"),
            None,
            Some("kotlin"),
            &["android", "jetpack-compose"],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn zig_cli_and_zap_allowed_together() {
        let t = tree();
        // zig-cli не занимает лимит «главного» — веб-фреймворк Zig рядом легален.
        let issues = validate(
            &t,
            Some("custom"),
            Some("zig"),
            None,
            &["zig-cli", "zap"],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn electron_with_ui_library_allowed() {
        let t = tree();
        // Electron + React/Vue/Svelte — классическая связка (UI-компаньон).
        for ui in ["react", "vue", "svelte"] {
            let issues = validate(
                &t,
                Some("desktop-app"),
                None,
                Some("typescript"),
                &["electron", ui],
                "windows",
            );
            assert!(issues.is_empty(), "electron+{ui}: {issues:?}");
        }
    }

    #[test]
    fn two_web_servers_still_blocked() {
        let t = tree();
        // Два одинаковых по типу инструмента — тотальная блокировка остаётся.
        let issues = validate(
            &t,
            Some("web-app"),
            Some("python"),
            None,
            &["fastapi", "flask"],
            "windows",
        );
        assert!(
            issues
                .iter()
                .any(|i| i.message.contains("оба главные фреймворки")),
            "{issues:?}"
        );
        let issues = validate(
            &t,
            Some("rest-api"),
            Some("typescript"),
            None,
            &["express", "fastify"],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
            "{issues:?}"
        );
    }

    #[test]
    fn fullstack_blocks_pure_ui_still() {
        let t = tree();
        // Next.js + React — жёсткая блокировка (React встроен в Next.js).
        let issues = validate(
            &t,
            Some("web-app"),
            None,
            Some("typescript"),
            &["nextjs", "react"],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
            "{issues:?}"
        );
        let issues = validate(
            &t,
            Some("web-app"),
            None,
            Some("typescript"),
            &["nuxt", "vue"],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
            "{issues:?}"
        );
    }

    #[test]
    fn qt_and_tauri_conflict() {
        let t = tree();
        // Два десктоп-каркаса в одном проекте — теперь явный конфликт.
        let issues = validate(
            &t,
            Some("desktop-app"),
            Some("cpp"),
            Some("rust"),
            &["qt", "tauri"],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
            "{issues:?}"
        );
    }

    #[test]
    fn tauri_and_flutter_conflict() {
        let t = tree();
        let issues = validate(
            &t,
            Some("custom"),
            Some("rust"),
            Some("dart"),
            &["tauri", "flutter"],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
            "{issues:?}"
        );
    }

    // ----------------------------------------------------------
    // Правило: warning_pairs (Phoenix LiveView + SPA)
    // ----------------------------------------------------------

    #[test]
    fn phoenix_with_nextjs_warns_but_does_not_block() {
        let t = tree();
        let issues = validate(
            &t,
            Some("web-app"),
            Some("elixir"),
            Some("typescript"),
            &["phoenix", "nextjs"],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("Phoenix")),
            "{issues:?}"
        );
        assert!(
            !issues
                .iter()
                .any(|i| matches!(i.severity, StackSeverity::Error)),
            "предупреждение не должно блокировать: {issues:?}"
        );
    }

    #[test]
    fn phoenix_with_nuxt_warns() {
        let t = tree();
        let issues = validate(
            &t,
            Some("web-app"),
            Some("elixir"),
            Some("typescript"),
            &["phoenix", "nuxt"],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("Phoenix")),
            "{issues:?}"
        );
    }

    #[test]
    fn phoenix_with_react_is_clean() {
        let t = tree();
        // Чистый UI (React/Vue/Svelte) с Phoenix — обычная архитектура API+SPA.
        let issues = validate(
            &t,
            Some("web-app"),
            Some("elixir"),
            Some("typescript"),
            &["phoenix", "react"],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn backend_mvc_with_metaframework_warns() {
        let t = tree();
        // Laravel + Next.js — два full-stack фреймворка с собственным
        // роутингом и сервером: Warning, но не блокировка.
        let issues = validate(
            &t,
            Some("web-app"),
            Some("php"),
            Some("typescript"),
            &["laravel", "nextjs"],
            "windows",
        );
        let warn = issues
            .iter()
            .find(|i| matches!(i.severity, StackSeverity::Warning));
        assert!(
            warn.is_some_and(|i| i.message.contains("Laravel") && i.message.contains("Next.js")),
            "{issues:?}"
        );
        assert!(
            !issues
                .iter()
                .any(|i| matches!(i.severity, StackSeverity::Error)),
            "предупреждение не должно блокировать: {issues:?}"
        );
    }

    #[test]
    fn django_with_nuxt_warns() {
        let t = tree();
        let issues = validate(
            &t,
            Some("web-app"),
            Some("python"),
            Some("typescript"),
            &["django", "nuxt"],
            "windows",
        );
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.severity, StackSeverity::Warning)
                    && i.message.contains("Django")
                    && i.message.contains("Nuxt")),
            "{issues:?}"
        );
    }

    #[test]
    fn backend_mvc_with_pure_spa_is_clean() {
        let t = tree();
        // Laravel + чистый SPA (React) — легальная архитектура API + SPA.
        let issues = validate(
            &t,
            Some("web-app"),
            Some("php"),
            Some("typescript"),
            &["laravel", "react"],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn backend_with_electron_warns_about_sidecar() {
        let t = tree();
        // Laravel + Electron: бэкенд на PHP придётся запускать сайдкаром.
        let issues = validate(
            &t,
            Some("desktop-app"),
            Some("php"),
            Some("typescript"),
            &["laravel", "electron"],
            "windows",
        );
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.severity, StackSeverity::Warning)
                    && i.message.contains("Laravel")
                    && i.message.contains("Electron")),
            "{issues:?}"
        );
        // Spring Boot + Electron — та же логика, язык Java подставляется.
        let issues2 = validate(
            &t,
            Some("desktop-app"),
            Some("java"),
            Some("typescript"),
            &["spring-boot", "electron"],
            "windows",
        );
        assert!(
            issues2
                .iter()
                .any(|i| matches!(i.severity, StackSeverity::Warning)
                    && i.message.contains("Spring Boot")
                    && i.message.contains("Electron")),
            "{issues2:?}"
        );
    }

    // ----------------------------------------------------------
    // Правило: тип проекта
    // ----------------------------------------------------------

    #[test]
    fn project_type_filters_frameworks() {
        let t = tree();
        // telegraf не подходит для embedded.
        let issues = validate(
            &t,
            Some("embedded"),
            Some("cpp"),
            None,
            &["telegraf"],
            "windows",
        );
        assert!(
            issues
                .iter()
                .any(|i| i.message.contains("не подходит для проекта")),
            "{issues:?}"
        );
    }

    #[test]
    fn cli_frameworks_allowed_for_cli_tool() {
        let t = tree();
        let issues = validate(
            &t,
            Some("cli-tool"),
            Some("rust"),
            None,
            &["clap"],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    // ----------------------------------------------------------
    // Старые правила, которые остались
    // ----------------------------------------------------------

    #[test]
    fn macos_only_blocked_on_windows() {
        let t = tree();
        let issues = validate(
            &t,
            Some("mobile-app"),
            None,
            Some("swift"),
            &["swiftui"],
            "windows",
        );
        assert!(
            issues
                .iter()
                .any(|i| i.message.contains("SwiftUI") && i.message.contains("macos")),
            "{issues:?}"
        );
    }

    #[test]
    fn macos_only_allowed_on_macos() {
        let t = tree();
        let issues = validate(
            &t,
            Some("mobile-app"),
            None,
            Some("swift"),
            &["swiftui"],
            "macos",
        );
        assert!(
            !issues.iter().any(|i| i.message.contains("SwiftUI")),
            "{issues:?}"
        );
    }

    #[test]
    fn conflicts_still_enforced() {
        let t = tree();
        // nest + nextjs конфликтуют явно (tauri-ветки), но здесь — fallback-слой:
        // даже без side-правил список conflicts обязан работать.
        let issues = validate(
            &t,
            Some("web-app"),
            Some("typescript"),
            None,
            &["nest", "express"],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("несовместим")),
            "{issues:?}"
        );
    }

    #[test]
    fn maui_ok_with_csharp_on_both_sides() {
        let t = tree();
        // C# на обеих сторонах (MAUI на фронте + ASP.NET Core на бэке) —
        // валидация должна видеть csharp на фронтенде для MAUI, несмотря
        // на то что csharp есть и в backend_langs. Тип «custom» выбран
        // потому, что ASP.NET Core не поддерживает desktop-app (см. данные).
        let issues = validate(
            &t,
            Some("custom"),
            Some("csharp"),
            Some("csharp"),
            &["maui", "aspnetcore"],
            "windows",
        );
        assert!(
            !issues
                .iter()
                .any(|i| matches!(i.severity, StackSeverity::Error)),
            "MAUI + ASP.NET Core с C# на обеих сторонах не должны блокироваться: {:?}",
            issues
        );
    }

    #[test]
    fn maui_alone_with_csharp_frontend() {
        let t = tree();
        // MAUI с C# только на фронтенде — валидна.
        let issues = validate(
            &t,
            Some("mobile-app"),
            None,
            Some("csharp"),
            &["maui"],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }
}

// ============================================================
// Tool-валидация (Session 2)
// ============================================================

#[cfg(test)]
mod tool_validation_tests {
    use super::*;
    use crate::modules::project_creator::models::ToolWarningReason;

    fn tree() -> WizardTreeData {
        let raw = include_str!("knowledge/wizard_tree.json");
        serde_json::from_str(raw).expect("wizard_tree.json должен парситься")
    }

    fn validate(
        tree: &WizardTreeData,
        pt: Option<&str>,
        b: Option<&str>,
        f: Option<&str>,
        fws: &[&str],
        tools: &[&str],
        os: &str,
    ) -> Vec<StackIssue> {
        let fws: Vec<String> = fws.iter().map(|s| s.to_string()).collect();
        let tools: Vec<String> = tools.iter().map(|s| s.to_string()).collect();
        let b_langs: Vec<String> = b.map(|s| s.to_string()).into_iter().collect();
        let f_langs: Vec<String> = f.map(|s| s.to_string()).into_iter().collect();
        // Выбранные языки = объединение сторон (тестовый помощник).
        let mut langs = b_langs.clone();
        langs.extend(f_langs.iter().cloned());
        validate_stack(tree, pt, &langs, &b_langs, &f_langs, &fws, &tools, os)
    }

    fn has_key(issues: &[StackIssue], key: &str) -> bool {
        issues.iter().any(|i| i.message_key.as_deref() == Some(key))
    }

    fn mut_tool<'a>(tree: &'a mut WizardTreeData, id: &str) -> &'a mut ToolDef {
        tree.tools
            .iter_mut()
            .find(|t| t.id == id)
            .unwrap_or_else(|| panic!("инструмент {id} не найден в дереве"))
    }

    fn mut_framework<'a>(tree: &'a mut WizardTreeData, id: &str) -> &'a mut FrameworkDef {
        tree.frameworks
            .iter_mut()
            .find(|f| f.id == id)
            .unwrap_or_else(|| panic!("фреймворк {id} не найден в дереве"))
    }

    // ----------------------------------------------------------
    // tool.requires: зависимость не выбрана
    // ----------------------------------------------------------

    #[test]
    fn test_alembic_without_sqlalchemy() {
        let t = tree();
        let issues = validate(
            &t,
            Some("rest-api"),
            Some("python"),
            None,
            &[],
            &["alembic"],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message_key.as_deref()
                == Some("stack.tool.missing_dependency")
                && matches!(i.severity, StackSeverity::Error)),
            "alembic без sqlalchemy должен дать Error: {issues:?}"
        );
        assert!(
            issues
                .iter()
                .find(|i| i.message_key.as_deref() == Some("stack.tool.missing_dependency"))
                .is_some_and(|i| i.args.get("tool").is_some_and(|v| v == "Alembic")
                    && i.args.get("required").is_some_and(|v| v == "SQLAlchemy")),
            "в args должны быть label'ы Alembic и SQLAlchemy: {issues:?}"
        );
    }

    #[test]
    fn test_alembic_with_sqlalchemy_is_clean() {
        let t = tree();
        let issues = validate(
            &t,
            Some("rest-api"),
            Some("python"),
            None,
            &[],
            &["alembic", "sqlalchemy"],
            "windows",
        );
        assert!(
            issues.is_empty(),
            "alembic + sqlalchemy на python-стеке не должны давать проблем: {issues:?}"
        );
    }

    // ----------------------------------------------------------
    // tool.for_languages: инструмент требует совместимый язык
    // ----------------------------------------------------------

    #[test]
    fn test_npm_without_js_language() {
        let t = tree();
        let issues = validate(
            &t,
            Some("rest-api"),
            Some("cpp"),
            None,
            &[],
            &["npm"],
            "windows",
        );
        assert!(
            issues.iter().any(
                |i| i.message_key.as_deref() == Some("stack.tool.language_mismatch")
                    && matches!(i.severity, StackSeverity::Error)
            ),
            "npm без JS/TS-языка должен дать Error: {issues:?}"
        );
    }

    #[test]
    fn test_npm_with_typescript_is_clean() {
        let t = tree();
        let issues = validate(
            &t,
            Some("web-app"),
            Some("typescript"),
            None,
            &[],
            &["npm"],
            "windows",
        );
        assert!(
            issues.is_empty(),
            "npm на typescript-стеке не должен давать проблем: {issues:?}"
        );
    }

    #[test]
    fn test_universal_tool_matches_any_language() {
        let t = tree();
        // postgresql без for_languages — универсальный инструмент.
        let issues = validate(
            &t,
            Some("rest-api"),
            Some("cpp"),
            None,
            &[],
            &["postgresql"],
            "windows",
        );
        assert!(
            issues.is_empty(),
            "универсальный инструмент не должен зависеть от языков: {issues:?}"
        );
    }

    // ----------------------------------------------------------
    // Краевые случаи: пустой список / неизвестный инструмент
    // ----------------------------------------------------------

    #[test]
    fn test_unknown_tool_and_empty_tools_are_ignored() {
        let t = tree();
        let issues = validate(
            &t,
            Some("rest-api"),
            Some("python"),
            None,
            &[],
            &["definitely-not-a-tool"],
            "windows",
        );
        assert!(
            issues.is_empty(),
            "неизвестный инструмент должен пропускаться молча: {issues:?}"
        );
        let issues = validate(
            &t,
            Some("rest-api"),
            Some("python"),
            None,
            &[],
            &[],
            "windows",
        );
        assert!(
            issues.is_empty(),
            "пустой список инструментов не должен давать проблем: {issues:?}"
        );
    }

    // ----------------------------------------------------------
    // tool.responsibility + alternative_policy
    // (в wizard_tree.json политики сейчас пустые — тесты патчат данные)
    // ----------------------------------------------------------

    #[test]
    fn test_exclusive_responsibility_overlap_blocked() {
        let mut t = tree();
        for id in ["prisma", "drizzle"] {
            let tool = mut_tool(&mut t, id);
            tool.responsibility = Some("orm".to_string());
            tool.alternative_policy = AlternativePolicy::Exclusive;
        }
        let issues = validate(
            &t,
            Some("web-app"),
            Some("typescript"),
            None,
            &[],
            &["prisma", "drizzle"],
            "windows",
        );
        assert_eq!(
            issues.len(),
            1,
            "exclusive-пара должна дать ровно одну проблему: {issues:?}"
        );
        let issue = &issues[0];
        assert_eq!(
            issue.message_key.as_deref(),
            Some("stack.tool.exclusive_alternatives")
        );
        assert!(matches!(issue.severity, StackSeverity::Error), "{issues:?}");
    }

    #[test]
    fn test_warn_responsibility_overlap_warns() {
        let mut t = tree();
        for id in ["prisma", "drizzle"] {
            let tool = mut_tool(&mut t, id);
            tool.responsibility = Some("orm".to_string());
            tool.alternative_policy = AlternativePolicy::Warn;
        }
        let issues = validate(
            &t,
            Some("web-app"),
            Some("typescript"),
            None,
            &[],
            &["prisma", "drizzle"],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message_key.as_deref()
                == Some("stack.tool.overlapping_responsibility")
                && matches!(i.severity, StackSeverity::Warning)),
            "warn-пара должна дать Warning: {issues:?}"
        );
    }

    #[test]
    fn test_strictest_alternative_policy_wins() {
        // Exclusive + Allow → всё равно Error (более строгая побеждает).
        let mut t = tree();
        mut_tool(&mut t, "prisma").responsibility = Some("orm".to_string());
        mut_tool(&mut t, "prisma").alternative_policy = AlternativePolicy::Exclusive;
        mut_tool(&mut t, "drizzle").responsibility = Some("orm".to_string());
        mut_tool(&mut t, "drizzle").alternative_policy = AlternativePolicy::Allow;
        let issues = validate(
            &t,
            Some("web-app"),
            Some("typescript"),
            None,
            &[],
            &["prisma", "drizzle"],
            "windows",
        );
        assert!(
            has_key(&issues, "stack.tool.exclusive_alternatives"),
            "{issues:?}"
        );

        // Allow + Allow → молча.
        let mut t = tree();
        for id in ["prisma", "drizzle"] {
            let tool = mut_tool(&mut t, id);
            tool.responsibility = Some("orm".to_string());
            tool.alternative_policy = AlternativePolicy::Allow;
        }
        let issues = validate(
            &t,
            Some("web-app"),
            Some("typescript"),
            None,
            &[],
            &["prisma", "drizzle"],
            "windows",
        );
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn test_tools_without_responsibility_not_compared() {
        let t = tree();
        // В реальных данных responsibility у инструментов пустой — пара
        // любых инструментов (sqlite + mongodb — обе «базы данных» по
        // category) не должна считаться пересечением ответственностей.
        let issues = validate(
            &t,
            Some("rest-api"),
            Some("python"),
            None,
            &[],
            &["sqlite", "mongodb"],
            "windows",
        );
        assert!(
            !has_key(&issues, "stack.tool.exclusive_alternatives")
                && !has_key(&issues, "stack.tool.overlapping_responsibility"),
            "{issues:?}"
        );
    }

    // ----------------------------------------------------------
    // Фреймворк ↔ инструмент: tool_conflicts / tool_warnings
    // (в wizard_tree.json списки сейчас пустые — тесты патчат данные)
    // ----------------------------------------------------------

    #[test]
    fn test_framework_tool_hard_conflict_blocks() {
        let mut t = tree();
        mut_framework(&mut t, "django")
            .tool_conflicts
            .push("pytest".to_string());
        let issues = validate(
            &t,
            Some("web-app"),
            Some("python"),
            None,
            &["django"],
            &["pytest"],
            "windows",
        );
        assert!(
            issues.iter().any(|i| i.message_key.as_deref()
                == Some("stack.framework_tool_conflict")
                && matches!(i.severity, StackSeverity::Error)),
            "django, запрещающий pytest, должен дать Error: {issues:?}"
        );
    }

    #[test]
    fn test_framework_tool_warning_is_reported() {
        let mut t = tree();
        mut_framework(&mut t, "django").tool_warnings.insert(
            "pytest".to_string(),
            ToolWarningReason {
                reason: "pytest дублирует встроенный runner Django".to_string(),
                recommendation: "используйте manage.py test".to_string(),
            },
        );
        let issues = validate(
            &t,
            Some("web-app"),
            Some("python"),
            None,
            &["django"],
            &["pytest"],
            "windows",
        );
        let warn = issues
            .iter()
            .find(|i| i.message_key.as_deref() == Some("stack.framework_tool_warning"));
        assert!(
            warn.is_some_and(|i| matches!(i.severity, StackSeverity::Warning)
                && i.message
                    .contains("pytest дублирует встроенный runner Django")
                && i.message.contains("manage.py test")
                && i.args.get("framework").is_some_and(|v| v == "Django")
                && i.args.get("tool").is_some_and(|v| v == "Pytest")),
            "{issues:?}"
        );
        assert!(
            !issues
                .iter()
                .any(|i| matches!(i.severity, StackSeverity::Error)),
            "warning не должен блокировать: {issues:?}"
        );
    }

    #[test]
    fn test_framework_without_tools_is_clean() {
        let t = tree();
        let issues = validate(
            &t,
            Some("web-app"),
            Some("python"),
            None,
            &["django"],
            &[],
            "windows",
        );
        assert!(
            issues.is_empty(),
            "django без инструментов не должен давать проблем: {issues:?}"
        );
    }
}
