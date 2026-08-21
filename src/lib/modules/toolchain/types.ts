// ============================================================
// Toolchain — TypeScript mirrors of the backend contracts
// ============================================================
// Все поля snake_case: так сериализует serde (см. project_creator/types.ts).
// Top-level аргументы invoke() при этом camelCase (соглашение Tauri v2).
//
// Во время миграции сосуществуют ДВА протокола (контракт §7–§8):
//
//  A. ЛЕГАСИ (`tc_*`, Project Creator) — serde-формат по умолчанию:
//     внешне тегированные PascalCase-варианты ("Missing", {"Installed": …}).
//     Эти типы должны оставаться байт-совместимыми (раздел A).
//
//  B. КАНОН (`tcx_*`, Control Center) — контракт §8:
//     - доменные перечисления (domain/*): внутренний тег
//       {"kind": "snake_case", ...поля}; unit-варианты — объекты из одного
//       поля {"kind": "missing"}, НИКОГДА не голые строки;
//     - движковые перечисления (engine/*): ВНЕШНИЙ тег snake_case
//       ({"already_installed": {...}} / "docker_managed") — так их реально
//       сериализует бэкенд; зафиксировано как отступление от общего правила
//       §8.1 в docs/toolchain-progress.md;
//     - enum'ы без данных — плоские snake_case-строки.
//
// Правило различения для B: читаем `kind` (или единственный ключ внешнего
// тега), никогда не «снифаем форму» через `"X" in status`. Неизвестный
// kind рендерится как «неизвестное состояние», но не роняет UI
// (см. parseToolState / toolStateInfo).

// ############################################################
// РАЗДЕЛ A — легаси-типы tc_* (Project Creator; байт-совместимость)
// ############################################################

export type ToolStatus =
  | "Missing"
  | { Installed: { version: string } }
  | { UpdateAvailable: { installed: string; recommended: string } }
  | { PathBroken: { reason: string } }
  | { ManualInstall: { reason: string } }
  | "RunInDocker";

/** Способ исполнения источника (serde rename_all = snake_case). */
export type ExecutionKind =
  | "git_clone"
  | "exe"
  | "script"
  | "phar"
  | "archive"
  | "auto";

/** Чем ставим (serde по умолчанию → PascalCase-строки). */
export type InstallSourceKind = "PkgManager" | "Official" | "Script" | "QtOnline";

export type InstallSource = {
  kind: InstallSourceKind;
  id: string;
  url: string | null;
  args: string[];
  extra_args: string[];
  dynamic_args: boolean;
  /** Имя файла для сохранения (rustup-init.exe и т.п.). */
  file_name?: string | null;
  /** Куда распаковывать архив / клонировать репозиторий. */
  install_dir?: string | null;
  /** Переопределение needs_admin для конкретного источника. */
  needs_admin?: boolean | null;
  /** Явный способ исполнения (иначе автоопределение). */
  execution?: ExecutionKind | null;
  /** SHA-256 (hex); отсутствие = источник без контроля целостности. */
  sha256?: string | null;
};

export type DetectionRules = {
  version_probes: string[][];
  known_paths: string[];
  registry_keys: string[];
};

export type VersionRules = { min: string | null; recommended: string | null };

export type HealthCheckDef = { label: string; command: string[] };

/** Заявленные возможности обслуживания: отсутствующее поле = «не заявлено». */
export type DeclaredCapabilities = {
  removable?: boolean | null;
  repairable?: boolean | null;
};

export type DockerCapability = {
  image?: string | null;
  notes?: string | null;
};

/**
 * Расширенные метаданные каталога. Бэкенд сериализует их «вплоскую»
 * (serde flatten) с skip_serializing_if — поля могут отсутствовать.
 */
