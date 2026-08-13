// ============================================================
// Toolchain Manager — модели данных
// ============================================================
// Модуль отвечает за управление локальным окружением разработчика:
// обнаружение ПО, валидация версий, установка, обновление, PATH, health.
//
// Этот файл содержит ТОЛЬКО данные и их формы (struct/enum).
// Никакой логики — вся логика живёт в сервисах (core/, platforms/).
// Благодаря serde эти типы сериализуются в JSON и уезжают на фронтенд
// (тот же подход, что в project_creator::models).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ------------------------------------------------------------
// Статичные определения инструментов (tools.json)
// ------------------------------------------------------------

/// Полное определение инструмента. Загружается из tools.json при старте приложения.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Уникальный id. Там, где возможно, совпадает с id из wizard_tree.json
    /// (postgresql, docker, npm, ...), чтобы маппинг был тривиальным.
    pub id: String,
    /// Категория: language / package_manager / compiler / database /
    /// container / vcs / editor / utility
    pub category: String,
    /// Человекочитаемое имя (Node.js, Docker Desktop, ...)
    pub display: String,
    pub description: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// Правила обнаружения на машине пользователя
    pub detection: DetectionRules,
    /// Минимальная и рекомендуемая версии (advisory, не блокирующие)
    #[serde(default)]
    pub versions: VersionRules,
    /// Источники установки по ОС. Порядок внутри списка = приоритет:
    /// сначала менеджер пакетов, потом официальный установщик.
    #[serde(default)]
    pub sources: InstallSources,
    /// Примерный размер загрузки установщика, МБ (для предупреждений)
    #[serde(default)]
    pub size_mb: u32,
    /// Требуются ли права администратора
    #[serde(default)]
    pub needs_admin: bool,
    /// Каталоги, которые нужно добавить в PATH после установки
    #[serde(default)]
    pub path_entries: Vec<String>,
    /// id инструмента, с которым этот поставляется «в комплекте»
    /// (npm→node, cargo→rust, pip→python). Такие тулы не устанавливаются отдельно.
    #[serde(default)]
    pub bundled_with: Option<String>,
    /// Дополнительные health-проверки (помимо версии)
    #[serde(default)]
    pub health_checks: Vec<HealthCheck>,
    /// Заметки пользователю (WSL2 для Docker, лицензии и т.п.)
    #[serde(default)]
    pub notes: Option<String>,
    /// Если задано — инструмент НЕ устанавливается автоматически
    /// (движки, SDK и т.п.), а в отчёте показывается честное предупреждение
    /// о ручной установке. Значение — текст предупреждения пользователю.
    #[serde(default)]
    pub manual_install: Option<String>,
}

impl ToolDefinition {
    /// Есть ли хоть один источник установки.
    /// false = информационный тул (curl, tar, sqlite) — он не устанавливается,
    /// только проверяется, и никогда не блокирует создание проекта.
    pub fn installable(&self) -> bool {
        !self.sources.windows.is_empty()
            || !self.sources.linux.is_empty()
            || !self.sources.macos.is_empty()
    }
}

/// Правила обнаружения инструмента на машине.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionRules {
    /// Пробы версии: список команд, каждая — [бинарник, аргументы...].
    /// Пробуются по очереди, пока одна не вернёт ответ
    /// (например python на Linux может быть только python3).
    #[serde(default)]
    pub version_probes: Vec<Vec<String>>,
    /// Типовые пути установки (проверяются на существование).
    #[serde(default)]
    pub known_paths: Vec<String>,
    /// Ключи реестра Windows (reg query), проверяются на существование.
    #[serde(default)]
    pub registry_keys: Vec<String>,
}

/// Advisory-версии. min — «ниже этого работа проекта не гарантирована»,
/// recommended — «рекомендуем обновиться».
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VersionRules {
    #[serde(default)]
    pub min: Option<String>,
    #[serde(default)]
    pub recommended: Option<String>,
}

/// Наборы источников установки по ОС.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallSources {
    #[serde(default)]
    pub windows: Vec<InstallSource>,
    #[serde(default)]
    pub linux: Vec<InstallSource>,
    #[serde(default)]
    pub macos: Vec<InstallSource>,
}

