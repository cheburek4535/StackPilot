// ============================================================
// Toolchain — витрина инструментов (определение-driven маркетплейс)
// ============================================================
// Маркетплейс работает БЕЗ снапшота скана: источник правды — каталог
// standalone Toolchain (`tcx_get_catalog` → toolchain.definitions).
// Удалённые из каталога инструменты (unity/unreal/godot) физически
// отсутствуют в этом наборе — в витрину попасть не могут.
//
// Снапшот (если есть) НАКЛАДЫВАЕТСЯ как живые факты: текущее состояние,
// версия, здоровье, происхождение. Нет снапшота — эти поля честно
// отсутствуют («нет данных скана»), а не выдумываются.
//
// Фильтры: определение-driven для определённых фактов (категория,
// установляемость, docker-альтернатива, права админа, ручная установка),
// снапшот-driven для рантайм-фактов (состояние, здоровье, происхождение,
// обновление). Значения в одной группе — OR, группы между собой — AND.

import type {
  CatalogFilters,
  DockerCapability,
  EnvironmentSnapshot,
  ExecutionMode,
  InstallSource,
  ToolDefinition,
  ToolPlatformCapabilities,
  ToolScanResult,
} from "./types";
import { isRecord } from "./types";
import {
  matchesCapabilities,
  matchesDefinitionSearch,
} from "./filters";

// ------------------------------------------------------------
// Платформы
// ------------------------------------------------------------

export type MarketplacePlatform = "windows" | "linux" | "macos";

export const MARKETPLACE_PLATFORMS: MarketplacePlatform[] = ["windows", "linux", "macos"];

/** Платформы по источникам каталога (факты, без догадок). */
function platformsBySources(def: ToolDefinition): MarketplacePlatform[] {
  const out: MarketplacePlatform[] = [];
  if (def.sources.windows.length > 0) out.push("windows");
  if (def.sources.linux.length > 0) out.push("linux");
  if (def.sources.macos.length > 0) out.push("macos");
  return out;
}

/** Платформы, на которых каталог предоставляет инструмент: заявленная
 *  availability объединяется с фактическими источниками (порядок фиксирован). */
export function platformsOfDefinition(def: ToolDefinition): MarketplacePlatform[] {
  const out: MarketplacePlatform[] = [];
  const push = (p: MarketplacePlatform): void => {
    if (!out.includes(p)) out.push(p);
  };
  for (const p of MARKETPLACE_PLATFORMS) {
    if ((def.platform_availability ?? []).includes(p)) push(p);
  }
  for (const p of platformsBySources(def)) push(p);
  return out;
}

/** Источники инструмента для конкретной ОС. */
export function sourcesForPlatform(def: ToolDefinition, os: string): InstallSource[] {
  if (os === "windows") return def.sources.windows;
  if (os === "linux") return def.sources.linux;
  return def.sources.macos;
}

// ------------------------------------------------------------
// Возможности, выводимые из определения (зеркало бэкенда)
// ------------------------------------------------------------

/** Ручная установка по каталогу: manual_install БЕЗ bundled-хоста.
 *  Bundled-тулы (pip с python, npm с node) приходят вместе с хостом —
 *  «ставьте вручную» для них ложь. */
export function isManualOnly(def: ToolDefinition): boolean {
  return !!def.manual_install && !def.bundled_with;
}

/** Флаги возможностей из ОДНОГО определения (для режима без снапшота). */
export function capabilitiesOfDefinition(def: ToolDefinition): ToolPlatformCapabilities {
  const detectable =
    def.detection.version_probes.length > 0 ||
    def.detection.known_paths.length > 0 ||
    def.detection.registry_keys.length > 0;
  const installable =
    (def.sources.windows.length > 0 ||
      def.sources.linux.length > 0 ||
      def.sources.macos.length > 0) &&
    !isManualOnly(def);
  return {
    detectable,
    installable,
    updatable: installable,
    removable: def.declared_capabilities?.removable === true,
    repairable: def.declared_capabilities?.repairable === true,
    health_checkable: def.health_checks.length > 0,
    manual_instructions_available: isManualOnly(def),
    docker_alternative_available: !!def.docker,
  };
}

// ------------------------------------------------------------
// Элемент витрины
// ------------------------------------------------------------

export type MarketplaceItem = {
  def: ToolDefinition;
  /** ОС, для которой строится витрина. */
  os: string;
  /** Живой результат скана (null — скана ещё нет). */
  scan: ToolScanResult | null;
  /** Есть источник локальной установки на этой ОС (не manual-only). */
  installable: boolean;
  /** Применимость: manual-only / нет источника / устанавливаемо / встроенный. */
  applicability: "installable" | "manual_only" | "no_source" | "built_in";
  /** Источники для текущей ОС. */
  sources_for_os: InstallSource[];
  /** Платформы из каталога (заявленные + с источниками). */
  availability: MarketplacePlatform[];
  /** Контроль целостности источников текущей ОС. */
  checksum: "all_verified" | "partial" | "none" | "no_sources";
  /** Docker-альтернатива как МЕТАДАННЫЕ (не режим исполнения). */
  docker_alternative: DockerCapability | null;
};