export type ToolDefinition = {
  id: string;
  category: string;
  display: string;
  description: string;
  icon: string | null;
  detection: DetectionRules;
  versions: VersionRules;
  sources: {
    windows: InstallSource[];
    linux: InstallSource[];
    macos: InstallSource[];
  };
  size_mb: number;
  needs_admin: boolean;
  path_entries: string[];
  bundled_with: string | null;
  health_checks: HealthCheckDef[];
  notes: string | null;
  /** Ручная установка: текст предупреждения (движки, SDK). */
  manual_install?: string | null;
  // --- flattened ToolExtendedMetadata (могут отсутствовать) ---
  aliases?: string[];
  dependencies?: string[];
  conflicts?: string[];
  docs_url?: string | null;
  source_url?: string | null;
  platform_availability?: string[];
  declared_capabilities?: DeclaredCapabilities;
  docker?: DockerCapability | null;
};

/** Что бэкенд реально умеет на текущей ОС. */
export type PlatformCapabilities = {
  install_execution_supported: boolean;
  elevation_supported: boolean;
};

export type ProjectRequirements = {
  languages: string[];
  frameworks: string[];
  tools: string[];
  /** Docker-инструменты мастера (postgresql, redis, ...), выбранные для
   * ЛОКАЛЬНОЙ установки вместо docker-compose. */
  local_infra_tools: string[];
  git_init: boolean;
  vscode_config: boolean;
  docker: boolean;
};

export type ToolRequirement = {
  tool_id: string;
  display: string;
  category: string;
  /** Имя файла иконки из tools.json (рендерится как /images/<icon>) */
  icon: string | null;
  status: ToolStatus;
  size_mb: number;
  needs_admin: boolean;
  source_description: string;
  install_options?: string[];
};

export type EnvironmentCheck = {
  os: string;
  requirements: ToolRequirement[];
  /** Опциональные требования: docker-инструменты мастера, которые по
   * умолчанию разворачиваются контейнерами проекта. */
  optional_requirements: ToolRequirement[];
  total_size_mb: number;
  free_space_mb: number;
  enough_space: boolean;
  needs_admin_any: boolean;
  all_ready: boolean;
  /** true — опрошены все инструменты; false — отчёт ЧАСТИЧНЫЙ. */
  complete?: boolean;
  /** Инструменты, не успевшие провериться до дедлайна. */
  scan_timed_out?: string[];
};

export type EnvironmentInfo = {
  os: string;
  os_version: string;
  package_managers: string[];
  tool_count: number;
  capabilities?: PlatformCapabilities;
};

// ------------------------------------------------------------
// Легаси-план установки и сессия
// ------------------------------------------------------------

export type TaskPhase = "Downloading" | "Installing" | "Verifying" | "UpdatingPath";

export type TaskState =
  | "Pending"
  | { Running: { phase: TaskPhase } }
  | { Success: { version: string } }
  | { Failed: { error: string } }
  | { Skipped: { reason: string } };

export type InstallTask = {
  task_id: string;
  tool_id: string;
  display: string;
  /** Имя файла иконки из tools.json (рендерится как /images/<icon>) */
  icon: string | null;
  size_mb: number;
  needs_admin: boolean;
  source_description: string;
  install_options?: string[];
  state: TaskState;
};

export type InstallPlan = {
  tasks: InstallTask[];
  total_size_mb: number;
  os: string;
  /** Идентификатор установки: события чужой сессии отбрасываются. */
  session_id?: string;
};

/** Авторитетное состояние сессии (serde rename_all = snake_case). */
export type InstallSessionStatus =
  | "running"
  | "completed"
  | "failed"
  | "cancelled"
  | "interrupted";

export type InstallSession = {
  started_at: string;
  running: boolean;
  plan: InstallPlan;
  status?: InstallSessionStatus;
  // СЕКРЕТОВ ЗДЕСЬ НЕТ: бэкенд сериализует их с #[serde(skip)] и выдаёт
  // только одноразово через tc_take_new_secrets (getNewSecrets()).
};

// ------------------------------------------------------------
// Легаси-события (toolchain:* )
// ------------------------------------------------------------

