use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ============================================================
// Wizard Tree — гибкое дерево решений
// ============================================================

/// Полный набор данных для мастера (загружается из JSON)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WizardTreeData {
    /// Легальные связки: пары «главных» фреймворков одной стороны, которым
    /// разрешено сосуществовать (gin+cobra, axum+clap, android+jetpack-compose...).
    /// Двигатель генерации гарантирует им непересекающиеся файлы.
    #[serde(default)]
    pub allowed_main_pairs: Vec<Vec<String>>,
    /// Фреймворки, которые не занимают лимит «одного главного на сторону»
    /// (zig-cli: std-CLI не является каркасом приложения).
    #[serde(default)]
    pub main_limit_exempt: Vec<String>,
    /// Нежёсткие предупреждения для сочетаний, которые «не ломают», но
    /// противоречат концепции (Phoenix LiveView + тяжёлый SPA). Warning.
    #[serde(default)]
    pub warning_pairs: Vec<WarningPair>,
    pub project_types: Vec<ProjectTypeDef>,
    pub languages: Vec<LanguageDef>,
    pub frameworks: Vec<FrameworkDef>,
    pub tools: Vec<ToolDef>,
    /// Готовые рецепты для вкладки «Шаблоны» — каждая комбинация проходит
    /// каноническую валидацию (см. validate.rs: every_preset_valid).
    #[serde(default)]
    pub presets: Vec<ProjectPreset>,
    pub project_language_map: std::collections::HashMap<String, Vec<String>>,
    pub language_framework_map: std::collections::HashMap<String, Vec<String>>,
    pub framework_tool_map: std::collections::HashMap<String, Vec<String>>,
    /// Инструменты, рекомендованные для типа проекта (data-pipeline → airflow,
    /// kafka, clickhouse...). Используется рекомендациями конструктора.
    #[serde(default)]
    pub project_tool_map: std::collections::HashMap<String, Vec<String>>,
    /// Инструменты, рекомендованные для языка (python → pytest, ruff...).
    #[serde(default)]
    pub language_tool_map: std::collections::HashMap<String, Vec<String>>,
}

/// Тип проекта (REST API, Desktop App, CLI Tool...)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectTypeDef {
    pub id: String,
    pub label: String,
    pub description: String,
    pub icon: Option<String>,
    pub tags: Vec<String>,
    pub allow_custom_stack: bool,
    /// Есть ли у типа проекта серверная сторона. false (browser-extension) —
    /// фронтенд скрывает шаги «Backend Language» и «Backend Framework».
    #[serde(default = "default_has_backend")]
    pub has_backend: bool,
}

fn default_has_backend() -> bool {
    true
}

/// Пара-предупреждение: совместный выбор `a` и `b` не блокируется, но
/// помечается Warning с объяснением и рекомендацией альтернативы.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarningPair {
    pub a: String,
    pub b: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub alternative: String,
}

/// Язык программирования
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageDef {
    pub id: String,
    pub label: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    #[serde(default)]
    pub category: Option<String>, // "backend", "frontend", "static", или null/None
    pub knowledge_key: Option<String>,
    /// ОС, на которых язык доступен. Пустой список = все ОС.
    #[serde(default)]
    pub platforms: Vec<String>,
}

