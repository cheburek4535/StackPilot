// ============================================================
// Toolchain — чистая логика переходов состояния контроллера
// ============================================================
// Здесь живут ЧИСТЫЕ функции правил из контракта (§4.7, §5):
//  - кэш рендерится мгновенно; скан НЕ стирает полезные прежние данные;
//  - прогресс скана обновляет состояние инкрементально;
//  - мутация не стартует при активной конфликтующей мутации;
//  - задание не помечается успешным до терминального события/результата;
//  - проваленный API-вызов сохраняет предыдущее валидное состояние
//    (функции возвращают новое значение, а не мутируют);
//  - stale-payload'ы (чужие job_id / отстающие seq) не трогают состояние.
//
// Функции без зависимостей от Svelte/Tauri — покрываются юнит-тестами.

import type {
  EnvironmentSnapshot,
  JobEvent,
  JobStatus,
  OperationKind,
  PersistedJob,
  ScanProgressEvent,
  ToolScanResult,
  ToolState,
} from "./types";
import {
  TOOL_STATE_KINDS,
  jobEventBelongsTo,
  jobStatusIsTerminal,
  operationMutatesMachine,
} from "./types";

// ------------------------------------------------------------
// Живой срез поверх кэшированного снапшота
// ------------------------------------------------------------

/** Результат живого инструмента: из инкрементальных событий скана. */
export type LiveToolMap = Record<string, ToolScanResult>;

/**
 * Инкрементальное обновление одного инструмента по событию прогресса.
 * `tool_state` в событии — СТРОКОВОЕ имя варианта (state_name на бэкенде):
 * неизвестное имя игнорируется (таблица не ломается), известное —
 * восстанавливается в минимальный вариант канонического union; payload
 * варианта (версии и т.п.) сохраняется от прежнего состояния, а полный
 * ToolScanResult придёт с финальным снапшотом. Возвращает НОВЫЙ словарь.
 */
export function applyScanProgress(
  tools: LiveToolMap,
  event: ScanProgressEvent,
): LiveToolMap {
  if (!event.tool_id) return tools;
  const existing = tools[event.tool_id];
  const state = toolStateFromEventName(event.tool_state, existing?.state ?? null, event.error ?? null);
  if (!state) return tools;
  return {
    ...tools,
    [event.tool_id]: {
      ...(existing ?? emptyToolResult(event)),
      state,
      error: event.error ?? null,
    },
  };
}

function toolStateFromEventName(
  name: string,
  keep: ToolState | null,
  error: string | null,
): ToolState | null {
  if (!(TOOL_STATE_KINDS as readonly string[]).includes(name)) return null;
  // Тот же вариант, что был: сохраняем его payload (версии и т.п.).
  if (keep && keep.kind === name) return keep;
  return minimalState(name as ToolState["kind"], error);
}

/** Минимально честный вариант по имени; payload заполнит снапшот. */
function minimalState(kind: ToolState["kind"], error: string | null): ToolState {
  switch (kind) {
    case "scan_failed":
      return { kind, reason: error ?? "" };
    case "installed_healthy":
    case "installed_health_unknown":
    case "installed_unhealthy":
      return { kind, version: "" };
    case "update_available":
      return { kind, installed: "", recommended: "" };
    case "path_broken":
      return { kind, reason: "" };
    case "manual_install":
      return { kind, reason: "" };
    default:
      return { kind } as ToolState;
  }
}

function emptyToolResult(event: ScanProgressEvent): ToolScanResult {
  return {
    tool_id: event.tool_id,
    display: event.display_name || event.tool_id,
    category: "",
    icon: event.icon ?? null,
    detection: { kind: "pending" },
    installs: [],
    path_findings: [],
    health: null,
    applicability: { kind: "installable" },
    capabilities: {
      detectable: false,
      installable: false,
      updatable: false,
      removable: false,
      repairable: false,
      health_checkable: false,
      manual_instructions_available: false,
      docker_alternative_available: false,
    },
    provenance: { kind: "unknown" },
    bundled_with: null,
    version_assessment: { kind: "unknown" },
    state: { kind: "scan_pending" },
    error: event.error ?? null,
    duration_ms: 0,
  };
}