export type ToolchainEvent = {
  event_type:
    | "TaskStarted"
    | { TaskPhaseChanged: { phase: TaskPhase } }
    | { TaskProgress: { line: string } }
    | { TaskCompleted: { state: TaskState } }
    | { AllCompleted: { success_count: number; failed: string[] } }
    | { Error: { message: string } };
  task_index: number;
  total_tasks: number;
  task_id: string;
  tool_id: string;
  timestamp: string;
  /** Идентификатор установки-владельца события. */
  session_id?: string;
};

// Payload события toolchain:install_done — финальный InstallPlan
// с состояниями всех задач (source of truth по завершении).

export type CheckProgressEvent = {
  done: number;
  total: number;
  tool_id: string;
  display: string;
  /** Имя файла иконки из tools.json (рендерится как /images/<icon>) */
  icon: string | null;
  status: ToolStatus;
  /** Идентификатор запуска проверки: чужие запуски фильтруются. */
  scan_id?: string;
};

// ------------------------------------------------------------
// Легаси-metadata / health
// ------------------------------------------------------------

export type InstalledToolInfo = {
  path: string;
  version: string;
  installed_at: string;
  path_entries: string[];
};

/**
 * Выдача tc_get_metadata — санитизированное представление
 * (ToolchainMetadataView): СЕКРЕТОВ в ответе нет и быть не может.
 */
export type ToolchainMetadata = {
  last_scan: string | null;
  tools: Record<string, InstalledToolInfo>;
  prefs: Record<string, string>;
  /** Явно усыновлённые инструменты (tcx_adopt_tool): track-метка. */
  adopted?: Record<string, string>;
};

export type HealthCheckResult = { label: string; ok: boolean; detail: string };

/** Легаси-состояние здоровья (serde по умолчанию → PascalCase). */
export type HealthStateV1 = "NotChecked" | "Healthy" | "Failed" | "Unavailable";

export type ToolHealth = {
  tool_id: string;
  display: string;
  /** Имя файла иконки из tools.json (рендерится как /images/<icon>) */
  icon: string | null;
  checks: HealthCheckResult[];
  state?: HealthStateV1;
  /** Совместимость: true только при Healthy. */
  ok: boolean;
};

export type HealthReport = { tools: ToolHealth[]; score: number; scanned_at: string };

// ------------------------------------------------------------
// Легаси-хелперы дискриминаторов (используются Project Creator)
// ------------------------------------------------------------

export function statusIsOk(status: ToolStatus | undefined): boolean {
  if (!status || status === "Missing" || status === "RunInDocker") return false;
  return "Installed" in status;
}

export function statusLabel(status: ToolStatus): string {
  if (status === "Missing") return "Не установлен";
  if (status === "RunInDocker") return "В Docker (docker-compose)";
  if ("Installed" in status) return `✓ ${status.Installed.version}`;
  if ("UpdateAvailable" in status) return `Обновить до ${status.UpdateAvailable.recommended}`;
  if ("ManualInstall" in status) return `⚠ Вручную: ${status.ManualInstall.reason}`;
  return `⚠ ${status.PathBroken.reason}`;
}

export function statusKind(status: ToolStatus): "ok" | "update" | "broken" | "missing" | "manual" | "docker" {
  if (status === "Missing") return "missing";
  if (status === "RunInDocker") return "docker";
  if ("Installed" in status) return "ok";
  if ("UpdateAvailable" in status) return "update";
  if ("ManualInstall" in status) return "manual";
  return "broken";
}

export function taskStateKind(state: TaskState): "pending" | "running" | "success" | "failed" | "skipped" {
  if (state === "Pending") return "pending";
  if ("Running" in state) return "running";
  if ("Success" in state) return "success";
  if ("Failed" in state) return "failed";
  return "skipped";
}

