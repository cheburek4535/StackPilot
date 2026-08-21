// ============================================================
// Domain-модели движка чтения и диагностики (domain/models.rs)
// ============================================================
// Целевая модель данных «Toolchain Control Center» по контракту
// (docs/toolchain-contract.md §2–§3): независимые размерности
// (обнаружение / здоровье / версия / платформа / происхождение) и
// производимое из них презентационное состояние.
//
// Ключевые правила, зашитые в типы:
//   - скан ТОЛЬКО читает машину; снапшот — единственный результат;
//   - scan-failed ≠ missing: ошибка опроса не выдаётся за отсутствие;
//   - отсутствие проверок здоровья — «не проверяли», а не «нездоров»;
//   - события без идентичности операции (job/scan id) не существуют;
//   - кэш честно помечается устаревшим (stale), никогда не притворяется
//     живыми данными.

use serde::{Deserialize, Serialize};

// ------------------------------------------------------------
// Презентационное состояние инструмента (контракт §3)
// ------------------------------------------------------------

/// Состояние инструмента после скана. Это КОМПОЗИЦИЯ независимых
/// размерностей (см. поля ToolScanResult), а не замена им.
///
/// Сериализация: ВСЕ перечисления домена с данными используют ОДИН
/// внутренне-тегированный формат {"kind": "...", ...поля} в snake_case —
/// фронтенд различает вариант по полю kind и никогда не гадает,
/// строка пришла или объект (контракт §10 сериализации).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolState {
    /// Скан для инструмента ещё не выполнялся/не завершён.
    ScanPending,
    /// Опрос завершился ошибкой или таймаутом — результат «неизвестно».
    /// Явно НЕ «отсутствует»: missing здесь запрещён контрактом.
    ScanFailed { reason: String },
    /// Не найден на машине; на этой платформе устанавливаемо.
    Missing,
    /// Найден, проверки здоровья прошли, версия ≥ рекомендуемой.
    InstalledHealthy { version: String },
    /// Найден, но проверок здоровья в каталоге нет («не знаем»).
    InstalledHealthUnknown { version: String },
    /// Найден, но хотя бы одна проверка здоровья упала.
    InstalledUnhealthy { version: String },
    /// Работает, но ниже рекомендуемой версии (деградация, не поломка).
    UpdateAvailable {
        installed: String,
        recommended: String,
    },
    /// Установка найдена, но бинарь не отвечает через PATH.
    PathBroken { reason: String },
    /// Ставится только вручную (движки, SDK). Причина — текст инструкции.
    ManualInstall { reason: String },
    /// «Двойной» docker-инструмент мастера: по умолчанию живёт в
    /// docker-compose проекта, локальная установка — opt-in.
    DockerManaged,
    /// Предоставляется ОС (curl, tar); отдельная установка не нужна.
    BuiltInSystem,
    /// Каталог не поддерживает эту платформу для инструмента
    /// (например msvc-build-tools на Linux).
    UnsupportedPlatform,
    /// Инструмент был бы нужен, но источника установки на этой ОС нет.
    InstallUnavailable,
}

// ------------------------------------------------------------
// Размерности (контракт §2) — хранятся независимо
// ------------------------------------------------------------

/// Откуда взята улика об установке.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceKind {
    /// Проба версии ответила через PATH.
    VersionProbe,
    /// Бинарь запущен напрямую из известного каталога установки.
    KnownPath,
    /// Найден только след (каталог/ключ реестра), проба молчит.
    Footprint,
}

/// Одна конкретная установка инструмента (их может быть несколько).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedInstall {
    /// Первая строка сырого вывода пробы (санитизирована, ограничена).
    #[serde(default)]
    pub raw_version: String,
    /// Разобранная версия в каноническом виде «22.12.0» (если парсится).
    #[serde(default)]
    pub parsed_version: Option<String>,
    /// Нормализованный путь к бинарю/каталогу установки (может быть пуст).
    #[serde(default)]
    pub location: String,
    /// Вид улики.
    pub evidence: EvidenceKind,
    /// Виден ли каталог установки в PATH процесса.
    pub reachable_via_path: bool,
}

/// Итог живого обнаружения. `Failed` — ошибка/таймаут опроса,
/// НЕ отсутствие инструмента.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DetectionOutcome {
    /// Ещё не сканировался (в частичном снапшоте).
    Pending,
    /// Чистый отрицательный результат: проб и следов нет.
    NotDetected,
    /// Опрос не удался (таймаут/ошибка ввода-вывода) — неизвестно.
    Failed { reason: String },
}

