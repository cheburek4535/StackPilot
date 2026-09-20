//! Типизированная, композируемая генерация README.md.
//!
//! Заменяет старый `content::generate_readme(name, lang, framework, ...)`,
//! который видел только ПЕРВЫЙ язык и ПЕРВЫЙ фреймворк. Новый модуль:
//!
//!   - получает ПОЛНЫЙ `WizardContext` и каноническую `ProjectLayout`
//!     (раскладку проекта), поэтому знает все языки, фреймворки, инструменты,
//!     расположение сегментов (backend/, frontend/, src-tauri/) и режимы
//!     развёртывания (локальный vs Docker);
//!   - собирает документ из типизированных секций в фиксированном
//!     детерминированном порядке (см. `ReadmeDoc` и `generate_readme`);
//!   - знания о языках/фреймворках/инструментах живут в отдельных
//!     функциях-провайдерах этого модуля, а НЕ в wizard_tree.json — JSON
//!     остаётся источником выбора и совместимости, а не базой документации;
//!   - соблюдает правила сочетаний: встроенный фронтенд (Tauri/Electron/Qt)
//!     не описывается как самостоятельный сайт, бэкенд не описывается как
//!     браузерный рантайм, Docker не «запускает» локально выбранный
//!     инструмент, зависимости упоминаются только когда реально выбраны.
//!
//! README генерируется движком в финальной фазе шаблонизации (см.
//! `steps_for_readme` в mod.rs) — ПОСЛЕ всех скаффолдеров, установки
//! зависимостей, Docker-конфигурации и интеграции путей.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use crate::modules::project_creator::models::{
    FrameworkDef, ToolDef, WizardContext, WizardTreeData,
};

use super::wizard_tree;
use super::LayoutClass;
use super::ProjectLayout;

// ============================================================================
// Типизированная модель документа
// ============================================================================

/// Одна секция README: заголовок + тело (Markdown, без «##»).
#[derive(Debug, Clone)]
pub struct Section {
    pub title: String,
    pub body: String,
}

/// Композируемый документ: секции складываются в детерминированном порядке.
#[derive(Debug, Clone, Default)]
pub struct ReadmeDoc {
    pub sections: Vec<Section>,
}

impl ReadmeDoc {
    pub fn add(&mut self, title: impl Into<String>, body: impl Into<String>) -> &mut Self {
        self.sections.push(Section {
            title: title.into(),
            body: body.into(),
        });
        self
    }

    /// Собрать Markdown: `# <name>` + секции `## ...` в порядке добавления.
    /// `generated_by` — локализованный футер (i18n-ключ `readme.generated_by`).
    pub fn render(&self, project_name: &str, generated_by: &str) -> String {
        let mut out = format!("# {}\n", project_name);
        for section in &self.sections {
            out.push_str(&format!(
                "\n## {}\n\n{}\n",
                section.title,
                section.body.trim()
            ));
        }
        out.push_str(&format!("\n---\n\n{}", generated_by));
        out
    }
}

/// Тип архитектуры проекта (для секции «Architecture»).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    /// Оболочка (tauri) владеет корнем и встраивает веб-фронтенд.
    Connected,
    /// backend/ + frontend/ — два независимых приложения.
    Separated,
    /// Только серверная часть.
    BackendOnly,
    /// Только клиентская часть.
    FrontendOnly,
    /// Неинтегрированная клиентская оболочка (electron, expo, react-native,
    /// plasmo) + REST API-бэкенд: клиент в frontend/, API в backend/.
    ShellClientApi,
    /// Пустой/нетипичный стек.
    Other,
}

impl Architecture {
    /// i18n-ключ метки (переводится через `ReadmeI18n`).
    pub fn key(self) -> &'static str {
        match self {
            Self::Connected => "readme.arch.connected",
            Self::Separated => "readme.arch.separated",
            Self::BackendOnly => "readme.arch.backend_only",
            Self::FrontendOnly => "readme.arch.frontend_only",
            Self::ShellClientApi => "readme.arch.shell_client_api",
            Self::Other => "readme.arch.other",
        }
    }
}

/// Как запускается выбранный инструмент.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolMode {
    /// Локальная установка (выбран пользователем в local_infra_tools).
    Local,
    /// Контейнер из docker-compose.yaml.
    Docker,
    /// Библиотека/CLI внутри проекта — отдельного сервиса нет.
    Embedded,
    /// Ни Docker, ни локальная установка не настроены.
    Unmanaged,
}

impl ToolMode {
    /// i18n-ключ метки (переводится через `ReadmeI18n`).
    pub fn key(self) -> &'static str {
        match self {
            Self::Local => "readme.tool_mode.local",
            Self::Docker => "readme.tool_mode.docker",
            Self::Embedded => "readme.tool_mode.embedded",
            Self::Unmanaged => "readme.tool_mode.unmanaged",
        }
    }
}

// ============================================================================
// i18n README: словари из общей базы локализации (en.ts/ru.ts)
// ============================================================================

/// Словари README из общей i18n-базы фронтенда
/// (`src/lib/core/locales/readme/*.json`, подключаются в en.ts/ru.ts).
/// Загрузка — один раз на процесс; выбор языка — индексация карты по id,
/// без ветвлений RU/EN.
static README_DICTS: OnceLock<HashMap<&'static str, HashMap<String, String>>> = OnceLock::new();

fn readme_dicts() -> &'static HashMap<&'static str, HashMap<String, String>> {
    README_DICTS.get_or_init(|| {
        let mut dicts: HashMap<&'static str, HashMap<String, String>> = HashMap::new();
        let sources: [(&'static str, &'static str); 2] = [
            (
                "en",
                include_str!("../../../../../src/lib/core/locales/readme/en.json"),
            ),
            (
                "ru",
                include_str!("../../../../../src/lib/core/locales/readme/ru.json"),
            ),
        ];
        for (locale, raw) in sources {
            if let Ok(dict) = serde_json::from_str::<HashMap<String, String>>(raw) {
                dicts.insert(locale, dict);
            }
        }
        dicts
    })
}

/// Переводчик README на выбранную локаль.
pub struct ReadmeI18n {
    strings: &'static HashMap<String, String>,
}

impl ReadmeI18n {
    /// Словарь локали; неизвестная/отсутствующая локаль → английский.
    pub fn for_locale(locale: Option<&str>) -> Self {
        let dicts = readme_dicts();
        let strings = locale
            .and_then(|l| dicts.get(l))
            .or_else(|| dicts.get("en"))
            .expect("README en dictionary must be valid JSON");
        Self { strings }
    }

    /// Перевод по ключу; отсутствующий ключ возвращается как есть — тест
    /// `readme_has_no_untranslated_keys` ловит такие случаи.
    pub fn t(&self, key: &str) -> String {
        self.strings
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    /// Перевод с подстановкой `{name}`/`{{name}}`-плейсхолдеров.
    pub fn tf(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut text = self.t(key);
        for (name, value) in args {
            text = text.replace(&format!("{{{name}}}"), value);
            text = text.replace(&format!("{{{{{name}}}}}"), value);
        }
        text
    }
}

/// Контекст, который получает каждый провайдер: ПОЛНЫЙ контекст мастера,
/// каноническая раскладка проекта и переводчик README.
#[derive(Clone, Copy)]
pub struct ReadmeContext<'a> {
    pub context: &'a WizardContext,
    pub layout: &'a ProjectLayout,
    pub project_name: &'a str,
    pub i18n: &'a ReadmeI18n,
}

impl ReadmeContext<'_> {
    /// Короткий доступ к переводчику на выбранную локаль.
    pub fn t(&self, key: &str) -> String {
        self.i18n.t(key)
    }

    pub fn tf(&self, key: &str, args: &[(&str, &str)]) -> String {
        self.i18n.tf(key, args)
    }
}

// ============================================================================
// Типизированные профили провайдеров
// ============================================================================

/// Профиль языка: роль в проекте и типовые команды.
#[derive(Debug, Clone)]
pub struct LanguageProfile {
    pub name: &'static str,
    pub summary: &'static str,
    pub first_code: &'static str,
    pub dev_cmd: &'static str,
    pub build_cmd: &'static str,
    pub test_cmd: Option<&'static str>,
    pub tips: &'static [&'static str],
}

/// Профиль фреймворка: что делает, куда писать первый код, команды.
#[derive(Debug, Clone)]
pub struct FrameworkProfile {
    pub name: &'static str,
    pub what: &'static str,
    pub first_code: &'static str,
    pub entry: &'static str,
    pub dev_cmd: &'static str,
    pub build_cmd: &'static str,
    pub test_cmd: Option<&'static str>,
}

/// Профиль инструмента: 7 обязательных пунктов (что это, где конфиг,
/// как запускается, где креды, какие переменные окружения, чем проверить).
#[derive(Debug, Clone)]
pub struct ToolProfile {
    pub name: &'static str,
    pub what: &'static str,
    pub config: &'static str,
    /// Базовые значения (для инструментов без отдельного сервиса).
    pub start: &'static str,
    pub credentials: &'static str,
    pub verify: &'static str,
    pub env: &'static [(&'static str, &'static str)],
    /// Переопределения для Docker-режима.
    pub start_docker: Option<&'static str>,
    pub credentials_docker: Option<&'static str>,
    pub verify_docker: Option<&'static str>,
    /// Переопределения для локального режима.
    pub start_local: Option<&'static str>,
    pub credentials_local: Option<&'static str>,
    pub verify_local: Option<&'static str>,
}

// ============================================================================
// Точка входа
// ============================================================================

/// Сгенерировать полный README.md для проекта. Детерминированно: одинаковый
/// контекст → одинаковый текст (все списки сортируются).
pub fn generate_readme(
    layout: &ProjectLayout,
    context: &WizardContext,
    project_name: &str,
) -> String {
    generate_readme_with_tree(wizard_tree(), layout, context, project_name)
}

/// Внутренняя точка входа с явным деревом (данные мастер-дерева) — публичная
/// `generate_readme` использует каноническое дерево из wizard_tree.json,
/// тесты могут подставить изменённую копию.
fn generate_readme_with_tree(
    tree: &WizardTreeData,
    layout: &ProjectLayout,
    context: &WizardContext,
    project_name: &str,
) -> String {
    let i18n = ReadmeI18n::for_locale(context.readme_locale.as_deref());
    let ctx = ReadmeContext {
        context,
        layout,
        project_name,
        i18n: &i18n,
    };
    let mut doc = ReadmeDoc::default();
    section_overview(&ctx, &mut doc);
    section_architecture(&ctx, &mut doc);
    section_selected_stack(&ctx, &mut doc);
    section_directory_map(&ctx, &mut doc);
    section_how_created(&ctx, &mut doc);
    section_frameworks(&ctx, &mut doc);
    section_tools(&ctx, &mut doc);
    section_prerequisites(&ctx, &mut doc);
    section_quick_start(&ctx, &mut doc);
    // Architectural Decisions документирует неочевидные решения стека и стоит
    // ПОСЛЕ «Quick start» (как «Getting started»), но ДО первого кода/разработки.
    section_architectural_decisions(&ctx, tree, &mut doc);
    section_first_code(&ctx, &mut doc);
    section_development(&ctx, &mut doc);
    section_building(&ctx, &mut doc);
    section_testing(&ctx, &mut doc);
    section_environment(&ctx, &mut doc);
    section_docker(&ctx, &mut doc);
    section_mistakes(&ctx, &mut doc);
    section_next_steps(&ctx, &mut doc);
    doc.render(project_name, &ctx.t("readme.generated_by"))
}

// ============================================================================
// Архитектурный анализ: неочевидные решения стека
// ============================================================================

/// Одно архитектурное решение, документированное в README: заголовок,
/// контекст (почему связка спорная/дублирующая) и рекомендация.
#[derive(Debug, Clone)]
struct ArchitecturalDecision {
    title: String,
    reason: String,
    recommendation: String,
}

/// Английские пояснения для известных спорных связок фреймворк↔инструмент.
/// README генерируется на английском, а тексты tool_warnings в wizard_tree.json
/// написаны для UI (русские). Для связок, реально заведённых в данные,
/// авторский английский текст живёт здесь; для неизвестных связок
/// analyze_architectural_decisions подставляет содержимое предупреждения
/// как есть (fallback).
fn decision_notes_known(fw_id: &str, tool_id: &str) -> Option<(&'static str, &'static str)> {
    let (reason, recommendation) = match (fw_id, tool_id) {
        ("django", "sqlalchemy") => (
            "readme.decision.django_sqlalchemy.reason",
            "readme.decision.django_sqlalchemy.recommendation",
        ),
        ("django", "alembic") => (
            "readme.decision.django_alembic.reason",
            "readme.decision.django_alembic.recommendation",
        ),
        ("django", "prisma") => (
            "readme.decision.django_prisma.reason",
            "readme.decision.django_prisma.recommendation",
        ),
        ("fastapi", "prisma") => (
            "readme.decision.fastapi_prisma.reason",
            "readme.decision.fastapi_prisma.recommendation",
        ),
        _ => return None,
    };
    Some((reason, recommendation))
}

/// Проанализировать стек на архитектурные решения, которые стоит
/// задокументировать в README:
///   - фреймворк предупреждает о спорном инструменте (FrameworkDef.tool_warnings);
///   - несколько выбранных инструментов делят одну ответственность
///     (ToolDef.responsibility) — их роли пересекаются.
/// Детерминированно: фреймворки и инструменты обходятся в алфавитном порядке.
fn analyze_architectural_decisions(
    tree: &WizardTreeData,
    frameworks: &[String],
    tools: &[String],
    i18n: &ReadmeI18n,
) -> Vec<ArchitecturalDecision> {
    let mut decisions: Vec<ArchitecturalDecision> = Vec::new();

    let mut selected_frameworks: Vec<&FrameworkDef> = frameworks
        .iter()
        .filter_map(|id| tree.frameworks.iter().find(|f| &f.id == id))
        .collect();
    selected_frameworks.sort_by(|a, b| a.id.cmp(&b.id));

    let mut selected_tools: Vec<&ToolDef> = tools
        .iter()
        .filter_map(|id| tree.tools.iter().find(|t| &t.id == id))
        .collect();
    selected_tools.sort_by(|a, b| a.id.cmp(&b.id));
    let mut tool_ids: Vec<&String> = tools.iter().collect();
    tool_ids.sort_unstable();
    tool_ids.dedup();

    // Предупреждения «фреймворк ↔ инструмент» (tool_warnings фреймворка).
    for fw in &selected_frameworks {
        for tool_id in &tool_ids {
            let Some(warning) = fw.tool_warnings.get(*tool_id) else {
                continue;
            };
            let fw_label = fw.label.as_str();
            // Метка инструмента; неизвестный id — сам id (fallback).
            let tool_label = selected_tools
                .iter()
                .find(|t| &t.id == *tool_id)
                .map(|t| t.label.as_str())
                .unwrap_or(tool_id.as_str());
            let (reason, recommendation) = decision_notes_known(&fw.id, tool_id)
                .map(|(reason_key, rec_key)| (i18n.t(reason_key), i18n.t(rec_key)))
                .unwrap_or_else(|| (warning.reason.clone(), warning.recommendation.clone()));
            decisions.push(ArchitecturalDecision {
                title: i18n.tf(
                    "readme.decisions.title_using",
                    &[("tool", tool_label), ("framework", fw_label)],
                ),
                reason,
                recommendation,
            });
        }
    }

    // Инструменты с одинаковой ответственностью (ToolDef.responsibility).
    let mut by_responsibility: BTreeMap<&str, Vec<&ToolDef>> = BTreeMap::new();
    for tool in &selected_tools {
        if let Some(resp) = &tool.responsibility {
            by_responsibility
                .entry(resp.as_str())
                .or_default()
                .push(tool);
        }
    }
    for (responsibility, tools_in_group) in &by_responsibility {
        if tools_in_group.len() > 1 {
            let tool_names: Vec<&str> = tools_in_group.iter().map(|t| t.label.as_str()).collect();
            decisions.push(ArchitecturalDecision {
                title: i18n.tf(
                    "readme.decisions.title_multiple",
                    &[
                        ("responsibility", responsibility),
                        ("tools", &tool_names.join(", ")),
                    ],
                ),
                reason: i18n.tf(
                    "readme.decisions.reason_multiple",
                    &[("responsibility", responsibility)],
                ),
                recommendation: i18n.t("readme.decisions.recommendation_multiple"),
            });
        }
    }

    decisions
}

// ============================================================================
// Общие помощники
// ============================================================================

