// ============================================================
// Toolchain Manager — типы (зеркало src-tauri models.rs)
// ============================================================
// Все поля snake_case: так сериализует serde (см. project_creator/types.ts).
// Top-level аргументы invoke() при этом camelCase (соглашение Tauri v2).
//
// ВАЖНО про дискриминаторы (serde, формат по умолчанию):
//   - unit-вариант (без данных)  -> строка:   "Missing", "Pending"
//   - вариант с данными          -> объект:   { "Installed": { "version": "..." } }
// Поэтому проверки вида `"X" in status` допустимы ТОЛЬКО после
// исключения строкового варианта (status !== "Missing").

export type ToolStatus =
  | "Missing"
  | { Installed: { version: string } }
  | { UpdateAvailable: { installed: string; recommended: string } }
  | { PathBroken: { reason: string } }
  | { ManualInstall: { reason: string } };

export type ToolDefinition = {
  id: string;
  category: string;
  display: string;
  description: string;
  icon: string | null;
  detection: {
    version_probes: string[][];
    known_paths: string[];
    registry_keys: string[];
  };
  versions: { min: string | null; recommended: string | null };
  sources: {
    windows: InstallSource[];
    linux: InstallSource[];
    macos: InstallSource[];
  };
  size_mb: number;
  needs_admin: boolean;
  path_entries: string[];
  bundled_with: string | null;
  health_checks: { label: string; command: string[] }[];
  notes: string | null;
};

export type InstallSource = {
  kind: { PkgManager: null } | { Official: null } | { Script: null };
  id: string;
  url: string | null;
  args: string[];
  extra_args: string[];
  dynamic_args: boolean;
};

// ============================================================
// Проверка окружения
// ============================================================

export type ProjectRequirements = {
  languages: string[];
  frameworks: string[];
  tools: string[];
  git_init: boolean;
  vscode_config: boolean;
  docker: boolean;
};

export type ToolRequirement = {
  tool_id: string;
  display: string;
  category: string;
  status: ToolStatus;
  size_mb: number;
  needs_admin: boolean;
  source_description: string;
};

export type EnvironmentCheck = {
  os: string;
  requirements: ToolRequirement[];
  total_size_mb: number;
  free_space_mb: number;
  enough_space: boolean;
  needs_admin_any: boolean;
  all_ready: boolean;
};

export type EnvironmentInfo = {
  os: string;
  os_version: string;
  package_managers: string[];
  tool_count: number;
};

// ============================================================
// План установки
// ============================================================

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
  size_mb: number;
  needs_admin: boolean;
  source_description: string;
  state: TaskState;
};

export type InstallPlan = {
  tasks: InstallTask[];
  total_size_mb: number;
  os: string;
};

export type InstallSession = {
  started_at: string;
  running: boolean;
  plan: InstallPlan;
  secrets: Record<string, string>;
};

// ============================================================
// События (toolchain:task_event / toolchain:install_done)
// ============================================================

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
};

// Payload события toolchain:install_done — финальный InstallPlan
// с состояниями всех задач (Source of truth по завершении).

// ============================================================
// Metadata / Health
// ============================================================

export type InstalledToolInfo = {
  path: string;
  version: string;
  installed_at: string;
  path_entries: string[];
};

export type ToolchainMetadata = {
  last_scan: string | null;
  tools: Record<string, InstalledToolInfo>;
  secrets: Record<string, string>;
  prefs: Record<string, string>;
};

export type HealthCheckResult = { label: string; ok: boolean; detail: string };
export type ToolHealth = { tool_id: string; display: string; checks: HealthCheckResult[]; ok: boolean };
export type HealthReport = { tools: ToolHealth[]; score: number; scanned_at: string };

// Событие toolchain:check_progress — по одному на проверенный инструмент
export type CheckProgressEvent = {
  done: number;
  total: number;
  tool_id: string;
  display: string;
  status: ToolStatus;
};

// ============================================================
// Хелперы для работы с дискриминаторами
// ============================================================

export function statusIsOk(status: ToolStatus | undefined): boolean {
  return !!status && status !== "Missing" && "Installed" in status;
}

export function statusLabel(status: ToolStatus): string {
  if (status === "Missing") return "Не установлен";
  if ("Installed" in status) return `✓ ${status.Installed.version}`;
  if ("UpdateAvailable" in status) return `Обновить до ${status.UpdateAvailable.recommended}`;
  if ("ManualInstall" in status) return `⚠ Вручную: ${status.ManualInstall.reason}`;
  return `⚠ ${status.PathBroken.reason}`;
}

export function statusKind(status: ToolStatus): "ok" | "update" | "broken" | "missing" | "manual" {
  if (status === "Missing") return "missing";
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