/// Оценка версии относительно правил каталога (advisory).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VersionAssessment {
    /// Нечего оценивать (не найден) — «версии нет».
    Unknown,
    /// Версия не разбирается (мусорный вывод) — установлен, но сравнить нельзя.
    Unparseable,
    /// Версия ≥ рекомендуемой.
    MeetsRecommended,
    /// Ниже рекомендуемой, но ≥ минимума.
    BelowRecommended,
    /// Ниже минимума — работа проекта не гарантируется.
    BelowMin,
    /// Нарушение заявленной политики версий каталога
    /// (например min > recommended в данных — конфигурационная ошибка),
    /// когда установленную версию сравнить с корректной политикой нельзя.
    PolicyViolation,
}

/// Происхождение установки (кто поставил).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provenance {
    /// Установлен StackPilot (запись в state.json).
    StackPilotManaged,
    /// Найден живым обнаружением, нами не ставился; способ установки
    /// неизвестен (скачал с сайта, скопировал и т.п.).
    External,
    /// Поставлен менеджером пакетов ОС (winget/choco/apt/brew) —
    /// в том числе нами, но через пакетный источник: обновляется им же.
    PackageManager,
    /// Предоставлен ОС.
    System,
    /// Приходит в комплекте с другим инструментом (npm→node).
    BundledWith {
        /// id инструмента-хозяина.
        tool: String,
    },
    /// Работает в Docker/docker-compose вместо хост-установки.
    Docker,
    /// Неизвестно (не найден / нет данных).
    Unknown,
}

impl Provenance {
    /// Происхождение из персистентных метаданных установок:
    /// запись в state.json = ставили мы (StackPilot), иначе — внешний.
    /// Потребитель — командный слой на следующем этапе (tcx_*).
    #[allow(dead_code)]
    pub fn from_metadata(managed_by_stackpilot: bool) -> Self {
        if managed_by_stackpilot {
            Provenance::StackPilotManaged
        } else {
            Provenance::External
        }
    }
}

/// Применимость инструмента к текущей платформе.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlatformApplicability {
    /// Есть источник установки на этой ОС.
    Installable,
    /// Только ручная установка (manual_install в каталоге).
    ManualOnly,
    /// Двойной docker-инструмент (по умолчанию — docker-compose).
    DockerDefault,
    /// Встроенный в ОС / информационный (источников нет нигде).
    BuiltIn,
    /// Источники есть, но не для этой ОС.
    UnsupportedOnPlatform,
}

// ------------------------------------------------------------
// Возможности платформы (контракт §2, D6) — восемь независимых флагов
// ------------------------------------------------------------

/// Что бэкенд может С ЭТИМ инструментом на текущей платформе.
/// Флаги независимы и вычисляются по явным правилам из каталога
/// (см. ToolPlatformCapabilities::for_definition); отсутствие
/// метаданных в старых записях каталога даёт честное false,
/// а не догадку.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ToolPlatformCapabilities {
    /// Есть правила обнаружения — инструмент можно искать на машине.
    pub detectable: bool,
    /// Есть источник установки для этой ОС и исполнитель установок
    /// реализован; ручная установка не единственный способ.
    pub installable: bool,
    /// Установку можно повторить поверх существующей (обновление).
    pub updatable: bool,
    /// ЯВНО заявлено удаление (declared_capabilities.removable).
    pub removable: bool,
    /// ЯВНО заявлено восстановление (declared_capabilities.repairable).
    pub repairable: bool,
    /// В каталоге есть health-проверки — здоровье можно измерить.
    pub health_checkable: bool,
    /// Есть текст инструкции по ручной установке.
    pub manual_instructions_available: bool,
    /// Заявлена docker-альтернатива хост-установке.
    pub docker_alternative_available: bool,
}

// ------------------------------------------------------------
// Здоровье (явные объявления из каталога)
// ------------------------------------------------------------

/// Состояние здоровья: полный набор явных состояний (контракт §4).
/// Отсутствие данных — «не проверяли»/«проверок нет», а НЕ вердикт;
/// ошибка запуска проверки — «не смогли проверить», а не «сломано».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HealthState {
    /// Проверки ещё не запускались в этом сеансе (данных нет вообще).
    NotChecked,
    /// Проверки выполняются прямо сейчас (промежуточное состояние скана).
    Checking,
    /// Все объявленные проверки прошли.
    Healthy,
    /// Проверки прошли, но версия ниже рекомендуемой — работает с деградацией.
    Degraded,
    /// Хотя бы одна проверка не прошла (процесс выполнился, условие — нет).
    Unhealthy,
    /// Проверки неприменимы (инструмент не найден / сломан путь).
    Unavailable,
    /// Инструмент не поддерживается на этой платформе — проверять нечего.
    Unsupported,
    /// health_checks в каталоге нет — измерить здоровье нечем.
    NoChecksDefined,
    /// Проверку не удалось ВЫПОЛНИТЬ (процесс не запустился/таймаут) —
    /// результат неизвестен; явно отличается от «проверка провалена».
    FailedToRun,
}