fn sorted_unique<'a>(items: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut v: Vec<&str> = items.collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Список «a, b, and c» (оксфордская запятая, детерминированный порядок).
/// Коннекторы локализованы (EN "and" / RU "и") через i18n-ключи.
fn join_and(items: &[String], i18n: &ReadmeI18n) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 => i18n.tf(
            "readme.common.and_two",
            &[("a", items[0].as_str()), ("b", items[1].as_str())],
        ),
        _ => {
            let mut s = items[..items.len() - 1].join(", ");
            let last = items
                .last()
                .expect("match arm is `_` (len >= 3), so items is non-empty");
            s.push_str(&i18n.tf("readme.common.and_last", &[("last", last.as_str())]));
            s
        }
    }
}

fn has_language(ctx: &ReadmeContext, id: &str) -> bool {
    ctx.context.languages.iter().any(|l| l == id)
}

fn has_framework(ctx: &ReadmeContext, id: &str) -> bool {
    ctx.context.frameworks.iter().any(|f| f == id)
}

fn architecture(ctx: &ReadmeContext) -> Architecture {
    if ctx.context.languages.is_empty() && ctx.context.frameworks.is_empty() {
        return Architecture::Other;
    }
    // Оболочка, встраивающая веб-фронтенд, делает стек connected независимо
    // от раскладки (tauri; Qt — только в WebEngine-режиме, остальные режимы
    // Qt не встраивают веб-UI). Electron — НЕ connected: это клиентская
    // оболочка, живущая в frontend/ (см. LayoutClass::ShellClientApi).
    match embedded_shell(ctx) {
        Some("Tauri") => return Architecture::Connected,
        Some("Qt WebEngine") | Some("Qt") if qt_is_webengine(ctx) => {
            return Architecture::Connected;
        }
        _ => {}
    }
    match ctx.layout.class {
        LayoutClass::Separated => Architecture::Separated,
        LayoutClass::Connected => Architecture::Connected,
        LayoutClass::BackendOnly => Architecture::BackendOnly,
        LayoutClass::FrontendOnly => Architecture::FrontendOnly,
        LayoutClass::ShellClientApi => Architecture::ShellClientApi,
        LayoutClass::Custom => Architecture::Other,
    }
}

fn language_name(id: &str) -> String {
    language_profile(id)
        .map(|p| p.name.to_string())
        .unwrap_or_else(|| id.to_string())
}

fn framework_name(id: &str) -> String {
    framework_profile(id)
        .map(|p| p.name.to_string())
        .unwrap_or_else(|| id.to_string())
}

fn tool_name(id: &str) -> String {
    tool_profile(id)
        .map(|p| p.name.to_string())
        .unwrap_or_else(|| id.to_string())
}

/// Инструменты, которые являются сервисами (БД/кеши/брокеры/UI).
const SERVICE_TOOLS: &[&str] = &[
    "postgresql",
    "mysql",
    "mongodb",
    "redis",
    "kafka",
    "clickhouse",
    "rabbitmq",
    "minio",
    "mailpit",
    "grafana",
    "airflow",
];

/// Веб-фронтенды, которые могут быть ВСТРОЕНЫ в оболочку (Tauri/Electron/Qt).
fn is_web_frontend(id: &str) -> bool {
    matches!(
        id,
        "react" | "vue" | "svelte" | "nextjs" | "sveltekit" | "nuxt" | "solidjs" | "solidstart"
    )
}

/// Оболочка, встраивающая веб-фронтенд (Some(label) — присутствует).
fn embedded_shell(ctx: &ReadmeContext) -> Option<&'static str> {
    let mut fws: Vec<&str> = ctx.context.frameworks.iter().map(String::as_str).collect();
    fws.sort_unstable();
    for fw in fws {
        match fw {
            "tauri" => return Some("Tauri"),
            "qt-webengine" => return Some("Qt WebEngine"),
            "qt" | "qt-qml" | "qt-widgets" | "qt-kirigami" => return Some("Qt"),
            _ => {}
        }
    }
    None
}

/// Выбран ли Qt в режиме WebEngine (answers["qt_ui"] или фреймворк qt-webengine).
fn qt_is_webengine(ctx: &ReadmeContext) -> bool {
    if has_framework(ctx, "qt-webengine") {
        return true;
    }
    ctx.context
        .answers
        .get("qt_ui")
        .and_then(|v| v.first())
        .map(|m| m == "qt-webengine")
        .unwrap_or(false)
}

/// Режим запуска выбранного инструмента (см. ToolMode).
fn tool_mode(ctx: &ReadmeContext, tool: &str) -> ToolMode {
    if tool == "sqlite" || !SERVICE_TOOLS.contains(&tool) {
        return ToolMode::Embedded;
    }
    if ctx.context.local_infra_tools.iter().any(|t| t == tool) {
        return ToolMode::Local;
    }
    if ctx.context.docker {
        return ToolMode::Docker;
    }
    ToolMode::Unmanaged
}

/// Инструменты, управляемые docker-compose (не выбранные локально).
fn docker_managed_tools(ctx: &ReadmeContext) -> Vec<String> {
    ctx.context
        .tools
        .iter()
        .filter(|t| tool_mode(ctx, t) == ToolMode::Docker)
        .cloned()
        .collect()
}

/// Инструменты, выбранные для ЛОКАЛЬНОЙ установки.
fn local_tools(ctx: &ReadmeContext) -> Vec<String> {
    ctx.context
        .tools
        .iter()
        .filter(|t| tool_mode(ctx, t) == ToolMode::Local)
        .cloned()
        .collect()
}

/// Каталог venv проекта относительно корня: `venv` в одно-сторонней
/// раскладке, `<segment>/venv` (например `backend/venv`) в split. Совпадает
/// с каноническим venv движка (см. preflight::venv_abs).
fn venv_dir_rel(ctx: &ReadmeContext) -> String {
    let seg = python_segment_dir(ctx);
    if seg == "." {
        "venv".to_string()
    } else {
        format!("{}/venv", seg)
    }
}

/// Интерпретатор venv проекта ОТ КОРНЯ проекта (`venv/bin/python` или
/// `backend/venv/bin/python`; на Windows — `venv\Scripts\python`).
/// Единственный канонический venv создаётся в каталоге python-сегмента, а
/// не в `.venv` корня.
fn venv_prefix(ctx: &ReadmeContext) -> String {
    let dir = venv_dir_rel(ctx);
    if cfg!(target_os = "windows") {
        format!("{}\\Scripts\\python", dir.replace('/', "\\"))
    } else {
        format!("{}/bin/python", dir)
    }
}

/// Интерпретатор venv ОТНОСИТЕЛЬНО каталога python-сегмента — для команд,
/// которые выполняются после `cd <segment>` (см. fill_python_paths).
fn venv_segment_prefix() -> &'static str {
    if cfg!(target_os = "windows") {
        "venv\\Scripts\\python"
    } else {
        "venv/bin/python"
    }
}

/// Подстановка python-путей в шаблон команды README:
///   {venv}     — интерпретатор venv от корня проекта;
///   {venv_seg} — интерпретатор venv от каталога python-сегмента (вместе с {cd});
///   {cd}       — `cd <segment> && `, если python-код лежит в сегменте
///                (backend/ в split-раскладке), иначе пусто.
/// Строки без плейсхолдеров возвращаются без изменений.
fn fill_python_paths(ctx: &ReadmeContext, template: &str) -> String {
    let seg = python_segment_dir(ctx);
    let cd = if seg == "." {
        String::new()
    } else {
        format!("cd {seg} && ")
    };
    // {venv_seg} обязан быть заменён ДО {venv}: "{venv}" — подстрока
    // "{venv_seg}", преждевременная замена испортила бы плейсхолдер.
    template
        .replace("{venv_seg}", venv_segment_prefix())
        .replace("{venv}", &venv_prefix(ctx))
        .replace("{cd}", &cd)
}

// ============================================================================
// Провайдеры языков
// ============================================================================

pub fn language_profile(id: &str) -> Option<LanguageProfile> {
    match id {
        "python" => Some(lang_python()),
        "rust" => Some(lang_rust()),
        "go" => Some(lang_go()),
        "typescript" => Some(lang_typescript()),
        "javascript" => Some(lang_javascript()),
        "java" => Some(lang_java()),
        "kotlin" => Some(lang_kotlin()),
        "csharp" => Some(lang_csharp()),
        "cpp" | "c" => Some(lang_cpp()),
        "dart" => Some(lang_dart()),
        "php" => Some(lang_php()),
        "swift" => Some(lang_swift()),
        "zig" => Some(lang_zig()),
        "elixir" => Some(lang_elixir()),
        "gleam" => Some(lang_gleam()),
        "html" => Some(lang_html()),
        _ => None,
    }
}

fn lang_python() -> LanguageProfile {
    LanguageProfile {
        name: "Python",
        summary: "readme.lang.python.summary",
        first_code: "readme.lang.python.first_code",
        dev_cmd: "readme.lang.python.dev_cmd",
        build_cmd: "readme.lang.python.build_cmd",
        test_cmd: Some("readme.lang.python.test_cmd"),
        tips: &[
            "readme.lang.python.tip.0",
            "readme.lang.python.tip.1",
        ],
    }
}

fn lang_rust() -> LanguageProfile {
    LanguageProfile {
        name: "Rust",
        summary: "readme.lang.rust.summary",
        first_code: "readme.lang.rust.first_code",
        dev_cmd: "readme.lang.rust.dev_cmd",
        build_cmd: "readme.lang.rust.build_cmd",
        test_cmd: Some("readme.lang.rust.test_cmd"),
        tips: &[
            "readme.lang.rust.tip.0",
            "readme.lang.rust.tip.1",
        ],
    }
}

fn lang_go() -> LanguageProfile {
    LanguageProfile {
        name: "Go",
        summary: "readme.lang.go.summary",
        first_code: "readme.lang.go.first_code",
        dev_cmd: "readme.lang.go.dev_cmd",
        build_cmd: "readme.lang.go.build_cmd",
        test_cmd: Some("readme.lang.go.test_cmd"),
        tips: &[
            "readme.lang.go.tip.0",
            "readme.lang.go.tip.1",
        ],
    }
}

fn lang_typescript() -> LanguageProfile {
    LanguageProfile {
        name: "TypeScript",
        summary: "readme.lang.typescript.summary",
        first_code: "readme.lang.typescript.first_code",
        dev_cmd: "readme.lang.typescript.dev_cmd",
        build_cmd: "readme.lang.typescript.build_cmd",
        test_cmd: Some("readme.lang.typescript.test_cmd"),
        tips: &[
            "readme.lang.typescript.tip.0",
            "readme.lang.typescript.tip.1",
        ],
    }
}

fn lang_javascript() -> LanguageProfile {
    LanguageProfile {
        name: "JavaScript",
        summary: "readme.lang.javascript.summary",
        first_code: "readme.lang.javascript.first_code",
        dev_cmd: "readme.lang.javascript.dev_cmd",
        build_cmd: "readme.lang.javascript.build_cmd",
        test_cmd: Some("readme.lang.javascript.test_cmd"),
        tips: &[
            "readme.lang.javascript.tip.0",
            "readme.lang.javascript.tip.1",
        ],
    }
}

fn lang_java() -> LanguageProfile {
    LanguageProfile {
        name: "Java",
        summary: "readme.lang.java.summary",
        first_code: "readme.lang.java.first_code",
        dev_cmd: "readme.lang.java.dev_cmd",
        build_cmd: "readme.lang.java.build_cmd",
        test_cmd: Some("readme.lang.java.test_cmd"),
        tips: &[
            "readme.lang.java.tip.0",
            "readme.lang.java.tip.1",
        ],
    }
}

fn lang_kotlin() -> LanguageProfile {
    LanguageProfile {
        name: "Kotlin",
        summary: "readme.lang.kotlin.summary",
        first_code: "readme.lang.kotlin.first_code",
        dev_cmd: "readme.lang.kotlin.dev_cmd",
        build_cmd: "readme.lang.kotlin.build_cmd",
        test_cmd: Some("readme.lang.kotlin.test_cmd"),
        tips: &[
            "readme.lang.kotlin.tip.0",
            "readme.lang.kotlin.tip.1",
        ],
    }
}

fn lang_csharp() -> LanguageProfile {
    LanguageProfile {
        name: "C#",
        summary: "readme.lang.csharp.summary",
        first_code: "readme.lang.csharp.first_code",
        dev_cmd: "readme.lang.csharp.dev_cmd",
        build_cmd: "readme.lang.csharp.build_cmd",
        test_cmd: Some("readme.lang.csharp.test_cmd"),
        tips: &[
            "readme.lang.csharp.tip.0",
            "readme.lang.csharp.tip.1",
        ],
    }
}

fn lang_cpp() -> LanguageProfile {
    LanguageProfile {
        name: "C++",
        summary: "readme.lang.cpp.summary",
        first_code: "readme.lang.cpp.first_code",
        dev_cmd: "readme.lang.cpp.dev_cmd",
        build_cmd: "readme.lang.cpp.build_cmd",
        test_cmd: None,
        tips: &[
            "readme.lang.cpp.tip.0",
            "readme.lang.cpp.tip.1",
        ],
    }
}

fn lang_dart() -> LanguageProfile {
    LanguageProfile {
        name: "Dart",
        summary: "readme.lang.dart.summary",
        first_code: "readme.lang.dart.first_code",
        dev_cmd: "readme.lang.dart.dev_cmd",
        build_cmd: "readme.lang.dart.build_cmd",
        test_cmd: Some("readme.lang.dart.test_cmd"),
        tips: &[
            "readme.lang.dart.tip.0",
            "readme.lang.dart.tip.1",
        ],
    }
}

fn lang_php() -> LanguageProfile {
    LanguageProfile {
        name: "PHP",
        summary: "readme.lang.php.summary",
        first_code: "readme.lang.php.first_code",
        dev_cmd: "readme.lang.php.dev_cmd",
        build_cmd: "readme.lang.php.build_cmd",
        test_cmd: Some("readme.lang.php.test_cmd"),
        tips: &[
            "readme.lang.php.tip.0",
            "readme.lang.php.tip.1",
        ],
    }
}

fn lang_swift() -> LanguageProfile {
    LanguageProfile {
        name: "Swift",
        summary: "readme.lang.swift.summary",
        first_code: "readme.lang.swift.first_code",
        dev_cmd: "readme.lang.swift.dev_cmd",
        build_cmd: "readme.lang.swift.build_cmd",
        test_cmd: Some("readme.lang.swift.test_cmd"),
        tips: &[
            "readme.lang.swift.tip.0",
            "readme.lang.swift.tip.1",
        ],
    }
}

fn lang_zig() -> LanguageProfile {
    LanguageProfile {
        name: "Zig",
        summary: "readme.lang.zig.summary",
        first_code: "readme.lang.zig.first_code",
        dev_cmd: "readme.lang.zig.dev_cmd",
        build_cmd: "readme.lang.zig.build_cmd",
        test_cmd: Some("readme.lang.zig.test_cmd"),
        tips: &[
            "readme.lang.zig.tip.0",
            "readme.lang.zig.tip.1",
        ],
    }
}

fn lang_elixir() -> LanguageProfile {
    LanguageProfile {
        name: "Elixir",
        summary: "readme.lang.elixir.summary",
        first_code: "readme.lang.elixir.first_code",
        dev_cmd: "readme.lang.elixir.dev_cmd",
        build_cmd: "readme.lang.elixir.build_cmd",
        test_cmd: Some("readme.lang.elixir.test_cmd"),
        tips: &[
            "readme.lang.elixir.tip.0",
            "readme.lang.elixir.tip.1",
        ],
    }
}

fn lang_gleam() -> LanguageProfile {
    LanguageProfile {
        name: "Gleam",
        summary: "readme.lang.gleam.summary",
        first_code: "readme.lang.gleam.first_code",
        dev_cmd: "readme.lang.gleam.dev_cmd",
        build_cmd: "readme.lang.gleam.build_cmd",
        test_cmd: Some("readme.lang.gleam.test_cmd"),
        tips: &[
            "readme.lang.gleam.tip.0",
            "readme.lang.gleam.tip.1",
        ],
    }
}

fn lang_html() -> LanguageProfile {
    LanguageProfile {
        name: "HTML",
        summary: "readme.lang.html.summary",
        first_code: "readme.lang.html.first_code",
        dev_cmd: "readme.lang.html.dev_cmd",
        build_cmd: "readme.lang.html.build_cmd",
        test_cmd: None,
        tips: &[
            "readme.lang.html.tip.0",
            "readme.lang.html.tip.1",
        ],
    }
}