export function taskStateLabel(state: TaskState): string {
  if (state === "Pending") return "В очереди";
  if ("Running" in state) {
    const phase = state.Running.phase;
    if (phase === "Downloading") return "Скачивание…";
    if (phase === "Installing") return "Установка…";
    if (phase === "Verifying") return "Проверка…";
    if (phase === "UpdatingPath") return "Обновление PATH…";
  }
  if ("Success" in state) return `Готово (${state.Success.version})`;
  if ("Failed" in state) return `Ошибка: ${state.Failed.error}`;
  if ("Skipped" in state) return `Пропущено: ${state.Skipped.reason}`;
  return "В очереди";
}

// ############################################################
// РАЗДЕЛ B — канонические доменные модели (domain/*, контракт §8)
// ############################################################
// Единый внутренний тег {"kind": ...}; unit-вариант — объект из одного
// поля. Различение ТОЛЬКО по kind.

/** Презентационное состояние инструмента (композиция размерностей). */
export type ToolState =
  | { kind: "scan_pending" }
  | { kind: "scan_failed"; reason: string }
  | { kind: "missing" }
  | { kind: "installed_healthy"; version: string }
  | { kind: "installed_health_unknown"; version: string }
  | { kind: "installed_unhealthy"; version: string }
  | { kind: "update_available"; installed: string; recommended: string }
  | { kind: "path_broken"; reason: string }
  | { kind: "manual_install"; reason: string }
  | { kind: "docker_managed" }
  | { kind: "built_in_system" }
  | { kind: "unsupported_platform" }
  | { kind: "install_unavailable" };

export const TOOL_STATE_KINDS = [
  "scan_pending",
  "scan_failed",
  "missing",
  "installed_healthy",
  "installed_health_unknown",
  "installed_unhealthy",
  "update_available",
  "path_broken",
  "manual_install",
  "docker_managed",
  "built_in_system",
  "unsupported_platform",
  "install_unavailable",
] as const;

export type ToolStateKind = (typeof TOOL_STATE_KINDS)[number];

export function toolStateKind(state: ToolState): ToolStateKind {
  return state.kind;
}

/**
 * Безопасный разбор состояния из недоверенного payload (события, кэш).
 * Неизвестный kind → null: вызывающий рендерит «неизвестное состояние»,
 * а не падает (контракт §8.1).
 */
export function parseToolState(raw: unknown): ToolState | null {
  if (typeof raw !== "object" || raw === null) return null;
  const kind = (raw as { kind?: unknown }).kind;
  if (typeof kind !== "string") return null;
  if (!(TOOL_STATE_KINDS as readonly string[]).includes(kind)) return null;
  return raw as ToolState;
}

/** Откуда взята улика об установке. */
export type EvidenceKind =
  | { kind: "version_probe" }
  | { kind: "known_path" }
  | { kind: "footprint" };

/** Одна конкретная установка инструмента (их может быть несколько). */
export type DetectedInstall = {
  raw_version: string;
  parsed_version: string | null;
  location: string;
  evidence: EvidenceKind;
  reachable_via_path: boolean;
};

/** Итог живого обнаружения: failed ≠ not_detected. */
export type DetectionOutcome =
  | { kind: "pending" }
  | { kind: "not_detected" }
  | { kind: "failed"; reason: string };

/** Оценка версии относительно политики каталога (advisory). */
export type VersionAssessment =
  | { kind: "unknown" }
  | { kind: "unparseable" }
  | { kind: "meets_recommended" }
  | { kind: "below_recommended" }
  | { kind: "below_min" }
  | { kind: "policy_violation" };

/** Происхождение установки (кто поставил) — 7 видов. */
export type Provenance =
  | { kind: "stack_pilot_managed" }
  | { kind: "external" }
  | { kind: "package_manager" }
  | { kind: "system" }
  | { kind: "bundled_with"; tool: string }
  | { kind: "docker" }
  | { kind: "unknown" };

export type ProvenanceKind = Provenance["kind"];

/** Применимость инструмента к текущей платформе. */
export type PlatformApplicability =
  | { kind: "installable" }
  | { kind: "manual_only" }
  | { kind: "docker_default" }
  | { kind: "built_in" }
  | { kind: "unsupported_on_platform" };

