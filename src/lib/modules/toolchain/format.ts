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
import { i18n } from "$lib/core/i18n.svelte";
import type { TranslationKey } from "$lib/core/i18n.svelte";

export type InfoTone = "neutral" | "violet" | "cyan" | "blue" | "lime" | "amber" | "red";

export type StateInfo = { label: string; tone: InfoTone; short?: string };

// ------------------------------------------------------------
// Размеры: bytes / MB / GB
// ------------------------------------------------------------

/** Форматирует мегабайты в человекочитаемый размер (МБ/ГБ). */
export function formatSizeMb(mb: number | null | undefined): string {
  if (mb == null || Number.isNaN(mb)) return "—";
  if (mb <= 0) return `0 ${i18n.t("tc.time.mb")}`;
  if (mb < 1024) return `${Math.round(mb)} ${i18n.t("tc.time.mb")}`;
  const gb = mb / 1024;
  return `${gb >= 100 ? Math.round(gb) : gb.toFixed(1)} ${i18n.t("tc.time.gb")}`;
}

/** Форматирует байты (Б/КБ/МБ/ГБ) — для будущих потоков загрузки. */
export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || Number.isNaN(bytes) || bytes < 0) return "—";
  if (bytes < 1024) return `${Math.round(bytes)} ${i18n.t("tc.time.bytes")}`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(kb >= 100 ? 0 : 1)} ${i18n.t("tc.time.kb")}`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb.toFixed(mb >= 100 ? 0 : 1)} ${i18n.t("tc.time.mb")}`;
  const gb = mb / 1024;
  return `${gb.toFixed(gb >= 100 ? 0 : 1)} ${i18n.t("tc.time.gb")}`;
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
  if (diffSec < 45) return i18n.t("tc.time.just_now") as TranslationKey;
  const diffMin = Math.round(diffSec / 60);
  if (diffMin < 60) return i18n.t("tc.time.min_ago", { n: diffMin }) as TranslationKey;
  const diffH = Math.round(diffMin / 60);
  if (diffH < 24) return i18n.t("tc.time.hour_ago", { n: diffH }) as TranslationKey;
  const diffD = Math.round(diffH / 24);
  if (diffD < 30) return i18n.t("tc.time.day_ago", { n: diffD }) as TranslationKey;
  try {
    return date.toLocaleDateString(i18n.locale === "ru" ? "ru-RU" : "en-US");
  } catch {
    return date.toISOString().slice(0, 10);
  }
}

/** Возраст снапшота по age_seconds (поле бэкенда). */
export function formatAgeSeconds(seconds: number | null | undefined): string {
  if (seconds == null || Number.isNaN(seconds)) return "—";
  if (seconds < 45) return i18n.t("tc.time.just_now") as TranslationKey;
  if (seconds < 60) return i18n.t("tc.time.sec_ago", { n: Math.round(seconds) }) as TranslationKey;
  return formatRelativeTime(new Date(Date.now() - seconds * 1000));
}

// ------------------------------------------------------------
// Презентационное состояние инструмента (ToolState)
// ------------------------------------------------------------

const TOOL_STATE_INFO: { [K in ToolState["kind"]]: StateInfo } = {
  scan_pending: { label: i18n.t("tc.state.scanning") as TranslationKey, tone: "neutral", short: "…" },
  scan_failed: { label: i18n.t("tc.state.scan_error") as TranslationKey, tone: "amber", short: "?" },
  missing: { label: i18n.t("tc.state.missing") as TranslationKey, tone: "neutral", short: "—" },
  installed_healthy: { label: i18n.t("tc.state.installed") as TranslationKey, tone: "lime", short: "✓" },
  installed_health_unknown: { label: i18n.t("tc.state.health_unknown") as TranslationKey, tone: "cyan", short: "?" },
  installed_unhealthy: { label: i18n.t("tc.state.unhealthy") as TranslationKey, tone: "red", short: "✕" },
  update_available: { label: i18n.t("tc.state.update") as TranslationKey, tone: "amber", short: "↑" },
  path_broken: { label: i18n.t("tc.state.path_broken") as TranslationKey, tone: "red", short: "!" },
  manual_install: { label: i18n.t("tc.state.manual") as TranslationKey, tone: "violet", short: "✋" },
  docker_managed: { label: i18n.t("tc.state.docker") as TranslationKey, tone: "blue", short: "🐳" },
  built_in_system: { label: i18n.t("tc.state.builtin") as TranslationKey, tone: "neutral", short: "•" },
  unsupported_platform: { label: i18n.t("tc.state.unsupported") as TranslationKey, tone: "neutral", short: "×" },
};