impl HealthState {
    /// true — состояние является финальным вердиктом о работоспособности
    /// (а не «данных нет/не применимо»). Потребитель — командный слой
    /// и UI-маппинг на следующем этапе.
    #[allow(dead_code)]
    pub fn is_verdict(&self) -> bool {
        matches!(
            self,
            HealthState::Healthy | HealthState::Degraded | HealthState::Unhealthy
        )
    }
}

/// Результат одной проверки здоровья.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckResult {
    pub label: String,
    /// true — проверка пройдена (процесс завершился успешно).
    pub passed: bool,
    /// Различение причин отказа: true — процесс не запустился/упал/таймаут;
    /// false — процесс выполнился, но условие не выполнено (ненулевой код).
    pub process_failed: bool,
    /// Санитизированный вывод (без секретов окружения, ограничен по длине).
    pub detail: String,
    pub duration_ms: u64,
}

/// Аггрегат здоровья одного инструмента.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthOutcome {
    pub state: HealthState,
    pub results: Vec<HealthCheckResult>,
}

impl HealthOutcome {
    /// None — вердикта нет (данных нет/не применимо);
    /// Some(true/false) — все ли проверки прошли.
    /// Потребители — тесты и будущий health-маппинг командного слоя.
    #[allow(dead_code)]
    pub fn all_passed(&self) -> Option<bool> {
        match self.state {
            HealthState::Healthy | HealthState::Degraded => Some(true),
            HealthState::Unhealthy | HealthState::FailedToRun => Some(false),
            HealthState::NotChecked
            | HealthState::Checking
            | HealthState::Unavailable
            | HealthState::Unsupported
            | HealthState::NoChecksDefined => None,
        }
    }
}

// ------------------------------------------------------------
// Диагностика PATH (только чтение)
// ------------------------------------------------------------

/// Вид находки диагностики PATH.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathFindingKind {
    /// Бинарь найден на диске, но его каталог отсутствует в PATH.
    BinaryNotOnPath,
    /// Ожидаемый каталогом записи PATH нет в PATH.
    EntryMissing,
    /// Запись PATH указывает на несуществующий каталог.
    StaleEntry,
    /// Точный дубликат другой записи.
    DuplicateEntry,
    /// Дубликат с точностью до нормализации (регистр/слеши).
    CaseDuplicateEntry,
    /// Запись содержит нераскрытые переменные (%VAR%) — требует раскрытия.
    RequiresExpansion,
    /// Существование записи невозможно проверить (пустая/некорректная).
    UnverifiableEntry,
}

/// Одна находка диагностики PATH.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathFinding {
    pub kind: PathFindingKind,
    /// Запись/путь, к которому относится находка.
    pub entry: String,
    /// Человекочитаемое пояснение (для UI/логов, без секретов).
    pub detail: String,
}

/// Разбор одной записи PATH.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathEntryReport {
    /// Запись как она записана (сырая).
    pub raw: String,
    /// Раскрытая форма (%VAR% → значение).
    pub expanded: String,
    /// Раскрытый каталог существует.
    pub exists: bool,
    /// Существование можно было проверить вообще (не пустая и т.п.).
    pub verifiable: bool,
    /// Индекс первой точной копии (если это дубликат).
    pub duplicate_of: Option<usize>,
    /// Индекс первой копии с точностью до нормализации.
    pub case_duplicate_of: Option<usize>,
    /// Есть нераскрытые %VAR%.
    pub requires_expansion: bool,
}

/// Полный отчёт о PATH (только чтение; мутации — отдельные задания).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PathReport {
    pub entries: Vec<PathEntryReport>,
    pub findings: Vec<PathFinding>,
}

// ------------------------------------------------------------
// Задание скана (job) и события
// ------------------------------------------------------------

/// Фаза выполнения задания скана.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanPhase {
    Queued,
    /// Сбор сведений об окружении (ОС, PATH).
    Environment,
    /// Опрос инструментов (ограниченный параллелизм).
    Tools,
    /// PATH-отчёт, скоринг, сборка снапшота.
    Finalizing,
    Done,
}

/// Терминальное/живое состояние задания.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanTerminal {
    /// Ещё идёт.
    Running,
    /// Все инструменты опрошены.
    Completed,
    /// Завершено не полностью (дедлайн/отмена): незакрытые инструменты
    /// остаются в состоянии ScanPending — это честное «не проверено».
    Partial,
    Cancelled,
    Failed,
    /// Перезапуск приложения посреди скана (восстановлено из журнала).
    Interrupted,
}

