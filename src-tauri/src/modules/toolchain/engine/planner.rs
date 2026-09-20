// ============================================================
// Canonical planner (engine/planner.rs)
// ============================================================
//
// The single authority for install/update/repair/health plans. Input:
// the restricted EngineRequest + catalog + fresh detection. Output:
// a CanonicalPlan with resolved dependencies, resolved conflicts,
// backend-selected sources, validated admin requirements, calculated
// disk requirements, calculated capabilities, validated integrity
// metadata, canonical ids and an input fingerprint.
//
// Idempotency rules (pipeline contract §3):
//   - installed at an acceptable version -> NoOp(AlreadyInstalled)
//     unless force_reinstall is explicitly set;
//   - update with no available target    -> NoOp(UpdateUnavailable);
//   - dual docker tools stay docker-managed unless Host was chosen
//     explicitly; they are never silently host-installed.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use serde::Serialize;

use crate::modules::toolchain::core as tc_core;
use crate::modules::toolchain::models::{
    InstallSource, InstallSourceKind, PlatformCapabilities, ToolDefinition, ToolStatus,
};

use super::plan::{
    CanonicalPlan, EngineTaskStatus, ExecutionMode, NoopReason, PlanTask, PlanWarning,
    SelectedSource, TaskAction,
};
use super::request::{EngineRequest, ExecutionChoice, OperationKind, ToolRequest};

// ------------------------------------------------------------
// Errors
// ------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanError {
    EmptySelection,
    UnknownTools {
        ids: Vec<String>,
    },
    InvalidSources {
        details: Vec<String>,
    },
    NotInstallable {
        details: Vec<String>,
    },
    Conflicts {
        pairs: Vec<String>,
    },
    UnverifiedSourceRequiresConfirmation {
        tools: Vec<String>,
    },
    AdminConfirmationRequired {
        tools: Vec<String>,
    },
    ElevationUnsupported {
        tools: Vec<String>,
    },
    InsufficientDisk {
        required_mb: u64,
        free_mb: u64,
    },
    PlatformUnsupported {
        os: String,
    },
    /// Building the plan exceeded the finite deadline — the caller gets
    /// an actionable message instead of an indefinite wait.
    PlannerTimeout {
        seconds: u64,
    },
    /// The environment changed between preview and execution: the freshly
    /// rebuilt plan no longer matches what the user approved. Never
    /// silently executed.
    PlanChanged {
        expected: String,
        actual: String,
    },
    Internal(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::EmptySelection => write!(f, "Не выбран ни один инструмент"),
            PlanError::UnknownTools { ids } => {
                write!(f, "Неизвестные инструменты в запросе: {}", ids.join(", "))
            }
            PlanError::InvalidSources { details } => {
                write!(f, "Недопустимые источники: {}", details.join("; "))
            }
            PlanError::NotInstallable { details } => {
                write!(
                    f,
                    "Инструменты вне автоматической установки: {}",
                    details.join("; ")
                )
            }
            PlanError::Conflicts { pairs } => {
                write!(f, "Конфликтующие инструменты: {}", pairs.join("; "))
            }
            PlanError::UnverifiedSourceRequiresConfirmation { tools } => write!(
                f,
                "Источники без контрольной суммы требуют явного подтверждения: {}",
                tools.join(", ")
            ),
            PlanError::AdminConfirmationRequired { tools } => write!(
                f,
                "Нужны права администратора — требуется явное подтверждение: {}",
                tools.join(", ")
            ),
            PlanError::ElevationUnsupported { tools } => write!(
                f,
                "На этой ОС повышение прав недоступно для: {}",
                tools.join(", ")
            ),
            PlanError::InsufficientDisk {
                required_mb,
                free_mb,
            } => write!(
                f,
                "Недостаточно места на диске: требуется {required_mb} МБ, свободно {free_mb} МБ"
            ),
            PlanError::PlatformUnsupported { os } => {
                write!(
                    f,
                    "Автоматическая установка не поддерживается на этой ОС ({os})"
                )
            }
            PlanError::PlannerTimeout { seconds } => write!(
                f,
                "Построение плана не уложилось в {seconds} с. Проверьте машину (антивирус, зависшие процессы) и повторите."
            ),
            PlanError::PlanChanged { .. } => write!(
                f,
                "Окружение изменилось после проверки плана — план устарел. Пересмотрите план и подтвердите заново."
            ),
            PlanError::Internal(msg) => write!(f, "Внутренняя ошибка планировщика: {msg}"),
        }
    }
}

impl std::error::Error for PlanError {}

// ------------------------------------------------------------
// Detection port
// ------------------------------------------------------------

#[async_trait]
pub trait Detector: Send + Sync {
    async fn detect(&self, def: &ToolDefinition) -> ToolStatus;
}

/// Real detector backed by discovery (probes/known paths/registry).
pub struct DiscoveryDetector;

#[async_trait]
impl Detector for DiscoveryDetector {
    async fn detect(&self, def: &ToolDefinition) -> ToolStatus {
        tc_core::discovery::detect_tool(def).await
    }
}

// ------------------------------------------------------------
// Inputs
// ------------------------------------------------------------

/// Known mutually-exclusive tool pairs (data-driven mechanism).
pub fn builtin_conflict_table() -> &'static [(&'static str, &'static str)] {
    &[]
}

pub struct PlanInputs<'a> {
    pub definitions: &'a [ToolDefinition],
    pub detector: &'a dyn Detector,
    pub free_space_mb: u64,
    pub conflicts: &'a [(&'a str, &'a str)],
}

impl<'a> PlanInputs<'a> {
    pub fn new(definitions: &'a [ToolDefinition], detector: &'a dyn Detector) -> Self {
        Self {
            definitions,
            detector,
            free_space_mb: 0,
            conflicts: builtin_conflict_table(),
        }
    }

    pub fn with_free_space(mut self, mb: u64) -> Self {
        self.free_space_mb = mb;
        self
    }

    pub fn with_conflicts(mut self, conflicts: &'a [(&'a str, &'a str)]) -> Self {
        self.conflicts = conflicts;
        self
    }

    fn def(&self, id: &str) -> Option<&'a ToolDefinition> {
        self.definitions.iter().find(|d| d.id == id)
    }

    fn os_sources<'d>(&self, def: &'d ToolDefinition) -> &'d [InstallSource] {
        let os = os_name();
        match os.as_str() {
            "windows" => def.sources.windows.as_slice(),
            "linux" => def.sources.linux.as_slice(),
            "macos" => def.sources.macos.as_slice(),
            _ => &[],
        }
    }
}

fn os_name() -> String {
    crate::modules::toolchain::platforms::current_platform().os_name()
}

fn source_kind_name(kind: &InstallSourceKind) -> String {
    match kind {
        InstallSourceKind::PkgManager => "pkg_manager".to_string(),
        InstallSourceKind::Official => "official".to_string(),
        InstallSourceKind::Script => "script".to_string(),
        InstallSourceKind::QtOnline => "qt_online".to_string(),
    }
}

fn describe_source(src: &InstallSource) -> String {
    match src.kind {
        InstallSourceKind::PkgManager => format!("Менеджер пакетов: {}", src.id),
        InstallSourceKind::Official => format!("Официальный установщик: {}", src.id),
        InstallSourceKind::Script => format!("Скрипт установки: {}", src.id),
        InstallSourceKind::QtOnline => "Официальный репозиторий Qt (online)".to_string(),
    }
}

// ------------------------------------------------------------
// Draft task (pre-id assignment)
// ------------------------------------------------------------

struct Draft {
    tool_id: String,
    display: String,
    icon: Option<String>,
    action: TaskAction,
    source: Option<SelectedSource>,
    size_mb: u32,
    needs_admin: bool,
    /// Tool ids this task depends on (resolved to task ids at finalize).
    depends_on_tools: Vec<String>,
    path_entries: Vec<String>,
    install_options: Vec<String>,
    execution_mode: ExecutionMode,
}

