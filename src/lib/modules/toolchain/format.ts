// ============================================================
// Toolchain — форматирование и подписи (UI-слой, чистые функции)
// ============================================================
// Все подписи — на языке интерфейса приложения (русский), тоны —
// варианты Badge из дизайн-системы (src/lib/components/ui/Badge.svelte):
// "neutral" | "violet" | "cyan" | "blue" | "lime" | "amber" | "red".
//
// Правила честности:
//  - неизвестный kind НИКОГДА не роняет форматтер: отдаётся
//    «Неизвестное состояние» с нейтральным тоном;
//  - отсутствие данных («не проверяли») не приукрашивается.

import type {
  DetectedInstall,
  HealthCheckResultV2,
  HealthState,
  InstallSource,
  JobStatus,
  OperationKind,
  PathScope,
  Phase,
  PlanTask,
  ProbeLog,
  Provenance,
  ProvenanceKind,
  SelectedSource,
  ToolPlatformCapabilities,
  ToolScanResult,
  ToolState,
  ToolStateKind,
  VersionAssessment,
} from "./types";
import { isRecord } from "./types";

export type InfoTone = "neutral" | "violet" | "cyan" | "blue" | "lime" | "amber" | "red";

export type StateInfo = { label: string; tone: InfoTone; short?: string };

// ------------------------------------------------------------
// Размеры: bytes / MB / GB
// ------------------------------------------------------------

/** Форматирует мегабайты в человекочитаемый размер (МБ/ГБ). */
export function formatSizeMb(mb: number | null | undefined): string {
  if (mb == null || Number.isNaN(mb)) return "—";
  if (mb <= 0) return "0 МБ";
  if (mb < 1024) return `${Math.round(mb)} МБ`;
  const gb = mb / 1024;
  return `${gb >= 100 ? Math.round(gb) : gb.toFixed(1)} ГБ`;
}

/** Форматирует байты (Б/КБ/МБ/ГБ) — для будущих потоков загрузки. */
export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || Number.isNaN(bytes) || bytes < 0) return "—";
  if (bytes < 1024) return `${Math.round(bytes)} Б`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(kb >= 100 ? 0 : 1)} КБ`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb.toFixed(mb >= 100 ? 0 : 1)} МБ`;
  const gb = mb / 1024;
  return `${gb.toFixed(gb >= 100 ? 0 : 1)} ГБ`;
}

// ------------------------------------------------------------
// Относительное время
// ------------------------------------------------------------

/** «только что», «5 мин назад», «2 ч назад», «3 дн назад», дата. */
export function formatRelativeTime(
  timestamp: string | number | Date | null | undefined,
  now: Date = new Date(),
): string {
  if (timestamp == null) return "—";
  const date = timestamp instanceof Date ? timestamp : new Date(timestamp);
  const ms = date.getTime();
  if (Number.isNaN(ms)) return "—";
  const diffSec = Math.max(0, Math.round((now.getTime() - ms) / 1000));
  if (diffSec < 45) return "только что";
  const diffMin = Math.round(diffSec / 60);
  if (diffMin < 60) return `${diffMin} мин назад`;
  const diffH = Math.round(diffMin / 60);
  if (diffH < 24) return `${diffH} ч назад`;
  const diffD = Math.round(diffH / 24);
  if (diffD < 30) return `${diffD} дн назад`;
  try {
    return date.toLocaleDateString("ru-RU");
  } catch {
    return date.toISOString().slice(0, 10);
  }
}

/** Возраст снапшота по age_seconds (поле бэкенда). */
export function formatAgeSeconds(seconds: number | null | undefined): string {
  if (seconds == null || Number.isNaN(seconds)) return "—";
  if (seconds < 45) return "только что";
  if (seconds < 60) return `${Math.round(seconds)} с назад`;
  return formatRelativeTime(new Date(Date.now() - seconds * 1000));
}