/// Снимок состояния задания скана (для reconnect и восстановления).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanJobSnapshot {
    /// Идентификатор задания (стабилен, переживает reconnect).
    pub job_id: String,
    /// Идентификатор запуска скана (все события несут его же).
    pub scan_id: String,
    pub started_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
    pub phase: ScanPhase,
    pub total_tools: usize,
    pub completed_tools: usize,
    /// Инструмент, опрашиваемый прямо сейчас (последний начатый).
    pub current_tool: Option<String>,
    pub running: bool,
    /// Отмена запрошена (задание ещё добивает текущие пробы).
    pub cancel_requested: bool,
    pub terminal: ScanTerminal,
    /// Состояние восстановлено из журнала после перезапуска.
    #[serde(default)]
    pub recovered: bool,
}

/// Типизированное событие прогресса скана. Не существует без
/// идентичности операции: конструктор требует непустые job/scan id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanProgressEvent {
    pub job_id: String,
    pub scan_id: String,
    /// Сколько инструментов закрыто (включая этот).
    pub completed_count: usize,
    pub total_count: usize,
    pub tool_id: String,
    pub display_name: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// Строковое имя состояния (вариант ToolState) — компактно для UI.
    pub tool_state: String,
    pub timestamp: String,
    #[serde(default)]
    pub error: Option<String>,
}

impl ScanProgressEvent {
    /// Собирает событие. None — если нет идентичности операции
    /// (пустые job/scan id): такие события запрещены контрактом.
    pub fn try_new(
        job_id: &str,
        scan_id: &str,
        completed_count: usize,
        total_count: usize,
        tool_id: &str,
        display_name: &str,
        icon: Option<&str>,
        state: &ToolState,
        error: Option<&str>,
    ) -> Option<Self> {
        if job_id.is_empty() || scan_id.is_empty() || tool_id.is_empty() {
            return None;
        }
        Some(Self {
            job_id: job_id.to_string(),
            scan_id: scan_id.to_string(),
            completed_count,
            total_count,
            tool_id: tool_id.to_string(),
            display_name: display_name.to_string(),
            icon: icon.map(str::to_string),
            tool_state: state_name(state).to_string(),
            timestamp: super::super::core::console::timestamp(),
            error: error.map(str::to_string),
        })
    }
}

/// Короткое строковое имя состояния для событий/UI.
pub fn state_name(state: &ToolState) -> &'static str {
    match state {
        ToolState::ScanPending => "scan_pending",
        ToolState::ScanFailed { .. } => "scan_failed",
        ToolState::Missing => "missing",
        ToolState::InstalledHealthy { .. } => "installed_healthy",
        ToolState::InstalledHealthUnknown { .. } => "installed_health_unknown",
        ToolState::InstalledUnhealthy { .. } => "installed_unhealthy",
        ToolState::UpdateAvailable { .. } => "update_available",
        ToolState::PathBroken { .. } => "path_broken",
        ToolState::ManualInstall { .. } => "manual_install",
        ToolState::DockerManaged => "docker_managed",
        ToolState::BuiltInSystem => "built_in_system",
        ToolState::UnsupportedPlatform => "unsupported_platform",
        ToolState::InstallUnavailable => "install_unavailable",
    }
}

// ------------------------------------------------------------
// Сводка оценки (score)
// ------------------------------------------------------------

/// Итог скоринга окружения. Формула задокументирована в score.rs;
/// ключевые гарантии, отражённые в полях:
///   - инструменты без проверок и неприменимые НЕ штрафуют оценку;
///   - scan-failed исключён из числителя и знаменателя (неизвестно ≠ плохо);
///   - деградированные (update available) считаются половиной здоровья.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScoreSummary {
    /// Итоговая оценка 0..100.
    pub score: u8,
    /// Сколько требуемых применимых инструментов вошло в знаменатель.
    pub counted_tools: usize,
    pub healthy_required: usize,
    pub degraded: usize,
    pub missing_required: usize,
    pub broken_required: usize,
    pub unhealthy_required: usize,
    /// Ошибка опроса (исключены из формулы, но видны отдельно).
    pub scan_failed: usize,
    /// Без проверок здоровья (исключены из формулы).
    pub unchecked: usize,
    /// Опциональные/docker/ручные/встроенные (вне знаменателя).
    pub optional: usize,
    /// Неприменимые к платформе (вне знаменателя).
    pub not_applicable: usize,
}

// ------------------------------------------------------------
// Снапшот окружения
// ------------------------------------------------------------