/**
 * Явное состояние здоровья — 9 состояний (контракт §8.3).
 * Отсутствие данных — «не проверяли», а НЕ вердикт; ошибка запуска
 * проверки — «не смогли проверить», а не «сломано».
 */
export type HealthState =
  | { kind: "not_checked" }
  | { kind: "checking" }
  | { kind: "healthy" }
  | { kind: "degraded" }
  | { kind: "unhealthy" }
  | { kind: "unavailable" }
  | { kind: "unsupported" }
  | { kind: "no_checks_defined" }
  | { kind: "failed_to_run" };

/** Только Healthy/Degraded/Unhealthy — финальные вердикты. */
export function healthIsVerdict(state: HealthState): boolean {
  return (
    state.kind === "healthy" || state.kind === "degraded" || state.kind === "unhealthy"
  );
}

export type HealthCheckResultV2 = {
  label: string;
  passed: boolean;
  /** true — процесс не выполнился (запуск/таймаут); false — провал условия. */
  process_failed: boolean;
  detail: string;
  duration_ms: number;
};

export type HealthOutcome = {
  state: HealthState;
  results: HealthCheckResultV2[];
};

/** Совместимое имя (старые импорты этого модуля). */
export type HealthOutcomeV2 = HealthOutcome;

/** Восемь независимых флагов возможностей платформы (контракт §8.3). */
export type ToolPlatformCapabilities = {
  detectable: boolean;
  installable: boolean;
  updatable: boolean;
  removable: boolean;
  repairable: boolean;
  health_checkable: boolean;
  manual_instructions_available: boolean;
  docker_alternative_available: boolean;
};

// ------------------------------------------------------------
// Диагностика PATH (unit-only enum → плоские snake_case-строки)
// ------------------------------------------------------------

export type PathFindingKind =
  | "binary_not_on_path"
  | "entry_missing"
  | "stale_entry"
  | "duplicate_entry"
  | "case_duplicate_entry"
  | "requires_expansion"
  | "unverifiable_entry";

export type PathFinding = {
  kind: PathFindingKind;
  entry: string;
  detail: string;
};

export type PathEntryReport = {
  raw: string;
  expanded: string;
  exists: boolean;
  verifiable: boolean;
  duplicate_of: number | null;
  case_duplicate_of: number | null;
  requires_expansion: boolean;
};

export type PathReport = {
  entries: PathEntryReport[];
  findings: PathFinding[];
};

// ------------------------------------------------------------
// Задание скана и события скана
// ------------------------------------------------------------

/** Фазы скана (serde по умолчанию → PascalCase-строки). */
export type ScanPhase = "Queued" | "Environment" | "Tools" | "Finalizing" | "Done";

/** Терминальные/живые состояния задания скана. */
export type ScanTerminal =
  | "Running"
  | "Completed"
  | "Partial"
  | "Cancelled"
  | "Failed"
  | "Interrupted";

export function scanIsTerminal(terminal: ScanTerminal): boolean {
  return terminal !== "Running";
}

export type ScanJobSnapshot = {
  job_id: string;
  scan_id: string;
  started_at: string;
  updated_at: string;
  finished_at: string | null;
  phase: ScanPhase;
  total_tools: number;
  completed_tools: number;
  current_tool: string | null;
  running: boolean;
  cancel_requested: boolean;
  terminal: ScanTerminal;
  recovered: boolean;
};

/** Результат tcx_start_scan: новый запуск или reconnect к идущему
 * (внешний тег — serde по умолчанию). */
export type ScanStartOutcome =
  | { Started: ScanJobSnapshot }
  | { AlreadyRunning: ScanJobSnapshot };

export function scanStartJobId(outcome: ScanStartOutcome): ScanJobSnapshot {
  return "Started" in outcome ? outcome.Started : outcome.AlreadyRunning;
}

export type ScoreSummary = {
  score: number;
  counted_tools: number;
  healthy_required: number;
  degraded: number;
  missing_required: number;
  broken_required: number;
  unhealthy_required: number;
  scan_failed: number;
  unchecked: number;
  optional: number;
  not_applicable: number;
};