// ------------------------------------------------------------
// Презентационное состояние инструмента (ToolState)
// ------------------------------------------------------------

const TOOL_STATE_INFO: { [K in ToolState["kind"]]: StateInfo } = {
  scan_pending: { label: "Проверяется…", tone: "neutral", short: "…" },
  scan_failed: { label: "Ошибка проверки", tone: "amber", short: "?" },
  missing: { label: "Не установлен", tone: "neutral", short: "—" },
  installed_healthy: { label: "Установлен", tone: "lime", short: "✓" },
  installed_health_unknown: { label: "Здоровье не проверялось", tone: "cyan", short: "?" },
  installed_unhealthy: { label: "Нездоров", tone: "red", short: "✕" },
  update_available: { label: "Доступно обновление", tone: "amber", short: "↑" },
  path_broken: { label: "PATH сломан", tone: "red", short: "!" },
  manual_install: { label: "Только вручную", tone: "violet", short: "✋" },
  docker_managed: { label: "В Docker", tone: "blue", short: "🐳" },
  built_in_system: { label: "Встроен в ОС", tone: "neutral", short: "•" },
  unsupported_platform: { label: "Не поддерживается", tone: "neutral", short: "×" },
};

/** Подпись+тон состояния; неизвестный kind → честное «неизвестно». */
export function toolStateInfo(state: ToolState): StateInfo {
  const known = (TOOL_STATE_INFO as Record<string, StateInfo | undefined>)[state.kind];
  if (known) {
    // Детали варианта — в подсказку/детали, здесь только класс состояния.
    return known;
  }
  return { label: "Неизвестное состояние", tone: "neutral", short: "?" };
}

/** Вариант по kind (для фильтров/счётчиков, где объекта состояния нет). */
export function toolStateKindInfo(kind: ToolStateKind): StateInfo {
  const known = (TOOL_STATE_INFO as Record<string, StateInfo | undefined>)[kind];
  return known ?? { label: "Неизвестное состояние", tone: "neutral" };
}

/** Все kind состояний с подписями — для фильтра состояний. */
export function allToolStateKinds(): { kind: ToolStateKind; info: StateInfo }[] {
  return (
    Object.keys(TOOL_STATE_INFO) as ToolStateKind[]
  ).map((kind) => ({ kind, info: TOOL_STATE_INFO[kind] }));
}

export function toolStateLabel(state: ToolState): string {
  return toolStateInfo(state).label;
}

/** Версия из состояния (если вариант её несёт). */
export function toolStateVersion(state: ToolState): string | null {
  switch (state.kind) {
    case "installed_healthy":
    case "installed_health_unknown":
    case "installed_unhealthy":
      return state.version || null;
    case "update_available":
      return state.installed || null;
    default:
      return null;
  }
}

// ------------------------------------------------------------
// Версия инструмента по всем уликам (карта/дровер/снапшот)
// ------------------------------------------------------------

export type VersionDisplay = {
  /** Текст для показа; никогда не пустая строка при installed=true. */
  text: string;
  /** true — версия разобрана; false — сырой вывод, НЕ разобранный парсером. */
  parsed: boolean;
};

/**
 * Показываемая версия установленного инструмента. Приоритет: версия из
 * состояния → каноническая установка → первая улика с выводом.
 * Непарсируемая версия показывается КАК СЫРОЙ ВЫВОД (parsed=false) —
 * молча подставлять пустую строку запрещено контрактом.
 */
