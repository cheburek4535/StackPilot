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
// Все guard'ы рантаймные: `in` только после isRecord — малиформированный
// payload из IPC обязан давать «неизвестное состояние», а не краш.
// ------------------------------------------------------------

/** Безопасное извлечение строкового поля объекта-payload. */
function fieldString(obj: unknown, key: string): string | null {
  if (!isRecord(obj)) return null;
  const value = obj[key];
  return typeof value === "string" ? value : null;
}

export function statusIsOk(status: unknown): boolean {
  // `in` допустим только на проверенном объекте — не на unknown снаружи.
  if (!isRecord(status)) return false;
  return "Installed" in status;
}

export function statusLabel(status: unknown): string {
  if (status === "Missing") return "Не установлен";
  if (status === "RunInDocker") return "В Docker (docker-compose)";
  if (isRecord(status)) {
    const keys = Object.keys(status);
    if (keys.length === 1) {
      switch (keys[0]) {
        case "Installed": {
          const version = fieldString(status.Installed, "version");
          return version !== null ? `✓ ${version}` : "✓ Установлен";
        }
        case "UpdateAvailable": {
          const payload = status.UpdateAvailable;
          const recommended = fieldString(payload, "recommended") ?? "?";
          return `Обновить до ${recommended}`;
        }
        case "ManualInstall": {
          const reason = fieldString(status.ManualInstall, "reason") ?? "";
          return reason ? `⚠ Вручную: ${reason}` : "⚠ Требуется ручная установка";
        }
        case "PathBroken": {
          const reason = fieldString(status.PathBroken, "reason") ?? "";
          return reason ? `⚠ ${reason}` : "⚠ PATH сломан";
        }
      }
    }
  }
  return "Неизвестное состояние";
}

export function statusKind(status: unknown): "ok" | "update" | "broken" | "missing" | "manual" | "docker" {
  if (typeof status === "string") {
    if (status === "Missing") return "missing";
    if (status === "RunInDocker") return "docker";
  }
  if (isRecord(status)) {
    const key = Object.keys(status)[0];
    if (key === "Installed") return "ok";
    if (key === "UpdateAvailable") return "update";
    if (key === "ManualInstall") return "manual";
    if (key === "PathBroken") return "broken";
  }
  return "missing";
}

const LEGACY_PHASE_LABELS: Record<string, string> = {
  Downloading: "Скачивание…",
  Installing: "Установка…",
  Verifying: "Проверка…",
  UpdatingPath: "Обновление PATH…",
};

export function taskStateKind(state: unknown): "pending" | "running" | "success" | "failed" | "skipped" {
  if (state === "Pending") return "pending";
  if (isRecord(state)) {
    const key = Object.keys(state)[0];
    if (key === "Running") return "running";
    if (key === "Success") return "success";
    if (key === "Failed") return "failed";
    if (key === "Skipped") return "skipped";
  }
  // Малиформированное состояние — безопасный нейтральный «в очереди».
  return "pending";
}

export function taskStateLabel(state: unknown): string {
  if (state === "Pending") return "В очереди";
  if (isRecord(state)) {
    const key = Object.keys(state)[0];
    switch (key) {
      case "Running": {
        const phase = fieldString(state.Running, "phase") ?? "";
        return LEGACY_PHASE_LABELS[phase] ?? "Выполняется…";
      }
      case "Success": {
        const version = fieldString(state.Success, "version");
        return version ? `Готово (${version})` : "Готово";
      }
      case "Failed": {
        const error = fieldString(state.Failed, "error");
        return error ? `Ошибка: ${error}` : "Ошибка (причина неизвестна)";
      }
      case "Skipped": {
        const reason = fieldString(state.Skipped, "reason");
        return reason ? `Пропущено: ${reason}` : "Пропущено (причина не указана)";
      }
    }
  }
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
  | { kind: "unsupported_platform" };

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
  if (!isRecord(raw)) return null;
  const kind = raw.kind;
  if (typeof kind !== "string") return null;
  if (!(TOOL_STATE_KINDS as readonly string[]).includes(kind)) return null;
  return raw as ToolState;
}

/** Откуда взята улика об установке. */
export type EvidenceKind =
  | { kind: "version_probe" }
  | { kind: "known_path" }
  | { kind: "footprint" };

/**
 * Где бинарь установки доступен относительно PATH — правда по слоям,
 * а не единый «сломан/не сломан» (null — старый снапшот без поля).
 */
export type PathScope =
  | { kind: "process_path" }
  | { kind: "persisted_path_only" }
  | { kind: "outside_path" };