/** Количества инструментов по каждому презентационному состоянию. */
export type StatusCounts = {
  scan_pending: number;
  scan_failed: number;
  missing: number;
  installed_healthy: number;
  installed_health_unknown: number;
  installed_unhealthy: number;
  update_available: number;
  path_broken: number;
  manual_install: number;
  docker_managed: number;
  built_in_system: number;
  unsupported_platform: number;
  install_unavailable: number;
};

export type DiskSpaceInfo = { root: string; free_mb: number };

export type AdminCapability = {
  elevation_supported: boolean;
  required_by_tools: boolean;
};

/** Предупреждение/ошибка снапшота (без секретов по построению). */
export type SnapshotIssue = { code: string; message: string };

export type ToolScanResult = {
  tool_id: string;
  display: string;
  category: string;
  icon: string | null;
  detection: DetectionOutcome;
  installs: DetectedInstall[];
  path_findings: PathFinding[];
  /** null — ещё не добрались (ScanPending). */
  health: HealthOutcome | null;
  applicability: PlatformApplicability;
  capabilities: ToolPlatformCapabilities;
  provenance: Provenance;
  bundled_with: string | null;
  version_assessment: VersionAssessment;
  state: ToolState;
  error: string | null;
  duration_ms: number;
};

/** Полный снапшот окружения — результат одного скана. */
export type EnvironmentSnapshot = {
  snapshot_id: string;
  job_id: string;
  scan_id: string;
  os: string;
  os_version: string;
  arch: string;
  package_managers: string[];
  disk: DiskSpaceInfo[];
  admin: AdminCapability;
  started_at: string;
  finished_at: string;
  complete: boolean;
  cancelled: boolean;
  tools: ToolScanResult[];
  path_report: PathReport;
  score: ScoreSummary;
  summary: StatusCounts;
  warnings: SnapshotIssue[];
  errors: SnapshotIssue[];
  /** id заданий, активных в момент выдачи (данные могли устареть). */
  active_jobs: string[];
  age_seconds: number;
  stale: boolean;
  from_cache: boolean;
};

// Событие toolchainx:scan_progress — строго с идентичностью операции
export type ScanProgressEvent = {
  job_id: string;
  scan_id: string;
  completed_count: number;
  total_count: number;
  tool_id: string;
  display_name: string;
  icon: string | null;
  tool_state: string;
  timestamp: string;
  error: string | null;
};

// Событие toolchainx:scan_done
export type ScanDoneEvent = {
  job_id: string;
  scan_id: string;
  terminal: ScanTerminal;
  completed: number;
  total: number;
};

// ############################################################
// РАЗДЕЛ C — движок заданий (engine/*)
// ############################################################
// ВАЖНО: эти перечисления сериализуются ВНЕШНИМ тегом snake_case
// (serde-умолчание + rename_all) — см. примечание в шапке файла.

export type OperationKind = "install" | "update" | "repair_path" | "health_check";

export function operationMutatesMachine(op: OperationKind): boolean {
  return op === "install" || op === "update" || op === "repair_path";
}

export type ExecutionChoice = "host" | "docker";
export type VersionChannel = "recommended" | "latest";

/** Ограниченный запрос на один инструмент: только id и явные выборы. */
export type ToolRequest = {
  tool_id: string;
  source_id?: string | null;
  execution?: ExecutionChoice | null;
  install_options?: string[];
  force_reinstall?: boolean;
};

/** Единственная форма запроса, принимаемая бэкендом (deny_unknown_fields):
 * URL/пути/аргументы/версии/состояния физически невыразимы. */
export type EngineRequest = {
  operation: OperationKind;
  tools?: ToolRequest[];
  version_channel?: VersionChannel | null;
  confirm_unverified_sources?: boolean;
  confirm_admin_elevation?: boolean;
};