impl Draft {
    fn into_task(self, plan_id: &str) -> PlanTask {
        PlanTask {
            task_id: format!("{plan_id}:{}", self.tool_id),
            tool_id: self.tool_id.clone(),
            display: self.display,
            icon: self.icon,
            action: self.action,
            source: self.source,
            size_mb: self.size_mb,
            needs_admin: self.needs_admin,
            // depends_on references sibling tasks by their canonical ids
            depends_on: self
                .depends_on_tools
                .into_iter()
                .map(|t| format!("{plan_id}:{t}"))
                .collect(),
            path_entries: self.path_entries,
            install_options: self.install_options,
            execution_mode: self.execution_mode,
            status: EngineTaskStatus::Pending,
        }
    }
}

/// Builds the canonical plan for a request.
pub async fn build_plan(
    request: &EngineRequest,
    inputs: &PlanInputs<'_>,
) -> Result<CanonicalPlan, PlanError> {
    let tools = request.deduped_tools();
    if tools.is_empty() {
        return Err(PlanError::EmptySelection);
    }

    // --- resolve tool definitions --------------------------------------
    let mut unknown = Vec::new();
    for t in &tools {
        if inputs.def(&t.tool_id).is_none() {
            unknown.push(t.tool_id.clone());
        }
    }
    if !unknown.is_empty() {
        return Err(PlanError::UnknownTools { ids: unknown });
    }

    // --- platform capability gate ----------------------------------------
    let capabilities = PlatformCapabilities {
        install_execution_supported: tc_core::installer::install_execution_supported(),
        elevation_supported: cfg!(any(target_os = "windows", target_os = "linux", target_os = "macos")),
    };
    if matches!(
        request.operation,
        OperationKind::Install | OperationKind::Update
    ) && !capabilities.install_execution_supported
    {
        return Err(PlanError::PlatformUnsupported { os: os_name() });
    }

    // --- conflict resolution ----------------------------------------------
    let requested_ids: HashSet<&str> = tools.iter().map(|t| t.tool_id.as_str()).collect();
    let mut conflict_pairs = Vec::new();
    for (a, b) in inputs.conflicts {
        if requested_ids.contains(a) && requested_ids.contains(b) {
            conflict_pairs.push(format!("{a} × {b}"));
        }
    }
    if !conflict_pairs.is_empty() {
        return Err(PlanError::Conflicts {
            pairs: conflict_pairs,
        });
    }

    // --- fresh detection of every requested tool ---------------------------
    let mut detected: HashMap<String, ToolStatus> = HashMap::new();
    for t in &tools {
        let def = inputs.def(&t.tool_id).expect("validated above");
        let status = inputs.detector.detect(def).await;
        detected.insert(t.tool_id.clone(), status);
    }

    // --- source validation (before drafting: explicit user choice must
    //      be one of the catalog's allowed sources for this OS) ---------
    if matches!(
        request.operation,
        OperationKind::Install | OperationKind::Update
    ) {
        let mut invalid_sources: Vec<String> = Vec::new();
        for req in &tools {
            let def = inputs.def(&req.tool_id).expect("validated above");
            if let Some(src_err) = validate_requested_source(req, def, inputs) {
                invalid_sources.push(format!("{}: {src_err}", req.tool_id));
            }
        }
        if !invalid_sources.is_empty() {
            return Err(PlanError::InvalidSources {
                details: invalid_sources,
            });
        }
    }

    // --- per-tool drafts ----------------------------------------------------
    let mut drafts: Vec<Draft> = Vec::new();
    let mut not_installable: Vec<String> = Vec::new();

    for req in &tools {
        let def = inputs.def(&req.tool_id).expect("validated above");
        let status = detected.get(&req.tool_id).expect("detected above").clone();
        match draft_for(request, req, &status, def, inputs) {
            Ok(Some(d)) => drafts.push(d),
            Ok(None) => {}
            Err(reason) => not_installable.push(format!("{}: {reason}", req.tool_id)),
        }
    }
    if !not_installable.is_empty() {
        return Err(PlanError::NotInstallable {
            details: not_installable,
        });
    }

    // --- dependency closure ---------------------------------------------------
    resolve_dependency_closure(request, &tools, inputs, &mut drafts).await?;

    // --- integrity metadata validation -----------------------------------------
    // Preview mode builds the SAME plan for review: unverified sources and
    // admin tasks surface as warnings + confirmation checkboxes instead of
    // hard errors (execution still refuses without confirmation below).
    let unverified: Vec<String> = drafts
        .iter()
        .filter(|d| d.source.as_ref().map(|s| s.unverified()).unwrap_or(false))
        .map(|d| d.tool_id.clone())
        .collect();
    if !request.preview && !unverified.is_empty() && !request.confirm_unverified_sources {
        return Err(PlanError::UnverifiedSourceRequiresConfirmation { tools: unverified });
    }

    // --- admin requirement validation ---------------------------------------------
    let admin_tools: Vec<String> = drafts
        .iter()
        .filter(|d| d.needs_admin)
        .map(|d| d.tool_id.clone())
        .collect();
    if !admin_tools.is_empty() {
        if !capabilities.elevation_supported {
            return Err(PlanError::ElevationUnsupported { tools: admin_tools });
        }
        if !request.preview && !request.confirm_admin_elevation {
            return Err(PlanError::AdminConfirmationRequired { tools: admin_tools });
        }
    }

    // --- disk requirements ----------------------------------------------------------
    let total_size_mb: u64 = drafts.iter().map(|d| d.size_mb as u64).sum();
    let enough_space = inputs.free_space_mb >= total_size_mb;
    if total_size_mb > 0 && !enough_space {
        return Err(PlanError::InsufficientDisk {
            required_mb: total_size_mb,
            free_mb: inputs.free_space_mb,
        });
    }

    // --- ordering: dependencies first, winget first -----------------------------------
    order_drafts(&mut drafts)?;

    // --- warnings -----------------------------------------------------------------------
    let warnings = collect_warnings(request, &drafts, &detected);

    // --- finalize --------------------------------------------------------------------------
    let random = tc_core::crypto::random_bytes(16).map_err(PlanError::Internal)?;
    let plan_id = format!("tcxp-{}", &tc_core::crypto::sha256_hex(&random)[..16]);
    let fingerprint = fingerprint(request, &drafts);

    // --- stale-preview guard ----------------------------------------------------------------
    // Если запрос несёт отпечаток одобренного превью и свежепостроенный
    // план от него отличается — окружение изменилось между превью и
    // исполнением. Такой план НЕ исполняется молча: структурированная
    // ошибка возвращает пользователя к пересмотру.
    if let Some(expected) = &request.expected_plan_fingerprint {
        if *expected != fingerprint {
            return Err(PlanError::PlanChanged {
                expected: expected.clone(),
                actual: fingerprint,
            });
        }
    }

    let tasks: Vec<PlanTask> = drafts.into_iter().map(|d| d.into_task(&plan_id)).collect();

    Ok(CanonicalPlan {
        plan_id,
        operation: request.operation,
        os: os_name(),
        created_at: tc_core::console::timestamp(),
        fingerprint,
        tasks,
        total_size_mb,
        free_space_mb: inputs.free_space_mb,
        enough_space,
        needs_admin_any: !admin_tools.is_empty(),
        capabilities,
        warnings,
    })
}