/** Журнал одной пробы обнаружения (полный лог для UI). */
export type ProbeLog = {
  command: string[];
  stdout: string;
  stderr: string;
  exit_code: number | null;
  timed_out: boolean;
  not_found: boolean;
  launch_error: string | null;
  duration_ms: number;
};

/** Одна конкретная установка инструмента (их может быть несколько).
 *  Поля-дополнения опциональны: старые кэши снапшотов их не содержат,
 *  и отсутствие поля честно трактуется как «нет данных», а не догадка. */
export type DetectedInstall = {
  raw_version: string;
  parsed_version: string | null;
  location: string;
  evidence: EvidenceKind;
  reachable_via_path: boolean;
  /** Слои PATH; undefined/null — поле из старого снапшота отсутствует. */
  path_scope?: PathScope | null;
  /** Журнал пробы, породившей улику; undefined — старый снапшот/след. */
  probe_log?: ProbeLog | null;
};

/** Итог живого обнаружения: failed ≠ not_detected ≠ detected.
 *  detected — положительный результат: улики есть в installs
 *  (аддитивное расширение контракта §8; старые payload'ы читаются). */
export type DetectionOutcome =
  | { kind: "pending" }
  | { kind: "not_detected" }
  | { kind: "detected" }
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
  | { kind: "unsupported_on_platform" }
  /** Только для живых заготовок идущего скана (бэкенд это не шлёт):
   *  честное «данных нет» вместо выдуманного installable. */
  | { kind: "unknown" };

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
  /** Компактная безопасная сводка (одна строка). */
  detail: string;
  duration_ms: number;
  // --- Поля-дополнения (старые кэши их не содержат → undefined честно) ---
  /** Команда проверки (идентичность того, что запускалось). */
  command?: string[];
  /** Полный санитизированный stdout проверки (лента логов UI). */
  stdout?: string;
  /** Полный санитизированный stderr (daemon-диагностика docker и т.п.). */
  stderr?: string;
  /** Код выхода процесса, когда он выполнился. */
  exit_code?: number | null;
  /** true — проверка не уложилась в таймаут (процесс убит). */
  timed_out?: boolean;
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
 *  (внешний тег — serde по умолчанию). */
export type ScanStartOutcome =
  | { Started: ScanJobSnapshot }
  | { AlreadyRunning: ScanJobSnapshot };

/** Безопасное извлечение задания из reconnect-исхода; малиформация → null. */
export function scanStartJobId(outcome: unknown): ScanJobSnapshot | null {
  if (!isRecord(outcome)) return null;
  const started = outcome.Started ?? outcome.AlreadyRunning;
  return isRecord(started) && typeof started.job_id === "string"
    ? (started as ScanJobSnapshot)
    : null;
}