/** Typed job/task phases (12 значений, плоские строки). */
export type Phase =
  | "validating"
  | "preparing"
  | "downloading"
  | "verifying"
  | "installing"
  | "configuring"
  | "updating_path"
  | "checking_health"
  | "completed"
  | "failed"
  | "cancelled"
  | "interrupted";

/** Терминальные/живые статусы задания (плоские строки). */
export type JobStatus =
  | "queued"
  | "running"
  | "succeeded"
  | "partial"
  | "failed"
  | "cancelled"
  | "interrupted";

export function jobStatusIsTerminal(status: JobStatus): boolean {
  return (
    status === "succeeded" ||
    status === "partial" ||
    status === "failed" ||
    status === "cancelled" ||
    status === "interrupted"
  );
}

/** Почему задача не требует работы — всегда правдиво. */
export type NoopReason =
  | { already_installed: { version: string } }
  | { update_unavailable: { version: string } }
  | "docker_managed";

/** Что исполнитель делает для задачи (внешний тег snake_case). */
export type TaskAction =
  | { install_new: { target_version: string | null } }
  | { update: { current_version: string; target_version: string | null } }
  | "repair_path"
  | "health_check"
  | { noop: NoopReason };

export function taskActionIsNoop(action: TaskAction): boolean {
  return typeof action === "object" && "noop" in action;
}

export type ExecutionMode = "host" | "docker";

/** Источник, выбранный бэкендом: URL остаётся внутри каталога. */
export type SelectedSource = {
  kind: string;
  id: string;
  description: string;
  /** null = источник без контроля целостности (нужно подтверждение). */
  sha256: string | null;
  needs_admin: boolean;
};

export function selectedSourceUnverified(source: SelectedSource): boolean {
  return source.sha256 === null;
}

/** Живое состояние одной задачи плана (внешний тег snake_case). */
export type EngineTaskStatus =
  | "pending"
  | { running: { phase: Phase } }
  | { succeeded: { version: string } }
  | { failed: { error: string } }
  | "cancelled"
  | "interrupted";

export function engineTaskStatusKind(status: EngineTaskStatus): string {
  if (typeof status === "string") return status;
  return Object.keys(status)[0];
}

export function engineTaskStatusIsTerminal(status: EngineTaskStatus): boolean {
  const kind = engineTaskStatusKind(status);
  return kind !== "pending" && kind !== "running";
}

/** Одна каноническая задача: только факты, разрешённые бэкендом. */
export type PlanTask = {
  task_id: string;
  tool_id: string;
  display: string;
  icon: string | null;
  action: TaskAction;
  source: SelectedSource | null;
  size_mb: number;
  needs_admin: boolean;
  depends_on: string[];
  /** PATH-записи применяются только одобренным заданием (аудит). */
  path_entries: string[];
  install_options: string[];
  execution_mode: ExecutionMode;
  status: EngineTaskStatus;
};

/** Предупреждения экрана подтверждения (внешний тег snake_case). */
export type PlanWarning =
  | { unverified_source: { tool_id: string; source_id: string } }
  | { admin_required: { tool_id: string } }
  | { reinstall_on_broken: { tool_id: string } };

/** Аудированное изменение PATH (before/after). */
export type PathChangeRecord = {
  tool_id: string;
  added: string[];
};

/** Канонический план. Производится исключительно бэкендом. */
export type CanonicalPlan = {
  plan_id: string;
  operation: OperationKind;
  os: string;
  created_at: string;
  /** Отпечаток входов плана; перепроверяется при исполнении. */
  fingerprint: string;
  tasks: PlanTask[];
  total_size_mb: number;
  free_space_mb: number;
  enough_space: boolean;
  needs_admin_any: boolean;
  capabilities: PlatformCapabilities;
  warnings: PlanWarning[];
};