/// Фреймворк / библиотека
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameworkDef {
    pub id: String,
    pub label: String,
    pub description: String,
    pub icon: Option<String>,
    /// Роль фреймворка в конструкторе:
    ///   - "standalone" — сам создаёт полное приложение (nextjs, django,
    ///     flutter, tauri...). На каждую сторону проекта — не более одного.
    ///   - "inplace" — мини-каркас, дописывающий файлы в проект языка
    ///     (fastapi, express, gin, clap...). На каждую сторону — не более
    ///     одного inplace-фреймворка.
    #[serde(default)]
    pub class: String,
    /// Уровень фреймворка в иерархии конструктора:
    ///   - "app"  — главный: создаёт каркас приложения/сервера/клиента
    ///     (django, nest, nextjs, tauri...). На одну сторону — не более
    ///     одного app-фреймворка (кроме side="either" — универсальные).
    ///   - "side" — побочный: дописывается к главному и не конфликтует
    ///     (aiogram, telegraf). Можно несколько, ограничения только из
    ///     conflicts.
    #[serde(default)]
    pub kind: String,
    /// Фреймворки, которые мастер подсветит как рекомендованные, когда
    /// выбран этот (например nest → react). Чисто рекомендации — никак
    /// не ограничивают выбор.
    #[serde(default)]
    pub recommends: Vec<FrameworkRecommendation>,
    /// На какой стороне живёт фреймворк:
    ///   - "backend" — требует язык именно бэкенд-стороны;
    ///   - "frontend" — требует язык фронтенд-стороны;
    ///   - "either" — язык может находиться на любой стороне.
    #[serde(default)]
    pub side: String,
    /// Языки, совместимые с фреймворком (эквивалент старого requires_language,
    /// но привязан к стороне через `side`).
    #[serde(default)]
    pub languages: Vec<String>,
    /// Язык, который конструктор подставит автоматически (ровно один,
    /// обязательно из `languages`; проверяется тестами).
    #[serde(default)]
    pub recommended_language: String,
    /// Типы проектов, для которых фреймворк доступен. Пустой список = везде.
    #[serde(default)]
    pub project_types: Vec<String>,
    /// ОС, на которых фреймворк доступен. Пустой список = все ОС.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Как фреймворк участвует в создании каркаса проекта:
    ///   - "root"  — сам создаёт проект в корне (tauri, spring-boot, django);
    ///   - "subdir" — сам создаёт полный проект в подпапке <project_name>
    ///     (nextjs, flutter, electron...);
    ///   - None — «микро»: только дописывает файлы в существующий проект
    ///     (fastapi, express, axum...).
    /// На проект может быть не более одного "root" и не более одного "subdir":
    /// два таких фреймворка перезапишут/создадут одну и ту же структуру.
    #[serde(default)]
    pub scaffold: Option<String>,
    /// Куда фреймворк создаёт файлы при сегментации проекта:
    ///   - "backend" — в папку backend/
    ///   - "frontend" — в папку frontend/
    ///   - None — корень проекта (по умолчанию)
    /// Применяется к фреймворкам с scaffold="subdir" или inplace-фреймворкам.
    #[serde(default)]
    pub output_subdir: Option<String>,
    /// Системные инструменты, без которых фреймворк нельзя собрать/запустить
    /// независимо от выбора пользователя (npm для JS/TS-фреймворков,
    /// maven/gradle для JVM). В отличие от framework_tool_map (тулы, которые
    /// мастер лишь ПРЕДЛАГАЕТ выбрать), это безусловные требования окружения.
    #[serde(default)]
    pub required_tools: Vec<String>,
    /// Фреймворк сам создаёт полный каркас проекта для своих
    /// languages языков (dotnet new webapi, create-next-app и т.п.) —
    /// generic-скаффолд языка не нужен и конфликтует с ним.
    #[serde(default)]
    pub suppresses_language_scaffold: bool,
    pub knowledge_key: Option<String>,
    #[serde(default)]
    pub conflicts: Vec<String>, // id фреймворков, с которыми несовместим
    /// Человеческое объяснение, почему фреймворк несовместим с конкретным
    /// конфликтом (conflict_id → текст). Показывается в UI и валидации.
    #[serde(default)]
    pub conflict_notes: std::collections::HashMap<String, String>,
    /// Фреймворки, которые можно выбрать как UI-компаньона (tauri → svelte/
    /// vue/react, electron → react/vue/svelte). Связка обязана быть в
    /// allowed_main_pairs и не иметь взаимных conflicts.
    #[serde(default)]
    pub companions: Vec<String>,
    /// Технологии UI внутри фреймворка с собственным стеком (Qt → QML,
    /// Widgets, WebEngine, Kirigami). id совпадает с id фреймворка-варианта
    /// (qt-qml и т.п.); варианты живут в companions и рендерятся мастером
    /// в попапе владельца, а не как самостоятельные карточки.
    #[serde(default)]
    pub qt_ui_options: Vec<QtUiOption>,
}