/// Полный снапшот окружения — результат одного скана.
/// Единственный источник правды о живом состоянии машины;
/// метаданные установок — историческая провenance, а не замена.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentSnapshot {
    /// Идентификатор снапшота (для сопоставления кэша и рендера).
    pub snapshot_id: String,
    pub job_id: String,
    pub scan_id: String,
    pub os: String,
    pub os_version: String,
    /// Архитектура процессора (std::env::consts::ARCH).
    #[serde(default)]
    pub arch: String,
    pub package_managers: Vec<String>,
    /// Диски/место под установки (на момент скана; только чтение).
    #[serde(default)]
    pub disk: Vec<DiskSpaceInfo>,
    /// Может ли бэкенд на этой машине повышать права (UAC и т.п.)
    /// и требовал ли кто-то из инструментов прав.
    #[serde(default)]
    pub admin: AdminCapability,
    pub started_at: String,
    /// Момент завершения (или последнего обновления при частичном).
    pub finished_at: String,
    /// true — все инструменты каталога опрошены.
    pub complete: bool,
    pub cancelled: bool,
    /// Результаты по ВСЕМ инструментам каталога, в детерминированном
    /// порядке каталога (независимо от реального порядка опроса).
    pub tools: Vec<ToolScanResult>,
    pub path_report: PathReport,
    pub score: ScoreSummary,
    /// Сводка количеств по презентационным состояниям (для дашборда).
    #[serde(default)]
    pub summary: StatusCounts,
    /// Предупреждения скана (не блокирующие, но важные для UI).
    #[serde(default)]
    pub warnings: Vec<SnapshotIssue>,
    /// Ошибки скана (частичный отчёт, недоступные данные и т.п.).
    #[serde(default)]
    pub errors: Vec<SnapshotIssue>,
    /// id заданий, активных в момент выдачи снапшота (установки и т.п.):
    /// потребитель видит, что данные могли устареть ИЗ-ЗА идущих работ.
    #[serde(default)]
    pub active_jobs: Vec<String>,
    /// Возраст снапшота в секундах на момент выдачи (пересчитывается).
    #[serde(default)]
    pub age_seconds: u64,
    /// Старше порога свежести — данные исторические, не живые.
    #[serde(default)]
    pub stale: bool,
    /// Снапшот прочитан из персистентного кэша (предыдущий прогон).
    #[serde(default)]
    pub from_cache: bool,
}

/// Свободное место на одном диске (на момент скана).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskSpaceInfo {
    /// Корень диска / точка монтирования («C:\»).
    pub root: String,
    /// Свободное место, МБ (0 = не удалось определить).
    pub free_mb: u64,
}

/// Возможности администрирования на этой машине.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AdminCapability {
    /// Исполнитель установок умеет повышать права на этой ОС (UAC).
    pub elevation_supported: bool,
    /// Хотя бы один инструмент каталога требует прав администратора.
    pub required_by_tools: bool,
}

/// Количество инструментов по каждому презентационному состоянию.
/// Поля независимы; сумма равна числу инструментов снапшота.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StatusCounts {
    pub scan_pending: usize,
    pub scan_failed: usize,
    pub missing: usize,
    pub installed_healthy: usize,
    pub installed_health_unknown: usize,
    pub installed_unhealthy: usize,
    pub update_available: usize,
    pub path_broken: usize,
    pub manual_install: usize,
    pub docker_managed: usize,
    pub built_in_system: usize,
    pub unsupported_platform: usize,
    pub install_unavailable: usize,
}

impl StatusCounts {
    /// Сводка по списку результатов (детерминированная, без состояния).
    pub fn from_results(tools: &[ToolScanResult]) -> Self {
        let mut counts = Self::default();
        for tool in tools {
            match &tool.state {
                ToolState::ScanPending => counts.scan_pending += 1,
                ToolState::ScanFailed { .. } => counts.scan_failed += 1,
                ToolState::Missing => counts.missing += 1,
                ToolState::InstalledHealthy { .. } => counts.installed_healthy += 1,
                ToolState::InstalledHealthUnknown { .. } => counts.installed_health_unknown += 1,
                ToolState::InstalledUnhealthy { .. } => counts.installed_unhealthy += 1,
                ToolState::UpdateAvailable { .. } => counts.update_available += 1,
                ToolState::PathBroken { .. } => counts.path_broken += 1,
                ToolState::ManualInstall { .. } => counts.manual_install += 1,
                ToolState::DockerManaged => counts.docker_managed += 1,
                ToolState::BuiltInSystem => counts.built_in_system += 1,
                ToolState::UnsupportedPlatform => counts.unsupported_platform += 1,
                ToolState::InstallUnavailable => counts.install_unavailable += 1,
            }
        }
        counts
    }