/** Подпись+тон состояния; неизвестный kind → честное «неизвестно». */
export function toolStateInfo(state: ToolState): StateInfo {
  const known = (TOOL_STATE_INFO as Record<string, StateInfo | undefined>)[state.kind];
  if (known) {
    // Детали варианта — в подсказку/детали, здесь только класс состояния.
    return known;
  }
  return { label: i18n.t("tc.state.unknown") as TranslationKey, tone: "neutral", short: "?" };
}

/** Вариант по kind (для фильтров/счётчиков, где объекта состояния нет). */
export function toolStateKindInfo(kind: ToolStateKind): StateInfo {
  const known = (TOOL_STATE_INFO as Record<string, StateInfo | undefined>)[kind];
  return known ?? { label: i18n.t("tc.state.unknown") as TranslationKey, tone: "neutral" };
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
      return { label: i18n.t("tc.health.healthy") as TranslationKey, tone: "lime" };
    case "degraded":
      return { label: i18n.t("tc.health.degraded") as TranslationKey, tone: "amber" };
    case "unhealthy":
      return { label: i18n.t("tc.health.unhealthy") as TranslationKey, tone: "red" };
    case "checking":
      return { label: i18n.t("tc.health.checking") as TranslationKey, tone: "cyan" };
    case "not_checked":
      return { label: i18n.t("tc.health.not_checked") as TranslationKey, tone: "neutral" };
    case "no_checks_defined":
      return { label: i18n.t("tc.health.no_checks") as TranslationKey, tone: "neutral" };
    case "unavailable":
      return { label: i18n.t("tc.health.unavailable") as TranslationKey, tone: "neutral" };
    case "unsupported":
      return { label: i18n.t("tc.health.unsupported") as TranslationKey, tone: "neutral" };
    case "failed_to_run":
      return { label: i18n.t("tc.health.failed") as TranslationKey, tone: "amber" };
    default:
      return { label: i18n.t("tc.state.unknown") as TranslationKey, tone: "neutral" };
  }
}

// ------------------------------------------------------------
// Происхождение (Provenance)
// ------------------------------------------------------------

export function provenanceInfo(provenance: Provenance): StateInfo {
  switch (provenance.kind) {
    case "stack_pilot_managed":
      return { label: i18n.t("tc.prov.managed") as TranslationKey, tone: "violet" };
    case "package_manager":
      return { label: i18n.t("tc.prov.pkg") as TranslationKey, tone: "blue" };
    case "external":
      return { label: i18n.t("tc.prov.external") as TranslationKey, tone: "cyan" };
    case "system":
      return { label: i18n.t("tc.prov.system") as TranslationKey, tone: "neutral" };
    case "bundled_with":
      return { label: i18n.t("tc.prov.bundled", { tool: provenance.tool }) as TranslationKey, tone: "cyan" };
    case "docker":
      return { label: i18n.t("tc.prov.docker") as TranslationKey, tone: "blue" };
    case "unknown":
      return { label: i18n.t("tc.prov.unknown") as TranslationKey, tone: "neutral" };
    default:
      return { label: i18n.t("tc.prov.unknown") as TranslationKey, tone: "neutral" };
  }
}