export function toolVersionDisplay(tool: ToolScanResult | null | undefined): VersionDisplay | null {
  if (!tool) return null;

  // 1. Версия из презентационного состояния.
  const fromState = toolStateVersion(tool.state);
  if (fromState) return { text: fromState, parsed: true };

  const installs: unknown = tool.installs;
  if (!Array.isArray(installs)) return null;

  // 2. Каноническая установка (бэкенд уже выбрал и объяснил выбор).
  const canonicalIndex =
    typeof tool.canonical_install === "number" ? tool.canonical_install : null;
  const candidates: DetectedInstall[] = installs.filter(isRecord) as unknown as DetectedInstall[];
  const canonical =
    canonicalIndex !== null ? candidates[canonicalIndex] ?? null : null;
  const withOutput =
    canonical && (canonical.parsed_version || canonical.raw_version)
      ? canonical
      : candidates.find((i) => i.parsed_version || i.raw_version) ?? null;

  if (!withOutput) return null;
  if (withOutput.parsed_version) return { text: withOutput.parsed_version, parsed: true };
  if (withOutput.raw_version) return { text: withOutput.raw_version, parsed: false };
  return null;
}

/**
 * Полный текст лога одной пробы обнаружения (для <pre> в дровере).
 */
export function probeLogText(log: ProbeLog | null | undefined): string {
  if (!log) return "";
  const lines: string[] = [];
  lines.push(`$ ${log.command.join(" ")}`);
  if (log.timed_out) lines.push("[таймаут: процесс убит]");
  if (log.not_found) lines.push("[бинарь не найден]");
  if (log.launch_error) lines.push(`[ошибка запуска] ${log.launch_error}`);
  if (log.exit_code !== null && log.exit_code !== undefined) {
    lines.push(`[код выхода] ${log.exit_code}`);
  }
  if (log.stdout) lines.push(`--- stdout ---\n${log.stdout}`);
  if (log.stderr) lines.push(`--- stderr ---\n${log.stderr}`);
  if (log.duration_ms > 0) lines.push(`[длительность] ${log.duration_ms} мс`);
  return lines.join("\n");
}

/** Есть ли развёртываемый лог у пробы. */
export function probeLogHasContent(log: ProbeLog | null | undefined): boolean {
  return probeLogText(log).length > 0;
}

/**
 * Полный текст лога проверки здоровья: команда, код выхода, таймаут,
 * stdout/stderr целиком (в пределах санитизации бэкенда).
 */
export function healthCheckLogText(check: HealthCheckResultV2): string {
  const lines: string[] = [];
  lines.push(
    `$ ${(check.command ?? []).join(" ") || "(команда неизвестна)"}`,
  );
  if (check.timed_out) lines.push("[таймаут: проверка не уложилась, процесс убит]");
  if (check.process_failed && !check.timed_out) {
    lines.push("[процесс не выполнился — это «не смогли проверить», а не провал условия]");
  }
  if (check.exit_code !== null && check.exit_code !== undefined) {
    lines.push(`[код выхода] ${check.exit_code}`);
  }
  lines.push(`[итог] ${check.passed ? "пройдено" : "не пройдено"}`);
  if (check.stdout) lines.push(`--- stdout ---\n${check.stdout}`);
  if (check.stderr) lines.push(`--- stderr ---\n${check.stderr}`);
  if (!check.stdout && !check.stderr) lines.push("--- вывода нет ---");
  lines.push(`[длительность] ${check.duration_ms} мс`);
  return lines.join("\n");
}

// ------------------------------------------------------------
// Классификация PATH (слои правды)
// ------------------------------------------------------------

const PATH_SCOPE_INFO: { [K in PathScope["kind"]]: StateInfo } = {
  process_path: { label: "в PATH процесса", tone: "lime" },
  persisted_path_only: { label: "только в постоянном PATH", tone: "amber" },
  outside_path: { label: "вне PATH", tone: "neutral" },
};

/** Подпись слоя доступности установки; null/мусор → «нет данных». */
export function pathScopeInfo(scope: PathScope | null | undefined): StateInfo {
  if (!scope || typeof scope !== "object") {
    return { label: "PATH: нет данных", tone: "neutral" };
  }
  const known = PATH_SCOPE_INFO[scope.kind as PathScope["kind"]];
  return known ?? { label: "PATH: нет данных", tone: "neutral" };
}