/// Один источник установки инструмента.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallSource {
    pub kind: InstallSourceKind,
    /// id пакета в менеджере пакетов (OpenJS.NodeJS.LTS) или имя источника
    pub id: String,
    /// Прямая ссылка для Official/Script источников
    #[serde(default)]
    pub url: Option<String>,
    /// Имя файла, под которым сохраняется скачанный установщик.
    /// Нужно для программ, которые определяют своё поведение по имени
    /// файла: rustup-init.exe должен остаться rustup-init.exe, иначе он
    /// решит, что его вызвали как «прокси» (unknown proxy name).
    #[serde(default)]
    pub file_name: Option<String>,
    /// Статичные аргументы тихой установки (winget: --silent; MSI: /quiet)
    #[serde(default)]
    pub args: Vec<String>,
    /// Дополнительные аргументы менеджера пакетов (winget --override ...)
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// true — аргументы дополняются кодом на этапе установки
    /// (пароль PostgreSQL генерируется в рантайме, его нельзя хранить в JSON)
    #[serde(default)]
    pub dynamic_args: bool,
    /// Куда распаковывать zip-архив (gradle, maven) или клонировать
    /// git-репозиторий. Без этого поля zip-источник считается
    /// некорректным и не запускается.
    #[serde(default)]
    pub install_dir: Option<String>,
    /// Переопределение needs_admin для конкретного источника (zip-распаковка
    /// не требует UAC, даже если у инструмента в целом needs_admin=true).
    #[serde(default)]
    pub needs_admin: Option<bool>,
    /// Способ исполнения источника (см. ExecutionKind). По умолчанию
    /// (None) движок определяет его автоматически: по URL (.git →
    /// git clone) и расширению скачанного файла (.phar → php,
    /// .ps1/.sh → интерпретатор, .zip/.tgz → распаковка, иначе exe).
    /// Явное значение нужно для файлов без расширения (composer-installer)
    /// и для жёсткой гарантии поведения.
    #[serde(default)]
    pub execution: Option<ExecutionKind>,
}

/// Способ исполнения скачанного источника: чем движок «запускает» файл.
/// Поле InstallSource.execution; Auto — автоматическое определение
/// (правила в installer.rs::resolve_execution).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    /// Git-репозиторий: `git clone <url> <каталог>`. Файл НЕ скачивается
    /// и НЕ запускается — иначе «%1 не является приложением Win32»
    /// (os error 193), как было с flutter.git.
    GitClone,
    /// Бинарь/инсталлятор (.exe, .msi, .msix): запускается напрямую
    /// (msi/msix — через свои механизмы).
    Exe,
    /// Скрипт интерпретатора: .ps1 → `powershell -File`, .sh → `bash`.
    Script,
    /// PHP-скрипт (.phar — composer и т.п.): `php <файл>` — CreateProcess
    /// phar не понимает (os error 193).
    Phar,
    /// Архив (.zip/.tgz): распаковывается в install_dir, а не запускается.
    Archive,
    /// Автоопределение по URL/расширению (значение по умолчанию).
    Auto,
}

/// Чем ставим: менеджером пакетов ОС, официальным установщиком или скриптом.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InstallSourceKind {
    PkgManager,
    Official,
    Script,
    /// Прямая установка из официального online-репозитория (Qt).
    /// url — базовый каталог репозитория (например
    /// https://download.qt.io/online/qtsdkrepository/windows_x86);
    /// install_dir — корень, куда складывается Qt (обычно
    /// %LOCALAPPDATA%/Programs/Qt). Пакеты, версии и каталоги
    /// извлечения разбираются из Updates.xml репозитория.
    QtOnline,
}

/// Дополнительная health-проверка: команда должна выполниться успешно.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub label: String,
    pub command: Vec<String>,
}

// ------------------------------------------------------------
// Рантайм-модели: что происходит на машине пользователя
// ------------------------------------------------------------