// ============================================================================
// Провайдеры фреймворков
// ============================================================================

pub fn framework_profile(id: &str) -> Option<FrameworkProfile> {
    match id {
        "fastapi" => Some(fw_fastapi()),
        "django" => Some(fw_django()),
        "flask" => Some(fw_flask()),
        "axum" => Some(fw_axum()),
        "tauri" => Some(fw_tauri()),
        "clap" => Some(fw_clap()),
        "gin" => Some(fw_gin()),
        "cobra" => Some(fw_cobra()),
        "nextjs" => Some(fw_nextjs()),
        "sveltekit" => Some(fw_sveltekit()),
        "nuxt" => Some(fw_nuxt()),
        "express" => Some(fw_express()),
        "electron" => Some(fw_electron()),
        "aiogram" => Some(fw_aiogram()),
        "telegraf" => Some(fw_telegraf()),
        "spring-boot" => Some(fw_spring_boot()),
        "aspnetcore" => Some(fw_aspnetcore()),
        "maui" => Some(fw_maui()),
        "qt" | "qt-qml" | "qt-widgets" | "qt-webengine" | "qt-kirigami" => Some(fw_qt(id)),
        "flutter" => Some(fw_flutter()),
        "laravel" => Some(fw_laravel()),
        "symfony" => Some(fw_symfony()),
        "jetpack-compose" => Some(fw_jetpack_compose()),
        "ktor" => Some(fw_ktor()),
        "swiftui" => Some(fw_swiftui()),
        "vapor" => Some(fw_vapor()),
        "react-native" => Some(fw_react_native()),
        "plasmo" => Some(fw_plasmo()),
        "zig-cli" => Some(fw_zig_cli()),
        "zap" => Some(fw_zap()),
        "expo" => Some(fw_expo()),
        "phoenix" => Some(fw_phoenix()),
        "nest" => Some(fw_nest()),
        "fastify" => Some(fw_fastify()),
        "solidjs" | "solidstart" => Some(fw_solidjs()),
        "react" => Some(fw_react()),
        "vue" => Some(fw_vue()),
        "svelte" => Some(fw_svelte()),
        _ => None,
    }
}

fn fw_fastapi() -> FrameworkProfile {
    FrameworkProfile {
        name: "FastAPI",
        what: "readme.fw.fastapi.what",
        first_code: "readme.fw.fastapi.first_code",
        entry: "readme.fw.fastapi.entry",
        dev_cmd: "readme.fw.fastapi.dev_cmd",
        build_cmd: "readme.fw.fastapi.build_cmd",
        test_cmd: Some("readme.fw.fastapi.test_cmd"),
    }
}

fn fw_django() -> FrameworkProfile {
    FrameworkProfile {
        name: "Django",
        what: "readme.fw.django.what",
        first_code: "readme.fw.django.first_code",
        entry: "readme.fw.django.entry",
        dev_cmd: "readme.fw.django.dev_cmd",
        build_cmd: "readme.fw.django.build_cmd",
        test_cmd: Some("readme.fw.django.test_cmd"),
    }
}

fn fw_flask() -> FrameworkProfile {
    FrameworkProfile {
        name: "Flask",
        what: "readme.fw.flask.what",
        first_code: "readme.fw.flask.first_code",
        entry: "readme.fw.flask.entry",
        dev_cmd: "readme.fw.flask.dev_cmd",
        build_cmd: "readme.fw.flask.build_cmd",
        test_cmd: Some("readme.fw.flask.test_cmd"),
    }
}

fn fw_axum() -> FrameworkProfile {
    FrameworkProfile {
        name: "Axum",
        what: "readme.fw.axum.what",
        first_code: "readme.fw.axum.first_code",
        entry: "readme.fw.axum.entry",
        dev_cmd: "readme.fw.axum.dev_cmd",
        build_cmd: "readme.fw.axum.build_cmd",
        test_cmd: Some("readme.fw.axum.test_cmd"),
    }
}

fn fw_tauri() -> FrameworkProfile {
    FrameworkProfile {
        name: "Tauri",
        what: "readme.fw.tauri.what",
        first_code: "readme.fw.tauri.first_code",
        entry: "readme.fw.tauri.entry",
        dev_cmd: "readme.fw.tauri.dev_cmd",
        build_cmd: "readme.fw.tauri.build_cmd",
        test_cmd: Some("readme.fw.tauri.test_cmd"),
    }
}

fn fw_clap() -> FrameworkProfile {
    FrameworkProfile {
        name: "Clap",
        what: "readme.fw.clap.what",
        first_code: "readme.fw.clap.first_code",
        entry: "readme.fw.clap.entry",
        dev_cmd: "readme.fw.clap.dev_cmd",
        build_cmd: "readme.fw.clap.build_cmd",
        test_cmd: Some("readme.fw.clap.test_cmd"),
    }
}

fn fw_gin() -> FrameworkProfile {
    FrameworkProfile {
        name: "Gin",
        what: "readme.fw.gin.what",
        first_code: "readme.fw.gin.first_code",
        entry: "readme.fw.gin.entry",
        dev_cmd: "readme.fw.gin.dev_cmd",
        build_cmd: "readme.fw.gin.build_cmd",
        test_cmd: Some("readme.fw.gin.test_cmd"),
    }
}

fn fw_cobra() -> FrameworkProfile {
    FrameworkProfile {
        name: "Cobra",
        what: "readme.fw.cobra.what",
        first_code: "readme.fw.cobra.first_code",
        entry: "readme.fw.cobra.entry",
        dev_cmd: "readme.fw.cobra.dev_cmd",
        build_cmd: "readme.fw.cobra.build_cmd",
        test_cmd: Some("readme.fw.cobra.test_cmd"),
    }
}

fn fw_nextjs() -> FrameworkProfile {
    FrameworkProfile {
        name: "Next.js",
        what: "readme.fw.nextjs.what",
        first_code: "readme.fw.nextjs.first_code",
        entry: "readme.fw.nextjs.entry",
        dev_cmd: "readme.fw.nextjs.dev_cmd",
        build_cmd: "readme.fw.nextjs.build_cmd",
        test_cmd: Some("readme.fw.nextjs.test_cmd"),
    }
}

fn fw_sveltekit() -> FrameworkProfile {
    FrameworkProfile {
        name: "SvelteKit",
        what: "readme.fw.sveltekit.what",
        first_code: "readme.fw.sveltekit.first_code",
        entry: "readme.fw.sveltekit.entry",
        dev_cmd: "readme.fw.sveltekit.dev_cmd",
        build_cmd: "readme.fw.sveltekit.build_cmd",
        test_cmd: Some("readme.fw.sveltekit.test_cmd"),
    }
}

fn fw_nuxt() -> FrameworkProfile {
    FrameworkProfile {
        name: "Nuxt",
        what: "readme.fw.nuxt.what",
        first_code: "readme.fw.nuxt.first_code",
        entry: "readme.fw.nuxt.entry",
        dev_cmd: "readme.fw.nuxt.dev_cmd",
        build_cmd: "readme.fw.nuxt.build_cmd",
        test_cmd: Some("readme.fw.nuxt.test_cmd"),
    }
}

fn fw_express() -> FrameworkProfile {
    FrameworkProfile {
        name: "Express",
        what: "readme.fw.express.what",
        first_code: "readme.fw.express.first_code",
        entry: "readme.fw.express.entry",
        dev_cmd: "readme.fw.express.dev_cmd",
        build_cmd: "readme.fw.express.build_cmd",
        test_cmd: Some("readme.fw.express.test_cmd"),
    }
}

fn fw_electron() -> FrameworkProfile {
    FrameworkProfile {
        name: "Electron",
        what: "readme.fw.electron.what",
        first_code: "readme.fw.electron.first_code",
        entry: "readme.fw.electron.entry",
        dev_cmd: "readme.fw.electron.dev_cmd",
        build_cmd: "readme.fw.electron.build_cmd",
        test_cmd: Some("readme.fw.electron.test_cmd"),
    }
}

fn fw_aiogram() -> FrameworkProfile {
    FrameworkProfile {
        name: "Aiogram",
        what: "readme.fw.aiogram.what",
        first_code: "readme.fw.aiogram.first_code",
        entry: "readme.fw.aiogram.entry",
        dev_cmd: "readme.fw.aiogram.dev_cmd",
        build_cmd: "readme.fw.aiogram.build_cmd",
        test_cmd: None,
    }
}

fn fw_telegraf() -> FrameworkProfile {
    FrameworkProfile {
        name: "Telegraf",
        what: "readme.fw.telegraf.what",
        first_code: "readme.fw.telegraf.first_code",
        entry: "readme.fw.telegraf.entry",
        dev_cmd: "readme.fw.telegraf.dev_cmd",
        build_cmd: "readme.fw.telegraf.build_cmd",
        test_cmd: None,
    }
}

fn fw_spring_boot() -> FrameworkProfile {
    FrameworkProfile {
        name: "Spring Boot",
        what: "readme.fw.spring_boot.what",
        first_code: "readme.fw.spring_boot.first_code",
        entry: "readme.fw.spring_boot.entry",
        dev_cmd: "readme.fw.spring_boot.dev_cmd",
        build_cmd: "readme.fw.spring_boot.build_cmd",
        test_cmd: Some("readme.fw.spring_boot.test_cmd"),
    }
}

fn fw_aspnetcore() -> FrameworkProfile {
    FrameworkProfile {
        name: "ASP.NET Core",
        what: "readme.fw.aspnetcore.what",
        first_code: "readme.fw.aspnetcore.first_code",
        entry: "readme.fw.aspnetcore.entry",
        dev_cmd: "readme.fw.aspnetcore.dev_cmd",
        build_cmd: "readme.fw.aspnetcore.build_cmd",
        test_cmd: Some("readme.fw.aspnetcore.test_cmd"),
    }
}

fn fw_maui() -> FrameworkProfile {
    FrameworkProfile {
        name: ".NET MAUI",
        what: "readme.fw.maui.what",
        first_code: "readme.fw.maui.first_code",
        entry: "readme.fw.maui.entry",
        dev_cmd: "readme.fw.maui.dev_cmd",
        build_cmd: "readme.fw.maui.build_cmd",
        test_cmd: Some("readme.fw.maui.test_cmd"),
    }
}

fn fw_qt(id: &str) -> FrameworkProfile {
    let (name, what) = match id {
        "qt-qml" => (
            "Qt (QML mode)",
            "A C++ cross-platform UI toolkit using declarative QML screens.",
        ),
        "qt-webengine" => (
            "Qt (WebEngine mode)",
            "A C++ desktop shell that embeds the web frontend in a native window (Qt WebEngine).",
        ),
        "qt-kirigami" => (
            "Qt (Kirigami mode)",
            "A C++ UI toolkit with the convergent Kirigami framework for desktop and mobile.",
        ),
        _ => (
            "Qt (Widgets mode)",
            "A C++ cross-platform UI toolkit with classic widget-based windows.",
        ),
    };
    FrameworkProfile {
        name,
        what,
        first_code: "`src/main.cpp` (QML screens live in `.qml` files for QML mode).",
        entry: "`src/main.cpp`",
        dev_cmd: "`cmake --build build` then run the produced binary",
        build_cmd: "`cmake -B build && cmake --build build`",
        test_cmd: None,
    }
}

fn fw_flutter() -> FrameworkProfile {
    FrameworkProfile {
        name: "Flutter",
        what: "readme.fw.flutter.what",
        first_code: "readme.fw.flutter.first_code",
        entry: "readme.fw.flutter.entry",
        dev_cmd: "readme.fw.flutter.dev_cmd",
        build_cmd: "readme.fw.flutter.build_cmd",
        test_cmd: Some("readme.fw.flutter.test_cmd"),
    }
}

fn fw_laravel() -> FrameworkProfile {
    FrameworkProfile {
        name: "Laravel",
        what: "readme.fw.laravel.what",
        first_code: "readme.fw.laravel.first_code",
        entry: "readme.fw.laravel.entry",
        dev_cmd: "readme.fw.laravel.dev_cmd",
        build_cmd: "readme.fw.laravel.build_cmd",
        test_cmd: Some("readme.fw.laravel.test_cmd"),
    }
}

fn fw_symfony() -> FrameworkProfile {
    FrameworkProfile {
        name: "Symfony",
        what: "readme.fw.symfony.what",
        first_code: "readme.fw.symfony.first_code",
        entry: "readme.fw.symfony.entry",
        dev_cmd: "readme.fw.symfony.dev_cmd",
        build_cmd: "readme.fw.symfony.build_cmd",
        test_cmd: Some("readme.fw.symfony.test_cmd"),
    }
}

fn fw_jetpack_compose() -> FrameworkProfile {
    FrameworkProfile {
        name: "Jetpack Compose",
        what: "readme.fw.jetpack_compose.what",
        first_code: "readme.fw.jetpack_compose.first_code",
        entry: "readme.fw.jetpack_compose.entry",
        dev_cmd: "readme.fw.jetpack_compose.dev_cmd",
        build_cmd: "readme.fw.jetpack_compose.build_cmd",
        test_cmd: Some("readme.fw.jetpack_compose.test_cmd"),
    }
}

fn fw_ktor() -> FrameworkProfile {
    FrameworkProfile {
        name: "Ktor",
        what: "readme.fw.ktor.what",
        first_code: "readme.fw.ktor.first_code",
        entry: "readme.fw.ktor.entry",
        dev_cmd: "readme.fw.ktor.dev_cmd",
        build_cmd: "readme.fw.ktor.build_cmd",
        test_cmd: Some("readme.fw.ktor.test_cmd"),
    }
}

fn fw_swiftui() -> FrameworkProfile {
    FrameworkProfile {
        name: "SwiftUI",
        what: "readme.fw.swiftui.what",
        first_code: "readme.fw.swiftui.first_code",
        entry: "readme.fw.swiftui.entry",
        dev_cmd: "readme.fw.swiftui.dev_cmd",
        build_cmd: "readme.fw.swiftui.build_cmd",
        test_cmd: Some("readme.fw.swiftui.test_cmd"),
    }
}

fn fw_vapor() -> FrameworkProfile {
    FrameworkProfile {
        name: "Vapor",
        what: "readme.fw.vapor.what",
        first_code: "readme.fw.vapor.first_code",
        entry: "readme.fw.vapor.entry",
        dev_cmd: "readme.fw.vapor.dev_cmd",
        build_cmd: "readme.fw.vapor.build_cmd",
        test_cmd: Some("readme.fw.vapor.test_cmd"),
    }
}

fn fw_react_native() -> FrameworkProfile {
    FrameworkProfile {
        name: "React Native",
        what: "readme.fw.react_native.what",
        first_code: "readme.fw.react_native.first_code",
        entry: "readme.fw.react_native.entry",
        dev_cmd: "readme.fw.react_native.dev_cmd",
        build_cmd: "readme.fw.react_native.build_cmd",
        test_cmd: Some("readme.fw.react_native.test_cmd"),
    }
}

fn fw_plasmo() -> FrameworkProfile {
    FrameworkProfile {
        name: "Plasmo",
        what: "readme.fw.plasmo.what",
        first_code: "readme.fw.plasmo.first_code",
        entry: "readme.fw.plasmo.entry",
        dev_cmd: "readme.fw.plasmo.dev_cmd",
        build_cmd: "readme.fw.plasmo.build_cmd",
        test_cmd: None,
    }
}

fn fw_zig_cli() -> FrameworkProfile {
    FrameworkProfile {
        name: "Zig CLI",
        what: "readme.fw.zig_cli.what",
        first_code: "readme.fw.zig_cli.first_code",
        entry: "readme.fw.zig_cli.entry",
        dev_cmd: "readme.fw.zig_cli.dev_cmd",
        build_cmd: "readme.fw.zig_cli.build_cmd",
        test_cmd: Some("readme.fw.zig_cli.test_cmd"),
    }
}

fn fw_zap() -> FrameworkProfile {
    FrameworkProfile {
        name: "Zap",
        what: "readme.fw.zap.what",
        first_code: "readme.fw.zap.first_code",
        entry: "readme.fw.zap.entry",
        dev_cmd: "readme.fw.zap.dev_cmd",
        build_cmd: "readme.fw.zap.build_cmd",
        test_cmd: Some("readme.fw.zap.test_cmd"),
    }
}