/**
 * Живой снапшот = кэш + инкрементальные результаты текущего скана.
 *
 * Правила:
 *  - кэш (возможно stale) отдаётся КАК ЕСТЬ, пока нет живых данных;
 *  - скан никогда не очищает прежние инструменты: обновляются только
 *    те id, по которым пришли события;
 *  - summary пересчитывается только когда обновлено всё (иначе счётчики
 *    были бы лживой смесью эпох).
 */
export function mergeLiveSnapshot(
  cached: EnvironmentSnapshot | null,
  liveTools: LiveToolMap,
): EnvironmentSnapshot | null {
  if (!cached) return null;
  const ids = Object.keys(liveTools);
  if (ids.length === 0) return cached;
  const tools = cached.tools.map((t) => liveTools[t.tool_id] ?? t);
  const allTouched =
    tools.length > 0 && tools.every((t) => liveTools[t.tool_id] !== undefined);
  return {
    ...cached,
    tools,
    // Сводка честна только после полного покрытия живыми данными.
    summary: allTouched ? recomputeSummary(tools) : cached.summary,
  };
}

/** Пересчёт StatusCounts по списку результатов (зеркало бэкенда). */
export function recomputeSummary(tools: ToolScanResult[]): EnvironmentSnapshot["summary"] {
  const counts: EnvironmentSnapshot["summary"] = {
    scan_pending: 0,
    scan_failed: 0,
    missing: 0,
    installed_healthy: 0,
    installed_health_unknown: 0,
    installed_unhealthy: 0,
    update_available: 0,
    path_broken: 0,
    manual_install: 0,
    docker_managed: 0,
    built_in_system: 0,
    unsupported_platform: 0,
    install_unavailable: 0,
  };
  for (const tool of tools) counts[tool.state.kind] += 1;
  return counts;
}

// ------------------------------------------------------------
// Свежесть снапшота
// ------------------------------------------------------------

export const SNAPSHOT_FRESH_SECONDS = 300;

export type SnapshotFreshness = "live" | "stale" | "none";

export function snapshotFreshness(
  snapshot: EnvironmentSnapshot | null,
  maxAgeSeconds: number = SNAPSHOT_FRESH_SECONDS,
): SnapshotFreshness {
  if (!snapshot) return "none";
  return snapshot.stale || snapshot.age_seconds > maxAgeSeconds ? "stale" : "live";
}

// ------------------------------------------------------------
// Конкурентность заданий: мутации и сканы
// ------------------------------------------------------------

export type MutationGate =
  | { allowed: true }
  | { allowed: false; reason: string };

/**
 * Мутация (install/update/repair_path) не стартует, пока идёт другая
 * мутация: бэкенд держит одну активную мутирующую работу, фронтенд
 * дублирует правило, чтобы не гонять заведомо отвергнутые запросы.
 * health_check — read-only и разрешён всегда.
 */
export function gateMutation(
  currentJob: PersistedJob | null,
  operation: OperationKind,
): MutationGate {
  if (!operationMutatesMachine(operation)) return { allowed: true };
  if (currentJob && jobIsActive(currentJob)) {
    if (operationMutatesMachine(currentJob.operation)) {
      return {
        allowed: false,
        reason: `Уже выполняется задание «${currentJob.operation}» (${currentJob.job_id}). Дождитесь завершения или отмените его.`,
      };
    }
  }
  return { allowed: true };
}

/**
 * Скан во время мутации РАЗРЕШЁН: движок сканов строго read-only и
 * работает параллельно с заданиями (отдельный журнал, ноль мутаций).
 * Снапшот честно помечает active_jobs, чтобы UI показал возможную
 * устаревание данных.
 */
export const SCAN_DURING_MUTATION_ALLOWED = true;

/** Задание активно (не терминальное). */
export function jobIsActive(job: PersistedJob): boolean {
  return !jobStatusIsTerminal(job.status);
}

// ------------------------------------------------------------
// Применение событий задания (stale-guard по seq)
// ------------------------------------------------------------

