// ============================================================
// Toolchain — предикаты фильтрации каталога (чистые функции)
// ============================================================
// Фильтрация — единственное, что фронтенд «решает» сам (контракт §3):
// статусы и факты приходят из бэкенда, UI только отбирает и группирует.

import type {
  CatalogFilters,
  CatalogSort,
  EnvironmentSnapshot,
  ExecutionMode,
  HealthState,
  ProvenanceKind,
  ToolDefinition,
  ToolPlatformCapabilities,
  ToolScanResult,
  ToolStateKind,
} from "./types";
import { isRecord } from "./types";

export function defaultCatalogFilters(): CatalogFilters {
  return {
    search: "",
    categories: [],
    states: [],
    provenance: [],
    capabilities: [],
    execution_modes: [],
    health: [],
    admin_only: false,
    update_only: false,
    manual_only: false,
    installable: false,
    has_docker_alternative: false,
  };
}

// ------------------------------------------------------------
// Валидация персистентных настроек фильтров
// ------------------------------------------------------------

const KNOWN_STATE_KINDS = new Set<string>([
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
]);

const KNOWN_PROVENANCE_KINDS = new Set<string>([
  "stack_pilot_managed",
  "external",
  "package_manager",
  "system",
  "bundled_with",
  "docker",
  "unknown",
]);

const KNOWN_CAPABILITY_FLAGS = new Set<string>([
  "detectable",
  "installable",
  "updatable",
  "removable",
  "repairable",
  "health_checkable",
  "manual_instructions_available",
  "docker_alternative_available",
]);

const KNOWN_EXECUTION_MODES = new Set<string>(["host", "docker"]);

const KNOWN_HEALTH_KINDS = new Set<string>([
  "not_checked",
  "checking",
  "healthy",
  "degraded",
  "unhealthy",
  "unavailable",
  "unsupported",
  "no_checks_defined",
  "failed_to_run",
]);

function stringArrayOf(raw: unknown, known: Set<string>): string[] {
  if (!Array.isArray(raw)) return [];
  const out: string[] = [];
  for (const v of raw) {
    if (typeof v === "string" && known.has(v) && !out.includes(v)) out.push(v);
  }
  return out;
}

function stringArrayFree(raw: unknown): string[] {
  if (!Array.isArray(raw)) return [];
  const out: string[] = [];
  for (const v of raw) {
    if (typeof v === "string" && v && !out.includes(v)) out.push(v);
  }
  return out;
}

/**
 * Валидирует произвольный payload настроек фильтров (локальное хранилище,
 * IPC-границы): неизвестные kind'ы, неверные типы и дубликаты отбрасываются,
 * значения возвращаются к известным множествам. НИКОГДА не бросает.
 */
export function validateCatalogFilters(raw: unknown): CatalogFilters {
  const base = defaultCatalogFilters();
  if (!isRecord(raw)) return base;
  return {
    search: typeof raw.search === "string" ? raw.search : "",
    categories: stringArrayFree(raw.categories),
    states: stringArrayOf(raw.states, KNOWN_STATE_KINDS) as ToolStateKind[],
    provenance: stringArrayOf(raw.provenance, KNOWN_PROVENANCE_KINDS) as ProvenanceKind[],
    capabilities: stringArrayOf(raw.capabilities, KNOWN_CAPABILITY_FLAGS) as (
      keyof ToolPlatformCapabilities
    )[],
    execution_modes: stringArrayOf(raw.execution_modes, KNOWN_EXECUTION_MODES) as ExecutionMode[],
    health: stringArrayOf(raw.health, KNOWN_HEALTH_KINDS) as HealthState["kind"][],
    admin_only: raw.admin_only === true,
    update_only: raw.update_only === true,
    manual_only: raw.manual_only === true,
    installable: raw.installable === true,
    has_docker_alternative: raw.has_docker_alternative === true,
  };
}

/** Контекст предиката: метаданные каталога для фактов, которых нет в срезе скана. */
export type CatalogFilterContext = {
  definitions?: Record<string, ToolDefinition>;
};

/** Нормализованный поиск по имени/id/категории/описанию. */
export function matchesSearch(tool: ToolScanResult, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return (
    tool.display.toLowerCase().includes(q) ||
    tool.tool_id.toLowerCase().includes(q) ||
    tool.category.toLowerCase().includes(q)
  );
}

export function matchesCategory(tool: ToolScanResult, categories: string[]): boolean {
  return categories.length === 0 || categories.includes(tool.category);
}

/** Совпадение по презентационному состоянию (kind из discriminated union). */
export function matchesState(tool: ToolScanResult, states: CatalogFilters["states"]): boolean {
  return states.length === 0 || states.includes(tool.state.kind);
}

export function matchesProvenance(
  tool: ToolScanResult,
  provenance: ProvenanceKind[],
): boolean {
  if (provenance.length === 0) return true;
  return provenance.includes(tool.provenance.kind);
}

/** Требуемые флаги возможностей: И-логика между разными флагами. */
export function matchesCapabilities(
  caps: ToolPlatformCapabilities,
  required: (keyof ToolPlatformCapabilities)[],
): boolean {
  return required.every((flag) => caps[flag]);
}