/** Режим исполнения элемента (для фильтра): в standalone всё локально;
 *  docker — только если сам факт скана говорит о Docker-происхождении. */
export function marketplaceExecutionMode(item: MarketplaceItem): ExecutionMode {
  const scan = item.scan;
  if (scan && (scan.applicability.kind === "docker_default" || scan.provenance.kind === "docker")) {
    return "docker";
  }
  return "host";
}

export function checksumStatus(sources: InstallSource[]): MarketplaceItem["checksum"] {
  if (sources.length === 0) return "no_sources";
  let verified = 0;
  for (const s of sources) if (s.sha256) verified += 1;
  if (verified === sources.length) return "all_verified";
  if (verified > 0) return "partial";
  return "none";
}

function applicabilityOf(def: ToolDefinition, os: string): MarketplaceItem["applicability"] {
  if (isManualOnly(def)) return "manual_only";
  const hasAnySource =
    def.sources.windows.length > 0 ||
    def.sources.linux.length > 0 ||
    def.sources.macos.length > 0;
  if (!hasAnySource) {
    // Без источников нигде: встроенный в ОС (curl/tar) или чисто
    // информационная запись — по фактам каталога. Bundled-тулы
    // (pip с python) тоже попадают сюда: «в комплекте с хостом».
    return "built_in";
  }
  if (sourcesForPlatform(def, os).length === 0) return "no_source";
  return "installable";
}

function dockerOf(def: ToolDefinition): DockerCapability | null {
  const raw = def.docker;
  return isRecord(raw) ? (raw as DockerCapability) : null;
}

/**
 * Строит список элементов витрины из каталога standalone Toolchain.
 * Порядок — порядок каталога. Снапшот накладывается по tool_id (живые
 * данные перекрывают кэш через liveSnapshot вызывающего).
 */
export function buildMarketplaceItems(
  definitions: Record<string, ToolDefinition>,
  snapshot: EnvironmentSnapshot | null,
  os: string,
): MarketplaceItem[] {
  const scanById = new Map<string, ToolScanResult>();
  for (const t of snapshot?.tools ?? []) scanById.set(t.tool_id, t);

  const items: MarketplaceItem[] = [];
  for (const def of Object.values(definitions)) {
    const sources = sourcesForPlatform(def, os);
    items.push({
      def,
      os,
      scan: scanById.get(def.id) ?? null,
      installable: sources.length > 0 && !isManualOnly(def),
      applicability: applicabilityOf(def, os),
      sources_for_os: sources,
      availability: platformsOfDefinition(def),
      checksum: checksumStatus(sources),
      docker_alternative: dockerOf(def),
    });
  }
  return items;
}

// ------------------------------------------------------------
// Поиск и фильтрация
// ------------------------------------------------------------

/** Поиск по витрине: display, id, aliases, категория, описание,
 *  notes, docs_url, source_url (метаданные определения). */
export function matchesMarketplaceSearch(item: MarketplaceItem, query: string): boolean {
  return matchesDefinitionSearch(item.def, query);
}

/** Режим исполнения одного инструмента в standalone-витрине. */
function itemExecutionMode(item: MarketplaceItem): ExecutionMode {
  return marketplaceExecutionMode(item);
}

// ------------------------------------------------------------
// Правдивое действие карточки витрины
// ------------------------------------------------------------

export type MarketplaceAction =
  | { kind: "install"; label: string }
  | { kind: "update"; label: string }
  | { kind: "installed"; label: string }
  | { kind: "manual"; label: string }
  | { kind: "no_source"; label: string }
  | { kind: "built_in"; label: string };

/** Правдивое действие карточки: живой факт скана (если есть) важнее
 *  каталога. УСТАНОВЛЕННЫЙ инструмент никогда не предлагается ставить
 *  заново («Установить локально» — только при честном «не установлен»);
 *  доступное обновление — отдельная кнопка «Обновить», и только когда
 *  есть ИСТОЧНИК на этой ОС (bundled-тулы вроде pip обновляются вместе
 *  с хостом, а не отдельно — кнопка была бы тупиком). */
export function marketplaceAction(item: MarketplaceItem, busy = false): MarketplaceAction {
  const scan = item.scan;
  if (scan) {
    if (scan.state.kind === "update_available" && item.installable) {
      return { kind: "update", label: "Обновить" };
    }
    if (
      scan.state.kind === "installed_healthy" ||
      scan.state.kind === "installed_health_unknown" ||
      scan.state.kind === "installed_unhealthy"
    ) {
      return { kind: "installed", label: "Установлен" };
    }
  }
  if (item.installable) {
    return { kind: "install", label: busy ? "Проверка…" : "Установить локально" };
  }
  if (item.applicability === "manual_only") {
    return { kind: "manual", label: "Только вручную" };
  }
  if (item.applicability === "no_source") {
    return { kind: "no_source", label: "Нет источника на этой ОС" };
  }
  if (item.def.bundled_with) {
    return { kind: "built_in", label: `В комплекте с ${item.def.bundled_with}` };
  }
  return { kind: "built_in", label: "Встроен в ОС" };
}