/**
 * Применяет событие задания к его записи. Правила:
 *  - события чужих заданий отбрасываются (identity);
 *  - события с seq <= последнего применённого отбрасываются
 *    (дубликаты/гонки не откатывают состояние);
 *  - статус обновляется ТОЛЬКО вперёд: терминальный статус не
 *    перезаписывается «running»-событием, пришедшим с задержкой;
 *  - задача не становится succeeded до явного task_completed/job_finished.
 */
export function applyJobEvent(
  job: PersistedJob,
  event: JobEvent,
  lastSeq: number,
): { job: PersistedJob; lastSeq: number; applied: boolean } {
  if (!jobEventBelongsTo(event, job.job_id)) return { job, lastSeq, applied: false };
  if (event.seq <= lastSeq) return { job, lastSeq, applied: false };

  let next: PersistedJob = { ...job };
  const payload = event.payload;

  if ("task_started" in payload) {
    next = setTaskStatus(next, event.task_id, "pending");
  } else if ("task_phase" in payload) {
    next = setTaskStatus(next, event.task_id, { running: { phase: payload.task_phase.phase } });
  } else if ("task_completed" in payload) {
    next = setTaskStatus(next, event.task_id, payload.task_completed.status);
  } else if ("job_finished" in payload) {
    next = {
      ...next,
      status: payload.job_finished.status,
      errors: payload.job_finished.errors,
      finished_at: next.finished_at ?? new Date().toISOString(),
    };
  }

  return { job: next, lastSeq: event.seq, applied: true };
}

function setTaskStatus(job: PersistedJob, taskId: string, status: PersistedJob["plan"]["tasks"][number]["status"]): PersistedJob {
  const tasks = job.plan.tasks.map((t) =>
    t.task_id === taskId ? { ...t, status } : t,
  );
  return { ...job, plan: { ...job.plan, tasks } };
}

/**
 * Гарантия «успех только от бэкенда»: статус задания считается
 * успешным лишь если он пришёл из терминального источника
 * (job_finished-событие или status-запрос), а не выведен UI.
 */
export function isJobSucceeded(status: JobStatus): boolean {
  return status === "succeeded" || status === "partial";
}

// ------------------------------------------------------------
// История заданий сессии
// ------------------------------------------------------------

/** Обновляет/добавляет запись истории (новые сверху), ограничивая размер. */
export function upsertJobHistory(
  history: PersistedJob[],
  job: PersistedJob,
  limit: number = 20,
): PersistedJob[] {
  const rest = history.filter((j) => j.job_id !== job.job_id);
  return [job, ...rest].slice(0, limit);
}

// ------------------------------------------------------------
// Выборка инструментов для UI
// ------------------------------------------------------------

/** Состояние инструмента: живые данные > кэш. Никогда не выдумывает missing. */
export function resolveToolState(
  cached: EnvironmentSnapshot | null,
  liveTools: LiveToolMap,
  toolId: string,
): ToolScanResult | null {
  const live = liveTools[toolId];
  if (live) return live;
  const inCache = cached?.tools.find((t) => t.tool_id === toolId);
  return inCache ?? null;
}

// ------------------------------------------------------------
// Совместимость Project Creator: stale-события
// ------------------------------------------------------------

/**
 * Принадлежит ли входящее событие текущей операции мастера?
 *
 * Правила (контракт §7, регрессия «новые job_id не портят стейт»):
 *  - событие БЕЗ идентичности (session_id/scan_id = "") принимается —
 *    так работал старый бэкенд, совместимость не должна ломаться;
 *  - идентичность запуска ещё не установлена (current пуст) —
 *    принимается (первое событие устанавливает идентичность);
 *  - известная ЧУЖАЯ идентичность (поздний «хвост» прежней установки/
 *    проверки) — отбрасывается ДО попадания в состояние мастера.
 */
export function identityMatches(
  current: string | null | undefined,
  incoming: string | null | undefined,
): boolean {
  if (!incoming) return true;
  if (!current) return true;
  return incoming === current;
}