fn fw_expo() -> FrameworkProfile {
    FrameworkProfile {
        name: "Expo",
        what: "readme.fw.expo.what",
        first_code: "readme.fw.expo.first_code",
        entry: "readme.fw.expo.entry",
        dev_cmd: "readme.fw.expo.dev_cmd",
        build_cmd: "readme.fw.expo.build_cmd",
        test_cmd: Some("readme.fw.expo.test_cmd"),
    }
}

fn fw_phoenix() -> FrameworkProfile {
    FrameworkProfile {
        name: "Phoenix",
        what: "readme.fw.phoenix.what",
        first_code: "readme.fw.phoenix.first_code",
        entry: "readme.fw.phoenix.entry",
        dev_cmd: "readme.fw.phoenix.dev_cmd",
        build_cmd: "readme.fw.phoenix.build_cmd",
        test_cmd: Some("readme.fw.phoenix.test_cmd"),
    }
}

fn fw_nest() -> FrameworkProfile {
    FrameworkProfile {
        name: "NestJS",
        what: "readme.fw.nest.what",
        first_code: "readme.fw.nest.first_code",
        entry: "readme.fw.nest.entry",
        dev_cmd: "readme.fw.nest.dev_cmd",
        build_cmd: "readme.fw.nest.build_cmd",
        test_cmd: Some("readme.fw.nest.test_cmd"),
    }
}

fn fw_fastify() -> FrameworkProfile {
    FrameworkProfile {
        name: "Fastify",
        what: "readme.fw.fastify.what",
        first_code: "readme.fw.fastify.first_code",
        entry: "readme.fw.fastify.entry",
        dev_cmd: "readme.fw.fastify.dev_cmd",
        build_cmd: "readme.fw.fastify.build_cmd",
        test_cmd: Some("readme.fw.fastify.test_cmd"),
    }
}

fn fw_solidjs() -> FrameworkProfile {
    FrameworkProfile {
        name: "SolidStart",
        what: "readme.fw.solidjs.what",
        first_code: "readme.fw.solidjs.first_code",
        entry: "readme.fw.solidjs.entry",
        dev_cmd: "readme.fw.solidjs.dev_cmd",
        build_cmd: "readme.fw.solidjs.build_cmd",
        test_cmd: Some("readme.fw.solidjs.test_cmd"),
    }
}

fn fw_react() -> FrameworkProfile {
    FrameworkProfile {
        name: "React",
        what: "readme.fw.react.what",
        first_code: "readme.fw.react.first_code",
        entry: "readme.fw.react.entry",
        dev_cmd: "readme.fw.react.dev_cmd",
        build_cmd: "readme.fw.react.build_cmd",
        test_cmd: Some("readme.fw.react.test_cmd"),
    }
}

fn fw_vue() -> FrameworkProfile {
    FrameworkProfile {
        name: "Vue",
        what: "readme.fw.vue.what",
        first_code: "readme.fw.vue.first_code",
        entry: "readme.fw.vue.entry",
        dev_cmd: "readme.fw.vue.dev_cmd",
        build_cmd: "readme.fw.vue.build_cmd",
        test_cmd: Some("readme.fw.vue.test_cmd"),
    }
}

fn fw_svelte() -> FrameworkProfile {
    FrameworkProfile {
        name: "Svelte",
        what: "readme.fw.svelte.what",
        first_code: "readme.fw.svelte.first_code",
        entry: "readme.fw.svelte.entry",
        dev_cmd: "readme.fw.svelte.dev_cmd",
        build_cmd: "readme.fw.svelte.build_cmd",
        test_cmd: Some("readme.fw.svelte.test_cmd"),
    }
}

// ============================================================================
// Провайдеры инструментов
// ============================================================================

pub fn tool_profile(id: &str) -> Option<ToolProfile> {
    match id {
        "docker" => Some(tool_docker()),
        "postgresql" => Some(tool_postgresql()),
        "mysql" => Some(tool_mysql()),
        "mongodb" => Some(tool_mongodb()),
        "sqlite" => Some(tool_sqlite()),
        "redis" => Some(tool_redis()),
        "kafka" => Some(tool_kafka()),
        "clickhouse" => Some(tool_clickhouse()),
        "rabbitmq" => Some(tool_rabbitmq()),
        "minio" => Some(tool_minio()),
        "mailpit" => Some(tool_mailpit()),
        "grafana" => Some(tool_grafana()),
        "opentelemetry" => Some(tool_opentelemetry()),
        "airflow" => Some(tool_airflow()),
        "dbt" => Some(tool_dbt()),
        "prisma" => Some(tool_prisma()),
        "drizzle" => Some(tool_drizzle()),
        "sqlalchemy" => Some(tool_sqlalchemy()),
        "alembic" => Some(tool_alembic()),
        "pytest" => Some(tool_pytest()),
        "ruff" => Some(tool_ruff()),
        "npm" => Some(tool_npm()),
        "maven" => Some(tool_maven()),
        "gradle" => Some(tool_gradle()),
        _ => None,
    }
}