// ------------------------------------------------------------
// Оценка версии (VersionAssessment)
// ------------------------------------------------------------

export function versionAssessmentInfo(assessment: VersionAssessment): StateInfo {
  switch (assessment.kind) {
    case "meets_recommended":
      return { label: "Соответствует рекомендации", tone: "lime" };
    case "below_recommended":
      return { label: "Ниже рекомендуемой", tone: "amber" };
    case "below_min":
      return { label: "Ниже минимума", tone: "red" };
    case "unparseable":
      return { label: "Версия не распознана", tone: "amber" };
    case "policy_violation":
      return { label: "Противоречивая политика версий каталога", tone: "amber" };
    case "unknown":
      return { label: "Версия неизвестна", tone: "neutral" };
    default:
      return { label: "Неизвестное состояние версии", tone: "neutral" };
  }
}

// ------------------------------------------------------------
// Здоровье (HealthState, 9 состояний)
// ------------------------------------------------------------

export function healthStateInfo(state: HealthState): StateInfo {
  switch (state.kind) {
    case "healthy":
      return { label: "Здоров", tone: "lime" };
    case "degraded":
      return { label: "Работает с деградацией", tone: "amber" };
    case "unhealthy":
      return { label: "Нездоров", tone: "red" };
    case "checking":
      return { label: "Проверяется…", tone: "cyan" };
    case "not_checked":
      return { label: "Не проверялся", tone: "neutral" };
    case "no_checks_defined":
      return { label: "Проверки не заявлены", tone: "neutral" };
    case "unavailable":
      return { label: "Проверки неприменимы", tone: "neutral" };
    case "unsupported":
      return { label: "Не поддерживается здесь", tone: "neutral" };
    case "failed_to_run":
      return { label: "Не удалось проверить", tone: "amber" };
    default:
      return { label: "Неизвестное состояние здоровья", tone: "neutral" };
  }
}

// ------------------------------------------------------------
// Происхождение (Provenance)
// ------------------------------------------------------------

export function provenanceInfo(provenance: Provenance): StateInfo {
  switch (provenance.kind) {
    case "stack_pilot_managed":
      return { label: "Установлен StackPilot", tone: "violet" };
    case "package_manager":
      return { label: "Через менеджер пакетов", tone: "blue" };
    case "external":
      return { label: "Сторонняя установка", tone: "cyan" };
    case "system":
      return { label: "Предоставлен ОС", tone: "neutral" };
    case "bundled_with":
      return { label: `В комплекте с ${provenance.tool}`, tone: "cyan" };
    case "docker":
      return { label: "Работает в Docker", tone: "blue" };
    case "unknown":
      return { label: "Происхождение неизвестно", tone: "neutral" };
    default:
      return { label: "Неизвестное происхождение", tone: "neutral" };
  }
}

/** Вариант происхождения по kind (для фильтров; bundled подписывается хостом). */
export function provenanceKindInfo(kind: ProvenanceKind, host?: string): StateInfo {
  if (kind === "bundled_with") {
    return {
      label: host ? `В комплекте с ${host}` : "В комплекте с другим инструментом",
      tone: "cyan",
    };
  }
  return provenanceInfo({ kind } as Provenance);
}

/** Все kind происхождения с подписями — для фильтра. */
export function allProvenanceKinds(): { kind: ProvenanceKind; info: StateInfo }[] {
  const kinds: ProvenanceKind[] = [
    "stack_pilot_managed",
    "external",
    "package_manager",
    "system",
    "bundled_with",
    "docker",
    "unknown",
  ];
  return kinds.map((kind) => ({ kind, info: provenanceKindInfo(kind) }));
}

// ------------------------------------------------------------
// Возможности платформы (8 флагов)
// ------------------------------------------------------------