    /// Сумма по всем состояниям (санити-чек дашборда).
    #[allow(dead_code)]
    pub fn total(&self) -> usize {
        self.scan_pending
            + self.scan_failed
            + self.missing
            + self.installed_healthy
            + self.installed_health_unknown
            + self.installed_unhealthy
            + self.update_available
            + self.path_broken
            + self.manual_install
            + self.docker_managed
            + self.built_in_system
            + self.unsupported_platform
            + self.install_unavailable
    }
}

/// Одно сообщение снапшота (предупреждение или ошибка). Никогда не
/// содержит секретов — это текст для UI/логов.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotIssue {
    /// Машиночитаемый код причины (partial_scan, disk_probe_failed, ...).
    pub code: String,
    /// Человекочитаемое пояснение.
    pub message: String,
}

/// Результат сканирования одного инструмента: все размерности +
/// презентационное состояние.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolScanResult {
    pub tool_id: String,
    pub display: String,
    pub category: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// Живое обнаружение (источник правды).
    pub detection: DetectionOutcome,
    /// Конкретные установки (может быть несколько).
    #[serde(default)]
    pub installs: Vec<DetectedInstall>,
    /// Диагностика PATH по этому инструменту.
    #[serde(default)]
    pub path_findings: Vec<PathFinding>,
    /// Здоровье (None — ещё не добрались, ScanPending).
    #[serde(default)]
    pub health: Option<HealthOutcome>,
    pub applicability: PlatformApplicability,
    /// Восемь независимых флагов возможностей платформы для инструмента.
    #[serde(default)]
    pub capabilities: ToolPlatformCapabilities,
    pub provenance: Provenance,
    /// Связь с другим инструментом («npm приходит с node»).
    #[serde(default)]
    pub bundled_with: Option<String>,
    pub version_assessment: VersionAssessment,
    pub state: ToolState,
    #[serde(default)]
    pub error: Option<String>,
    pub duration_ms: u64,
}

impl ToolScanResult {
    /// Заготовка «ещё не сканировали» — используется в частичных отчётах.
    pub fn pending(def: &crate::modules::toolchain::models::ToolDefinition) -> Self {
        Self {
            tool_id: def.id.clone(),
            display: def.display.clone(),
            category: def.category.clone(),
            icon: def.icon.clone(),
            detection: DetectionOutcome::Pending,
            installs: Vec::new(),
            path_findings: Vec::new(),
            health: None,
            applicability: PlatformApplicability::Installable,
            capabilities: ToolPlatformCapabilities::default(),
            provenance: Provenance::Unknown,
            bundled_with: def.bundled_with.clone(),
            version_assessment: VersionAssessment::Unknown,
            state: ToolState::ScanPending,
            error: None,
            duration_ms: 0,
        }
    }
}

impl ToolPlatformCapabilities {
    /// Вычисляет флаги возможностей по ЯВНЫМ правилам:
    ///   - detectable — есть хоть одно правило обнаружения;
    ///   - installable — есть источник для этой ОС, исполнитель установок
    ///     реализован и инструмент не manual-only;
    ///   - updatable — установка возможна (повторный прогон источника
    ///     обновляет; для pkg-менеджеров это штатный upgrade);
    ///   - removable / repairable — ТОЛЬКО явное заявление каталога
    ///     (declared_capabilities); отсутствие записи = false, не догадка;
    ///   - health_checkable — в каталоге есть health-проверки;
    ///   - manual_instructions_available — есть текст ручной установки;
    ///   - docker_alternative_available — заявлена docker-альтернатива.
    pub fn for_definition(
        def: &crate::modules::toolchain::models::ToolDefinition,
        os: &str,
        install_execution_supported: bool,
    ) -> Self {
        use crate::modules::toolchain::core::check::sources_for_os;

        let detectable = !def.detection.version_probes.is_empty()
            || !def.detection.known_paths.is_empty()
            || !def.detection.registry_keys.is_empty();
        let has_os_source = !sources_for_os(def, os).is_empty();
        let manual_only = def.manual_install.is_some();

        let installable = has_os_source && install_execution_supported && !manual_only;

        Self {
            detectable,
            installable,
            updatable: installable,
            removable: def.extended.declared_capabilities.removable(),
            repairable: def.extended.declared_capabilities.repairable(),
            health_checkable: !def.health_checks.is_empty(),
            manual_instructions_available: manual_only,
            docker_alternative_available: def.extended.docker.is_some(),
        }
    }
}