/// Статус инструмента по результатам обнаружения.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolStatus {
    /// Инструмент не найден
    Missing,
    /// Установлен, версия устраивает
    Installed { version: String },
    /// Установлен, но вышла рекомендуемая версия
    UpdateAvailable { installed: String, recommended: String },
    /// Найден, но не работает (бинарь не в PATH, сломанная установка)
    PathBroken { reason: String },
    /// Инструмент не установлен и ставится ТОЛЬКО вручную (движки, SDK).
    /// Честное предупреждение в отчёте: не блокирует создание проекта,
    /// не участвует в установке. Причина — текст для пользователя.
    ManualInstall { reason: String },
    /// «Двойной» инструмент мастера (postgresql, mongodb, kafka, ...):
    /// по умолчанию разворачивается docker-compose.yaml проекта, локально
    /// не устанавливается. Показывается в ОПЦИОНАЛЬНОЙ секции отчёта с
    /// предложением поставить локально; в план установки не попадает.
    RunInDocker,
}

impl ToolStatus {
    /// «Всё в порядке, можно работать»?
    pub fn is_ok(&self) -> bool {
        matches!(self, ToolStatus::Installed { .. })
    }
}

/// Информация об ОС и менеджерах пакетов (для фронтенда и логов).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentInfo {
    pub os: String,
    /// Версия ОС — наполняется на этапе 5 (команда ОС)
    pub os_version: String,
    pub package_managers: Vec<String>,
    /// Сколько инструментов знает Toolchain Manager
    pub tool_count: usize,
}

/// Требования проекта к окружению — входной контракт tc_check_environment.
/// Фронтенд собирает его из WizardContext проекта (языки, фреймворки, тулы,
/// флаги git/vscode/docker) и присылает в toolchain.
///
/// Намеренно НЕ зависит от project_creator::models::WizardContext:
/// модули остаются независимыми, интеграция идёт только через фронтенд.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectRequirements {
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub frameworks: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    /// Docker-инструменты мастера (postgresql, redis, mongodb, kafka,
    /// grafana, mysql), которые пользователь решил ставить ЛОКАЛЬНО вместо
    /// docker-compose. Попадают в обычные requirements (как Missing и т.п.),
    /// а из docker-compose.yaml проекта исключаются (см. WizardContext.local_infra_tools).
    #[serde(default)]
    pub local_infra_tools: Vec<String>,
    #[serde(default)]
    pub git_init: bool,
    #[serde(default)]
    pub vscode_config: bool,
    #[serde(default)]
    pub docker: bool,
}

/// Требование проекта к окружению — по одному экземпляру на инструмент.
/// Из этого собирается экран проверки зависимостей.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRequirement {
    pub tool_id: String,
    pub display: String,
    pub category: String,
    /// Имя файла иконки (static/images/), из tools.json. Фронтенд
    /// рендерит /images/<icon>, отсутствующий файл → дефолтная SVG.
    #[serde(default)]
    pub icon: Option<String>,
    pub status: ToolStatus,
    /// Сколько МБ скачаем, если нужна установка
    pub size_mb: u32,
    pub needs_admin: bool,
    /// Человекочитаемое описание источника (например «winget: Git.Git»)
    pub source_description: String,
    /// Модули/компоненты установки (для Qt: qt-qml, qt-webengine, ...).
    /// Заполняется из выбора пользователя в мастере, переносится
    /// в InstallTask и передаётся установщику.
    #[serde(default)]
    pub install_options: Vec<String>,
}

/// Полный отчёт проверки окружения под конкретный проект.
/// Это ответ команды tc_check_environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentCheck {
    pub os: String,
    pub requirements: Vec<ToolRequirement>,
    /// «Опциональные» требования: docker-инструменты мастера (см.
    /// requirements::docker_optional_requirements), которые по умолчанию
    /// разворачиваются контейнерами проекта. Показываются отдельной
    /// секцией экрана окружения с возможностью переключиться на локальную
    /// установку (тогда они уезжают в requirements).
    #[serde(default)]
    pub optional_requirements: Vec<ToolRequirement>,
    /// Суммарный размер загрузки, МБ
    pub total_size_mb: u64,
    /// Свободное место на целевом диске, МБ (0 = ещё не проверялось)
    pub free_space_mb: u64,
    pub enough_space: bool,
    pub needs_admin_any: bool,
    /// true = всё установлено и можно создавать проект
    pub all_ready: bool,
}