const CAPABILITY_LABELS: { [K in keyof ToolPlatformCapabilities]: string } = {
  detectable: "Обнаруживаемый",
  installable: "Устанавливаемый",
  updatable: "Обновляемый",
  removable: "Удаляемый",
  repairable: "Восстанавливаемый",
  health_checkable: "Проверяемый",
  manual_instructions_available: "Есть инструкция",
  docker_alternative_available: "Docker-альтернатива",
};

/** Подписи ВКЛЮЧЁННЫХ возможностей (для карточек/фильтров). */
export function capabilityLabels(caps: ToolPlatformCapabilities): string[] {
  return (Object.keys(CAPABILITY_LABELS) as (keyof ToolPlatformCapabilities)[])
    .filter((k) => caps[k])
    .map((k) => CAPABILITY_LABELS[k]);
}

export function capabilityLabel(flag: keyof ToolPlatformCapabilities): string {
  return CAPABILITY_LABELS[flag] ?? flag;
}

// ------------------------------------------------------------
// Платформы и источники установки
// ------------------------------------------------------------

/** Человекочитаемое имя ОС по id каталога. */
export function platformName(os: string): string {
  switch (os.toLowerCase()) {
    case "windows":
      return "Windows";
    case "linux":
      return "Linux";
    case "macos":
    case "darwin":
      return "macOS";
    default:
      return os;
  }
}

const SOURCE_KIND_LABELS: Record<string, string> = {
  PkgManager: "менеджер пакетов",
  Official: "официальный установщик",
  Script: "скрипт",
  QtOnline: "online-репозиторий Qt",
};

function sourceKindLabel(kind: string): string {
  return SOURCE_KIND_LABELS[kind] ?? kind;
}

/** Описание источника каталога (InstallSource) без раскрытия URL. */
export function installSourceDescription(source: InstallSource): string {
  const kind = sourceKindLabel(source.kind);
  return source.id ? `${kind}: ${source.id}` : kind;
}

/** Описание источника канонического плана (SelectedSource). */
export function selectedSourceDescription(source: SelectedSource): string {
  const base = source.description || sourceKindLabel(source.kind);
  const integrity = source.sha256 ? "" : " (без контроля целостности)";
  return `${base}${integrity}`;
}

/** Сводка источников инструмента по текущей ОС. */
export function installSourcesSummary(
  def: { sources: { windows: InstallSource[]; linux: InstallSource[]; macos: InstallSource[] } },
  os: string,
): string[] {
  const list =
    os === "windows"
      ? def.sources.windows
      : os === "linux"
        ? def.sources.linux
        : def.sources.macos;
  return list.map(installSourceDescription);
}

// ------------------------------------------------------------
// Задания: операции, статусы, фазы, действия задач
// ------------------------------------------------------------

export function operationLabel(operation: OperationKind): string {
  switch (operation) {
    case "install":
      return "Установка";
    case "update":
      return "Обновление";
    case "repair_path":
      return "Ремонт PATH";
    case "health_check":
      return "Проверка здоровья";
    default:
      return operation;
  }
}

export function jobStatusLabel(status: JobStatus): StateInfo {
  switch (status) {
    case "queued":
      return { label: "В очереди", tone: "neutral" };
    case "running":
      return { label: "Выполняется", tone: "cyan" };
    case "succeeded":
      return { label: "Готово", tone: "lime" };
    case "partial":
      return { label: "Завершено частично", tone: "amber" };
    case "failed":
      return { label: "Ошибка", tone: "red" };
    case "cancelled":
      return { label: "Отменено", tone: "neutral" };
    case "interrupted":
      return { label: "Прервано перезапуском", tone: "amber" };
    default:
      return { label: "Неизвестный статус", tone: "neutral" };
  }
}