fn tool_docker() -> ToolProfile {
    ToolProfile {
        name: "Docker",
        what: "readme.tool.docker.what",
        config: "readme.tool.docker.config",
        start: "readme.tool.docker.start",
        credentials: "readme.tool.docker.credentials",
        verify: "readme.tool.docker.verify",
        env: &[],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_postgresql() -> ToolProfile {
    ToolProfile {
        name: "PostgreSQL",
        what: "readme.tool.postgresql.what",
        config: "readme.tool.postgresql.config",
        start: "readme.tool.postgresql.start",
        credentials: "readme.tool.postgresql.credentials",
        verify: "readme.tool.postgresql.verify",
        env: &[
            ("POSTGRES_USER", "readme.tool.postgresql.env.POSTGRES_USER"),
            ("POSTGRES_PASSWORD", "readme.tool.postgresql.env.POSTGRES_PASSWORD"),
            ("POSTGRES_DB", "readme.tool.postgresql.env.POSTGRES_DB"),
            ("POSTGRES_PORT", "readme.tool.postgresql.env.POSTGRES_PORT"),
            ("DATABASE_URL", "readme.tool.postgresql.env.DATABASE_URL"),
        ],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: Some("readme.tool.postgresql.start_local"),
        credentials_local: Some("readme.tool.postgresql.credentials_local"),
        verify_local: Some("readme.tool.postgresql.verify_local"),
    }
}

fn tool_mysql() -> ToolProfile {
    ToolProfile {
        name: "MySQL",
        what: "readme.tool.mysql.what",
        config: "readme.tool.mysql.config",
        start: "readme.tool.mysql.start",
        credentials: "readme.tool.mysql.credentials",
        verify: "readme.tool.mysql.verify",
        env: &[
            ("MYSQL_ROOT_PASSWORD", "readme.tool.mysql.env.MYSQL_ROOT_PASSWORD"),
            ("MYSQL_DATABASE", "readme.tool.mysql.env.MYSQL_DATABASE"),
            ("MYSQL_USER", "readme.tool.mysql.env.MYSQL_USER"),
            ("MYSQL_PASSWORD", "readme.tool.mysql.env.MYSQL_PASSWORD"),
            ("MYSQL_PORT", "readme.tool.mysql.env.MYSQL_PORT"),
        ],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: Some("readme.tool.mysql.start_local"),
        credentials_local: Some("readme.tool.mysql.credentials_local"),
        verify_local: Some("readme.tool.mysql.verify_local"),
    }
}

fn tool_mongodb() -> ToolProfile {
    ToolProfile {
        name: "MongoDB",
        what: "readme.tool.mongodb.what",
        config: "readme.tool.mongodb.config",
        start: "readme.tool.mongodb.start",
        credentials: "readme.tool.mongodb.credentials",
        verify: "readme.tool.mongodb.verify",
        env: &[
            ("MONGODB_URI", "readme.tool.mongodb.env.MONGODB_URI"),
            ("MONGODB_DB", "readme.tool.mongodb.env.MONGODB_DB"),
        ],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: Some("readme.tool.mongodb.start_local"),
        credentials_local: Some("readme.tool.mongodb.credentials_local"),
        verify_local: Some("readme.tool.mongodb.verify_local"),
    }
}

fn tool_sqlite() -> ToolProfile {
    ToolProfile {
        name: "SQLite",
        what: "readme.tool.sqlite.what",
        config: "readme.tool.sqlite.config",
        start: "readme.tool.sqlite.start",
        credentials: "readme.tool.sqlite.credentials",
        verify: "readme.tool.sqlite.verify",
        env: &[("DATABASE_URL", "readme.tool.sqlite.env.DATABASE_URL")],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_redis() -> ToolProfile {
    ToolProfile {
        name: "Redis",
        what: "readme.tool.redis.what",
        config: "readme.tool.redis.config",
        start: "readme.tool.redis.start",
        credentials: "readme.tool.redis.credentials",
        verify: "readme.tool.redis.verify",
        env: &[("REDIS_URL", "readme.tool.redis.env.REDIS_URL")],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: Some("readme.tool.redis.start_local"),
        credentials_local: Some("readme.tool.redis.credentials_local"),
        verify_local: Some("readme.tool.redis.verify_local"),
    }
}

fn tool_kafka() -> ToolProfile {
    ToolProfile {
        name: "Apache Kafka",
        what: "readme.tool.kafka.what",
        config: "readme.tool.kafka.config",
        start: "readme.tool.kafka.start",
        credentials: "readme.tool.kafka.credentials",
        verify: "readme.tool.kafka.verify",
        env: &[("KAFKA_BOOTSTRAP_SERVERS", "readme.tool.kafka.env.KAFKA_BOOTSTRAP_SERVERS")],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: Some("readme.tool.kafka.start_local"),
        credentials_local: Some("readme.tool.kafka.credentials_local"),
        verify_local: Some("readme.tool.kafka.verify_local"),
    }
}

fn tool_clickhouse() -> ToolProfile {
    ToolProfile {
        name: "ClickHouse",
        what: "readme.tool.clickhouse.what",
        config: "readme.tool.clickhouse.config",
        start: "readme.tool.clickhouse.start",
        credentials: "readme.tool.clickhouse.credentials",
        verify: "readme.tool.clickhouse.verify",
        env: &[
            ("CLICKHOUSE_HOST", "readme.tool.clickhouse.env.CLICKHOUSE_HOST"),
            ("CLICKHOUSE_HTTP_PORT", "readme.tool.clickhouse.env.CLICKHOUSE_HTTP_PORT"),
        ],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: Some("readme.tool.clickhouse.start_local"),
        credentials_local: Some("readme.tool.clickhouse.credentials_local"),
        verify_local: Some("readme.tool.clickhouse.verify_local"),
    }
}

fn tool_rabbitmq() -> ToolProfile {
    ToolProfile {
        name: "RabbitMQ",
        what: "readme.tool.rabbitmq.what",
        config: "readme.tool.rabbitmq.config",
        start: "readme.tool.rabbitmq.start",
        credentials: "readme.tool.rabbitmq.credentials",
        verify: "readme.tool.rabbitmq.verify",
        env: &[("AMQP_URL", "readme.tool.rabbitmq.env.AMQP_URL")],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_minio() -> ToolProfile {
    ToolProfile {
        name: "MinIO",
        what: "readme.tool.minio.what",
        config: "readme.tool.minio.config",
        start: "readme.tool.minio.start",
        credentials: "readme.tool.minio.credentials",
        verify: "readme.tool.minio.verify",
        env: &[
            ("MINIO_ENDPOINT", "readme.tool.minio.env.MINIO_ENDPOINT"),
            ("MINIO_ROOT_USER", "readme.tool.minio.env.MINIO_ROOT_USER"),
            ("MINIO_ROOT_PASSWORD", "readme.tool.minio.env.MINIO_ROOT_PASSWORD"),
        ],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_mailpit() -> ToolProfile {
    ToolProfile {
        name: "Mailpit",
        what: "readme.tool.mailpit.what",
        config: "readme.tool.mailpit.config",
        start: "readme.tool.mailpit.start",
        credentials: "readme.tool.mailpit.credentials",
        verify: "readme.tool.mailpit.verify",
        env: &[
            ("SMTP_HOST", "readme.tool.mailpit.env.SMTP_HOST"),
            ("SMTP_PORT", "readme.tool.mailpit.env.SMTP_PORT"),
            ("MAILPIT_UI_PORT", "readme.tool.mailpit.env.MAILPIT_UI_PORT"),
        ],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_grafana() -> ToolProfile {
    ToolProfile {
        name: "Grafana",
        what: "readme.tool.grafana.what",
        config: "readme.tool.grafana.config",
        start: "readme.tool.grafana.start",
        credentials: "readme.tool.grafana.credentials",
        verify: "readme.tool.grafana.verify",
        env: &[("GRAFANA_URL", "readme.tool.grafana.env.GRAFANA_URL")],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: Some("readme.tool.grafana.start_local"),
        credentials_local: Some("readme.tool.grafana.credentials_local"),
        verify_local: Some("readme.tool.grafana.verify_local"),
    }
}

fn tool_opentelemetry() -> ToolProfile {
    ToolProfile {
        name: "OpenTelemetry",
        what: "readme.tool.opentelemetry.what",
        config: "readme.tool.opentelemetry.config",
        start: "readme.tool.opentelemetry.start",
        credentials: "readme.tool.opentelemetry.credentials",
        verify: "readme.tool.opentelemetry.verify",
        env: &[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "readme.tool.opentelemetry.env.OTEL_EXPORTER_OTLP_ENDPOINT"),
            ("OTEL_SERVICE_NAME", "readme.tool.opentelemetry.env.OTEL_SERVICE_NAME"),
        ],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_airflow() -> ToolProfile {
    ToolProfile {
        name: "Airflow",
        what: "readme.tool.airflow.what",
        config: "readme.tool.airflow.config",
        start: "readme.tool.airflow.start",
        credentials: "readme.tool.airflow.credentials",
        verify: "readme.tool.airflow.verify",
        env: &[
            ("AIRFLOW__CORE__EXECUTOR", "readme.tool.airflow.env.AIRFLOW__CORE__EXECUTOR"),
            (
                "AIRFLOW__DATABASE__SQL_ALCHEMY_CONN",
                "Airflow's own database connection",
            ),
            ("AIRFLOW_WEBSERVER_PORT", "readme.tool.airflow.env.AIRFLOW_WEBSERVER_PORT"),
        ],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_dbt() -> ToolProfile {
    ToolProfile {
        name: "dbt",
        what: "readme.tool.dbt.what",
        config: "readme.tool.dbt.config",
        start: "readme.tool.dbt.start",
        credentials: "readme.tool.dbt.credentials",
        verify: "readme.tool.dbt.verify",
        env: &[("DATABASE_URL", "readme.tool.dbt.env.DATABASE_URL")],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_prisma() -> ToolProfile {
    ToolProfile {
        name: "Prisma",
        what: "readme.tool.prisma.what",
        config: "readme.tool.prisma.config",
        start: "readme.tool.prisma.start",
        credentials: "readme.tool.prisma.credentials",
        verify: "readme.tool.prisma.verify",
        env: &[("DATABASE_URL", "readme.tool.prisma.env.DATABASE_URL")],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_drizzle() -> ToolProfile {
    ToolProfile {
        name: "Drizzle",
        what: "readme.tool.drizzle.what",
        config: "readme.tool.drizzle.config",
        start: "readme.tool.drizzle.start",
        credentials: "readme.tool.drizzle.credentials",
        verify: "readme.tool.drizzle.verify",
        env: &[("DATABASE_URL", "readme.tool.drizzle.env.DATABASE_URL")],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_sqlalchemy() -> ToolProfile {
    ToolProfile {
        name: "SQLAlchemy",
        what: "readme.tool.sqlalchemy.what",
        config: "readme.tool.sqlalchemy.config",
        start: "readme.tool.sqlalchemy.start",
        credentials: "readme.tool.sqlalchemy.credentials",
        verify: "readme.tool.sqlalchemy.verify",
        env: &[("DATABASE_URL", "readme.tool.sqlalchemy.env.DATABASE_URL")],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_alembic() -> ToolProfile {
    ToolProfile {
        name: "Alembic",
        what: "readme.tool.alembic.what",
        config: "readme.tool.alembic.config",
        start: "readme.tool.alembic.start",
        credentials: "readme.tool.alembic.credentials",
        verify: "readme.tool.alembic.verify",
        env: &[("DATABASE_URL", "readme.tool.alembic.env.DATABASE_URL")],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_pytest() -> ToolProfile {
    ToolProfile {
        name: "Pytest",
        what: "readme.tool.pytest.what",
        config: "readme.tool.pytest.config",
        start: "readme.tool.pytest.start",
        credentials: "readme.tool.pytest.credentials",
        verify: "readme.tool.pytest.verify",
        env: &[],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_ruff() -> ToolProfile {
    ToolProfile {
        name: "Ruff",
        what: "readme.tool.ruff.what",
        config: "readme.tool.ruff.config",
        start: "readme.tool.ruff.start",
        credentials: "readme.tool.ruff.credentials",
        verify: "readme.tool.ruff.verify",
        env: &[],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_npm() -> ToolProfile {
    ToolProfile {
        name: "npm",
        what: "readme.tool.npm.what",
        config: "readme.tool.npm.config",
        start: "readme.tool.npm.start",
        credentials: "readme.tool.npm.credentials",
        verify: "readme.tool.npm.verify",
        env: &[],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_maven() -> ToolProfile {
    ToolProfile {
        name: "Maven",
        what: "readme.tool.maven.what",
        config: "readme.tool.maven.config",
        start: "readme.tool.maven.start",
        credentials: "readme.tool.maven.credentials",
        verify: "readme.tool.maven.verify",
        env: &[],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

fn tool_gradle() -> ToolProfile {
    ToolProfile {
        name: "Gradle",
        what: "readme.tool.gradle.what",
        config: "readme.tool.gradle.config",
        start: "readme.tool.gradle.start",
        credentials: "readme.tool.gradle.credentials",
        verify: "readme.tool.gradle.verify",
        env: &[],
        start_docker: None,
        credentials_docker: None,
        verify_docker: None,
        start_local: None,
        credentials_local: None,
        verify_local: None,
    }
}

// ============================================================================
// Композиция секций
// ============================================================================

fn section_overview(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let project_type = match ctx.context.project_type.as_deref() {
        Some(t) => t.to_string(),
        None => ctx.t("readme.common.project"),
    };
    let langs: Vec<String> = ctx
        .context
        .languages
        .iter()
        .map(|l| language_name(l))
        .collect();
    let fws: Vec<String> = ctx
        .context
        .frameworks
        .iter()
        .map(|f| framework_name(f))
        .collect();
    let tools: Vec<String> = ctx.context.tools.iter().map(|t| tool_name(t)).collect();

    let mut parts: Vec<String> = Vec::new();
    if !langs.is_empty() {
        parts.push(ctx.tf(
            "readme.overview.written_in",
            &[("list", &join_and(&langs, ctx.i18n))],
        ));
    }
    if !fws.is_empty() {
        parts.push(ctx.tf(
            "readme.overview.using",
            &[("list", &join_and(&fws, ctx.i18n))],
        ));
    }
    if !tools.is_empty() {
        parts.push(ctx.tf(
            "readme.overview.with_tools",
            &[("list", &join_and(&tools, ctx.i18n))],
        ));
    }
    let tail = if parts.is_empty() {
        ctx.t("readme.overview.minimal")
    } else {
        parts.join(&ctx.t("readme.overview.joiner"))
    };

    let body = ctx.tf(
        "readme.overview.body",
        &[
            ("name", ctx.project_name),
            ("type", &project_type),
            ("stack", &tail),
        ],
    );
    doc.add(ctx.t("readme.section.overview.title"), body);
}

fn section_architecture(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let arch = architecture(ctx);
    let body = match arch {
        Architecture::Connected => {
            let shell = match embedded_shell(ctx) {
                Some(shell) => shell.to_string(),
                None => ctx.t("readme.arch_body.desktop_shell"),
            };
            let mut text = ctx.tf("readme.arch_body.connected", &[("shell", &shell)]);
            let fw_dirs: Vec<(String, String)> = ctx
                .context
                .frameworks
                .iter()
                .filter_map(|f| ctx.layout.framework_dir(f).map(|d| (f.clone(), d)))
                .collect();
            let has_backend_companion = fw_dirs.iter().any(|(_, d)| d == "backend");
            let has_frontend_companion = fw_dirs.iter().any(|(_, d)| d == "frontend");
            if has_frontend_companion {
                text.push_str(&ctx.t("readme.arch_body.connected_frontend"));
            }
            if has_backend_companion {
                text.push_str(&ctx.t("readme.arch_body.connected_backend"));
            }
            text
        }
        Architecture::Separated => ctx.t("readme.arch_body.separated"),
        Architecture::ShellClientApi => ctx.t("readme.arch_body.shell_client_api"),
        Architecture::BackendOnly => ctx.t("readme.arch_body.backend_only"),
        Architecture::FrontendOnly => {
            let mut text = ctx.t("readme.arch_body.frontend_only");
            let fullstack = [
                "nextjs",
                "nuxt",
                "sveltekit",
                "phoenix",
                "laravel",
                "solidjs",
            ];
            if ctx
                .context
                .frameworks
                .iter()
                .any(|f| fullstack.contains(&f.as_str()))
            {
                text.push_str(&ctx.t("readme.arch_body.frontend_only_fullstack"));
            }
            text
        }
        Architecture::Other => ctx.t("readme.arch_body.other"),
    };
    doc.add(
        ctx.t("readme.section.architecture.title"),
        format!("_{}_", ctx.t(arch.key())),
    );
    doc.sections
        .last_mut()
        .expect("ReadmeDoc::add pushes a section immediately before this line")
        .body = body;
}

fn section_selected_stack(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let mut body = String::new();
    let langs: Vec<&str> = sorted_unique(ctx.context.languages.iter().map(String::as_str));
    if !langs.is_empty() {
        body.push_str(&format!("{}\n\n", ctx.t("readme.stack.languages")));
        for l in langs {
            body.push_str(&format!(
                "- **{}** — {}\n",
                language_name(l),
                language_profile(l)
                    .map(|p| ctx.t(p.summary))
                    .unwrap_or_default()
            ));
        }
        body.push('\n');
    }
    let fws: Vec<&str> = sorted_unique(ctx.context.frameworks.iter().map(String::as_str));
    if !fws.is_empty() {
        body.push_str(&format!("{}\n\n", ctx.t("readme.stack.frameworks")));
        for f in fws {
            body.push_str(&format!(
                "- **{}** — {}\n",
                framework_name(f),
                framework_profile(f)
                    .map(|p| ctx.t(p.what))
                    .unwrap_or_default()
            ));
        }
        body.push('\n');
    }
    let tools: Vec<&str> = sorted_unique(ctx.context.tools.iter().map(String::as_str));
    if !tools.is_empty() {
        body.push_str(&format!("{}\n\n", ctx.t("readme.stack.tools")));
        for t in tools {
            let mode = tool_mode(ctx, t);
            body.push_str(&format!(
                "- **{}** — {}\n",
                tool_name(t),
                ctx.tf("readme.stack.tool_runs_in", &[("mode", &ctx.t(mode.key()))])
            ));
        }
        body.push('\n');
    }
    let mut features: Vec<String> = Vec::new();
    if ctx.context.docker {
        features.push(ctx.t("readme.feature.docker"));
    }
    if ctx.context.testing {
        features.push(ctx.t("readme.feature.testing"));
    }
    if ctx.context.git_init {
        features.push(ctx.t("readme.feature.git"));
    }
    if ctx.context.ci {
        features.push(ctx.t("readme.feature.ci"));
    }
    if ctx.context.vscode_config {
        features.push(ctx.t("readme.feature.vscode"));
    }
    if !features.is_empty() {
        body.push_str(&format!(
            "{}\n\n- {}\n",
            ctx.t("readme.stack.features"),
            features.join("\n- ")
        ));
    }
    doc.add(ctx.t("readme.section.stack.title"), body);
}

fn section_directory_map(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let mut dirs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for fw in sorted_unique(ctx.context.frameworks.iter().map(String::as_str)) {
        let dir = ctx
            .layout
            .framework_dir(fw)
            .unwrap_or_else(|| ".".to_string());
        dirs.entry(dir).or_default().push(ctx.tf(
            "readme.dirmap.framework",
            &[("name", &framework_name(fw))],
        ));
    }
    for lang in sorted_unique(ctx.context.languages.iter().map(String::as_str)) {
        let dir = ctx
            .layout
            .language_dir(lang)
            .unwrap_or_else(|| ".".to_string());
        dirs.entry(dir).or_default().push(language_name(lang));
    }
    if let Some(owner) = &ctx.layout.root_owner {
        dirs.entry("src-tauri".to_string())
            .or_default()
            .push(ctx.tf("readme.dirmap.shell", &[("name", &framework_name(owner))]));
    }

    let mut lines: Vec<String> = vec![format!("{}/", ctx.project_name)];
    let root_entries: Vec<(String, Vec<String>)> = dirs
        .iter()
        .filter(|(dir, _)| dir.as_str() != ".")
        .map(|(dir, items)| (dir.clone(), items.clone()))
        .collect();
    let root_files: Vec<(String, String)> = {
        let mut files: Vec<(String, String)> =
            vec![("README.md".into(), ctx.t("readme.dirmap.this_file"))];
        if !ctx.context.tools.is_empty() {
            files.push((".env.example".into(), ctx.t("readme.dirmap.env_template")));
        }
        if !ctx.context.local_infra_tools.is_empty() {
            files.push(("LOCAL_INFRA.md".into(), ctx.t("readme.dirmap.local_infra")));
        }
        if ctx.context.docker {
            files.push(("Dockerfile".into(), ctx.t("readme.dirmap.dockerfile")));
            files.push((
                "docker-compose.yaml".into(),
                ctx.t("readme.dirmap.compose"),
            ));
        }
        if ctx.context.git_init {
            files.push((".gitignore".into(), ctx.t("readme.dirmap.gitignore")));
        }
        if ctx.context.ci {
            files.push((".github/workflows/ci.yaml".into(), ctx.t("readme.dirmap.ci")));
        }
        files
    };
    // Файлы каркасов (языковые манифесты). В split-раскладке манифест лежит
    // ВНУТРИ сегмента языка (backend/requirements.txt, frontend/package.json) —
    // путь обязан совпадать с фактической записью движка, а не корнем.
    let manifest_path = |lang: &str, name: &str| -> String {
        match ctx.layout.language_dir(lang) {
            Some(dir) => format!("{}/{}", dir, name),
            None => name.to_string(),
        }
    };
    let root_manifests: Vec<String> = {
        let mut m = Vec::new();
        if has_language(ctx, "python") {
            m.push(manifest_path("python", "requirements.txt"));
            m.push(manifest_path("python", "pyproject.toml"));
        }
        if has_language(ctx, "rust") && !has_framework(ctx, "tauri") {
            m.push(manifest_path("rust", "Cargo.toml"));
        }
        if has_language(ctx, "go") {
            m.push(manifest_path("go", "go.mod"));
        }
        if has_language(ctx, "java") || has_language(ctx, "kotlin") {
            let lang = if has_language(ctx, "java") { "java" } else { "kotlin" };
            m.push(match ctx.layout.language_dir(lang) {
                Some(dir) => format!("{dir}/pom.xml / {dir}/build.gradle.kts"),
                None => "pom.xml / build.gradle.kts".to_string(),
            });
        }
        if has_language(ctx, "typescript") || has_language(ctx, "javascript") {
            let lang = if has_language(ctx, "typescript") {
                "typescript"
            } else {
                "javascript"
            };
            m.push(manifest_path(lang, "package.json"));
        }
        if has_language(ctx, "php") {
            m.push(manifest_path("php", "composer.json"));
        }
        if has_language(ctx, "cpp") || has_language(ctx, "c") {
            let lang = if has_language(ctx, "cpp") { "cpp" } else { "c" };
            m.push(manifest_path(lang, "CMakeLists.txt"));
        }
        if has_language(ctx, "zig") {
            m.push(manifest_path("zig", "build.zig"));
        }
        if has_language(ctx, "elixir") {
            m.push(manifest_path("elixir", "mix.exs"));
        }
        if has_language(ctx, "gleam") {
            m.push(manifest_path("gleam", "gleam.toml"));
        }
        if has_language(ctx, "dart") {
            m.push(manifest_path("dart", "pubspec.yaml"));
        }
        if has_language(ctx, "swift") {
            m.push(manifest_path("swift", "Package.swift"));
        }
        m
    };

    let total_entries = root_entries.len() + root_files.len() + root_manifests.len();
    let mut idx = 0;
    for (dir, items) in &root_entries {
        idx += 1;
        let last = idx == total_entries;
        lines.push(format!(
            "{} {:<14} # {}",
            if last { "└──" } else { "├──" },
            format!("{}/", dir),
            items.join(", ")
        ));
    }
    for (path, desc) in &root_files {
        idx += 1;
        let last = idx == total_entries;
        lines.push(format!(
            "{} {:<14} # {}",
            if last { "└──" } else { "├──" },
            path,
            desc
        ));
    }
    for manifest in &root_manifests {
        idx += 1;
        let last = idx == total_entries;
        lines.push(format!(
            "{} {:<14} # {}",
            if last { "└──" } else { "├──" },
            manifest,
            ctx.t("readme.dirmap.language_manifest")
        ));
    }
    if total_entries == 0 {
        lines.push(ctx.t("readme.dirmap.empty"));
    }

    let body = format!("```text\n{}\n```", lines.join("\n"));
    doc.add(ctx.t("readme.section.directory_map.title"), body);
}

fn section_how_created(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let fws: Vec<String> = ctx
        .context
        .frameworks
        .iter()
        .map(|f| framework_name(f))
        .collect();
    let mut config_files: Vec<String> = Vec::new();
    if !ctx.context.tools.is_empty() {
        config_files.push("`.env.example`".to_string());
    }
    if ctx.context.docker {
        config_files.push("`docker-compose.yaml`".to_string());
    }
    if !ctx.context.local_infra_tools.is_empty() {
        config_files.push("`LOCAL_INFRA.md`".to_string());
    }
    config_files.push("`.gitignore`".to_string());
    let targets = if fws.is_empty() {
        ctx.t("readme.how_created.languages")
    } else {
        ctx.tf(
            "readme.how_created.frameworks",
            &[("list", &join_and(&fws, ctx.i18n))],
        )
    };
    let ci = if ctx.context.ci {
        ctx.t("readme.how_created.ci")
    } else {
        String::new()
    };
    let git = if ctx.context.git_init {
        ctx.t("readme.how_created.git")
    } else {
        String::new()
    };
    let body = ctx.tf(
        "readme.how_created.body",
        &[
            ("targets", &targets),
            ("ci", &ci),
            ("files", &config_files.join(", ")),
            ("git", &git),
        ],
    );
    doc.add(ctx.t("readme.section.how_created.title"), body);
}

/// Связь фреймворка с остальными компонентами (контекстно-зависимая).
fn framework_relationship(ctx: &ReadmeContext, fw: &str) -> String {
    let shell = embedded_shell(ctx);
    // Qt WebEngine встраивает веб-фронтенд — специфичный текст ДО общего
    // описания оболочки, иначе ветка `fw == "qt"` перехватит его первой.
    if qt_is_webengine(ctx) && matches!(fw, "qt" | "qt-webengine") {
        let web = ctx
            .context
            .frameworks
            .iter()
            .find(|f| is_web_frontend(f))
            .map(|f| framework_name(f).to_string())
            .unwrap_or_else(|| ctx.t("readme.common.the_web_ui"));
        return ctx.tf("readme.fw_rel.qt_webengine", &[("web", &web)]);
    }
    if let Some(shell_label) = shell {
        if fw == "tauri" || fw == "qt" || fw.starts_with("qt-") {
            return ctx.tf(
                "readme.fw_rel.shell_host",
                &[("name", &framework_name(fw))],
            );
        }
        if is_web_frontend(fw) {
            return ctx.tf("readme.fw_rel.embedded_ui", &[("shell", shell_label)]);
        }
    }
    // Неинтегрированные клиентские оболочки (electron, expo, react-native,
    // plasmo) НЕ встроены в корень проекта: они standalone-клиенты,
    // общающиеся с API по HTTP (LayoutClass::ShellClientApi / FrontendOnly).
    if matches!(fw, "electron" | "expo" | "react-native" | "plasmo") {
        return ctx.t("readme.fw_rel.client_shell");
    }
    let frontends: Vec<&str> = ctx
        .context
        .frameworks
        .iter()
        .map(String::as_str)
        .filter(|f| is_web_frontend(f))
        .collect();
    let backends: Vec<&str> = ctx
        .context
        .frameworks
        .iter()
        .map(String::as_str)
        .filter(|f| is_backend_framework(f))
        .collect();
    if backends.iter().any(|b| b == &fw) && !frontends.is_empty() && shell.is_none() {
        let names: Vec<String> = frontends
            .iter()
            .map(|f| framework_name(f).to_string())
            .collect();
        return ctx.tf(
            "readme.fw_rel.serves_api",
            &[("frontends", &join_and(&names, ctx.i18n))],
        );
    }
    if is_web_frontend(fw) && !backends.is_empty() && shell.is_none() {
        let names: Vec<String> = backends
            .iter()
            .map(|b| framework_name(b).to_string())
            .collect();
        return ctx.tf(
            "readme.fw_rel.consumes_api",
            &[("backends", &join_and(&names, ctx.i18n))],
        );
    }
    if matches!(fw, "aiogram" | "telegraf") {
        return ctx.t("readme.fw_rel.telegram");
    }
    if is_backend_framework(fw) && architecture(ctx) == Architecture::FrontendOnly {
        return ctx.t("readme.fw_rel.server_runtime");
    }
    if matches!(
        fw,
        "nextjs" | "nuxt" | "sveltekit" | "phoenix" | "laravel" | "solidjs"
    ) && backends.is_empty()
        && shell.is_none()
    {
        return ctx.t("readme.fw_rel.own_server");
    }
    ctx.tf(
        "readme.fw_rel.fallback",
        &[("arch", &ctx.t(architecture(ctx).key()))],
    )
}

fn is_backend_framework(fw: &str) -> bool {
    matches!(
        fw,
        "fastapi"
            | "django"
            | "flask"
            | "axum"
            | "gin"
            | "nest"
            | "express"
            | "fastify"
            | "spring-boot"
            | "aspnetcore"
            | "ktor"
            | "vapor"
            | "zap"
            | "phoenix"
            | "laravel"
            | "symfony"
    )
}

fn section_frameworks(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let fws = sorted_unique(ctx.context.frameworks.iter().map(String::as_str));
    if fws.is_empty() {
        return;
    }
    let mut body = String::new();
    for fw in fws {
        let profile = framework_profile(fw);
        let name = framework_name(fw);
        body.push_str(&format!("### {}\n\n", name));
        if let Some(p) = profile {
            body.push_str(&format!(
                "**{}** {}\n\n",
                ctx.t("readme.label.what_it_does"),
                ctx.t(p.what)
            ));
            body.push_str(&format!(
                "**{}** {}\n\n",
                ctx.t("readme.label.first_code"),
                ctx.t(p.first_code)
            ));
            body.push_str(&format!(
                "**{}** {}\n\n",
                ctx.t("readme.label.entry"),
                ctx.t(p.entry)
            ));
            // dev_cmd приходит с собственными кавычками из локали; python-
            // команды получают реальный интерпретатор venv и `cd <segment>`.
            body.push_str(&format!(
                "**{}** {}\n\n",
                ctx.t("readme.label.dev_cmd"),
                fill_python_paths(ctx, &ctx.t(p.dev_cmd))
            ));
            body.push_str(&format!(
                "**{}** {}\n\n",
                ctx.t("readme.label.build_cmd"),
                fill_python_paths(ctx, &ctx.t(p.build_cmd))
            ));
            match p.test_cmd {
                Some(cmd) => body.push_str(&format!(
                    "**{}** `{}`\n\n",
                    ctx.t("readme.label.test_cmd"),
                    fill_python_paths(ctx, &ctx.t(cmd))
                )),
                None => body.push_str(&format!(
                    "**{}** {}\n\n",
                    ctx.t("readme.label.test_cmd"),
                    ctx.t("readme.label.test_not_configured")
                )),
            }
            body.push_str(&format!(
                "**{}** {}\n\n",
                ctx.t("readme.label.relationship"),
                framework_relationship(ctx, fw)
            ));
        } else {
            body.push_str(&format!(
                "{}\n\n",
                ctx.tf(
                    "readme.section.frameworks.no_guide",
                    &[("name", name.as_str())]
                )
            ));
        }
    }
    doc.add(ctx.t("readme.section.frameworks.title"), body);
}

fn section_tools(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let tools = sorted_unique(ctx.context.tools.iter().map(String::as_str));
    if tools.is_empty() {
        return;
    }
    // Порт приложения и ремапы инфра-сервисов: единый план из ports.rs,
    // чтобы README не противоречил docker-compose.yaml/.env.example
    // (например, Airflow на 8081 рядом с Spring Boot, Grafana на 3001).
    let plan = crate::ports::PortPlan::new(
        &ctx.context.frameworks,
        &ctx.context.tools,
        &ctx.context.local_infra_tools,
    );
    let mut body = String::new();
    for tool in tools {
        let Some(p) = tool_profile(tool) else {
            body.push_str(&format!("### {}\n\n", tool));
            continue;
        };
        let mode = tool_mode(ctx, tool);
        let start = fill_python_paths(
            ctx,
            &ctx.t(match mode {
                ToolMode::Docker => p.start_docker.unwrap_or(p.start),
                ToolMode::Local => p.start_local.unwrap_or(p.start),
                _ => p.start,
            }),
        );
        let credentials = ctx.t(match mode {
            ToolMode::Docker => p.credentials_docker.unwrap_or(p.credentials),
            ToolMode::Local => p.credentials_local.unwrap_or(p.credentials),
            _ => p.credentials,
        });
        let credentials = match tool {
            // Airflow's credentials line also carries the host UI URL.
            "airflow" if mode == ToolMode::Docker => {
                let port = plan.tool_ports("airflow").first().copied().unwrap_or(8080);
                replace_host_port(&credentials, port)
            }
            _ => credentials,
        };
        let verify = fill_python_paths(
            ctx,
            &ctx.t(match mode {
                ToolMode::Docker => p.verify_docker.unwrap_or(p.verify),
                ToolMode::Local => p.verify_local.unwrap_or(p.verify),
                _ => p.verify,
            }),
        );
        // Host-порты инструментов, которые могли быть ремаплены из-за
        // конфликта с приложением (см. PortPlan).
        let verify = match tool {
            "airflow" if mode == ToolMode::Docker => {
                let port = plan.tool_ports("airflow").first().copied().unwrap_or(8080);
                replace_host_port(&verify, port)
            }
            "grafana" if mode == ToolMode::Docker => {
                let port = plan.tool_ports("grafana").first().copied().unwrap_or(3001);
                replace_host_port(&verify, port)
            }
            "grafana" if mode == ToolMode::Local => {
                replace_host_port(&verify, plan.grafana_local_port())
            }
            _ => verify,
        };
        body.push_str(&format!("### {}\n\n", p.name));
        body.push_str(&format!(
            "**{}** {}\n\n",
            ctx.t("readme.label.tool_what"),
            ctx.t(p.what)
        ));
        body.push_str(&format!(
            "**{}** {}\n\n",
            ctx.t("readme.label.tool_config"),
            ctx.t(p.config)
        ));
        body.push_str(&format!(
            "**{}** {}\n\n",
            ctx.t("readme.label.tool_mode"),
            ctx.t(mode.key())
        ));
        body.push_str(&format!(
            "**{}** {}\n\n",
            ctx.t("readme.label.tool_start"),
            start
        ));
        body.push_str(&format!(
            "**{}** {}\n\n",
            ctx.t("readme.label.tool_credentials"),
            credentials
        ));
        if !p.env.is_empty() {
            body.push_str(&format!("**{}**\n\n", ctx.t("readme.label.tool_env")));
            for (name, desc) in p.env {
                body.push_str(&format!("- `{}` — {}\n", name, ctx.t(desc)));
            }
            body.push('\n');
        }
        body.push_str(&format!(
            "**{}** {}\n\n",
            ctx.t("readme.label.tool_verify"),
            verify
        ));
    }
    doc.add(ctx.t("readme.section.tools.title"), body);
}

/// Заменить `http://localhost:<старый-порт>` в тексте на фактический порт
/// сервиса. Версии с префиксом `localhost:` заменяются на новый порт;
/// остальной текст (например контейнерные порты) не трогается.
fn replace_host_port(text: &str, port: u16) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut rest = text;
    while let Some(idx) = rest.find("localhost:") {
        out.push_str(&rest[..idx]);
        let after = &rest[idx + "localhost:".len()..];
        // Съедаем старый числовой порт (если есть).
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            out.push_str("localhost:");
            rest = after;
            continue;
        }
        out.push_str(&format!("localhost:{}", port));
        rest = &after[digits.len()..];
    }
    out.push_str(rest);
    out
}

fn section_prerequisites(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let mut items: Vec<String> = Vec::new();
    for lang in sorted_unique(ctx.context.languages.iter().map(String::as_str)) {
        let key = match lang {
            "python" => "readme.prereq.python",
            "rust" => "readme.prereq.rust",
            "go" => "readme.prereq.go",
            "typescript" | "javascript" => "readme.prereq.node",
            "java" | "kotlin" => "readme.prereq.jdk",
            "csharp" => "readme.prereq.dotnet",
            "cpp" | "c" => "readme.prereq.cpp",
            "dart" => "readme.prereq.dart",
            "php" => "readme.prereq.php",
            "swift" => "readme.prereq.swift",
            "zig" => "readme.prereq.zig",
            "elixir" => "readme.prereq.elixir",
            "gleam" => "readme.prereq.gleam",
            "html" => "readme.prereq.html",
            _ => "readme.prereq.generic",
        };
        let req = ctx.t(key);
        if !items.contains(&req) {
            items.push(req);
        }
    }
    for fw in sorted_unique(ctx.context.frameworks.iter().map(String::as_str)) {
        let key = match fw {
            "tauri" | "electron" => "readme.prereq.desktop",
            "flutter" => "readme.prereq.flutter",
            "maui" | "aspnetcore" | "csharp" => "readme.prereq.dotnet",
            "spring-boot" | "ktor" | "jetpack-compose" => "readme.prereq.jdk",
            "swiftui" | "vapor" => "readme.prereq.swift",
            "react-native" | "expo" => "readme.prereq.mobile",
            "qt" | "qt-qml" | "qt-widgets" | "qt-webengine" | "qt-kirigami" => {
                "readme.prereq.qt"
            }
            "phoenix" => "readme.prereq.phoenix",
            "laravel" | "symfony" => "readme.prereq.php",
            "zig-cli" | "zap" => "readme.prereq.zig",
            _ => "",
        };
        if key.is_empty() {
            continue;
        }
        let req = ctx.t(key);
        if !items.contains(&req) {
            items.push(req);
        }
    }
    for tool in sorted_unique(ctx.context.tools.iter().map(String::as_str)) {
        let key = match tool_mode(ctx, tool) {
            ToolMode::Docker => "readme.prereq.docker",
            ToolMode::Local => "readme.prereq.local_service",
            ToolMode::Embedded => match tool {
                "npm" => "readme.prereq.node",
                "maven" | "gradle" => "readme.prereq.jdk",
                "sqlite" => "readme.prereq.sqlite",
                _ => "",
            },
            ToolMode::Unmanaged => "",
        };
        if key.is_empty() {
            continue;
        }
        let req = ctx.t(key);
        if !items.contains(&req) {
            items.push(req);
        }
    }
    let docker_req = ctx.t("readme.prereq.docker");
    if ctx.context.docker && !items.contains(&docker_req) {
        items.push(docker_req);
    }
    if items.is_empty() {
        items.push(ctx.t("readme.prereq.none"));
    }
    let body = items
        .iter()
        .map(|i| format!("- {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    doc.add(ctx.t("readme.section.prerequisites.title"), body);
}

fn section_quick_start(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let mut steps: Vec<String> = Vec::new();
    steps.push(ctx.t("readme.quick.prereq"));
    let docker_tools = docker_managed_tools(ctx);
    if !docker_tools.is_empty() {
        // Start ONLY the infrastructure services: the compose file also
        // carries the app service, and starting it would race the local dev
        // server for the app port (both bind the same host port).
        let infra_services: Vec<String> = docker_tools
            .iter()
            .filter_map(|t| crate::ports::tool_service_name(t))
            .map(String::from)
            .collect();
        let up_cmd = if infra_services.is_empty() {
            "docker compose up -d".to_string()
        } else {
            format!("docker compose up -d {}", infra_services.join(" "))
        };
        steps.push(ctx.tf(
            "readme.quick.docker_infra",
            &[
                ("cmd", &up_cmd),
                (
                    "tools",
                    &docker_tools
                        .iter()
                        .map(|t| tool_name(t))
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            ],
        ));
    }
    if !local_tools(ctx).is_empty() {
        steps.push(ctx.t("readme.quick.local_services"));
    }
    if !ctx.context.tools.is_empty() {
        steps.push(ctx.t("readme.quick.env"));
    }
    let mut install_cmds: Vec<String> = Vec::new();
    let py_dir = python_segment_dir(ctx);
    if has_language(ctx, "python") {
        // Канонический venv (venv/ в корне или backend/venv) и реальный
        // манифест (requirements.txt) — те же пути, что создаёт пайплайн.
        install_cmds.push(ctx.tf(
            "readme.quick.py_venv",
            &[
                ("python", python_command()),
                ("venv", &venv_dir_rel(ctx)),
                ("venv_python", &venv_prefix(ctx)),
                (
                    "requirements",
                    &super::preflight::requirements_path(&py_dir),
                ),
            ],
        ));
    }
    if has_language(ctx, "typescript") || has_language(ctx, "javascript") {
        let dirs = js_package_dirs(ctx);
        if dirs.is_empty() {
            install_cmds.push(ctx.t("readme.quick.npm_install"));
        } else {
            install_cmds.push(ctx.tf(
                "readme.quick.npm_install_dirs",
                &[("dirs", &dirs.join(", "))],
            ));
        }
    }
    if has_language(ctx, "rust") {
        install_cmds.push(ctx.t("readme.quick.rust_install"));
    }
    if has_language(ctx, "go") {
        install_cmds.push(ctx.t("readme.quick.go_install"));
    }
    if has_language(ctx, "php") {
        install_cmds.push(ctx.t("readme.quick.php_install"));
    }
    if has_language(ctx, "elixir") {
        install_cmds.push(ctx.t("readme.quick.elixir_install"));
    }
    if has_language(ctx, "dart") {
        install_cmds.push(ctx.t("readme.quick.dart_install"));
    }
    if has_language(ctx, "gleam") {
        install_cmds.push(ctx.t("readme.quick.gleam_install"));
    }
    if has_language(ctx, "swift") {
        install_cmds.push(ctx.t("readme.quick.swift_install"));
    }
    if !install_cmds.is_empty() {
        steps.push(ctx.tf(
            "readme.quick.install_deps",
            &[("list", &install_cmds.join("\n"))],
        ));
    }
    steps.push(ctx.t("readme.quick.dev_cmd"));

    let body = steps
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}. {}", i + 1, s))
        .collect::<Vec<_>>()
        .join("\n");
    doc.add(ctx.t("readme.section.quick_start.title"), body);
}

/// Секция «Architectural Decisions»: появляется ТОЛЬКО когда анализ нашёл
/// неочевидные решения (спорные связки фреймворк↔инструмент или пересечение
/// ответственностей инструментов). Чистый стек секции не получает.
fn section_architectural_decisions(
    ctx: &ReadmeContext,
    tree: &WizardTreeData,
    doc: &mut ReadmeDoc,
) {
    let decisions = analyze_architectural_decisions(
        tree,
        &ctx.context.frameworks,
        &ctx.context.tools,
        ctx.i18n,
    );
    if decisions.is_empty() {
        return;
    }
    let mut body = ctx.t("readme.decisions.intro");
    for decision in &decisions {
        body.push_str(&format!("### {}\n\n", decision.title));
        body.push_str(&format!(
            "**{}** {}\n\n",
            ctx.t("readme.decisions.context"),
            decision.reason
        ));
        body.push_str(&format!(
            "**{}** {}\n\n",
            ctx.t("readme.decisions.recommendation"),
            decision.recommendation
        ));
    }
    doc.add(ctx.t("readme.section.decisions.title"), body);
}

/// Каталоги JS-части проекта (для npm install в quick start).
fn js_package_dirs(ctx: &ReadmeContext) -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();
    let mut push = |dir: String| {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    };
    for fw in sorted_unique(ctx.context.frameworks.iter().map(String::as_str)) {
        if is_js_framework_id(fw) {
            push(
                ctx.layout
                    .framework_dir(fw)
                    .unwrap_or_else(|| ".".to_string()),
            );
        }
    }
    for lang in sorted_unique(ctx.context.languages.iter().map(String::as_str)) {
        if matches!(lang, "typescript" | "javascript") {
            push(
                ctx.layout
                    .language_dir(lang)
                    .unwrap_or_else(|| ".".to_string()),
            );
        }
    }
    dirs.sort_unstable();
    dirs
}

fn is_js_framework_id(fw: &str) -> bool {
    matches!(
        fw,
        "nextjs"
            | "sveltekit"
            | "nuxt"
            | "express"
            | "electron"
            | "telegraf"
            | "nest"
            | "fastify"
            | "solidjs"
            | "solidstart"
            | "react"
            | "vue"
            | "svelte"
            | "react-native"
            | "expo"
            | "plasmo"
    )
}

fn python_command() -> &'static str {
    if cfg!(target_os = "windows") {
        "python"
    } else {
        "python3"
    }
}

/// Каталог python-кода (backend/ в моно-репозитории, иначе корень).
fn python_segment_dir(ctx: &ReadmeContext) -> String {
    if has_language(ctx, "python") {
        if let Some(seg) = ctx.layout.language_dir("python") {
            return seg;
        }
    }
    ".".to_string()
}

fn section_first_code(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let mut body = String::new();
    let fws = sorted_unique(ctx.context.frameworks.iter().map(String::as_str));
    if !fws.is_empty() {
        body.push_str(&ctx.t("readme.first_code.intro_fw"));
        for fw in fws {
            if let Some(p) = framework_profile(fw) {
                body.push_str(&format!("- **{}** — {}\n", p.name, ctx.t(p.first_code)));
            }
        }
    } else {
        body.push_str(&ctx.t("readme.first_code.intro_lang"));
        for lang in sorted_unique(ctx.context.languages.iter().map(String::as_str)) {
            if let Some(p) = language_profile(lang) {
                body.push_str(&format!("- **{}** — {}\n", p.name, ctx.t(p.first_code)));
            }
        }
    }
    doc.add(ctx.t("readme.section.first_code.title"), body);
}

fn section_development(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let fws = sorted_unique(ctx.context.frameworks.iter().map(String::as_str));
    if fws.is_empty() {
        let langs = sorted_unique(ctx.context.languages.iter().map(String::as_str));
        let body = if langs.is_empty() {
            ctx.t("readme.development.none")
        } else {
            let mut lines: Vec<String> = Vec::new();
            for lang in langs {
                if let Some(p) = language_profile(lang) {
                    lines.push(format!(
                        "- **{}** — {}",
                        p.name,
                        fill_python_paths(ctx, &ctx.t(p.dev_cmd))
                    ));
                }
            }
            lines.join("\n")
        };
        doc.add(ctx.t("readme.section.development.title"), body);
        return;
    }
    let mut body = String::new();
    for fw in fws {
        if let Some(p) = framework_profile(fw) {
            // Python-команды рендерятся с реальным интерпретатором venv и
            // `cd <segment>` для split-раскладки; строки без плейсхолдеров
            // остаются как есть (кавычки — в самой локализованной строке).
            let cmd = fill_python_paths(ctx, &ctx.t(p.dev_cmd));
            body.push_str(&format!("- **{}** — {}\n", p.name, cmd));
        }
    }
    body.push_str(&ctx.t("readme.development.footer"));
    doc.add(ctx.t("readme.section.development.title"), body);
}

fn section_building(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let fws = sorted_unique(ctx.context.frameworks.iter().map(String::as_str));
    if fws.is_empty() {
        let langs = sorted_unique(ctx.context.languages.iter().map(String::as_str));
        let body = if langs.is_empty() {
            ctx.t("readme.building.none")
        } else {
            let mut lines: Vec<String> = Vec::new();
            for lang in langs {
                if let Some(p) = language_profile(lang) {
                    lines.push(format!(
                        "- **{}** — {}",
                        p.name,
                        fill_python_paths(ctx, &ctx.t(p.build_cmd))
                    ));
                }
            }
            lines.join("\n")
        };
        doc.add(ctx.t("readme.section.building.title"), body);
        return;
    }
    let mut body = String::new();
    for fw in fws {
        if let Some(p) = framework_profile(fw) {
            body.push_str(&format!(
                "- **{}** — {}\n",
                p.name,
                fill_python_paths(ctx, &ctx.t(p.build_cmd))
            ));
        }
    }
    doc.add(ctx.t("readme.section.building.title"), body);
}

fn section_testing(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let mut commands: Vec<(String, String)> = Vec::new();
    for fw in sorted_unique(ctx.context.frameworks.iter().map(String::as_str)) {
        if let Some(p) = framework_profile(fw) {
            if let Some(cmd) = p.test_cmd {
                commands.push((p.name.to_string(), fill_python_paths(ctx, &ctx.t(cmd))));
            }
        }
    }
    for lang in sorted_unique(ctx.context.languages.iter().map(String::as_str)) {
        if let Some(p) = language_profile(lang) {
            if let Some(cmd) = p.test_cmd {
                let translated = fill_python_paths(ctx, &ctx.t(cmd));
                let exists = commands.iter().any(|(name, _)| name == p.name);
                if !exists && !commands.iter().any(|(_, c)| c == &translated) {
                    commands.push((p.name.to_string(), translated));
                }
            }
        }
    }
    let has_tool = ctx.context.tools.iter().any(|t| t == "pytest");
    if has_tool {
        // Pytest запускается интерпретатором venv из каталога python-сегмента
        // (backend/ в split): тесты и pytest.ini лежат рядом с venv.
        let cmd = fill_python_paths(ctx, "{cd}{venv_seg} -m pytest");
        if !commands.iter().any(|(_, c)| c == &cmd) {
            commands.push(("Pytest".to_string(), cmd));
        }
    }
    let body = if commands.is_empty() {
        if ctx.context.testing {
            ctx.t("readme.testing.no_runner")
        } else {
            ctx.t("readme.testing.disabled")
        }
    } else {
        commands
            .iter()
            .map(|(name, cmd)| format!("- **{}** — `{}`", name, cmd))
            .collect::<Vec<_>>()
            .join("\n")
    };
    doc.add(ctx.t("readme.section.testing.title"), body);
}

fn section_environment(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let tools = sorted_unique(ctx.context.tools.iter().map(String::as_str));
    let mut body = String::new();
    body.push_str(&ctx.t("readme.env.intro"));
    if tools.is_empty() {
        body.push_str(&ctx.t("readme.env.none"));
        doc.add(ctx.t("readme.section.environment.title"), body);
        return;
    }
    body.push_str(&ctx.t("readme.env.tools_intro"));
    for tool in tools {
        let Some(p) = tool_profile(tool) else {
            continue;
        };
        if p.env.is_empty() {
            continue;
        }
        body.push_str(&format!("**{}**\n\n", p.name));
        for (name, desc) in p.env {
            body.push_str(&format!("- `{}` — {}\n", name, ctx.t(desc)));
        }
        body.push('\n');
    }
    let local = local_tools(ctx);
    if !local.is_empty() {
        body.push_str(&ctx.tf(
            "readme.env.local_note",
            &[(
                "list",
                &local
                    .iter()
                    .map(|t| tool_name(t))
                    .collect::<Vec<_>>()
                    .join(", "),
            )],
        ));
    }
    doc.add(ctx.t("readme.section.environment.title"), body);
}

fn section_docker(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    if !ctx.context.docker {
        doc.add(
            ctx.t("readme.section.docker.title"),
            ctx.t("readme.docker.disabled"),
        );
        return;
    }
    let docker_tools = docker_managed_tools(ctx);
    if docker_tools.is_empty() {
        let mut body = ctx.t("readme.docker.no_infra");
        if !local_tools(ctx).is_empty() {
            body.push_str(&ctx.t("readme.docker.start_local"));
        } else {
            body.push_str(&ctx.t("readme.docker.build_image"));
        }
        doc.add(ctx.t("readme.section.docker.title"), body);
        return;
    }
    let mut body = ctx.tf(
        "readme.docker.infra_intro",
        &[(
            "list",
            &docker_tools
                .iter()
                .map(|t| tool_name(t))
                .collect::<Vec<_>>()
                .join(", "),
        )],
    );
    // The compose file also carries the app service. Starting ONLY the infra
    // services (`up -d <services>`) is the right command for a dev session:
    // the full `up -d` would publish the app container on the same host port
    // the local dev server binds.
    let infra_services: Vec<String> = docker_tools
        .iter()
        .filter_map(|t| crate::ports::tool_service_name(t))
        .map(String::from)
        .collect();
    let infra_up = if infra_services.is_empty() {
        "docker compose up -d".to_string()
    } else {
        format!("docker compose up -d {}", infra_services.join(" "))
    };
    body.push_str(&ctx.t("readme.docker.start_all"));
    body.push_str(&ctx.tf(
        "readme.docker.start_infra",
        &[("cmd", &infra_up)],
    ));
    body.push_str(&ctx.t("readme.docker.stop"));
    body.push_str(&ctx.t("readme.docker.logs"));
    let local = local_tools(ctx);
    if !local.is_empty() {
        body.push_str(&ctx.tf(
            "readme.docker.local_note",
            &[(
                "list",
                &local
                    .iter()
                    .map(|t| tool_name(t))
                    .collect::<Vec<_>>()
                    .join(", "),
            )],
        ));
    }
    doc.add(ctx.t("readme.section.docker.title"), body);
}

fn section_mistakes(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let mut items: Vec<String> = vec![ctx.t("readme.mistakes.wrong_dir")];
    if !docker_managed_tools(ctx).is_empty() {
        let infra_services: Vec<String> = docker_managed_tools(ctx)
            .iter()
            .filter_map(|t| crate::ports::tool_service_name(t))
            .map(String::from)
            .collect();
        let infra_up = if infra_services.is_empty() {
            "docker compose up -d".to_string()
        } else {
            format!("docker compose up -d {}", infra_services.join(" "))
        };
        items.push(ctx.tf(
            "readme.mistakes.infra_order",
            &[("cmd", &infra_up)],
        ));
    }
    if !local_tools(ctx).is_empty() {
        items.push(ctx.t("readme.mistakes.local_services"));
    }
    if has_language(ctx, "python") {
        items.push(ctx.tf(
            "readme.mistakes.system_python",
            &[("venv", &venv_prefix(ctx))],
        ));
    }
    if has_language(ctx, "typescript") || has_language(ctx, "javascript") {
        items.push(ctx.t("readme.mistakes.npm_dir"));
    }
    if embedded_shell(ctx).is_some() {
        items.push(ctx.t("readme.mistakes.embedded"));
    }
    for lang in sorted_unique(ctx.context.languages.iter().map(String::as_str)) {
        if let Some(p) = language_profile(lang) {
            for tip in p.tips {
                let tip_text = fill_python_paths(ctx, &ctx.t(tip));
                items.push(tip_text);
            }
        }
    }
    if items.len() > 6 {
        items.truncate(6);
    }
    let body = items
        .iter()
        .map(|i| format!("- {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    doc.add(ctx.t("readme.section.mistakes.title"), body);
}

fn section_next_steps(ctx: &ReadmeContext, doc: &mut ReadmeDoc) {
    let fws = sorted_unique(ctx.context.frameworks.iter().map(String::as_str));
    let mut items: Vec<String> = Vec::new();
    if let Some(first) = fws.first() {
        items.push(ctx.tf(
            "readme.next.feature",
            &[("name", &framework_name(first))],
        ));
    } else {
        items.push(ctx.t("readme.next.pick_framework"));
    }
    if !ctx.context.tools.is_empty() {
        items.push(ctx.t("readme.next.services"));
    }
    if ctx.context.testing {
        items.push(ctx.t("readme.next.testing"));
    } else {
        items.push(ctx.t("readme.next.testing_off"));
    }
    if ctx.context.git_init {
        items.push(ctx.t("readme.next.git"));
    }
    if ctx.context.ci {
        items.push(ctx.t("readme.next.ci"));
    } else {
        items.push(ctx.t("readme.next.ci_off"));
    }
    if ctx.context.docker {
        items.push(ctx.t("readme.next.docker"));
    } else {
        items.push(ctx.t("readme.next.docker_off"));
    }
    let body = items
        .iter()
        .map(|i| format!("- {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    doc.add(ctx.t("readme.section.next_steps.title"), body);
}

// ============================================================================
// Тесты
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with(
        project_type: &str,
        languages: &[&str],
        backend_langs: &[&str],
        frontend_langs: &[&str],
        frameworks: &[&str],
        tools: &[&str],
        local_infra: &[&str],
        docker: bool,
    ) -> WizardContext {
        WizardContext {
            project_name: Some("myapp".into()),
            project_path: Some("C:\\dev\\myapp".into()),
            languages: languages.iter().map(|s| s.to_string()).collect(),
            backend_languages: backend_langs.iter().map(|s| s.to_string()).collect(),
            frontend_languages: frontend_langs.iter().map(|s| s.to_string()).collect(),
            frameworks: frameworks.iter().map(|s| s.to_string()).collect(),
            tools: tools.iter().map(|s| s.to_string()).collect(),
            local_infra_tools: local_infra.iter().map(|s| s.to_string()).collect(),
            project_type: Some(project_type.to_string()),
            docker,
            // Полная сессия мастера: git и vscode включены явно (default() —
            // всё off).
            git_init: true,
            vscode_config: true,
            ..Default::default()
        }
    }

    fn render(ctx: &WizardContext) -> String {
        generate_readme(&ProjectLayout::compute(ctx), ctx, "myapp")
    }

    #[test]
    fn directory_map_prefixes_manifests_with_language_segment() {
        // В split-раскладке манифесты лежат в сегментах (backend/,
        // frontend/) — включая составную java-строку Maven/Gradle.
        let ctx = ctx_with(
            "web-app",
            &["java", "typescript"],
            &["java"],
            &["typescript"],
            &["spring-boot", "react"],
            &[],
            &[],
            false,
        );
        let md = render(&ctx);
        assert!(
            md.contains("backend/pom.xml / backend/build.gradle.kts"),
            "java-манифесты в сегменте: {md}"
        );
        assert!(md.contains("frontend/package.json"), "{md}");
    }

    #[test]
    fn python_readme_documents_canonical_venv() {
        // Регрессия: README указывал несуществующий `.venv`, literal
        // `{venv}` в командах инструментов и системный `python` вместо
        // интерпретатора venv. Канонический venv — `venv/` (backend/venv в
        // split), команды обязаны работать от корня проекта.
        let python = if cfg!(target_os = "windows") {
            "python"
        } else {
            "python3"
        };
        let root = ctx_with(
            "rest-api",
            &["python"],
            &["python"],
            &[],
            &["fastapi"],
            &["sqlalchemy", "alembic", "ruff", "pytest"],
            &[],
            false,
        );
        let md = render(&root);
        assert!(!md.contains("{venv"), "literal-плейсхолдеры недопустимы: {md}");
        assert!(!md.contains(".venv"), "venv называется venv, а не .venv: {md}");
        assert!(md.contains(&format!("{python} -m venv venv")), "{md}");
        assert!(md.contains("pip install -r requirements.txt"), "{md}");
        assert!(md.contains("-m alembic upgrade head"), "{md}");
        assert!(md.contains("-m uvicorn src.main:app --reload"), "{md}");

        let split = ctx_with(
            "rest-api",
            &["python", "typescript"],
            &["python"],
            &["typescript"],
            &["django", "react"],
            &["alembic", "pytest"],
            &[],
            false,
        );
        let md = render(&split);
        assert!(!md.contains("{venv"), "literal-плейсхолдеры недопустимы: {md}");
        assert!(!md.contains(".venv"), "venv называется venv, а не .venv: {md}");
        assert!(md.contains(&format!("{python} -m venv backend/venv")), "{md}");
        assert!(
            md.contains("pip install -r backend/requirements.txt"),
            "{md}"
        );
        assert!(
            md.contains("cd backend && "),
            "split-команды обязаны выполняться из backend/: {md}"
        );
        assert!(md.contains("manage.py runserver"), "{md}");
    }

    fn tree() -> WizardTreeData {
        let raw = include_str!("../knowledge/wizard_tree.json");
        serde_json::from_str(raw).expect("wizard_tree.json должен парситься")
    }

    fn render_with_tree(tree: &WizardTreeData, ctx: &WizardContext) -> String {
        generate_readme_with_tree(tree, &ProjectLayout::compute(ctx), ctx, "myapp")
    }

    #[test]
    fn aspnetcore_vue_typescript_postgresql_in_docker() {
        // 1. ASP.NET Core + Vue + TypeScript + PostgreSQL (Docker).
        let ctx = ctx_with(
            "web-app",
            &["csharp", "typescript"],
            &["csharp"],
            &["typescript"],
            &["aspnetcore", "vue"],
            &["postgresql"],
            &[],
            true,
        );
        let md = render(&ctx);
        assert!(md.contains("## Architecture"), "{md}");
        assert!(md.contains("separated"), "{md}");
        assert!(md.contains("ASP.NET Core"), "{md}");
        assert!(md.contains("Vue"), "{md}");
        assert!(md.contains("TypeScript"), "{md}");
        assert!(md.contains("PostgreSQL"), "{md}");
        assert!(md.contains("docker compose up -d"), "{md}");
        assert!(md.contains("DATABASE_URL"), "{md}");
        assert!(md.contains("### ASP.NET Core"), "{md}");
        assert!(md.contains("### Vue"), "{md}");
        // Vue — отдельный фронтенд (split), НЕ встроенный в оболочку.
        assert!(!md.contains("standalone website"), "{md}");
        // Детерминированность: тот же контекст → тот же текст.
        assert_eq!(md, render(&ctx));
    }

    #[test]
    fn fastapi_only_with_postgresql_local() {
        // 2. FastAPI only + PostgreSQL локально.
        let ctx = ctx_with(
            "rest-api",
            &["python"],
            &["python"],
            &[],
            &["fastapi"],
            &["postgresql"],
            &["postgresql"],
            true,
        );
        let md = render(&ctx);
        assert!(md.contains("FastAPI"), "{md}");
        assert!(md.contains("backend-only"), "{md}");
        assert!(md.contains("**Runs in.** local install"), "{md}");
        assert!(md.contains("createdb"), "{md}");
        assert!(md.contains("Dockerfile"), "{md}");
        // Локальный постгрес НЕ запускается Docker'ом.
        assert!(!md.contains("docker compose up -d postgres"), "{md}");
        assert!(!md.contains("docker compose up -d"), "{md}");
    }

    #[test]
    fn tauri_svelte_typescript_is_connected() {
        // 3. Tauri + Svelte + TypeScript — встроенный фронтенд.
        let ctx = ctx_with(
            "desktop-app",
            &["typescript", "rust"],
            &["rust"],
            &["typescript"],
            &["tauri", "svelte"],
            &[],
            &[],
            false,
        );
        let md = render(&ctx);
        assert!(md.contains("connected"), "{md}");
        assert!(md.contains("Tauri"), "{md}");
        assert!(md.contains("Svelte"), "{md}");
        assert!(md.contains("src-tauri"), "{md}");
        assert!(md.contains("cargo tauri dev"), "{md}");
        // Svelte — встроенный UI, не самостоятельный сайт.
        assert!(md.contains("standalone website"), "{md}");
        assert!(md.contains("embedded"), "{md}");
    }

    #[test]
    fn electron_react_typescript_is_frontend_only() {
        // 4. Electron + React + TypeScript. Electron — НЕ integrated-оболочка
        // (как tauri): это клиентская оболочка в frontend/, standalone-приложение.
        let ctx = ctx_with(
            "desktop-app",
            &["typescript"],
            &[],
            &["typescript"],
            &["electron", "react"],
            &[],
            &[],
            false,
        );
        let md = render(&ctx);
        assert!(md.contains("Electron"), "{md}");
        assert!(md.contains("React"), "{md}");
        assert!(md.contains("frontend-only"), "{md}");
        assert!(md.contains("client shell"), "{md}");
        assert!(md.contains("npm start"), "{md}");
    }

    #[test]
    fn electron_with_django_is_shell_plus_api() {
        // 4b. Electron + Django: неинтегрированная клиентская оболочка +
        // REST API-бэкенд → "shell + api" (клиент в frontend/, API в backend/).
        let ctx = ctx_with(
            "desktop-app",
            &["typescript", "python"],
            &["python"],
            &["typescript"],
            &["electron", "django"],
            &[],
            &[],
            false,
        );
        let md = render(&ctx);
        assert!(md.contains("Electron"), "{md}");
        assert!(md.contains("Django"), "{md}");
        assert!(md.contains("shell + api"), "{md}");
        assert!(
            md.contains("`frontend/`") && md.contains("`backend/`"),
            "{md}"
        );
        assert!(!md.contains("connected"), "{md}");
    }

    #[test]
    fn qt_webengine_vue_embeds_the_frontend() {
        // 5. Qt WebEngine + Vue.
        let mut ctx = ctx_with(
            "desktop-app",
            &["cpp", "typescript"],
            &["cpp"],
            &["typescript"],
            &["qt", "vue"],
            &[],
            &[],
            false,
        );
        ctx.answers
            .insert("qt_ui".to_string(), vec!["qt-webengine".to_string()]);
        let md = render(&ctx);
        assert!(md.contains("Qt"), "{md}");
        assert!(md.contains("WebEngine"), "{md}");
        assert!(md.contains("Vue"), "{md}");
        assert!(md.contains("standalone website"), "{md}");
        assert!(md.contains("Qt WebEngine mode embeds"), "{md}");
    }

    #[test]
    fn flutter_only() {
        // 6. Flutter only.
        let ctx = ctx_with(
            "mobile-app",
            &["dart"],
            &[],
            &["dart"],
            &["flutter"],
            &[],
            &[],
            false,
        );
        let md = render(&ctx);
        assert!(md.contains("Flutter"), "{md}");
        assert!(md.contains("lib/main.dart"), "{md}");
        assert!(md.contains("flutter run"), "{md}");
        assert!(md.contains("frontend-only"), "{md}");
    }

    #[test]
    fn html_only() {
        // 7. HTML only.
        let ctx = ctx_with("static-html", &["html"], &[], &[], &[], &[], &[], false);
        let md = render(&ctx);
        assert!(md.contains("HTML"), "{md}");
        assert!(md.contains("index.html"), "{md}");
        assert!(md.contains("no build step"), "{md}");
        assert!(md.contains("Docker is **not** configured"), "{md}");
        assert!(md.contains("frontend-only"), "{md}");
        // Пустой стек не падает и без секций фреймворков/инструментов.
        assert!(!md.contains("Frameworks in this project"), "{md}");
        assert!(!md.contains("Database & tool usage"), "{md}");
    }

    #[test]
    fn frontend_only_project_has_no_backend() {
        // 8. Проект без бэкенда.
        let ctx = ctx_with(
            "web-app",
            &["typescript"],
            &[],
            &["typescript"],
            &["vue"],
            &[],
            &[],
            false,
        );
        let md = render(&ctx);
        assert!(md.contains("frontend-only"), "{md}");
        assert!(md.contains("no separate server-side backend"), "{md}");
        assert!(!md.contains("backend-only"), "{md}");
    }

    #[test]
    fn backend_only_project_has_no_frontend() {
        // 9. Проект без фронтенда.
        let ctx = ctx_with(
            "rest-api",
            &["python"],
            &["python"],
            &[],
            &["fastapi"],
            &[],
            &[],
            false,
        );
        let md = render(&ctx);
        assert!(md.contains("backend-only"), "{md}");
        assert!(md.contains("no separate frontend application"), "{md}");
        assert!(!md.contains("frontend-only"), "{md}");
    }

    #[test]
    fn same_tool_local_vs_docker() {
        // 10. Один и тот же инструмент: локально vs в Docker.
        let local = ctx_with(
            "rest-api",
            &["python"],
            &["python"],
            &[],
            &["fastapi"],
            &["postgresql"],
            &["postgresql"],
            true,
        );
        let docker = ctx_with(
            "rest-api",
            &["python"],
            &["python"],
            &[],
            &["fastapi"],
            &["postgresql"],
            &[],
            true,
        );
        let md_local = render(&local);
        let md_docker = render(&docker);

        assert!(
            md_local.contains("**Runs in.** local install"),
            "{md_local}"
        );
        assert!(md_local.contains("createdb"), "{md_local}");
        assert!(!md_local.contains("docker compose up -d"), "{md_local}");

        assert!(md_docker.contains("**Runs in.** Docker"), "{md_docker}");
        assert!(md_docker.contains("docker compose up -d"), "{md_docker}");
        assert!(md_docker.contains("docker compose ps"), "{md_docker}");
        assert!(
            !md_docker.contains("**Runs in.** local install"),
            "{md_docker}"
        );
        // Локальный вариант ссылается на LOCAL_INFRA.md, docker-вариант — нет.
        assert!(md_local.contains("LOCAL_INFRA.md"), "{md_local}");
        assert!(!md_docker.contains("LOCAL_INFRA.md"), "{md_docker}");
    }

    #[test]
    fn readme_with_django_sqlalchemy_documents_decision() {
        // Django + SQLAlchemy: Django предупреждает о втором ORM в tool_warnings.
        let ctx = ctx_with(
            "rest-api",
            &["python"],
            &["python"],
            &[],
            &["django"],
            &["sqlalchemy"],
            &[],
            false,
        );
        let md = render(&ctx);
        // Секция появилась и документирует выбор Django ORM vs SQLAlchemy.
        assert!(md.contains("## Architectural Decisions"), "{md}");
        assert!(md.contains("### Using SQLAlchemy with Django"), "{md}");
        assert!(md.contains("two ORMs"), "{md}");
        assert!(md.contains("Django ORM"), "{md}");
        // Позиция: после Quick start, до первого кода.
        let quick_start = md.find("## Quick start").expect("quick start");
        let decisions = md.find("## Architectural Decisions").expect("decisions");
        let first_code = md
            .find("## Where to write your first code")
            .expect("first code");
        assert!(quick_start < decisions && decisions < first_code, "{md}");
    }

    #[test]
    fn readme_without_decisions_has_no_section() {
        // FastAPI + SQLAlchemy + Alembic — согласованный стек: предупреждений
        // нет, ответственности не пересекаются (orm/migrations по одному).
        let ctx = ctx_with(
            "rest-api",
            &["python"],
            &["python"],
            &[],
            &["fastapi"],
            &["sqlalchemy", "alembic"],
            &[],
            false,
        );
        let md = render(&ctx);
        assert!(!md.contains("## Architectural Decisions"), "{md}");
    }

    #[test]
    fn readme_with_multiple_orm_tools_documents_overlap() {
        // SQLAlchemy + Prisma делят ответственность "orm" — README объясняет
        // пересечение ролей.
        let ctx = ctx_with(
            "rest-api",
            &["python", "typescript"],
            &["python"],
            &["typescript"],
            &[],
            &["sqlalchemy", "prisma"],
            &[],
            false,
        );
        let md = render(&ctx);
        assert!(md.contains("## Architectural Decisions"), "{md}");
        assert!(
            md.contains("### Multiple orm tools: Prisma, SQLAlchemy"),
            "{md}"
        );
        assert!(
            md.contains("multiple tools that provide orm functionality"),
            "{md}"
        );
    }

    #[test]
    fn readme_with_django_alembic_documents_decision() {
        // Django + Alembic: встроенные миграции Django против Alembic.
        let ctx = ctx_with(
            "rest-api",
            &["python"],
            &["python"],
            &[],
            &["django"],
            &["alembic"],
            &[],
            false,
        );
        let md = render(&ctx);
        assert!(md.contains("## Architectural Decisions"), "{md}");
        assert!(md.contains("### Using Alembic with Django"), "{md}");
        assert!(md.contains("built-in migrations"), "{md}");
    }

    #[test]
    fn unknown_warning_pair_falls_back_to_data_text() {
        // Связки, которой нет в авторской английской таблице, документируются
        // текстом предупреждения из данных дерева (fallback).
        use crate::modules::project_creator::models::ToolWarningReason;
        let mut custom_tree = tree();
        custom_tree
            .frameworks
            .iter_mut()
            .find(|f| f.id == "django")
            .expect("django в дереве")
            .tool_warnings
            .insert(
                "redis".to_string(),
                ToolWarningReason {
                    reason: "Custom pairing reason text".to_string(),
                    recommendation: "Custom pairing recommendation text".to_string(),
                },
            );
        let ctx = ctx_with(
            "rest-api",
            &["python"],
            &["python"],
            &[],
            &["django"],
            &["redis"],
            &[],
            false,
        );
        let md = render_with_tree(&custom_tree, &ctx);
        assert!(md.contains("## Architectural Decisions"), "{md}");
        assert!(md.contains("### Using Redis with Django"), "{md}");
        assert!(md.contains("Custom pairing reason text"), "{md}");
        assert!(md.contains("Custom pairing recommendation text"), "{md}");
    }

    #[test]
    fn multiple_warnings_and_overlaps_all_documented() {
        // Django + SQLAlchemy + Prisma: два предупреждения фреймворка +
        // пересечение orm-ответственности — документируется всё.
        let ctx = ctx_with(
            "rest-api",
            &["python", "typescript"],
            &["python"],
            &["typescript"],
            &["django"],
            &["sqlalchemy", "prisma"],
            &[],
            false,
        );
        let md = render(&ctx);
        assert!(md.contains("## Architectural Decisions"), "{md}");
        assert!(md.contains("### Using SQLAlchemy with Django"), "{md}");
        assert!(md.contains("### Using Prisma with Django"), "{md}");
        assert!(
            md.contains("### Multiple orm tools: Prisma, SQLAlchemy"),
            "{md}"
        );
        // Детерминированность сохраняется и с секцией решений.
        assert_eq!(md, render(&ctx), "{md}");
    }

    #[test]
    fn russian_readme_is_localized_via_i18n() {
        // Полный README на русском: секции, подписи и футер берутся из
        // ru-словаря; английские заголовки не протекают.
        let mut ctx = ctx_with(
            "web-app",
            &["csharp", "typescript"],
            &["csharp"],
            &["typescript"],
            &["aspnetcore", "vue"],
            &["postgresql"],
            &[],
            true,
        );
        ctx.readme_locale = Some("ru".to_string());
        let md = render(&ctx);
        for expected in [
            "## Обзор",
            "## Архитектура",
            "## Выбранный стек",
            "## Карта каталогов",
            "## Предпосылки",
            "## Быстрый старт",
            "## Тестирование",
            "## Переменные окружения",
            "## Использование Docker",
            "## Типичные ошибки новичков",
            "## Следующие рекомендуемые шаги",
            "Сгенерировано StackPilot",
        ] {
            assert!(md.contains(expected), "missing {expected:?} in:\n{md}");
        }
        assert!(!md.contains("Architectural Decisions"), "{md}");
        assert!(!md.contains("Quick start"), "{md}");
        assert!(!md.contains("readme."), "raw key leaked: {md}");
    }

    #[test]
    fn readme_has_no_untranslated_keys_across_stacks() {
        // Словари полные: ни в EN-, ни в RU-README не должно остаться
        // неразрешённых i18n-ключей (`readme.` — префикс ключей).
        let stacks: [(&str, &[&str], &[&str], &[&str], &[&str], &[&str]); 8] = [
            (
                "rest-api",
                &["python", "typescript"],
                &["python"],
                &["typescript"],
                &["fastapi", "django", "react"],
                &["postgresql", "sqlalchemy", "alembic", "pytest", "ruff"],
            ),
            (
                "desktop-app",
                &["rust", "typescript"],
                &["rust"],
                &["typescript"],
                &["tauri", "svelte"],
                &["sqlite"],
            ),
            (
                "web-app",
                &["php", "typescript"],
                &["php"],
                &["typescript"],
                &["laravel", "react"],
                &["mysql", "redis", "minio", "grafana"],
            ),
            (
                "rest-api",
                &["java"],
                &["java"],
                &[],
                &["spring-boot"],
                &["maven", "kafka", "clickhouse"],
            ),
            (
                "cli-tool",
                &["go"],
                &["go"],
                &[],
                &["gin", "cobra"],
                &["sqlite"],
            ),
            (
                "rest-api",
                &["elixir"],
                &["elixir"],
                &[],
                &["phoenix"],
                &["postgresql"],
            ),
            (
                "rest-api",
                &["typescript"],
                &["typescript"],
                &["typescript"],
                &["nest", "nextjs"],
                &["prisma", "npm"],
            ),
            (
                "desktop-app",
                &["cpp", "typescript"],
                &["cpp"],
                &["typescript"],
                &["qt", "vue"],
                &["postgresql", "mailpit"],
            ),
        ];
        for (project_type, langs, back, front, fws, tools) in stacks {
            let mut ctx = ctx_with(
                project_type,
                langs,
                back,
                front,
                fws,
                tools,
                &["postgresql", "redis"],
                true,
            );
            for locale in ["en", "ru"] {
                ctx.readme_locale = Some(locale.to_string());
                let md = render(&ctx);
                assert!(
                    !md.contains("readme."),
                    "{locale}/{project_type}: raw key leaked:\n{md}"
                );
            }
        }
    }
}
