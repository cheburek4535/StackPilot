// ============================================================
// Toolchain — типизированный слой API (Tauri invoke / listen)
// ============================================================
// Правила слоя:
//  - каждая функция типизирована зеркалом serde-контракта (types.ts);
//  - слушатели событий возвращают функцию очистки и фильтруют чужие
//    операции по идентичности (job_id/scan_id/session_id) — см. events.ts;
//  - секреты через этот слой НЕ хранятся: единственный канал —
//    одноразовый getNewSecrets() (keep-and-clear на бэкенде);
//  - легаси-обёртки tc_* сохранены для Project Creator (контракт §7);
//    новые потребители должны использовать compat.ts.
//
// Зафиксированные расхождения с контрактом (см. docs/toolchain-progress.md):
//  - tcx_profile_resolve зарегистрирован на бэкенде (этап UI);
//  - отдельной tcx-команды каталога нет — используется легаси
//    tc_get_tool_definitions (ToolDefinition уже несёт расширенные
//    метаданные: aliases/dependencies/conflicts/docker/...).

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ProjectRequirements,
  EnvironmentCheck,
  EnvironmentInfo,
  PlatformCapabilities,
  ToolDefinition,
  InstallPlan,
  InstallSession,
  ToolchainEvent,
  ToolchainMetadata,
  HealthReport,
  CheckProgressEvent,
  EnvironmentSnapshot,
  EnvironmentProfile,
  ScanJobSnapshot,
  ScanStartOutcome,
  ScanProgressEvent,
  ScanDoneEvent,
  ToolScanResult,
  EngineRequest,
  CanonicalPlan,
  PersistedJob,
  JobEvent,
  OperationKind,
} from "./types";

// ============================================================
// Легаси-команды tc_* (Project Creator; контракт §7)
// ============================================================

export function pingToolchain(): Promise<string> {
  return invoke("ping_toolchain");
}

export function getToolDefinitions(): Promise<ToolDefinition[]> {
  return invoke("tc_get_tool_definitions");
}

export function getEnvironmentInfo(): Promise<EnvironmentInfo> {
  return invoke("tc_get_environment_info");
}

export function checkEnvironment(
  requirements: ProjectRequirements,
): Promise<EnvironmentCheck> {
  return invoke("tc_check_environment", { requirements });
}

export function buildInstallPlan(
  check: EnvironmentCheck,
  selectedToolIds?: string[],
): Promise<InstallPlan> {
  return invoke("tc_build_install_plan", { check, selectedToolIds });
}

export function runInstall(plan: InstallPlan): Promise<void> {
  return invoke("tc_run_install", { plan });
}

export function getInstallStatus(): Promise<InstallSession | null> {
  return invoke("tc_get_install_status");
}

export function abortInstall(): Promise<boolean> {
  return invoke("tc_abort_install");
}

/** ЕДИНСТВЕННЫЙ канал секретов: забрал → на бэкенде очистилось. */
export function getNewSecrets(): Promise<Record<string, string>> {
  return invoke("tc_take_new_secrets");
}

/** Санитизированная выдача state.json: без секретов (View). */
export function getToolchainMetadata(): Promise<ToolchainMetadata> {
  return invoke("tc_get_metadata");
}

export function getHealthReport(): Promise<HealthReport> {
  return invoke("tc_get_health_report");
}

// ------------------------------------------------------------
// Легаси-слушатели (тонкие обёртки; идентичность фильтрует вызывающий)
// ------------------------------------------------------------

export function listenToolchainEvents(
  handler: (event: ToolchainEvent) => void,
): Promise<UnlistenFn> {
  return listen<ToolchainEvent>("toolchain:task_event", (e) => handler(e.payload));
}

export function listenInstallDone(
  handler: (event: InstallPlan) => void,
): Promise<UnlistenFn> {
  return listen<InstallPlan>("toolchain:install_done", (e) => handler(e.payload));
}