export function phaseLabel(phase: Phase): string {
  switch (phase) {
    case "validating":
      return "Валидация…";
    case "preparing":
      return "Подготовка…";
    case "downloading":
      return "Скачивание…";
    case "verifying":
      return "Проверка целостности…";
    case "installing":
      return "Установка…";
    case "configuring":
      return "Настройка…";
    case "updating_path":
      return "Обновление PATH…";
    case "checking_health":
      return "Проверка здоровья…";
    case "completed":
      return "Завершено";
    case "failed":
      return "Ошибка";
    case "cancelled":
      return "Отменено";
    case "interrupted":
      return "Прервано";
    default:
      return phase;
  }
}

/** Подпись причины noop; неизвестная/отсутствующая причина — честный текст. */
export function noopReasonLabel(reason: unknown): string {
  if (typeof reason === "string") {
    return reason === "docker_managed"
      ? "Управляется Docker (без действий)"
      : "Без действий (причина неизвестна)";
  }
  if (!isRecord(reason)) {
    return "Без действий (причина не указана)";
  }
  const keys = Object.keys(reason);
  if (keys.length !== 1) return "Без действий (причина не указана)";
  switch (keys[0]) {
    case "already_installed": {
      const version = isRecord(reason.already_installed)
        ? reason.already_installed.version
        : null;
      return typeof version === "string" && version
        ? `Уже установлен (${version})`
        : "Уже установлен";
    }
    case "update_unavailable": {
      const version = isRecord(reason.update_unavailable)
        ? reason.update_unavailable.version
        : null;
      return typeof version === "string" && version
        ? `Обновление недоступно (${version})`
        : "Обновление недоступно";
    }
    default:
      // Неизвестная причина НЕ угадывается — честное «неизвестно».
      return "Без действий (неизвестная причина)";
  }
}

/**
 * Подпись действия канонической задачи (экран плана).
 * Граница IPC: task.action может отсутствовать/быть мусором — тогда
 * «Неизвестное действие», а НЕ краш «in operator … in undefined».
 */
export function taskActionLabel(task: PlanTask | null | undefined): string {
  if (!isRecord(task)) return "Неизвестное действие";
  const action: unknown = task.action;

  if (typeof action === "string") {
    if (action === "repair_path") return "Ремонт PATH";
    if (action === "health_check") return "Проверка здоровья";
    return "Неизвестное действие";
  }
  if (!isRecord(action)) return "Неизвестное действие";

  const keys = Object.keys(action);
  // Малиформация (0 или >1 ключей) — безопасное «неизвестно».
  if (keys.length !== 1) return "Неизвестное действие";

  switch (keys[0]) {
    case "install_new": {
      const payload = action.install_new;
      const target = isRecord(payload) ? payload.target_version : null;
      return typeof target === "string" && target ? `Установка (${target})` : "Установка";
    }
    case "update": {
      const payload = action.update;
      const target = isRecord(payload) ? payload.target_version : null;
      return typeof target === "string" && target ? `Обновление до ${target}` : "Обновление";
    }
    case "noop":
    case "no_op":
      return noopReasonLabel(action[keys[0]]);
    default:
      return "Неизвестное действие";
  }
}

/** true — задача требует действий (не noop и не малиформированная). */
export function planTaskIsActionable(task: unknown): boolean {
  if (!isRecord(task)) return false;
  const action = task.action;
  if (typeof action === "string") return action === "repair_path" || action === "health_check";
  if (!isRecord(action)) return false;
  const keys = Object.keys(action);
  return keys.length === 1 && (keys[0] === "install_new" || keys[0] === "update");
}

/** true — задача «без действий» (noop любого вида, даже без причины).
 *  Ключ no_op — наследие старой сериализации (персистентные записи). */
export function planTaskIsNoop(task: unknown): boolean {
  if (!isRecord(task)) return false;
  const action = task.action;
  return (
    isRecord(action) &&
    Object.keys(action).length === 1 &&
    ("noop" in action || "no_op" in action)
  );
}