// ============================================================
// Тесты: контракт сериализации, сводки, правила возможностей
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::toolchain::models::{
        DeclaredCapabilities, DetectionRules, DockerCapability, InstallSource, InstallSourceKind,
        InstallSources,
    };

    fn fake_def(id: &str) -> crate::modules::toolchain::models::ToolDefinition {
        crate::modules::toolchain::models::ToolDefinition {
            id: id.to_string(),
            category: "utility".to_string(),
            display: format!("Display {id}"),
            description: String::new(),
            icon: None,
            detection: DetectionRules::default(),
            versions: Default::default(),
            sources: InstallSources::default(),
            size_mb: 10,
            needs_admin: false,
            path_entries: vec![],
            bundled_with: None,
            health_checks: vec![],
            notes: None,
            manual_install: None,
            extended: Default::default(),
        }
    }

    // ------------------------------------------------------------
    // Единый тегированный формат {"kind": ...}
    // ------------------------------------------------------------

    #[test]
    fn tool_state_uses_internal_kind_tag() {
        let json = serde_json::to_string(&ToolState::UpdateAvailable {
            installed: "1.0".into(),
            recommended: "2.0".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"kind":"update_available","installed":"1.0","recommended":"2.0"}"#
        );

        let unit = serde_json::to_string(&ToolState::DockerManaged).unwrap();
        assert_eq!(unit, r#"{"kind":"docker_managed"}"#);

        let back: ToolState = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back,
            ToolState::UpdateAvailable {
                installed: "1.0".into(),
                recommended: "2.0".into()
            }
        );
    }

    #[test]
    fn provenance_covers_all_seven_kinds() {
        let cases: Vec<(Provenance, &str)> = vec![
            (Provenance::StackPilotManaged, "stack_pilot_managed"),
            (Provenance::External, "external"),
            (Provenance::PackageManager, "package_manager"),
            (Provenance::System, "system"),
            (Provenance::Docker, "docker"),
            (Provenance::Unknown, "unknown"),
        ];
        for (value, kind) in cases {
            let json = serde_json::to_string(&value).unwrap();
            assert_eq!(json, format!(r#"{{"kind":"{kind}"}}"#));
            let back: Provenance = serde_json::from_str(&json).unwrap();
            assert_eq!(back, value);
        }
        // Вариант с данными несёт payload рядом с тегом.
        let bundled = serde_json::to_string(&Provenance::BundledWith {
            tool: "node".into(),
        })
        .unwrap();
        assert_eq!(bundled, r#"{"kind":"bundled_with","tool":"node"}"#);
    }

    #[test]
    fn detection_outcome_round_trip_with_kind_tag() {
        let failed = DetectionOutcome::Failed {
            reason: "timeout".into(),
        };
        let json = serde_json::to_string(&failed).unwrap();
        assert_eq!(json, r#"{"kind":"failed","reason":"timeout"}"#);
        assert_eq!(
            serde_json::from_str::<DetectionOutcome>(&json).unwrap(),
            failed
        );
        assert_eq!(
            serde_json::to_string(&DetectionOutcome::NotDetected).unwrap(),
            r#"{"kind":"not_detected"}"#
        );
    }

    #[test]
    fn health_state_nine_states_round_trip() {
        let states = [
            HealthState::NotChecked,
            HealthState::Checking,
            HealthState::Healthy,
            HealthState::Degraded,
            HealthState::Unhealthy,
            HealthState::Unavailable,
            HealthState::Unsupported,
            HealthState::NoChecksDefined,
            HealthState::FailedToRun,
        ];
        for state in states {
            let json = serde_json::to_string(&state).unwrap();
            assert!(json.starts_with(r#"{"kind":""#), "нет kind-тега: {json}");
            assert_eq!(serde_json::from_str::<HealthState>(&json).unwrap(), state);
        }
        // Семантика вердиктов: только Healthy/Degraded/Unhealthy — вердикты.
        assert!(HealthState::Healthy.is_verdict());
        assert!(HealthState::Degraded.is_verdict());
        assert!(HealthState::Unhealthy.is_verdict());
        assert!(!HealthState::FailedToRun.is_verdict());
        assert!(!HealthState::NoChecksDefined.is_verdict());
    }

    #[test]
    fn version_assessment_includes_policy_violation() {
        let json = serde_json::to_string(&VersionAssessment::PolicyViolation).unwrap();
        assert_eq!(json, r#"{"kind":"policy_violation"}"#);
        assert_eq!(
            serde_json::from_str::<VersionAssessment>(&json).unwrap(),
            VersionAssessment::PolicyViolation
        );
    }

    #[test]
    fn platform_applicability_round_trip() {
        let json = serde_json::to_string(&PlatformApplicability::UnsupportedOnPlatform).unwrap();
        assert_eq!(json, r#"{"kind":"unsupported_on_platform"}"#);
    }

    // ------------------------------------------------------------
    // Сводка состояний
    // ------------------------------------------------------------

    #[test]
    fn status_counts_sum_matches_total() {
        let mut a = ToolScanResult::pending(&fake_def("a"));
        a.state = ToolState::Missing;
        let mut b = ToolScanResult::pending(&fake_def("b"));
        b.state = ToolState::InstalledHealthy {
            version: "1".into(),
        };
        let mut c = ToolScanResult::pending(&fake_def("c"));
        c.state = ToolState::ScanFailed {
            reason: "boom".into(),
        };

        let counts = StatusCounts::from_results(&[a, b, c]);
        assert_eq!(counts.missing, 1);
        assert_eq!(counts.installed_healthy, 1);
        assert_eq!(counts.scan_failed, 1);
        assert_eq!(counts.total(), 3);
    }

    // ------------------------------------------------------------
    // Возможности платформы: только явные заявления
    // ------------------------------------------------------------

    #[test]
    fn capabilities_are_explicit_not_guessed() {
        let os = "windows";

        // Пустой каталог: ничего нельзя — и это честные false.
        let caps = ToolPlatformCapabilities::for_definition(&fake_def("bare"), os, true);
        assert!(!caps.detectable);
        assert!(!caps.installable);
        assert!(!caps.updatable);
        assert!(!caps.removable, "незаявленное удаление не угадывается");
        assert!(!caps.repairable, "незаявленный repair не угадывается");
        assert!(!caps.health_checkable);
        assert!(!caps.manual_instructions_available);
        assert!(!caps.docker_alternative_available);

        // Полный каталог: всё явно заявлено.
        let mut full = fake_def("full");
        full.detection.version_probes = vec![vec!["x".to_string()]];
        full.sources.windows = vec![InstallSource {
            kind: InstallSourceKind::PkgManager,
            id: "Fake.Full".to_string(),
            url: None,
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            file_name: None,
            execution: None,
            sha256: None,
        }];
        full.extended.declared_capabilities = DeclaredCapabilities {
            removable: Some(true),
            repairable: Some(false),
        };
        full.extended.docker = Some(DockerCapability::default());
        full.health_checks = vec![crate::modules::toolchain::models::HealthCheck {
            label: "ok".to_string(),
            command: vec!["cmd".to_string()],
        }];

        let caps = ToolPlatformCapabilities::for_definition(&full, os, true);
        assert!(caps.detectable);
        assert!(caps.installable);
        assert!(caps.updatable);
        assert!(caps.removable, "явное removable=true");
        assert!(!caps.repairable, "явное repairable=false");
        assert!(caps.health_checkable);
        assert!(caps.docker_alternative_available);
        assert!(!caps.manual_instructions_available);

        // Исполнитель установок не реализован → installable/updatable false.
        let caps = ToolPlatformCapabilities::for_definition(&full, os, false);
        assert!(!caps.installable);
        assert!(!caps.updatable);

        // Ручная установка: installable=false, инструкция доступна.
        let mut manual = full.clone();
        manual.manual_install = Some("Ставится вручную".to_string());
        let caps = ToolPlatformCapabilities::for_definition(&manual, os, true);
        assert!(!caps.installable);
        assert!(caps.manual_instructions_available);
    }

    #[test]
    fn snapshot_new_fields_survive_round_trip_and_default_for_old_files() {
        // Старый снапшот без новых полей читается с честными дефолтами.
        let legacy = r#"{
            "snapshot_id": "s1", "job_id": "j1", "scan_id": "sc1",
            "os": "windows", "os_version": "10", "package_managers": [],
            "started_at": "t0", "finished_at": "t1",
            "complete": true, "cancelled": false,
            "tools": [], "path_report": {"entries": [], "findings": []},
            "score": {"score": 50, "counted_tools": 1, "healthy_required": 0,
                       "degraded": 0, "missing_required": 0, "broken_required": 0,
                       "unhealthy_required": 0, "scan_failed": 0, "unchecked": 0,
                       "optional": 0, "not_applicable": 0}
        }"#;
        let snap: EnvironmentSnapshot = serde_json::from_str(legacy).unwrap();
        assert_eq!(snap.arch, "");
        assert!(snap.disk.is_empty());
        assert_eq!(snap.admin, AdminCapability::default());
        assert_eq!(snap.summary.total(), 0);
        assert!(snap.warnings.is_empty());
        assert!(snap.active_jobs.is_empty());

        // Обратная сериализация сохраняет новые поля.
        let mut snap = snap;
        snap.arch = "x86_64".to_string();
        snap.active_jobs = vec!["tcxj-1".to_string()];
        let raw = serde_json::to_string(&snap).unwrap();
        assert!(raw.contains(r#""arch":"x86_64""#));
        assert!(raw.contains(r#""active_jobs":["tcxj-1"]"#));
    }
}