export function listenCheckProgress(
  handler: (event: CheckProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<CheckProgressEvent>("toolchain:check_progress", (e) => handler(e.payload));
}

// ============================================================
// Канон: каталог, снапшот, сканы, здоровье (tcx_*)
// ============================================================

/** Каталог STANDALONE Toolchain (tcx_get_catalog): только tools.json,
 *  без легаси-совместимости Project Creator. */
export function getCatalog(): Promise<ToolDefinition[]> {
  return invoke("tcx_get_catalog");
}

/** Последний валидный снапшот (мгновенно из кэша; stale-пометка честная). */
export function getEnvironmentSnapshot(): Promise<EnvironmentSnapshot | null> {
  return invoke("tcx_get_environment_snapshot");
}

/** Запуск read-only скана. Если скан уже идёт — reconnect к нему. */
export function startScan(): Promise<ScanStartOutcome> {
  return invoke("tcx_start_scan");
}

/** Задание скана по id (включая восстановленные Interrupted). */
export function getScanJob(jobId: string): Promise<ScanJobSnapshot | null> {
  return invoke("tcx_get_scan_job", { jobId });
}

/** Актуальное задание скана (идущее или последнее завершённое). */
export function getLatestScanJob(): Promise<ScanJobSnapshot | null> {
  return invoke("tcx_get_latest_scan_job");
}

/** Отмена скана: новые инструменты не стартуются, идущие добираются. */
export function cancelScan(jobId: string): Promise<boolean> {
  return invoke("tcx_cancel_scan", { jobId });
}

/** Живое состояние одного инструмента (обнаружение + здоровье + PATH). */
export function getToolDetails(toolId: string): Promise<ToolScanResult> {
  return invoke("tcx_get_tool_details", { toolId });
}

/** Здоровье выбранных инструментов (ограниченный параллелизм, read-only). */
export function runHealthChecks(toolIds: string[]): Promise<ToolScanResult[]> {
  return invoke("tcx_run_health_checks", { toolIds });
}

/**
 * Разрешение канонического профиля Build Environment.
 * Бэкенд строит профиль из (выбор, каталог, ОС) и накладывает последний
 * кэшированный снапшот для честной готовности (satisfied_count).
 */
export function resolveEnvironmentProfile(
  requirements: ProjectRequirements,
): Promise<EnvironmentProfile> {
  return invoke("tcx_profile_resolve", { requirements });
}

// ============================================================
// Канон: план и задания движка (install/update/repair_path)
// ============================================================

/** Превью канонического плана: ничего не исполняет и не пишет в журнал. */
export function buildCanonicalPlan(request: EngineRequest): Promise<CanonicalPlan> {
  return invoke("tcx_build_plan", { request });
}

/** Старт задания из ограниченного запроса. План строится заново
 * бэкендом; возвращает job_id. */
export function startJob(request: EngineRequest): Promise<string> {
  return invoke("tcx_start_job", { request });
}

/** Статус задания по id (секретов в записи нет по построению). */
export function getJob(jobId: string): Promise<PersistedJob | null> {
  return invoke("tcx_get_job", { jobId });
}

/** История заданий (живые поверх записанных, новые сверху). */
export function listJobs(): Promise<PersistedJob[]> {
  return invoke("tcx_list_jobs");
}

/** Отмена задания: текущая задача добивается, остальные Cancelled. */
export function cancelJob(jobId: string): Promise<void> {
  return invoke("tcx_cancel_job", { jobId });
}

/** Повтор прерванного/терминального задания: план строится заново. */
export function retryJob(jobId: string): Promise<string> {
  return invoke("tcx_retry_job", { jobId });
}

/** Ремонт PATH выбранных инструментов — одобренное audited-задание. */
export function repairPath(toolIds: string[]): Promise<string> {
  const request: EngineRequest = {
    operation: "repair_path",
    tools: toolIds.map((tool_id) => ({ tool_id })),
  };
  return startJob(request);
}

/** Явное «усыновить» найденную ручную установку (track-метка). */
export function adoptTool(toolId: string): Promise<string> {
  return invoke("tcx_adopt_tool", { toolId });
}

// ============================================================
// Слушатели канона (тонкие обёртки; для UI используйте events.ts —
// там дедупликация слушателей, stale-guard и терминальность ровно раз)
// ============================================================

/** Прогресс скана (строго с job_id/scan_id — фильтруйте чужие). */
export function listenScanProgress(
  handler: (event: ScanProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<ScanProgressEvent>("toolchainx:scan_progress", (e) => handler(e.payload));
}

/** Терминальное событие скана. */
export function listenScanDone(
  handler: (event: ScanDoneEvent) => void,
): Promise<UnlistenFn> {
  return listen<ScanDoneEvent>("toolchainx:scan_done", (e) => handler(e.payload));
}

/** Типизированные события заданий движка (всегда с job_id/seq). */
export function listenJobEvents(
  handler: (event: JobEvent) => void,
): Promise<UnlistenFn> {
  return listen<JobEvent>("toolchainx:job_event", (e) => handler(e.payload));
}

/** Удобная типизация старта мутации (для UI-слоя). */
export function mutationRequest(
  operation: OperationKind,
  toolIds: string[],
  options: {
    confirm_unverified_sources?: boolean;
    confirm_admin_elevation?: boolean;
    version_channel?: "recommended" | "latest" | null;
    /** Превью плана (tcx_build_plan): подтверждения — чекбоксы, не ошибки. */
    preview?: boolean;
    /** Отпечаток одобренного превью (защита от устаревшего плана). */
    expected_plan_fingerprint?: string | null;
  } = {},
): EngineRequest {
  return {
    operation,
    tools: toolIds.map((tool_id) => ({ tool_id })),
    confirm_unverified_sources: options.confirm_unverified_sources ?? false,
    confirm_admin_elevation: options.confirm_admin_elevation ?? false,
    version_channel: options.version_channel ?? null,
    preview: options.preview ?? false,
    expected_plan_fingerprint: options.expected_plan_fingerprint ?? null,
  };
}

export function uninstallTool(toolId: string): Promise<void> {
  return invoke("tcx_uninstall_tool", { toolId });
}