/** Персистентная запись задания — переживает перезапуск приложения. */
export type PersistedJob = {
  job_id: string;
  plan_id: string;
  operation: OperationKind;
  requested_tool_ids: string[];
  source_choices: Record<string, string>;
  created_at: string;
  started_at: string | null;
  updated_at: string | null;
  finished_at: string | null;
  status: JobStatus;
  plan: CanonicalPlan;
  errors: string[];
  path_changes: PathChangeRecord[];
  recovered: boolean;
  // Поля секретов НЕ СУЩЕСТВУЕТ (assert_secret_free на бэкенде).
};

/** Типизированный payload события задания (внешний тег snake_case). */
export type JobEventPayload =
  | { job_started: { operation: OperationKind; total_tasks: number } }
  | { task_started: { index: number; total: number } }
  | { task_phase: { phase: Phase } }
  | { progress: { line: string } }
  | { path_updated: { record: PathChangeRecord } }
  | { task_completed: { status: EngineTaskStatus } }
  | { job_finished: { status: JobStatus; errors: string[] } };

export type JobEventPayloadKind = keyof JobEventPayload;

/** Событие задания: без идентичности (job_id/seq) не существует. */
export type JobEvent = {
  job_id: string;
  task_id: string;
  tool_id: string;
  seq: number;
  timestamp: string;
  payload: JobEventPayload;
};

export function jobEventKind(event: JobEvent): JobEventPayloadKind {
  return Object.keys(event.payload)[0] as JobEventPayloadKind;
}

/** Consumer-side guard: игнорировать всё от чужих заданий. */
export function jobEventBelongsTo(event: JobEvent, jobId: string): boolean {
  return event.job_id === jobId;
}

// ############################################################
// РАЗДЕЛ D — профиль окружения (Build Environment, domain/profile.rs)
// ############################################################

export type ProfileTool = {
  tool_id: string;
  display: string;
  category: string;
  icon: string | null;
  size_mb: number;
  needs_admin: boolean;
  source_description: string;
  /** Почему включён (вычисляет бэкенд): явный выбор / зависимость / bundled. */
  reason?: string;
};

export type ProfileConflict = { tool: string; conflicts_with: string };

export type ProfileWarning = { code: string; message: string };

/** Готовность: satisfied_count заполняется только наложением снапшота. */
export type ReadinessSummary = {
  required_total: number;
  satisfied_count: number | null;
  all_ready: boolean;
};

export type EnvironmentProfile = {
  created_at: string;
  os: string;
  requirements: ProjectRequirements;
  required: ProfileTool[];
  /** Зарезервировано: пусто, пока в каталоге нет метаданных рекомендаций. */
  recommended: ProfileTool[];
  optional: ProfileTool[];
  docker_managed: ProfileTool[];
  local_alternatives: string[];
  manual: ProfileTool[];
  unsupported: ProfileTool[];
  conflicts: ProfileConflict[];
  dependency_closure: string[];
  estimated_download_size_mb: number;
  warnings: ProfileWarning[];
  readiness: ReadinessSummary;
};

// ############################################################
// РАЗДЕЛ E — фильтры/пагинация каталога (UI-метаданные)
// ############################################################

export type CatalogFilters = {
  search: string;
  categories: string[];
  states: ToolStateKind[];
  provenance: ProvenanceKind[];
  /** Требуемые флаги возможностей (И-логика между разными флагами). */
  capabilities: (keyof ToolPlatformCapabilities)[];
  execution_modes: ExecutionMode[];
  /** Требуемые состояния здоровья (healthy/degraded/unhealthy/...). */
  health: HealthState["kind"][];
  /** Только инструменты, требующие прав администратора (факт каталога). */
  admin_only: boolean;
  /** Только с доступным обновлением. */
  update_only: boolean;
  /** Только ручная установка (применимость manual_only). */
  manual_only: boolean;
};

/** Порядок сортировки каталога Manage Everything. */
export type CatalogSort =
  | "catalog"
  | "name_asc"
  | "name_desc"
  | "category"
  | "status";

export type PageRequest = { offset: number; limit: number };

export type PageMeta = {
  total: number;
  filtered: number;
  offset: number;
  limit: number;
};

export type Paged<T> = { items: T[]; meta: PageMeta };