/** Вариант происхождения по kind (для фильтров; bundled подписывается хостом). */
export function provenanceKindInfo(kind: ProvenanceKind, host?: string): StateInfo {
  if (kind === "bundled_with") {
    return {
      label: host ? i18n.t("tc.prov.bundled", { tool: host }) as TranslationKey : i18n.t("tc.prov.bundled", { tool: i18n.t("tc.prov.unknown") }) as TranslationKey,
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
  detectable: i18n.t("tc.cap.detectable"),
  installable: i18n.t("tc.cap.installable"),
  updatable: i18n.t("tc.cap.updatable"),
  removable: i18n.t("tc.cap.removable"),
  repairable: i18n.t("tc.cap.repairable"),
  health_checkable: i18n.t("tc.cap.health_checkable"),
  manual_instructions_available: i18n.t("tc.cap.manual_instructions"),
  docker_alternative_available: i18n.t("tc.cap.docker_alt"),
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
  PkgManager: i18n.t("tc.src.pkg_manager"),
  Official: i18n.t("tc.src.official"),
  Script: i18n.t("tc.src.script"),
  QtOnline: i18n.t("tc.src.qt_online"),
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
  const integrity = source.sha256 ? "" : ` (${i18n.t("tc.drawer.no_checksum")})`;
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
      return i18n.t("tc.op.install");
    case "update":
      return i18n.t("tc.op.update");
    case "repair_path":
      return i18n.t("tc.op.repair_path");
    case "health_check":
      return i18n.t("tc.op.health_check");
    default:
      return operation;
  }
}

export function jobStatusLabel(status: JobStatus): StateInfo {
  switch (status) {
    case "queued":
      return { label: i18n.t("tc.job.queued") as TranslationKey, tone: "neutral" };
    case "running":
      return { label: i18n.t("tc.job.running") as TranslationKey, tone: "cyan" };
    case "succeeded":
      return { label: i18n.t("tc.job.done") as TranslationKey, tone: "lime" };
    case "partial":
      return { label: i18n.t("tc.job.partial") as TranslationKey, tone: "amber" };
    case "failed":
      return { label: i18n.t("tc.job.failed") as TranslationKey, tone: "red" };
    case "cancelled":
      return { label: i18n.t("tc.job.cancelled") as TranslationKey, tone: "neutral" };
    case "interrupted":
      return { label: i18n.t("tc.job.interrupted") as TranslationKey, tone: "amber" };
    default:
      return { label: i18n.t("tc.job.unknown") as TranslationKey, tone: "neutral" };
  }
}

export function phaseLabel(phase: Phase): string {
  switch (phase) {
    case "validating":
      return i18n.t("tc.phase.validating");
    case "preparing":
      return i18n.t("tc.phase.preparing");
    case "downloading":
      return i18n.t("tc.phase.downloading");
    case "verifying":
      return i18n.t("tc.phase.verifying");
    case "installing":
      return i18n.t("tc.phase.installing");
    case "configuring":
      return i18n.t("tc.phase.configuring");
    case "updating_path":
      return i18n.t("tc.phase.updating_path");
    case "checking_health":
      return i18n.t("tc.phase.checking_health");
    case "completed":
      return i18n.t("tc.phase.completed");
    case "failed":
      return i18n.t("tc.phase.failed");
    case "cancelled":
      return i18n.t("tc.phase.cancelled");
    case "interrupted":
      return i18n.t("tc.phase.interrupted");
    default:
      return phase;
  }
}

/** Подпись причины noop; неизвестная/отсутствующая причина — честный текст. */
export function noopReasonLabel(reason: unknown): string {
  if (typeof reason === "string") {
    return reason === "docker_managed"
      ? i18n.t("tc.noop.docker_managed")
      : i18n.t("tc.noop.unknown_reason");
  }
  if (!isRecord(reason)) {
    return i18n.t("tc.noop.unknown_reason");
  }
  const keys = Object.keys(reason);
  if (keys.length !== 1) return i18n.t("tc.noop.unknown_reason");
  switch (keys[0]) {
    case "already_installed": {
      const version = isRecord(reason.already_installed)
        ? reason.already_installed.version
        : null;
      return typeof version === "string" && version
        ? i18n.t("tc.noop.already_installed_ver", { version })
        : i18n.t("tc.noop.already_installed");
    }
    case "update_unavailable": {
      const version = isRecord(reason.update_unavailable)
        ? reason.update_unavailable.version
        : null;
      return typeof version === "string" && version
        ? i18n.t("tc.noop.update_unavailable_ver", { version })
        : i18n.t("tc.noop.update_unavailable");
    }
    default:
      return i18n.t("tc.noop.unknown_reason");
  }
}

/**
 * Подпись действия канонической задачи (экран плана).
 * Граница IPC: task.action может отсутствовать/быть мусором — тогда
 * «Неизвестное действие», а НЕ краш «in operator … in undefined».
 */
export function taskActionLabel(task: PlanTask | null | undefined): string {
  if (!isRecord(task)) return i18n.t("tc.task.unknown");
  const action: unknown = task.action;

  if (typeof action === "string") {
    if (action === "repair_path") return i18n.t("tc.task.repair_path");
    if (action === "health_check") return i18n.t("tc.task.health_check");
    return i18n.t("tc.task.unknown");
  }
  if (!isRecord(action)) return i18n.t("tc.task.unknown");

  const keys = Object.keys(action);
  if (keys.length !== 1) return i18n.t("tc.task.unknown");

  switch (keys[0]) {
    case "install_new": {
      const payload = action.install_new;
      const target = isRecord(payload) ? payload.target_version : null;
      return typeof target === "string" && target ? i18n.t("tc.task.install_ver", { version: target }) : i18n.t("tc.task.install");
    }
    case "update": {
      const payload = action.update;
      const target = isRecord(payload) ? payload.target_version : null;
      return typeof target === "string" && target ? i18n.t("tc.task.update_to", { version: target }) : i18n.t("tc.op.update");
    }
    case "noop":
    case "no_op":
      return noopReasonLabel(action[keys[0]]);
    default:
      return i18n.t("tc.task.unknown");
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
    return { kind: "unknown", tone: "amber", text: i18n.t("tc.warn.unverified_source", { tool: "?", source: "?" }) };
  }
  const keys = Object.keys(warning);
  if (keys.length !== 1) {
    return { kind: "unknown", tone: "amber", text: i18n.t("tc.warn.unverified_source", { tool: "?", source: "?" }) };
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
        text: i18n.t("tc.warn.unverified_source", { tool: toolId, source: sourceId }),
      };
    }
    case "admin_required":
      return {
        kind: "admin_required",
        tone: "amber",
        text: i18n.t("tc.warn.admin_required", { tool: toolId }),
      };
    case "reinstall_on_broken":
      return {
        kind: "reinstall_on_broken",
        tone: "red",
        text: i18n.t("tc.warn.reinstall_on_broken", { tool: toolId }),
      };
    default:
      return { kind: "unknown", tone: "amber", text: i18n.t("tc.warn.unverified_source", { tool: "?", source: "?" }) };
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
      return i18n.t("tc.scan.queued");
    case "Environment":
      return i18n.t("tc.scan.environment");
    case "Tools":
      return i18n.t("tc.scan.tools");
    case "Finalizing":
      return i18n.t("tc.scan.finalizing");
    case "Done":
      return i18n.t("tc.scan.done");
    default:
      return phase;
  }
}

export function scanTerminalLabel(terminal: string): string {
  switch (terminal) {
    case "Running":
      return i18n.t("tc.scan.running");
    case "Completed":
      return i18n.t("tc.scan.completed");
    case "Partial":
      return i18n.t("tc.scan.partial_done");
    case "Cancelled":
      return i18n.t("tc.scan.cancelled");
    case "Failed":
      return i18n.t("tc.scan.failed");
    case "Interrupted":
      return i18n.t("tc.scan.interrupted");
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