export type ScoreSummary = {
  score: number;
  counted_tools: number;
  healthy: number;
  degraded: number;
  broken: number;
  missing: number;
  unhealthy: number;
  scan_failed: number;
  /** Не опрошены (частичный отчёт) — «не проверено», не «сломано». */
  scan_pending?: number;
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
  /** Индекс канонической установки в installs (undefined — старый кэш). */
  canonical_install?: number | null;
  /** Почему выбрана каноническая установка (объяснение для UI). */
  version_selected_because?: string;
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

export type VersionChannel = "recommended" | "latest";

/** Ограниченный запрос на один инструмент: только id и явные выборы.
 *  `execution` зеркалит бэкенд-выбор Host/Docker; standalone-планировщик
 *  отклоняет docker (молчаливой подмены хостом нет), UI его не шлёт. */
export type ToolRequest = {
  tool_id: string;
  source_id?: string | null;
  execution?: ExecutionMode | null;
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
  /** Превью (tcx_build_plan): план строится для просмотра, подтверждения
   *  показываются чекбоксами, а не ошибками; исполнение требует их явно. */
  preview?: boolean;
  /** Отпечаток одобренного превью: расхождение с freshly-built планом
   * отклоняется бэкендом (PlanChanged) вместо молчаливого исполнения. */
  expected_plan_fingerprint?: string | null;
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

/** Почему задача не требует работы — всегда правдиво.
 *  Малиформированные причины (undefined/null/чужой ключ) НЕ крашат
 *  потребителя: guard возвращает null и UI показывает «неизвестно». */
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

// ------------------------------------------------------------
// Runtime-границы IPC: НИКОГДА не использовать `in` на неизвестном/
// возможно-undefined значении — только через эти guard'ы.
// ------------------------------------------------------------

/** true только для настоящих объектов (null/массивы — не объекты здесь). */
export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Вид действия задачи по внешнему тегу; малиформация → null (не краш).
 *  Канонический ключ noop; «no_op» — наследие старой сериализации
 *  (персистентные записи заданий): оба читаются одинаково. */
export function taskActionKind(action: unknown): string | null {
  if (typeof action === "string") {
    return action === "repair_path" || action === "health_check" ? action : null;
  }
  if (!isRecord(action)) return null;
  const keys = Object.keys(action);
  if (keys.length !== 1) return null;
  const key = keys[0];
  return ["install_new", "update", "noop", "no_op"].includes(key) ? key : null;
}

/**
 * Безопасный разбор действия задачи из недоверенного payload.
 * Возвращает null для undefined/null/неизвестных вариантов/битого noop —
 * вызывающий рендерит «Неизвестное действие», а не падает.
 */
export function parseTaskAction(raw: unknown): TaskAction | null {
  if (typeof raw === "string") {
    return raw === "repair_path" || raw === "health_check" ? raw : null;
  }
  if (!isRecord(raw)) return null;
  const keys = Object.keys(raw);
  if (keys.length !== 1) return null;
  switch (keys[0]) {
    case "install_new": {
      const payload = raw.install_new;
      if (!isRecord(payload)) return null;
      const target = payload.target_version;
      return {
        install_new: { target_version: typeof target === "string" ? target : null },
      };
    }
    case "update": {
      const payload = raw.update;
      if (!isRecord(payload)) return null;
      const current = payload.current_version;
      if (typeof current !== "string") return null;
      const target = payload.target_version;
      return {
        update: {
          current_version: current,
          target_version: typeof target === "string" ? target : null,
        },
      };
    }
    case "noop":
    case "no_op": {
      // noop без причины / с неизвестной причиной — НЕ выдумываем её:
      // guard возвращает null и UI честно скажет «причина неизвестна».
      const reason = raw[keys[0]];
      if (typeof reason === "string") {
        return reason === "docker_managed" ? { noop: reason } : null;
      }
      if (!isRecord(reason)) return null; // noop без reason — не выдумываем её
      const rKeys = Object.keys(reason);
      if (rKeys.length !== 1) return null;
      switch (rKeys[0]) {
        case "already_installed":
        case "update_unavailable": {
          const inner = reason[rKeys[0]];
          const version = isRecord(inner) && typeof inner.version === "string" ? inner.version : "";
          return { noop: { [rKeys[0]]: { version } } } as TaskAction;
        }
        default:
          return null; // неизвестная причина — пусть UI скажет «неизвестно»
      }
    }
    default:
      return null;
  }
}

export function taskActionIsNoop(action: unknown): boolean {
  const kind = taskActionKind(action);
  return kind === "noop" || kind === "no_op";
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

/** Безопасный вид статуса задачи; малиформация → "unknown" (не краш). */
export function engineTaskStatusKind(status: unknown): string {
  if (typeof status === "string") return status;
  if (isRecord(status)) {
    const keys = Object.keys(status);
    if (keys.length === 1) return keys[0];
  }
  return "unknown";
}

export function engineTaskStatusIsTerminal(status: unknown): boolean {
  const kind = engineTaskStatusKind(status);
  return kind !== "pending" && kind !== "running" && kind !== "unknown";
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

/** Виды payload событий задания (безопасное ключевое множество для guard'ов). */
export type JobEventPayloadKind =
  | "job_started"
  | "task_started"
  | "task_phase"
  | "progress"
  | "path_updated"
  | "task_completed"
  | "job_finished";

/** Событие задания: без идентичности (job_id/seq) не существует. */
export type JobEvent = {
  job_id: string;
  task_id: string;
  tool_id: string;
  seq: number;
  timestamp: string;
  payload: JobEventPayload;
};

/** Безопасный вид payload события; малиформация → null (не краш). */
export function jobEventKind(event: unknown): JobEventPayloadKind | null {
  if (!isRecord(event)) return null;
  const payload = event.payload;
  if (!isRecord(payload)) return null;
  const keys = Object.keys(payload);
  if (keys.length !== 1) return null;
  const known: JobEventPayloadKind[] = [
    "job_started",
    "task_started",
    "task_phase",
    "progress",
    "path_updated",
    "task_completed",
    "job_finished",
  ];
  return known.includes(keys[0] as JobEventPayloadKind) ? (keys[0] as JobEventPayloadKind) : null;
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
  /** Только инструменты с источником локальной установки на этой ОС. */
  installable: boolean;
  /** Только инструменты с заявленной Docker-альтернативой. */
  has_docker_alternative: boolean;
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