/**
 * Полное описание предупреждения плана. Тотальная функция: неизвестный
 * вариант даёт безопасный текст, а не краш на `.tool_id` от undefined.
 * `kind` — машиночитаемый вариант (для подсчётов/подтверждений UI:
 * строковое сравнение текста запрещено — смена подписи не должна
 * ломать гейтинг «источники без суммы»).
 */
export function planWarningInfo(
  warning: unknown,
): { kind: PlanWarningKind; tone: "amber" | "red"; text: string } {
  if (!isRecord(warning)) {
    return { kind: "unknown", tone: "amber", text: "Неизвестное предупреждение плана." };
  }
  const keys = Object.keys(warning);
  if (keys.length !== 1) {
    return { kind: "unknown", tone: "amber", text: "Неизвестное предупреждение плана." };
  }
  const payload = warning[keys[0]];
  const toolId = isRecord(payload) && typeof payload.tool_id === "string" ? payload.tool_id : "?";
  switch (keys[0]) {
    case "unverified_source": {
      const sourceId =
        isRecord(payload) && typeof payload.source_id === "string" ? payload.source_id : "?";
      return {
        kind: "unverified_source",
        tone: "amber",
        text: `${toolId}: источник «${sourceId}» без контрольной суммы — целостность загрузки проверить нельзя.`,
      };
    }
    case "admin_required":
      return {
        kind: "admin_required",
        tone: "amber",
        text: `${toolId}: установка потребует повышения прав (UAC).`,
      };
    case "reinstall_on_broken":
      return {
        kind: "reinstall_on_broken",
        tone: "red",
        text: `${toolId}: инструмент сломан — будет выполнена переустановка.`,
      };
    default:
      return { kind: "unknown", tone: "amber", text: "Неизвестное предупреждение плана." };
  }
}

/** Машиночитаемые варианты предупреждений плана. */
export type PlanWarningKind =
  | "unverified_source"
  | "admin_required"
  | "reinstall_on_broken"
  | "unknown";

// ------------------------------------------------------------
// Скан: фазы и терминальные состояния (PascalCase строки бэкенда)
// ------------------------------------------------------------

export function scanPhaseLabel(phase: string): string {
  switch (phase) {
    case "Queued":
      return "в очереди";
    case "Environment":
      return "окружение";
    case "Tools":
      return "проверка инструментов";
    case "Finalizing":
      return "завершение";
    case "Done":
      return "готово";
    default:
      return phase;
  }
}

export function scanTerminalLabel(terminal: string): string {
  switch (terminal) {
    case "Running":
      return "выполняется";
    case "Completed":
      return "завершён полностью";
    case "Partial":
      return "завершён частично";
    case "Cancelled":
      return "отменён";
    case "Failed":
      return "ошибка";
    case "Interrupted":
      return "прерван перезапуском";
    default:
      return terminal;
  }
}

// ------------------------------------------------------------
// Санитизация сообщений об ошибках
// ------------------------------------------------------------

/** Токены, похожие на секреты, маскируются; пути укорачиваются. */
export function sanitizeErrorMessage(error: unknown, maxLength = 300): string {
  let raw: string;
  if (error == null) return "Неизвестная ошибка";
  if (typeof error === "string") raw = error;
  else if (error instanceof Error) raw = error.message;
  else {
    try {
      raw = JSON.stringify(error);
    } catch {
      raw = String(error);
    }
  }
  // Маскирование значений, похожих на секреты/токены.
  raw = raw.replace(/\b(sk|pk|token|secret|password|passwd|pwd)[_=:-][^\s"',;)]+/gi, "$1=***");
  // Схлопывание длинных путей до имени файла.
  raw = raw.replace(/([A-Za-z]:\\(?:[^\\:\s]+\\)+)/g, (m) => `…\\${m.split("\\").filter(Boolean).pop() ?? ""}`);
  raw = raw.replace(/\s+/g, " ").trim();
  if (raw.length > maxLength) raw = `${raw.slice(0, maxLength - 1)}…`;
  return raw || "Неизвестная ошибка";
}