/// Технология UI внутри фреймворка с собственным стеком (Qt).
/// `id` — id фреймворка-варианта (qt-qml/qt-widgets/qt-webengine/qt-kirigami);
/// `web_framework_options` — веб-фреймворки, которые режим умеет встраивать
/// (Qt WebEngine → react/vue/svelte).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QtUiOption {
    pub id: String,
    pub label: String,
    pub description: String,
    pub icon: Option<String>,
    #[serde(default)]
    pub web_framework_options: Vec<String>,
}

/// Рекомендуемый «компаньон»: фреймворк, который стоит подсветить,
/// когда выбран родительский. Поле `note` — объяснение «зачем» для UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameworkRecommendation {
    pub framework: String,
    #[serde(default)]
    pub note: String,
}

/// Инструмент (БД, кеш, CI, тесты...)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub id: String,
    pub label: String,
    pub description: String,
    pub icon: Option<String>,
    pub category: String,
    pub knowledge_key: Option<String>,
    #[serde(default)]
    pub requires_docker: bool,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub conflicts: Vec<String>,
    /// ОС, на которых инструмент доступен. Пусто = все.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Языки, с которыми инструмент сочетается (npm — только JS-стек).
    /// Пусто = для любых языков.
    #[serde(default)]
    pub for_languages: Vec<String>,
    /// Типы проектов, для которых инструмент уместен. Пусто = везде.
    #[serde(default)]
    pub for_project_types: Vec<String>,
}