/** Полный предикат витрины. Группы — AND, значения внутри группы — OR.
 *
 * Рантайм-группы (состояние/здоровье/происхождение/обновление) опираются
 * на факты СКАНА. Скана нет — фактов нет: эти группы БЕЗДЕЙСТВУЮТ
 * (регрессия «персистентные фильтры другого режима молча прячут всю
 * витрину», когда поля группы скрыты в UI, но предикат ещё применялся). */
export function makeMarketplacePredicate(filters: CatalogFilters) {
  return (item: MarketplaceItem): boolean => {
    if (!matchesMarketplaceSearch(item, filters.search)) return false;
    if (filters.categories.length > 0 && !filters.categories.includes(item.def.category)) {
      return false;
    }
    const scan = item.scan;
    if (scan && filters.states.length > 0) {
      if (!filters.states.includes(scan.state.kind)) return false;
    }
    if (scan && filters.provenance.length > 0) {
      if (!filters.provenance.includes(scan.provenance.kind)) return false;
    }
    const caps = scan ? scan.capabilities : capabilitiesOfDefinition(item.def);
    if (!matchesCapabilities(caps, filters.capabilities)) return false;
    if (filters.execution_modes.length > 0) {
      if (!filters.execution_modes.includes(itemExecutionMode(item))) return false;
    }
    if (scan && filters.health.length > 0) {
      const kind = scan.health?.state.kind ?? "not_checked";
      if (!filters.health.includes(kind)) return false;
    }
    if (filters.admin_only && item.def.needs_admin !== true) return false;
    if (filters.update_only) {
      // Факт «доступно обновление» — из скана; без скана фильтр невидим
      // (регрессия «reviewUpdates в Manage → витрина без скана пуста»).
      if (scan && scan.state.kind !== "update_available") return false;
    }
    if (filters.manual_only && !isManualOnly(item.def)) return false;
    if (filters.installable && !item.installable) return false;
    if (filters.has_docker_alternative && !item.def.docker) return false;
    return true;
  };
}

/** Отфильтрованный список витрины (порядок каталога сохранён). */
export function applyMarketplaceFilters(
  items: MarketplaceItem[],
  filters: CatalogFilters,
): MarketplaceItem[] {
  const predicate = makeMarketplacePredicate(filters);
  return items.filter(predicate);
}

// ------------------------------------------------------------
// Счётчики по группам фильтров (только по ТЕКУЩЕМУ набору данных)
// ------------------------------------------------------------

/** Количество по категориям (стабильный порядок набора). */
export function marketplaceCategoryCounts(items: MarketplaceItem[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const item of items) {
    counts.set(item.def.category, (counts.get(item.def.category) ?? 0) + 1);
  }
  return counts;
}

/** Количество по состояниям скана (0 — если скана нет). */
export function marketplaceStateCounts(items: MarketplaceItem[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const item of items) {
    const kind = item.scan?.state.kind ?? null;
    if (kind) counts.set(kind, (counts.get(kind) ?? 0) + 1);
  }
  return counts;
}

/** Количество по состояниям здоровья. */
export function marketplaceHealthCounts(items: MarketplaceItem[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const item of items) {
    const kind = item.scan?.health?.state.kind ?? "not_checked";
    counts.set(kind, (counts.get(kind) ?? 0) + 1);
  }
  return counts;
}

/** Количество по происхождению. */
export function marketplaceProvenanceCounts(items: MarketplaceItem[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const item of items) {
    const kind = item.scan?.provenance.kind ?? null;
    if (kind) counts.set(kind, (counts.get(kind) ?? 0) + 1);
  }
  return counts;
}

/** Количество по флагу возможности (определение или скан). */
export function marketplaceCapabilityCounts(
  items: MarketplaceItem[],
  flag: keyof ToolPlatformCapabilities,
): number {
  let n = 0;
  for (const item of items) {
    const caps = item.scan ? item.scan.capabilities : capabilitiesOfDefinition(item.def);
    if (caps[flag]) n += 1;
  }
  return n;
}

/** Количество по режиму исполнения. */
export function marketplaceExecutionCounts(
  items: MarketplaceItem[],
  mode: ExecutionMode,
): number {
  return items.filter((item) => marketplaceExecutionMode(item) === mode).length;
}

/** Количество по булевому признаку каталога. */
export function marketplaceBooleanCount(
  items: MarketplaceItem[],
  flag:
    | "installable"
    | "manual_only"
    | "update_only"
    | "admin_only"
    | "has_docker_alternative",
): number {
  let n = 0;
  for (const item of items) {
    switch (flag) {
      case "installable":
        if (item.installable) n += 1;
        break;
      case "manual_only":
        if (isManualOnly(item.def)) n += 1;
        break;
      case "update_only":
        if (item.scan?.state.kind === "update_available") n += 1;
        break;
      case "admin_only":
        if (item.def.needs_admin) n += 1;
        break;
      case "has_docker_alternative":
        if (item.def.docker) n += 1;
        break;
    }
  }
  return n;
}