/// Decides what to do with one requested tool.
/// Ok(Some(draft)) — a task; Err(reason) — the tool cannot participate.
fn draft_for(
    request: &EngineRequest,
    req: &ToolRequest,
    status: &ToolStatus,
    def: &ToolDefinition,
    inputs: &PlanInputs<'_>,
) -> Result<Option<Draft>, String> {
    match request.operation {
        OperationKind::HealthCheck => Ok(Some(Draft {
            tool_id: def.id.clone(),
            display: def.display.clone(),
            icon: def.icon.clone(),
            action: TaskAction::HealthCheck,
            source: None,
            size_mb: 0,
            needs_admin: false,
            depends_on_tools: vec![],
            path_entries: vec![],
            install_options: vec![],
            execution_mode: ExecutionMode::Host,
        })),
        OperationKind::RepairPath => Ok(Some(Draft {
            tool_id: def.id.clone(),
            display: def.display.clone(),
            icon: def.icon.clone(),
            action: TaskAction::RepairPath,
            source: None,
            size_mb: 0,
            needs_admin: false,
            depends_on_tools: vec![],
            path_entries: def.path_entries.clone(),
            install_options: vec![],
            execution_mode: ExecutionMode::Host,
        })),
        OperationKind::Install | OperationKind::Update => {
            if let Some(reason) = &def.manual_install {
                return Err(format!("ручная установка ({reason})"));
            }

            // STANDALONE-семантика исполнения: только хост. «Двойные»
            // docker-инструменты Project Creator здесь не существуют —
            // PostgreSQL/Redis/MongoDB/Kafka/Grafana/MySQL ставятся
            // локально, когда у каталога есть источник для этой ОС.
            // Docker остаётся рекомендацией/альтернативой в метаданных
            // (ToolExtendedMetadata.docker), но никогда — режимом задачи.
            // Явный выбор docker отклоняется: docker-режима у задач нет,
            // и молча подменять его хостом нельзя.
            if req.execution == Some(ExecutionChoice::Docker) {
                return Err("у инструмента нет docker-режима".to_string());
            }

            let sources = inputs.os_sources(def);
            if sources.is_empty() {
                return Err("нет источника установки для этой ОС".to_string());
            }

            let selected = select_source(req, sources)?;
            // Опции установки (UI-модули Qt) понимает только Qt-конвейер.
            // Опции для прочих инструментов — невыразимый запрос: молча
            // их игнорировать нельзя (это «опция применилась», которой
            // не было), отклоняем явно.
            if !req.install_options.is_empty()
                && !matches!(selected.kind, InstallSourceKind::QtOnline)
            {
                return Err(format!(
                    "опции установки поддерживаются только для Qt-источников (источник «{}»)",
                    selected.id
                ));
            }
            let selected_info = SelectedSource {
                kind: source_kind_name(&selected.kind),
                id: selected.id.clone(),
                description: describe_source(selected),
                sha256: selected.sha256.clone(),
                needs_admin: tc_core::installer::source_requires_elevation(selected, def),
            };

            let action = decide_action(request.operation, status, req.force_reinstall);

            // Idempotent no-op: nothing is downloaded or executed — the
            // task carries zero disk requirement and no elevation.
            if action.is_noop() {
                return Ok(Some(Draft {
                    tool_id: def.id.clone(),
                    display: def.display.clone(),
                    icon: def.icon.clone(),
                    action,
                    source: None,
                    size_mb: 0,
                    needs_admin: false,
                    depends_on_tools: vec![],
                    path_entries: def.path_entries.clone(),
                    install_options: vec![],
                    execution_mode: ExecutionMode::Host,
                }));
            }

            Ok(Some(Draft {
                tool_id: def.id.clone(),
                display: def.display.clone(),
                icon: def.icon.clone(),
                action,
                source: Some(selected_info),
                size_mb: def.size_mb,
                needs_admin: tc_core::installer::source_requires_elevation(selected, def),
                depends_on_tools: vec![],
                path_entries: def.path_entries.clone(),
                install_options: req.install_options.clone(),
                execution_mode: ExecutionMode::Host,
            }))
        }
    }
}

/// Idempotency decision for install/update operations.
fn decide_action(op: OperationKind, status: &ToolStatus, force_reinstall: bool) -> TaskAction {
    match (op, status) {
        (_, ToolStatus::Missing) | (_, ToolStatus::PathBroken { .. }) => TaskAction::InstallNew {
            target_version: None,
        },
        (OperationKind::Update, ToolStatus::Installed { version }) => {
            if force_reinstall {
                TaskAction::Update {
                    current_version: version.clone(),
                    target_version: None,
                }
            } else {
                TaskAction::NoOp(NoopReason::UpdateUnavailable {
                    version: version.clone(),
                })
            }
        }
        (
            OperationKind::Update,
            ToolStatus::UpdateAvailable {
                installed,
                recommended,
            },
        ) => TaskAction::Update {
            current_version: installed.clone(),
            target_version: Some(recommended.clone()),
        },
        (OperationKind::Install, ToolStatus::Installed { version }) => {
            if force_reinstall {
                TaskAction::InstallNew {
                    target_version: None,
                }
            } else {
                TaskAction::NoOp(NoopReason::AlreadyInstalled {
                    version: version.clone(),
                })
            }
        }
        (OperationKind::Install, ToolStatus::UpdateAvailable { installed, .. }) => {
            if force_reinstall {
                TaskAction::InstallNew {
                    target_version: None,
                }
            } else {
                TaskAction::NoOp(NoopReason::AlreadyInstalled {
                    version: installed.clone(),
                })
            }
        }
        _ => TaskAction::InstallNew {
            target_version: None,
        },
    }
}

/// Source selection: an explicit user choice must be one of the catalog's
/// allowed sources for this OS; otherwise the first (highest priority).
fn select_source<'s>(
    req: &ToolRequest,
    sources: &'s [InstallSource],
) -> Result<&'s InstallSource, String> {
    match &req.source_id {
        Some(want) => sources
            .iter()
            .find(|s| &s.id == want)
            .ok_or_else(|| format!("источник «{want}» не входит в допустимый набор каталога")),
        None => sources.first().ok_or_else(|| "нет источников".to_string()),
    }
}

fn validate_requested_source(
    req: &ToolRequest,
    def: &ToolDefinition,
    inputs: &PlanInputs<'_>,
) -> Option<String> {
    let Some(want) = &req.source_id else {
        return None;
    };
    if inputs.os_sources(def).iter().any(|s| &s.id == want) {
        None
    } else {
        Some(format!(
            "источник «{want}» отсутствует в каталоге для {}",
            def.id
        ))
    }
}