/// Готовый рецепт для вкладки «Шаблоны». Обязан проходить каноническую
/// валидацию (validate.rs), иначе не попадёт в UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectPreset {
    pub id: String,
    pub label: String,
    pub description: String,
    pub icon: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub stack: PresetStack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetStack {
    pub project_type: String,
    pub backend_lang: Option<String>,
    pub frontend_lang: Option<String>,
    #[serde(default)]
    pub frameworks: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    pub features: PresetFeatures,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetFeatures {
    #[serde(default)]
    pub testing: bool,
    #[serde(default)]
    pub git: bool,
    #[serde(default)]
    pub vscode: bool,
    #[serde(default)]
    pub docker: bool,
}

// Устаревшие типы — будут удалены после миграции
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub id: String,
    pub label: String,
    pub description: String,
    pub icon: Option<PathBuf>,
    pub tags: Vec<String>,
    pub children: Vec<SubCategory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubCategory {
    pub id: String,
    pub label: String,
    pub description: String,
    pub icon: Option<PathBuf>,
    pub tags: Vec<String>,
    pub knowledge_key: Option<String>,
}

// ============================================================
// Wizard — дерево решений
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WizardQuestion {
    pub id: String,
    pub label: String,
    pub description: String,
    pub question_type: QuestionType,
    pub condition: Option<WizardCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QuestionType {
    SingleChoice {
        options: Vec<ChoiceOption>,
    },
    MultiChoice {
        options: Vec<ChoiceOption>,
    },
    Confirm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub id: String,
    pub label: String,
    pub description: String,
    pub tags: Vec<String>,
    pub knowledge_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WizardCondition {
    AnswerEquals { question_id: String, value: String },
    AnswerContains { question_id: String, values: Vec<String> },
    TechnologyDetected { technology: String },
    TechnologyNotDetected { technology: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WizardContext {
    pub project_path: Option<PathBuf>,
    /// Пользовательское имя проекта (для package.json, README и т.д.).
    /// Может отличаться от имени папки на диске (см. conflictResolvedFolder).
    #[serde(default)]
    pub project_name: Option<String>,
    pub is_existing: bool,
    /// ID типа проекта (rest-api, desktop-app...)
    pub project_type: Option<String>,
    /// Выбранные языки
    pub languages: Vec<String>,
    /// Языки, назначенные пользователем серверной стороне (шаг «Backend»).
    /// Движок использует их для сегментации backend/ frontend/; когда список
    /// пуст (старые сессии), сторона выводится из category языка.
    #[serde(default)]
    pub backend_languages: Vec<String>,
    /// Языки, назначенные пользователем клиентской стороне (шаг «Frontend»).
    #[serde(default)]
    pub frontend_languages: Vec<String>,
    /// Выбранные фреймворки (language_id → framework_id)
    pub frameworks: Vec<String>,
    /// Выбранные инструменты
    pub tools: Vec<String>,
    /// Docker-инструменты мастера (postgresql, redis, mongodb, ...), выбранные
    /// пользователем для ЛОКАЛЬНОЙ установки вместо docker-compose.
    /// По умолчанию такие инструменты разворачиваются контейнерами; когда
    /// тул попадает в этот список, он исключается из docker-compose.yaml,
    /// а в .env.example и LOCAL_INFRA.md собираются локальные настройки.
    #[serde(default)]
    pub local_infra_tools: Vec<String>,
    /// Включённые фичи
    pub features: Vec<String>,
    /// Инфраструктурные компоненты
    pub infrastructure: Vec<String>,
    pub docker: bool,
    pub testing: bool,
    pub ci: bool,
    pub git_init: bool,
    pub vscode_config: bool,
    pub answers: std::collections::HashMap<String, Vec<String>>,
}

impl Default for WizardContext {
    fn default() -> Self {
        Self {
            project_path: None,
            project_name: None,
            is_existing: false,
            project_type: None,
            languages: Vec::new(),
            backend_languages: Vec::new(),
            frontend_languages: Vec::new(),
            frameworks: Vec::new(),
            tools: Vec::new(),
            local_infra_tools: Vec::new(),
            features: Vec::new(),
            infrastructure: Vec::new(),
            docker: true,
            testing: true,
            ci: false,
            git_init: true,
            vscode_config: true,
            answers: std::collections::HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WizardSession {
    pub current_step: usize,
    pub total_steps: usize,
    pub context: WizardContext,
    pub questions: Vec<WizardQuestion>,
    pub is_complete: bool,
}

// ============================================================
// Analysis — анализ существующего проекта
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub project_path: PathBuf,
    pub detected_technologies: Vec<DetectedTechnology>,
    pub existing_configs: Vec<String>,
    pub missing_configs: Vec<String>,
    pub has_docker: bool,
    pub has_git: bool,
    pub has_ci: bool,
    pub has_tests: bool,
    pub has_readme: bool,
    pub has_license: bool,
    pub project_type_hints: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedTechnology {
    pub name: String,
    pub version: Option<String>,
    pub confidence: DetectionConfidence,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DetectionConfidence {
    Certain,
    Likely,
    Possible,
}

// ============================================================
// Recipe Engine — рецепты
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub steps: Vec<Step>,
}

/// Один интерактивный ответ: обнаружли триггер в выводе → отправили response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractiveEntry {
    /// Подстрока для поиска в rolling‑буфере stdout (регистронезависимо)
    pub trigger: String,
    /// Что и как отправить в stdin процесса
    pub response_type: ResponseType,
}

/// Способ ответа на интерактивный запрос CLI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResponseType {
    /// Просто текст + enter (например "TypeScript / JavaScript\n")
    Text(String),
    /// Подтвердить / отказаться (y / n)
    Confirm(bool),
    /// Выбрать N-ый пункт в списке стрелками (0‑индексация)
    Select(usize),
    /// Произвольная последовательность байт (ANSI‑escape и т.п.)
    Keys(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Step {
    Command {
        id: String,
        label: String,
        description: String,
        command: String,
        args: Vec<String>,
        working_dir: Option<String>,
        env: Option<HashMap<String, String>>,
        timeout_secs: Option<u64>,
        condition: Option<StepCondition>,
        on_error: ErrorMode,
        /// Список ожидаемых интерактивных запросов и ответов на них
        #[serde(default)]
        interactive: Vec<InteractiveEntry>,
    },
    WriteFile {
        id: String,
        label: String,
        description: String,
        path: String,
        content: String,
        overwrite: bool,
        condition: Option<StepCondition>,
        on_error: ErrorMode,
    },
    // RenderTemplate {
    //     id: String,
    //     label: String,
    //     description: String,
    //     path: String,
    //     template: String,
    //     context: HashMap<String, String>,
    //     overwrite: bool,
    //     condition: Option<StepCondition>,
    //     on_error: ErrorMode,
    // },
    CreateDirectory {
        id: String,
        label: String,
        description: String,
        path: String,
        condition: Option<StepCondition>,
        on_error: ErrorMode,
    },
    Generate {
        id: String,
        label: String,
        description: String,
        generator_id: String,
        generator_config: serde_json::Value,
        condition: Option<StepCondition>,
        on_error: ErrorMode,
    },
    Parallel {
        id: String,
        label: String,
        description: String,
        steps: Vec<Step>,
        condition: Option<StepCondition>,
        on_error: ErrorMode,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepCondition {
    Always,
    ContextHas { key: String, value: String },
    ContextMissing { key: String },
    FileExists { path: String },
    FileNotExists { path: String },
    TechnologyDetected { name: String },
    TechnologyNotDetected { name: String },
    FeatureEnabled { feature: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ErrorMode {
    Abort,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepPreview {
    pub id: String,
    pub label: String,
    pub description: String,
    pub action: String,
    pub will_execute: bool,
    pub skip_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipePreview {
    pub recipe_id: String,
    pub recipe_name: String,
    pub step_previews: Vec<StepPreview>,
    pub total_steps: usize,
    pub will_execute_count: usize,
    pub will_skip_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_id: String,
    pub label: String,
    pub status: StepStatus,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepStatus {
    Pending,
    Running,
    Success { message: String },
    Skipped { reason: String },
    Failed { error: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OverallStatus {
    Success,
    PartialFailure { failed_steps: Vec<String> },
    Aborted { last_step: Option<String>, reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub recipe_id: String,
    pub total_duration_ms: u64,
    pub step_results: Vec<StepResult>,
    pub overall: OverallStatus,
}

/// План выполнения — конкретный список шагов для конкретного проекта
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub recipe: Recipe,
    pub context: WizardContext,
    pub project_path: PathBuf,
    /// «Развёрнутые» шаги (после раскрытия Parallel, после фильтрации по condition)
    pub steps: Vec<Step>,
}

impl ExecutionPlan {
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }
}

/// Событие выполнения шага — стримится на фронтенд через Tauri events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionEvent {
    pub event_type: ExecutionEventType,
    pub step_id: String,
    pub step_index: usize,
    pub total_steps: usize,
    pub step_name: String,
    pub step_description: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutionEventType {
    StepStarted,
    StepProgress { stdout: String, stderr: String },
    StepCompleted { status: StepStatus, duration_ms: u64 },
    AllCompleted { result: ExecutionResult },
    Error { message: String },
}

/// Снимок выполнения проекта для восстановления вкладки Create после
/// переключения маршрутов: работает ли выполнение и буфер его событий.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionSnapshot {
    pub running: bool,
    pub events: Vec<ExecutionEvent>,
}

// ============================================================
// Generators — генераторы
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationReport {
    pub created_files: Vec<String>,
    pub modified_files: Vec<String>,
    pub skipped_files: Vec<String>,
    pub message: String,
}

// ============================================================
// Packs — функциональные и инфраструктурные пакеты
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: PackKind,
    pub tags: Vec<String>,
    pub dependencies: Vec<String>,
    pub knowledge_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PackKind {
    Feature,
    Infrastructure,
    Tooling,
}

// ============================================================
// Knowledge — встроенная база знаний
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub key: String,
    pub title: String,
    pub content: String,
    pub source: Option<String>,
}