/// Задача установки одного инструмента. Задачи собираются в InstallPlan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallTask {
    pub task_id: String,
    pub tool_id: String,
    pub display: String,
    /// Имя файла иконки (static/images/). Копируется из ToolRequirement.
    #[serde(default)]
    pub icon: Option<String>,
    pub size_mb: u32,
    pub needs_admin: bool,
    pub source_description: String,
    /// Модули/компоненты установки (Qt: qt-qml/qt-widgets/qt-webengine/...),
    /// переносятся из ToolRequirement без изменений.
    #[serde(default)]
    pub install_options: Vec<String>,
    pub state: TaskState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskState {
    Pending,
    Running { phase: TaskPhase },
    Success { version: String },
    Failed { error: String },
    Skipped { reason: String },
}

/// Фаза установки — на фронтенде показывается как подпрогресс задачи.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskPhase {
    Downloading,
    Installing,
    Verifying,
    UpdatingPath,
}

/// План установки: упорядоченный список задач.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallPlan {
    pub tasks: Vec<InstallTask>,
    pub total_size_mb: u64,
    pub os: String,
}

/// Живая сессия установки — состояние для tc_get_install_status.
/// Хранится в ToolchainState, пока идёт/завершилась установка.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallSession {
    pub started_at: String,
    /// false = установка завершена (или ещё не начиналась)
    pub running: bool,
    pub plan: InstallPlan,
    /// Сгенерированные при установке секреты (пароль PostgreSQL и т.п.).
    /// Сохраняются в state.json на этапе 5, в проект не попадают.
    #[serde(default)]
    pub secrets: HashMap<String, String>,
}

/// Событие установки — стримится на фронтенд через tauri events
/// (тот же механизм, что project_creator:step_event).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolchainEvent {
    pub event_type: ToolchainEventType,
    pub task_index: usize,
    pub total_tasks: usize,
    pub task_id: String,
    pub tool_id: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolchainEventType {
    TaskStarted,
    TaskPhaseChanged { phase: TaskPhase },
    /// Строка вывода установщика
    TaskProgress { line: String },
    TaskCompleted { state: TaskState },
    AllCompleted { success_count: usize, failed: Vec<String> },
    Error { message: String },
}

/// Промежуточный прогресс проверки окружения. Стримится на фронтенд
/// событием `toolchain:check_progress` после каждого проверенного
/// инструмента — пользователь видит, что проверка идёт, а не висит.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckProgressEvent {
    /// Сколько инструментов уже проверено (включая этот)
    pub done: usize,
    /// Сколько всего проверяется
    pub total: usize,
    pub tool_id: String,
    pub display: String,
    /// Имя файла иконки (static/images/), из tools.json.
    #[serde(default)]
    pub icon: Option<String>,
    pub status: ToolStatus,
}

// ------------------------------------------------------------
// Health
// ------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckResult {
    pub label: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolHealth {
    pub tool_id: String,
    pub display: String,
    /// Имя файла иконки (static/images/), из tools.json.
    #[serde(default)]
    pub icon: Option<String>,
    pub checks: Vec<HealthCheckResult>,
    pub ok: bool,
}

/// Полный health-отчёт по окружению (для страницы окружения).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub tools: Vec<ToolHealth>,
    /// Процент здоровья 0..100
    pub score: u8,
    pub scanned_at: String,
}

// ------------------------------------------------------------
// Metadata — локальное хранилище состояния (state.json)
// ------------------------------------------------------------

/// Состояние Toolchain Manager, сохраняется в app_data_dir/toolchain/state.json.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolchainMetadata {
    #[serde(default)]
    pub last_scan: Option<String>,
    #[serde(default)]
    pub tools: HashMap<String, InstalledToolInfo>,
    /// Секреты (пароль PostgreSQL и т.п.). Хранятся локально,
    /// в проект никогда не попадают.
    #[serde(default)]
    pub secrets: HashMap<String, String>,
    #[serde(default)]
    pub prefs: HashMap<String, String>,
}

/// Информация об установленном инструменте, известная Toolchain Manager.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledToolInfo {
    pub path: String,
    pub version: String,
    pub installed_at: String,
    /// Каталоги, добавленные в PATH этим инструментом
    #[serde(default)]
    pub path_entries: Vec<String>,
}