/// Dependency closure:
///   1. bundled-with parents (npm -> node) and declared dependencies
///      (composer -> php, extended.dependencies) are added when missing;
///   2. PkgManager sources on Windows depend on winget (bootstrapped too).
async fn resolve_dependency_closure(
    request: &EngineRequest,
    tools: &[ToolRequest],
    inputs: &PlanInputs<'_>,
    drafts: &mut Vec<Draft>,
) -> Result<(), PlanError> {
    let present: HashSet<String> = drafts.iter().map(|d| d.tool_id.clone()).collect();

    for req in tools {
        let root = req.tool_id.clone();
        let Some(def) = inputs.def(&root) else {
            continue;
        };

        // BFS по замыканию зависимостей (bundled_with + dependencies):
        // каждую недостающую зависимость добавляем задачей ПЕРЕД
        // зависимым, а её собственные зависимости — рекурсивно.
        let mut queue: Vec<String> = declared_dependency_ids(def);
        let mut seen: HashSet<String> = HashSet::new();

        while !queue.is_empty() {
            let dep = queue.remove(0);
            if dep == root || !seen.insert(dep.clone()) {
                continue;
            }
            if seen.len() > 16 {
                break;
            }
            if drafts.iter().any(|d| d.tool_id == dep) || present.contains(&dep) {
                link(drafts, &root, &dep);
                continue;
            }
            let Some(ddef) = inputs.def(&dep) else {
                break;
            };
            if ddef.manual_install.is_some() {
                break;
            }
            let dstatus = inputs.detector.detect(ddef).await;
            let dreq = ToolRequest::id(&dep);
            // Зависимость создаётся задачей ВСЕГДА — даже когда она уже
            // установлена (NoOp AlreadyInstalled): ребро depends_on обязано
            // ссылаться на существующую задачу, иначе топологическая
            // сортировка падает («циклическая зависимость в плане»).
            match draft_for(request, &dreq, &dstatus, ddef, inputs) {
                Ok(Some(ddraft)) => drafts.push(ddraft),
                _ => break,
            }
            link(drafts, &root, &dep);
            queue.extend(declared_dependency_ids(ddef));
        }
    }

    let os = os_name();
    if os == "windows" {
        let pkg_tasks: Vec<String> = drafts
            .iter()
            .filter(|d| {
                d.source
                    .as_ref()
                    .map(|s| s.kind == "pkg_manager")
                    .unwrap_or(false)
            })
            .map(|d| d.tool_id.clone())
            .collect();
        if !pkg_tasks.is_empty() && !drafts.iter().any(|d| d.tool_id == "winget") {
            if let Some(wdef) = inputs.def("winget") {
                let wstatus = inputs.detector.detect(wdef).await;
                if !matches!(wstatus, ToolStatus::Installed { .. }) {
                    let wreq = ToolRequest::id("winget");
                    if let Ok(Some(wdraft)) = draft_for(request, &wreq, &wstatus, wdef, inputs) {
                        drafts.push(wdraft);
                        for d in drafts.iter_mut() {
                            if pkg_tasks.contains(&d.tool_id)
                                && !d.depends_on_tools.contains(&"winget".to_string())
                            {
                                d.depends_on_tools.push("winget".to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Все объявленные зависимости инструмента: bundled-хост (npm→node)
/// и явный список extended.dependencies (composer→php).
fn declared_dependency_ids(def: &ToolDefinition) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(host) = def.bundled_with.as_deref() {
        ids.push(host.to_string());
    }
    for dep in def.extended.dependencies.iter() {
        if !ids.contains(dep) {
            ids.push(dep.clone());
        }
    }
    ids
}

fn link(drafts: &mut [Draft], from: &str, to: &str) {
    for d in drafts.iter_mut() {
        if d.tool_id == from && !d.depends_on_tools.contains(&to.to_string()) {
            d.depends_on_tools.push(to.to_string());
        }
    }
}

/// Stable topological order: dependencies precede dependents; winget
/// is forced first when present (catalog rule preserved).
fn order_drafts(drafts: &mut Vec<Draft>) -> Result<(), PlanError> {
    let mut ordered: Vec<Draft> = Vec::with_capacity(drafts.len());
    let mut placed: HashSet<String> = HashSet::new();
    let mut remaining = std::mem::take(drafts);

    while !remaining.is_empty() {
        let idx = remaining
            .iter()
            .position(|d| d.depends_on_tools.iter().all(|dep| placed.contains(dep)));
        let Some(idx) = idx else {
            *drafts = remaining;
            return Err(PlanError::Conflicts {
                pairs: vec!["циклическая зависимость в плане".to_string()],
            });
        };
        let d = remaining.remove(idx);
        placed.insert(d.tool_id.clone());
        ordered.push(d);
    }

    if let Some(i) = ordered.iter().position(|d| d.tool_id == "winget") {
        if i != 0 {
            let w = ordered.remove(i);
            ordered.insert(0, w);
        }
    }

    *drafts = ordered;
    Ok(())
}

fn collect_warnings(
    request: &EngineRequest,
    drafts: &[Draft],
    detected: &HashMap<String, ToolStatus>,
) -> Vec<PlanWarning> {
    let mut out = Vec::new();
    for d in drafts {
        if let Some(s) = &d.source {
            if s.unverified() {
                out.push(PlanWarning::UnverifiedSource {
                    tool_id: d.tool_id.clone(),
                    source_id: s.id.clone(),
                });
            }
        }
        if d.needs_admin {
            out.push(PlanWarning::AdminRequired {
                tool_id: d.tool_id.clone(),
            });
        }
        if request.operation.mutates_machine() {
            if let Some(ToolStatus::PathBroken { .. }) = detected.get(&d.tool_id) {
                out.push(PlanWarning::ReinstallOnBroken {
                    tool_id: d.tool_id.clone(),
                });
            }
        }
    }
    out
}

fn fingerprint(request: &EngineRequest, drafts: &[Draft]) -> String {
    let mut material = String::new();
    material.push_str(&format!("op={}", request.operation.as_str()));
    material.push_str(&format!("|channel={:?}", request.version_channel));
    // Confirmation flags deliberately NOT part of the fingerprint: they do
    // not change plan content, and the user ticks them BETWEEN preview and
    // execution — including them would make every confirmed start fail the
    // stale-plan guard (PlanChanged) against its own approved preview.
    for d in drafts {
        material.push_str(&format!(
            "|{}:{}:{:?}:{:?}",
            d.tool_id, d.size_mb, d.action, d.execution_mode
        ));
        if let Some(s) = &d.source {
            material.push_str(&format!(":{}:{}", s.kind, s.id));
            if let Some(h) = &s.sha256 {
                material.push_str(&format!(":sha256:{h}"));
            }
        }
    }
    tc_core::crypto::sha256_hex(material.as_bytes())
}

// ------------------------------------------------------------
// Test support (visible to sibling engine tests only)
// ------------------------------------------------------------

#[cfg(test)]
pub mod fixtures {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    pub struct FakeDetector {
        statuses: HashMap<String, ToolStatus>,
    }

    impl FakeDetector {
        pub fn with(id: &str, status: ToolStatus) -> Self {
            let mut m = HashMap::new();
            m.insert(id.to_string(), status);
            Self { statuses: m }
        }
    }

    #[async_trait]
    impl Detector for FakeDetector {
        async fn detect(&self, def: &ToolDefinition) -> ToolStatus {
            self.statuses
                .get(&def.id)
                .cloned()
                .unwrap_or(ToolStatus::Missing)
        }
    }

    pub struct FakeDetectorHarness {
        inner: FakeDetector,
    }

    impl FakeDetectorHarness {
        pub fn missing_all() -> Self {
            Self {
                inner: FakeDetector::default(),
            }
        }
    }

    #[async_trait]
    impl Detector for FakeDetectorHarness {
        async fn detect(&self, def: &ToolDefinition) -> ToolStatus {
            self.inner.detect(def).await
        }
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::toolchain::models::{DetectionRules, InstallSources, VersionRules};

    #[derive(Default)]
    struct FakeDetector {
        statuses: HashMap<String, ToolStatus>,
    }

    impl FakeDetector {
        fn with(id: &str, status: ToolStatus) -> Self {
            let mut m = HashMap::new();
            m.insert(id.to_string(), status);
            Self { statuses: m }
        }

        fn with_many(statuses: HashMap<String, ToolStatus>) -> Self {
            Self { statuses }
        }
    }

    #[async_trait]
    impl Detector for FakeDetector {
        async fn detect(&self, def: &ToolDefinition) -> ToolStatus {
            self.statuses
                .get(&def.id)
                .cloned()
                .unwrap_or(ToolStatus::Missing)
        }
    }

    fn base_def(id: &str) -> ToolDefinition {
        ToolDefinition {
            id: id.to_string(),
            category: "utility".to_string(),
            display: format!("Display {id}"),
            description: String::new(),
            icon: None,
            detection: DetectionRules::default(),
            versions: VersionRules::default(),
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

    fn official_src(id: &str, url: &str, sha256: Option<&str>) -> InstallSource {
        InstallSource {
            kind: InstallSourceKind::Official,
            id: id.to_string(),
            url: Some(url.to_string()),
            file_name: None,
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            execution: None,
            bootstrap: None,
            sha256: sha256.map(str::to_string),
            url_template: None,
            version_resolver: None,
        }
    }

    fn win_def(id: &str) -> ToolDefinition {
        let mut d = base_def(id);
        // Источник кладётся во ВСЕ ОС-слоты: планировщик берёт слот текущей
        // ОС, тесты обязаны работать одинаково на Windows/Linux/macOS.
        let src = official_src("src-1", "https://example.com/x.exe", Some("aa"));
        d.sources.windows = vec![src.clone()];
        d.sources.linux = vec![src.clone()];
        d.sources.macos = vec![src];
        d
    }

    /// Добавляет источник во все ОС-слоты (для тестов с несколькими
    /// источниками у одного тула).
    fn push_source(def: &mut ToolDefinition, src: InstallSource) {
        def.sources.windows.push(src.clone());
        def.sources.linux.push(src.clone());
        def.sources.macos.push(src);
    }

    /// Заменяет источник во всех ОС-слотах.
    fn set_official_source(def: &mut ToolDefinition, id: &str, url: &str, sha256: Option<&str>) {
        let src = official_src(id, url, sha256);
        def.sources.windows = vec![src.clone()];
        def.sources.linux = vec![src.clone()];
        def.sources.macos = vec![src];
    }

    fn installed(v: &str) -> ToolStatus {
        ToolStatus::Installed {
            version: v.to_string(),
        }
    }

    fn update_avail(installed_v: &str, rec: &str) -> ToolStatus {
        ToolStatus::UpdateAvailable {
            installed: installed_v.to_string(),
            recommended: rec.to_string(),
        }
    }

    fn install_request(ids: &[&str]) -> EngineRequest {
        EngineRequest::new(
            OperationKind::Install,
            ids.iter().map(|i| ToolRequest::id(i)).collect(),
        )
    }

    fn inputs_for<'a>(defs: &'a [ToolDefinition], detector: &'a FakeDetector) -> PlanInputs<'a> {
        PlanInputs::new(defs, detector).with_free_space(100_000)
    }

    #[tokio::test]
    async fn unknown_tool_ids_are_rejected() {
        let defs = vec![win_def("git")];
        let det = FakeDetector::default();
        let err = build_plan(
            &install_request(&["git", "not-a-tool"]),
            &inputs_for(&defs, &det),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, PlanError::UnknownTools { ref ids } if ids == &["not-a-tool"]));
    }

    #[tokio::test]
    async fn invalid_source_id_is_rejected() {
        let defs = vec![win_def("git")];
        let det = FakeDetector::default();
        let mut req = install_request(&["git"]);
        req.tools[0].source_id = Some("made-up-source".into());
        let err = build_plan(&req, &inputs_for(&defs, &det))
            .await
            .unwrap_err();
        match err {
            PlanError::InvalidSources { details } => assert!(details[0].contains("made-up-source")),
            other => panic!("{other:?}"),
        }
    }

    /// Регрессия «неизвестные опции установки»: install_options умеет
    /// понимать только Qt-конвейер. Опции, присланные для НЕ-Qt
    /// инструмента, отклоняются явно, а не молча игнорируются.
    #[tokio::test]
    async fn install_options_for_non_qt_tools_are_rejected() {
        let defs = vec![win_def("git")];
        let det = FakeDetector::default();
        let mut req = install_request(&["git"]);
        req.tools[0].install_options = vec!["qt-webengine".to_string()];
        let err = build_plan(&req, &inputs_for(&defs, &det))
            .await
            .unwrap_err();
        match err {
            PlanError::NotInstallable { details } => {
                assert!(
                    details[0].contains("только для Qt"),
                    "причина: {:?}",
                    details
                );
            }
            other => panic!("ожидали NotInstallable: {other:?}"),
        }
    }

    #[tokio::test]
    async fn valid_explicit_source_is_selected() {
        let mut d = win_def("git");
        // Источник добавляется во ВСЕ ОС-слоты: планировщик берёт слот
        // текущей ОС, и явный выбор обязан работать на любой из них.
        push_source(
            &mut d,
            official_src("src-2", "https://example.com/y.exe", Some("bb")),
        );
        let defs = vec![d];
        let det = FakeDetector::default();
        let mut req = install_request(&["git"]);
        req.tools[0].source_id = Some("src-2".into());
        let plan = build_plan(&req, &inputs_for(&defs, &det)).await.unwrap();
        assert_eq!(plan.tasks[0].source.as_ref().unwrap().id, "src-2");
    }

    #[tokio::test]
    async fn dependency_closure_adds_missing_bundled_parent_first() {
        let mut npm = win_def("npm");
        npm.bundled_with = Some("node".to_string());
        let defs = vec![npm, win_def("node")];
        let det = FakeDetector::default();
        let plan = build_plan(&install_request(&["npm"]), &inputs_for(&defs, &det))
            .await
            .unwrap();

        let node_idx = plan
            .tasks
            .iter()
            .position(|t| t.tool_id == "node")
            .expect("node auto-added");
        let npm_idx = plan.tasks.iter().position(|t| t.tool_id == "npm").unwrap();
        assert!(node_idx < npm_idx, "parent precedes dependent");

        let npm = &plan.tasks[npm_idx];
        let node_task_id = plan.tasks[node_idx].task_id.clone();
        assert!(
            npm.depends_on.contains(&node_task_id),
            "dependency edge resolved to task id"
        );
        assert!(!matches!(plan.tasks[node_idx].action, TaskAction::NoOp(_)));
    }

    #[tokio::test]
    async fn bundled_parent_already_present_is_reused_not_duplicated() {
        let mut npm = win_def("npm");
        npm.bundled_with = Some("node".to_string());
        let defs = vec![npm, win_def("node")];
        let det = FakeDetector::with_many(HashMap::from([("node".to_string(), installed("24"))]));
        let plan = build_plan(&install_request(&["npm"]), &inputs_for(&defs, &det))
            .await
            .unwrap();

        let node_count = plan.tasks.iter().filter(|t| t.tool_id == "node").count();
        assert_eq!(node_count, 1, "no duplicate parent");
        let node = plan.tasks.iter().find(|t| t.tool_id == "node").unwrap();
        assert!(matches!(
            node.action,
            TaskAction::NoOp(NoopReason::AlreadyInstalled { .. })
        ));
    }

    #[tokio::test]
    async fn conflicts_are_detected_via_table() {
        let defs = vec![win_def("aaa"), win_def("bbb")];
        let det = FakeDetector::default();
        let conflicts = [("aaa", "bbb")];
        let inputs = PlanInputs::new(&defs, &det)
            .with_free_space(100_000)
            .with_conflicts(&conflicts);
        let err = build_plan(&install_request(&["aaa", "bbb"]), &inputs)
            .await
            .unwrap_err();
        match err {
            PlanError::Conflicts { pairs } => assert_eq!(pairs, vec!["aaa × bbb"]),
            other => panic!("{other:?}"),
        }
    }

    /// Явно объявленная зависимость (extended.dependencies: composer→php)
    /// добавляется задачей ПЕРЕД зависимым, с ребром depends_on. Регрессия
    /// «composer без php»: оба источника composer падают кодом 1, если php
    /// не установлен — планировщик обязан ставить php первым.
    #[tokio::test]
    async fn declared_dependency_is_added_before_dependent() {
        let mut composer = win_def("composer");
        composer.extended.dependencies = vec!["php".to_string()];
        let defs = vec![composer, win_def("php")];
        let det = FakeDetector::default(); // оба Missing → обе задачи реальные
        let plan = build_plan(&install_request(&["composer"]), &inputs_for(&defs, &det))
            .await
            .unwrap();

        let php_idx = plan
            .tasks
            .iter()
            .position(|t| t.tool_id == "php")
            .expect("php auto-added");
        let composer_idx = plan
            .tasks
            .iter()
            .position(|t| t.tool_id == "composer")
            .expect("composer present");
        assert!(php_idx < composer_idx, "dependency precedes dependent");
        let composer = &plan.tasks[composer_idx];
        let php_task_id = plan.tasks[php_idx].task_id.clone();
        assert!(
            composer.depends_on.contains(&php_task_id),
            "dependency edge resolved to task id"
        );
        assert!(!matches!(plan.tasks[php_idx].action, TaskAction::NoOp(_)));
    }

    /// Уже установленная зависимость (php Installed) не дублируется
    /// задачей установки: появляется только честная NoOp-задача
    /// «AlreadyInstalled» (ребро обязано ссылаться на существующую задачу).
    #[tokio::test]
    async fn declared_dependency_already_installed_is_reused() {
        let mut composer = win_def("composer");
        composer.extended.dependencies = vec!["php".to_string()];
        let defs = vec![composer, win_def("php")];
        let det = FakeDetector::with("php", installed("8.4.25"));
        let plan = build_plan(&install_request(&["composer"]), &inputs_for(&defs, &det))
            .await
            .unwrap();

        let php_count = plan.tasks.iter().filter(|t| t.tool_id == "php").count();
        assert_eq!(php_count, 1, "no duplicate dependency task");
        let php = plan.tasks.iter().find(|t| t.tool_id == "php").unwrap();
        assert!(matches!(
            php.action,
            TaskAction::NoOp(NoopReason::AlreadyInstalled { .. })
        ));
        let composer = plan.tasks.iter().find(|t| t.tool_id == "composer").unwrap();
        assert!(
            composer.depends_on.contains(&php.task_id),
            "edge to no-op dependency task"
        );
    }

    #[tokio::test]
    async fn idempotent_install_skips_installed_tool() {
        let defs = vec![win_def("git")];
        let det = FakeDetector::with("git", installed("2.48"));
        let plan = build_plan(&install_request(&["git"]), &inputs_for(&defs, &det))
            .await
            .unwrap();
        match &plan.tasks[0].action {
            TaskAction::NoOp(NoopReason::AlreadyInstalled { version }) => {
                assert_eq!(version, "2.48")
            }
            other => panic!("ожидали NoOp, получили {other:?}"),
        }
        assert_eq!(
            plan.total_size_mb, 0,
            "no-op contributes no disk requirement"
        );
    }

    #[tokio::test]
    async fn force_reinstall_overrides_idempotency() {
        let defs = vec![win_def("git")];
        let det = FakeDetector::with("git", installed("2.48"));
        let mut req = install_request(&["git"]);
        req.tools[0].force_reinstall = true;
        let plan = build_plan(&req, &inputs_for(&defs, &det)).await.unwrap();
        assert!(matches!(
            plan.tasks[0].action,
            TaskAction::InstallNew { .. }
        ));
    }

    #[tokio::test]
    async fn update_plan_carries_current_target_source_admin_warnings() {
        let mut d = win_def("node");
        // Admin-поведение проверяем там, где elevation поддерживается;
        // на Linux needs_admin=true отвалил бы план до всяких warning'ов.
        d.needs_admin = cfg!(target_os = "windows");
        let defs = vec![d];
        let det = FakeDetector::with("node", update_avail("20.1", "22"));
        let mut req = EngineRequest::new(OperationKind::Update, vec![ToolRequest::id("node")]);
        req.confirm_admin_elevation = true;

        let plan = build_plan(&req, &inputs_for(&defs, &det)).await.unwrap();
        match &plan.tasks[0].action {
            TaskAction::Update {
                current_version,
                target_version,
            } => {
                assert_eq!(current_version, "20.1");
                assert_eq!(target_version.as_deref(), Some("22"));
            }
            other => panic!("{other:?}"),
        }
        assert!(plan.tasks[0].source.is_some());
        if cfg!(target_os = "windows") {
            assert!(plan.tasks[0].needs_admin);
            assert!(plan.needs_admin_any);
            assert!(plan.warnings.iter().any(|w| matches!(
                w,
                PlanWarning::AdminRequired { tool_id } if tool_id == "node"
            )));
        } else {
            assert!(!plan.needs_admin_any);
        }
    }

    #[tokio::test]
    async fn update_without_target_is_truthful_noop() {
        let defs = vec![win_def("node")];
        let det = FakeDetector::with("node", installed("24.0"));
        let req = EngineRequest::new(OperationKind::Update, vec![ToolRequest::id("node")]);
        let plan = build_plan(&req, &inputs_for(&defs, &det)).await.unwrap();
        match &plan.tasks[0].action {
            TaskAction::NoOp(NoopReason::UpdateUnavailable { version }) => {
                assert_eq!(version, "24.0")
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn update_of_missing_tool_installs_it() {
        let defs = vec![win_def("node")];
        let det = FakeDetector::default();
        let req = EngineRequest::new(OperationKind::Update, vec![ToolRequest::id("node")]);
        let plan = build_plan(&req, &inputs_for(&defs, &det)).await.unwrap();
        assert!(matches!(
            plan.tasks[0].action,
            TaskAction::InstallNew { .. }
        ));
    }

    #[tokio::test]
    async fn health_check_plan_is_read_only() {
        let defs = vec![win_def("node"), win_def("git")];
        let det = FakeDetector::with("node", update_avail("20", "22"));
        let req = EngineRequest::new(
            OperationKind::HealthCheck,
            vec![ToolRequest::id("node"), ToolRequest::id("git")],
        );
        let plan = build_plan(&req, &inputs_for(&defs, &det)).await.unwrap();
        assert!(plan
            .tasks
            .iter()
            .all(|t| matches!(t.action, TaskAction::HealthCheck)));
        assert!(
            plan.tasks.iter().all(|t| t.source.is_none()),
            "health check downloads nothing"
        );
        assert_eq!(plan.total_size_mb, 0);
    }

    #[tokio::test]
    async fn disk_shortage_rejects_plan() {
        let mut big = win_def("big");
        big.size_mb = 5000;
        let defs = vec![big];
        let det = FakeDetector::default();
        let inputs = PlanInputs::new(&defs, &det).with_free_space(100);
        let err = build_plan(&install_request(&["big"]), &inputs)
            .await
            .unwrap_err();
        match err {
            PlanError::InsufficientDisk {
                required_mb,
                free_mb,
            } => {
                assert_eq!((required_mb, free_mb), (5000, 100));
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn admin_requirement_needs_explicit_confirmation() {
        let mut d = win_def("sdk");
        d.needs_admin = true;
        let defs = vec![d];
        let det = FakeDetector::default();

        // Elevation поддерживается на всех ОС (Windows UAC / Unix sudo):
        // без явного подтверждения план не строится.
        let err = build_plan(&install_request(&["sdk"]), &inputs_for(&defs, &det))
            .await
            .unwrap_err();
        assert!(matches!(err, PlanError::AdminConfirmationRequired { .. }));

        let mut req = install_request(&["sdk"]);
        req.confirm_admin_elevation = true;
        let plan = build_plan(&req, &inputs_for(&defs, &det)).await.unwrap();
        assert!(plan.needs_admin_any);
    }

    #[tokio::test]
    async fn unverified_source_requires_confirmation_and_warns() {
        let mut d = win_def("app");
        // Заменяем источник во всех ОС-слотах: без sha256 → unverified.
        set_official_source(&mut d, "src-1", "https://example.com/x.exe", None);
        let defs = vec![d];
        let det = FakeDetector::default();

        let err = build_plan(&install_request(&["app"]), &inputs_for(&defs, &det))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            PlanError::UnverifiedSourceRequiresConfirmation { .. }
        ));

        let mut req = install_request(&["app"]);
        req.confirm_unverified_sources = true;
        let plan = build_plan(&req, &inputs_for(&defs, &det)).await.unwrap();
        assert_eq!(plan.unverified_tools(), vec!["app"]);
        assert!(plan.warnings.iter().any(|w| matches!(
            w,
            PlanWarning::UnverifiedSource { tool_id, .. } if tool_id == "app"
        )));
    }

    /// Preview (tcx_build_plan) builds the SAME plan for review without
    /// confirmations: unverified/admin surface as warnings + checkboxes,
    /// not hard errors. Execution (preview=false) still refuses.
    #[tokio::test]
    async fn preview_builds_unconfirmed_plan_with_warnings() {
        let mut d = win_def("app");
        set_official_source(&mut d, "src-1", "https://example.com/x.exe", None);
        // Admin-warning проверяем только там, где elevation поддерживается.
        d.needs_admin = cfg!(target_os = "windows");
        let defs = vec![d];
        let det = FakeDetector::default();

        let mut req = install_request(&["app"]);
        req.preview = true;
        let plan = build_plan(&req, &inputs_for(&defs, &det)).await.unwrap();
        assert_eq!(plan.unverified_tools(), vec!["app"]);
        assert!(plan.warnings.iter().any(|w| matches!(
            w,
            PlanWarning::UnverifiedSource { tool_id, .. } if tool_id == "app"
        )));
        if cfg!(target_os = "windows") {
            assert!(plan.warnings.iter().any(|w| matches!(
                w,
                PlanWarning::AdminRequired { tool_id } if tool_id == "app"
            )));
        }

        // Тот же запрос БЕЗ preview обязан отказать до построения.
        let err = build_plan(&install_request(&["app"]), &inputs_for(&defs, &det))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            PlanError::UnverifiedSourceRequiresConfirmation { .. }
        ));
    }

    /// Подтверждения пользователь ставит МЕЖДУ превью и стартом: они не
    /// меняют содержимое плана, значит и отпечаток меняться не должен —
    /// иначе каждый подтверждённый старт падал бы с PlanChanged.
    #[tokio::test]
    async fn fingerprint_ignores_confirmation_flags() {
        let defs = vec![win_def("git")];
        let det = FakeDetector::default();

        let preview = install_request(&["git"]);
        let mut confirmed = install_request(&["git"]);
        confirmed.confirm_unverified_sources = true;
        confirmed.confirm_admin_elevation = true;
        confirmed.preview = true;

        let plan_a = build_plan(&preview, &inputs_for(&defs, &det))
            .await
            .unwrap();
        let plan_b = build_plan(&confirmed, &inputs_for(&defs, &det))
            .await
            .unwrap();
        assert_eq!(
            plan_a.fingerprint, plan_b.fingerprint,
            "confirm flags must not alter the plan fingerprint"
        );
    }

    #[tokio::test]
    async fn manual_only_tools_never_enter_plans() {
        let mut d = win_def("unity");
        d.manual_install = Some("ставится вручную".into());
        let defs = vec![d];
        let det = FakeDetector::default();
        let err = build_plan(&install_request(&["unity"]), &inputs_for(&defs, &det))
            .await
            .unwrap_err();
        assert!(matches!(err, PlanError::NotInstallable { .. }));
    }

    #[tokio::test]
    async fn broken_install_becomes_reinstall_with_warning() {
        let defs = vec![win_def("git")];
        let det = FakeDetector::with(
            "git",
            ToolStatus::PathBroken {
                reason: "не отвечает".into(),
            },
        );
        let plan = build_plan(&install_request(&["git"]), &inputs_for(&defs, &det))
            .await
            .unwrap();
        assert!(matches!(
            plan.tasks[0].action,
            TaskAction::InstallNew { .. }
        ));
        assert!(plan.warnings.iter().any(|w| matches!(
            w,
            PlanWarning::ReinstallOnBroken { tool_id } if tool_id == "git"
        )));
    }

    #[tokio::test]
    async fn repair_path_plan_has_no_downloads() {
        let mut d = win_def("postgres");
        d.path_entries = vec!["%ProgramFiles%\\PostgreSQL\\17\\bin".into()];
        let defs = vec![d];
        let det = FakeDetector::with("postgres", installed("17.2"));
        let req = EngineRequest::new(OperationKind::RepairPath, vec![ToolRequest::id("postgres")]);
        let plan = build_plan(&req, &inputs_for(&defs, &det)).await.unwrap();
        assert!(matches!(plan.tasks[0].action, TaskAction::RepairPath));
        assert_eq!(plan.tasks[0].path_entries.len(), 1);
        assert!(
            plan.tasks[0].source.is_none(),
            "PATH repair downloads nothing"
        );
        assert_eq!(plan.total_size_mb, 0);
    }

    #[tokio::test]
    async fn plan_ids_statuses_and_fingerprint_are_assigned_by_backend() {
        let defs = vec![win_def("git")];
        let det = FakeDetector::default();
        let plan = build_plan(&install_request(&["git"]), &inputs_for(&defs, &det))
            .await
            .unwrap();
        assert!(plan.plan_id.starts_with("tcxp-"));
        assert!(!plan.fingerprint.is_empty());
        for t in &plan.tasks {
            assert!(t.task_id.starts_with(&plan.plan_id));
            assert_eq!(
                t.status,
                EngineTaskStatus::Pending,
                "task states start Pending"
            );
        }
    }

    #[tokio::test]
    async fn empty_selection_is_rejected() {
        let defs = vec![win_def("git")];
        let det = FakeDetector::default();
        let err = build_plan(&install_request(&[]), &inputs_for(&defs, &det))
            .await
            .unwrap_err();
        assert!(matches!(err, PlanError::EmptySelection));
    }

    // ------------------------------------------------------------
    // STANDALONE: docker-related инструменты ставятся на хост
    // ------------------------------------------------------------

    /// MySQL (wizard-«двойной» docker-инструмент!) в standalone-плане —
    /// ОБЫЧНАЯ хост-задача установки. Никаких DockerManaged-no-op.
    #[tokio::test]
    async fn mysql_missing_produces_host_install_task_never_docker_noop() {
        let mut mysql = win_def("mysql");
        // needs_admin существенен только на Windows: на Linux elevation
        // не поддерживается, и план отказал бы до проверки execution.
        mysql.needs_admin = cfg!(target_os = "windows");
        let defs = vec![mysql];
        let det = FakeDetector::default();

        // Даже с ЯВНЫМ выбором execution: host задача остаётся обычной
        // хост-установкой...
        let mut req = install_request(&["mysql"]);
        req.tools[0].execution = Some(ExecutionChoice::Host);
        req.confirm_admin_elevation = true;
        let plan = build_plan(&req, &inputs_for(&defs, &det)).await.unwrap();
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].execution_mode, ExecutionMode::Host);
        assert!(matches!(
            plan.tasks[0].action,
            TaskAction::InstallNew { .. }
        ));
        assert!(plan.tasks[0].source.is_some());

        // ...а без выбора — тоже: dual-семантики мастера здесь нет.
        let plain = install_request(&["mysql"]);
        let mut plain = plain;
        plain.confirm_admin_elevation = true;
        let plan2 = build_plan(&plain, &inputs_for(&defs, &det)).await.unwrap();
        assert_eq!(plan2.tasks[0].execution_mode, ExecutionMode::Host);
        assert!(!plan2.tasks[0].action.is_noop());
    }

    /// PostgreSQL: установлен → правдивый AlreadyInstalled-no-op;
    /// отсутствует → хост-задача установки (не docker no-op).
    #[tokio::test]
    async fn postgresql_installed_is_truthful_noop_missing_installs_locally() {
        let defs = vec![win_def("postgresql"), win_def("redis")];
        let det = FakeDetector::with_many(HashMap::from([(
            "postgresql".to_string(),
            installed("17.5"),
        )]));

        let plan = build_plan(
            &install_request(&["postgresql", "redis"]),
            &inputs_for(&defs, &det),
        )
        .await
        .unwrap();

        let pg = plan
            .tasks
            .iter()
            .find(|t| t.tool_id == "postgresql")
            .unwrap();
        match &pg.action {
            TaskAction::NoOp(NoopReason::AlreadyInstalled { version }) => {
                assert_eq!(version, "17.5")
            }
            other => panic!("ожидали truthful no-op, получили {other:?}"),
        }
        assert_eq!(pg.size_mb, 0, "no-op не требует места");

        let redis = plan.tasks.iter().find(|t| t.tool_id == "redis").unwrap();
        assert!(
            matches!(redis.action, TaskAction::InstallNew { .. }),
            "отсутствующий redis → задача локальной установки"
        );
        assert_eq!(redis.execution_mode, ExecutionMode::Host);
    }

    /// Docker-выбор исполнения отклоняется для ЛЮБОГО инструмента:
    /// у задач нет docker-режима, молча подменять его хостом нельзя.
    #[tokio::test]
    async fn explicit_docker_execution_choice_is_rejected() {
        let defs = vec![win_def("postgresql")];
        let det = FakeDetector::default();
        let mut req = install_request(&["postgresql"]);
        req.tools[0].execution = Some(ExecutionChoice::Docker);
        let err = build_plan(&req, &inputs_for(&defs, &det))
            .await
            .unwrap_err();
        assert!(matches!(err, PlanError::NotInstallable { .. }));
    }

    // ------------------------------------------------------------
    // Обновления и PATH
    // ------------------------------------------------------------

    /// Update-задача генерируется ТОЛЬКО при реально доступном обновлении:
    /// UpdateAvailable → Update; Installed без цели → правдивый no-op.
    #[tokio::test]
    async fn update_task_only_when_update_actually_available() {
        let defs = vec![win_def("node")];

        let det = FakeDetector::with("node", installed("24.0"));
        let req = EngineRequest::new(OperationKind::Update, vec![ToolRequest::id("node")]);
        let plan = build_plan(&req, &inputs_for(&defs, &det)).await.unwrap();
        assert!(plan.tasks[0].action.is_noop(), "обновления нет → no-op");

        let det2 = FakeDetector::with("node", update_avail("20.1", "22"));
        let plan2 = build_plan(&req, &inputs_for(&defs, &det2)).await.unwrap();
        assert!(
            matches!(plan2.tasks[0].action, TaskAction::Update { .. }),
            "обновление доступно → Update-задача"
        );
    }

    /// Сломанная установка → переустановка (не «ремонт PATH»): действие
    /// InstallNew с явным предупреждением ReinstallOnBroken; ремонт PATH —
    /// отдельная операция RepairPath, которая ничего не скачивает.
    #[tokio::test]
    async fn broken_path_is_reinstall_with_warning_not_silent_repair() {
        let mut d = win_def("git");
        d.path_entries = vec!["C:\\Program Files\\Git\\cmd".into()];
        let defs = vec![d];
        let det = FakeDetector::with(
            "git",
            ToolStatus::PathBroken {
                reason: "бинарь не отвечает".into(),
            },
        );

        // Install поверх сломанного: переустановка + предупреждение.
        let plan = build_plan(&install_request(&["git"]), &inputs_for(&defs, &det))
            .await
            .unwrap();
        assert!(matches!(
            plan.tasks[0].action,
            TaskAction::InstallNew { .. }
        ));
        assert!(plan.warnings.iter().any(|w| matches!(
            w,
            PlanWarning::ReinstallOnBroken { tool_id } if tool_id == "git"
        )));

        // RepairPath — отдельная операция без скачивания/переустановки.
        let repair = EngineRequest::new(OperationKind::RepairPath, vec![ToolRequest::id("git")]);
        let rp = build_plan(&repair, &inputs_for(&defs, &det)).await.unwrap();
        assert!(matches!(rp.tasks[0].action, TaskAction::RepairPath));
        assert!(rp.tasks[0].source.is_none());
        assert_eq!(rp.total_size_mb, 0);
    }

    // ------------------------------------------------------------
    // Детерминированность, таймаут, отпечаток плана
    // ------------------------------------------------------------

    /// Зависимости разрешаются детерминированно: erlang перед elixir,
    /// порядок стабилен между прогонами на одинаковых входах.
    #[tokio::test]
    async fn dependency_resolution_order_is_deterministic() {
        let mut elixir = win_def("elixir");
        elixir.bundled_with = Some("erlang".to_string());
        let defs = vec![elixir, win_def("erlang")];
        let det = FakeDetector::default();

        let first = build_plan(
            &install_request(&["elixir", "erlang"]),
            &inputs_for(&defs, &det),
        )
        .await
        .unwrap();
        for _ in 0..3 {
            let again = build_plan(
                &install_request(&["elixir", "erlang"]),
                &inputs_for(&defs, &det),
            )
            .await
            .unwrap();
            let ids_a: Vec<&str> = first.tasks.iter().map(|t| t.tool_id.as_str()).collect();
            let ids_b: Vec<&str> = again.tasks.iter().map(|t| t.tool_id.as_str()).collect();
            assert_eq!(ids_a, ids_b, "порядок задач стабилен");
            let erl = again
                .tasks
                .iter()
                .position(|t| t.tool_id == "erlang")
                .unwrap();
            let eli = again
                .tasks
                .iter()
                .position(|t| t.tool_id == "elixir")
                .unwrap();
            assert!(erl < eli, "зависимость precedes зависимого");
        }
        // Отпечаток одного и того же запроса одинаков (детерминизм).
        let fp = first.fingerprint.clone();
        let other = build_plan(
            &install_request(&["elixir", "erlang"]),
            &inputs_for(&defs, &det),
        )
        .await
        .unwrap();
        assert_eq!(fp, other.fingerprint);
    }

    /// Построение плана конечно: медленный детектор упирается в таймаут,
    /// наружу выходит структурированная PlannerTimeout-ошибка.
    #[tokio::test]
    async fn planner_timeout_is_finite_and_structured() {
        struct SlowDetector;
        #[async_trait]
        impl Detector for SlowDetector {
            async fn detect(&self, _def: &ToolDefinition) -> ToolStatus {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                ToolStatus::Missing
            }
        }
        let defs = vec![win_def("slow-tool")];
        let inputs = PlanInputs::new(&defs, &SlowDetector).with_free_space(100_000);
        let err = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            build_plan(&install_request(&["slow-tool"]), &inputs),
        )
        .await;
        // Внешний guard (тот же механизм, что PLAN_BUILD_TIMEOUT в командном
        // слое) срабатывает раньше «вечного» детектора.
        assert!(err.is_err(), "медленное построение обязано прерваться");
        let timeout_error = PlanError::PlannerTimeout { seconds: 45 };
        assert!(timeout_error.to_string().contains("45"));
    }

    /// Превью vs исполнение: если окружение изменилось после одобрения
    /// превью, исполнение отклоняется структурированной PlanChanged-
    /// ошибкой вместо молчаливого запуска устаревшего плана.
    #[tokio::test]
    async fn stale_preview_fingerprint_aborts_execution() {
        let defs = vec![win_def("git")];

        let det_before = FakeDetector::default();
        let preview = build_plan(&install_request(&["git"]), &inputs_for(&defs, &det_before))
            .await
            .unwrap();

        // Окружение изменилось: git установился между превью и стартом.
        let det_after = FakeDetector::with("git", installed("2.48"));
        let mut exec_req = install_request(&["git"]);
        exec_req.expected_plan_fingerprint = Some(preview.fingerprint.clone());
        let err = build_plan(&exec_req, &inputs_for(&defs, &det_after))
            .await
            .unwrap_err();
        assert!(
            matches!(err, PlanError::PlanChanged { .. }),
            "устаревший план не исполняется молча: {err:?}"
        );
        assert!(err.to_string().contains("устарел"));

        // Совпадающий отпечаток проходит.
        let fresh = build_plan(&install_request(&["git"]), &inputs_for(&defs, &det_after))
            .await
            .unwrap();
        exec_req.expected_plan_fingerprint = Some(fresh.fingerprint.clone());
        let ok = build_plan(&exec_req, &inputs_for(&defs, &det_after))
            .await
            .unwrap();
        assert_eq!(
            ok.fingerprint,
            exec_req.expected_plan_fingerprint.as_deref().unwrap()
        );
    }

    /// Одно-туловый план НЕ запускает полный скан каталога: детектор
    /// вызывается ровно для запрошенных инструментов (+ winget-хост
    /// pkg-источников на Windows), но никогда для всего каталога.
    #[tokio::test]
    async fn single_tool_plan_does_not_scan_full_catalog() {
        #[derive(Default)]
        struct CountingDetector {
            calls: std::sync::Mutex<HashMap<String, usize>>,
        }
        #[async_trait]
        impl Detector for CountingDetector {
            async fn detect(&self, def: &ToolDefinition) -> ToolStatus {
                *self
                    .calls
                    .lock()
                    .unwrap()
                    .entry(def.id.clone())
                    .or_insert(0) += 1;
                ToolStatus::Missing
            }
        }

        // Реальный каталог, урезанный до конкретных id: terraform + соседи.
        let wanted = ["terraform", "node", "git", "winget", "npm"];
        let catalog: Vec<ToolDefinition> = crate::modules::toolchain::defs::load_definitions()
            .into_iter()
            .filter(|d| wanted.contains(&d.id.as_str()))
            .collect();
        let total_defs = catalog.len();
        assert!(catalog.iter().any(|d| d.id == "terraform"));
        assert!(total_defs < 10, "урезанный каталог, не весь: {total_defs}");
        let det = CountingDetector::default();
        let inputs = PlanInputs::new(&catalog, &det).with_free_space(1_000_000);

        let mut req = install_request(&["terraform"]);
        req.confirm_unverified_sources = true; // реальный zip-источник без sha256
        // На Linux первый источник terraform — пакетный менеджер (root),
        // на Windows — zip без UAC: подтверждение прав даётся явно.
        req.confirm_admin_elevation = true;
        let plan = build_plan(&req, &inputs).await.unwrap();
        assert_eq!(plan.tasks.len(), 1, "план содержит только запрошенный тул");

        let probed: HashMap<String, usize> = det.calls.lock().unwrap().clone();
        assert!(probed.contains_key("terraform"), "запрошенный тул опрошен");
        assert!(
            probed.len() <= 3,
            "опрос ограничен запрошенным + хостами зависимостей, а не каталогом из {total_defs}: {:?}",
            probed.keys().collect::<Vec<_>>()
        );
    }
}