export function matchesExecutionMode(
  tool: ToolScanResult,
  modes: ExecutionMode[],
): boolean {
  if (modes.length === 0) return true;
  // Режим исполнения выводится из применимости/происхождения:
  // docker_default/docker-происхождение = docker, иначе host.
  const mode: ExecutionMode =
    tool.applicability.kind === "docker_default" || tool.provenance.kind === "docker"
      ? "docker"
      : "host";
  return modes.includes(mode);
}

/** Здоровье: совпадение по фактическому состоянию (null = ещё не проверялся). */
export function matchesHealth(
  tool: ToolScanResult,
  health: CatalogFilters["health"],
): boolean {
  return health.length === 0 || health.includes(tool.health?.state.kind ?? "not_checked");
}

/** Обновление доступно (по презентационному состоянию от бэкенда). */
function hasUpdate(tool: ToolScanResult): boolean {
  return tool.state.kind === "update_available";
}

/** Ручная установка — применимость manual_only. */
function isManualOnly(tool: ToolScanResult): boolean {
  return tool.applicability.kind === "manual_only";
}

/** Полный предикат каталога Manage Everything. */
export function makeCatalogPredicate(
  filters: CatalogFilters,
  context: CatalogFilterContext = {},
) {
  return (tool: ToolScanResult): boolean => {
    if (!matchesSearch(tool, filters.search)) return false;
    if (!matchesCategory(tool, filters.categories)) return false;
    if (!matchesState(tool, filters.states)) return false;
    if (!matchesProvenance(tool, filters.provenance)) return false;
    if (!matchesCapabilities(tool.capabilities, filters.capabilities)) return false;
    if (!matchesExecutionMode(tool, filters.execution_modes)) return false;
    if (!matchesHealth(tool, filters.health)) return false;
    if (filters.update_only && !hasUpdate(tool)) return false;
    if (filters.manual_only && !isManualOnly(tool)) return false;
    if (filters.installable && !tool.capabilities.installable) return false;
    if (filters.has_docker_alternative && !tool.capabilities.docker_alternative_available) {
      return false;
    }
    if (filters.admin_only) {
      const def = context.definitions?.[tool.tool_id];
      // Факт «нужен админ» берётся только из каталога; нет метаданных —
      // инструмент под фильтр не попадает (не угадываем).
      if (def?.needs_admin !== true) return false;
    }
    return true;
  };
}

/** Отфильтрованный список инструментов снапшота (порядок каталога сохранён). */
export function applyCatalogFilters(
  snapshot: EnvironmentSnapshot | null,
  filters: CatalogFilters,
  context: CatalogFilterContext = {},
): ToolScanResult[] {
  if (!snapshot) return [];
  const predicate = makeCatalogPredicate(filters, context);
  return snapshot.tools.filter(predicate);
}

// ------------------------------------------------------------
// Сортировка каталога
// ------------------------------------------------------------

/** Порядок серьёзности состояния (для сортировки «проблемные сверху»). */
const STATE_SEVERITY: Record<ToolStateKind, number> = {
  path_broken: 0,
  installed_unhealthy: 1,
  scan_failed: 2,
  update_available: 3,
  missing: 4,
  installed_health_unknown: 5,
  scan_pending: 6,
  manual_install: 7,
  docker_managed: 8,
  unsupported_platform: 9,
  built_in_system: 10,
  installed_healthy: 11,
};

/** Отсортированная копия списка; исходный порядок не мутируется. */
export function sortCatalogTools(tools: ToolScanResult[], sort: CatalogSort): ToolScanResult[] {
  const copy = [...tools];
  switch (sort) {
    case "name_asc":
      copy.sort((a, b) => a.display.localeCompare(b.display));
      break;
    case "name_desc":
      copy.sort((a, b) => b.display.localeCompare(a.display));
      break;
    case "category":
      copy.sort(
        (a, b) =>
          a.category.localeCompare(b.category) || a.display.localeCompare(b.display),
      );
      break;
    case "status":
      copy.sort(
        (a, b) =>
          STATE_SEVERITY[a.state.kind] - STATE_SEVERITY[b.state.kind] ||
          a.display.localeCompare(b.display),
      );
      break;
    default:
      break; // catalog — порядок снапшота
  }
  return copy;
}

/** Доступные категории по снапшоту (стабильный порядок появления). */
export function availableCategories(snapshot: EnvironmentSnapshot | null): string[] {
  const seen: string[] = [];
  for (const tool of snapshot?.tools ?? []) {
    if (!seen.includes(tool.category)) seen.push(tool.category);
  }
  return seen;
}

/**
 * Поиск в СТАТИЧНОМ каталоге (до первого скана): живых состояний нет,
 * ищем по метаданным определения. Поля поиска: display, id, aliases,
 * категория, описание, notes, docs_url, source_url.
 */
export function matchesDefinitionSearch(def: ToolDefinition, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const haystack = [
    def.display,
    def.id,
    def.category,
    def.description,
    def.notes ?? "",
    def.docs_url ?? "",
    def.source_url ?? "",
    ...(def.aliases ?? []),
  ];
  // Защита от малиформированных определений: отсутствующее поле не роняет
  // поиск (честное «не совпало»), а не падает на undefined.toLowerCase().
  return haystack.some((field) => (field ?? "").toLowerCase().includes(q));
}
