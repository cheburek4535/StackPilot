// Тесты витрины инструментов: определение-driven маркетплейс.
import { describe, expect, it } from "vitest";
// Реальный standalone-каталог: читаем как JSON-модуль (без node:fs),
// чтобы проверить гарантию «удалённые движки отсутствуют в витрине».
import toolsJson from "../../../../src-tauri/src/modules/toolchain/tools.json";
import {
  applyMarketplaceFilters,
  buildMarketplaceItems,
  capabilitiesOfDefinition,
  checksumStatus,
  makeMarketplacePredicate,
  marketplaceAction,
  marketplaceBooleanCount,
  marketplaceCapabilityCounts,
  marketplaceExecutionCounts,
  marketplaceHealthCounts,
  marketplaceProvenanceCounts,
  marketplaceStateCounts,
  matchesMarketplaceSearch,
  platformsOfDefinition,
  sourcesForPlatform,
  type MarketplaceItem,
} from "./marketplace";
import { defaultCatalogFilters } from "./filters";
import type {
  CatalogFilters,
  EnvironmentSnapshot,
  ToolDefinition,
  ToolScanResult,
} from "./types";

/** Фильтры с типизированным патчем (литералы получают контекстный тип). */
function withFilters(patch: Partial<CatalogFilters>): CatalogFilters {
  return { ...defaultCatalogFilters(), ...patch };
}

// ------------------------------------------------------------
// Фабрики
// ------------------------------------------------------------

function def(overrides: Partial<ToolDefinition> = {}): ToolDefinition {
  return {
    id: "git",
    category: "vcs",
    display: "Git",
    description: "Система контроля версий",
    icon: "git.svg",
    detection: { version_probes: [["git", "--version"]], known_paths: [], registry_keys: [] },
    versions: { min: "2.30", recommended: "2.45" },
    sources: {
      windows: [{ kind: "Official", id: "git-exe", url: null, args: [], extra_args: [], dynamic_args: false, sha256: "abc" }],
      linux: [{ kind: "PkgManager", id: "git", url: null, args: [], extra_args: [], dynamic_args: false }],
      macos: [{ kind: "PkgManager", id: "git", url: null, args: [], extra_args: [], dynamic_args: false }],
    },
    size_mb: 60,
    needs_admin: false,
    path_entries: [],
    bundled_with: null,
    health_checks: [],
    notes: null,
    ...overrides,
  };
}

function scanResult(toolId: string, overrides: Partial<ToolScanResult> = {}): ToolScanResult {
  return {
    tool_id: toolId,
    display: toolId,
    category: "vcs",
    icon: null,
    detection: { kind: "not_detected" },
    installs: [],
    path_findings: [],
    health: null,
    applicability: { kind: "installable" },
    capabilities: {
      detectable: true,
      installable: true,
      updatable: true,
      removable: false,
      repairable: false,
      health_checkable: false,
      manual_instructions_available: false,
      docker_alternative_available: false,
    },
    provenance: { kind: "unknown" },
    bundled_with: null,
    version_assessment: { kind: "unknown" },
    state: { kind: "missing" },
    error: null,
    duration_ms: 1,
    ...overrides,
  };
}

const EMPTY_SNAPSHOT: EnvironmentSnapshot | null = null;

// ------------------------------------------------------------
// Поиск (только по определению — работает без скана)
// ------------------------------------------------------------

describe("поиск витрины — только определения", () => {
  const git = def();
  const postgres = def({
    id: "postgresql",
    display: "PostgreSQL",
    category: "database",
    description: "Реляционная СУБД",
    aliases: ["psql", "postgres"],
    notes: "docker: postgres:17",
    docs_url: "https://www.postgresql.org/docs/",
    source_url: "https://github.com/postgres/postgres",
  });

  it("ищет по display, id и категории", () => {
    expect(matchesMarketplaceSearch({ def: git, os: "windows" } as never, "GIT")).toBe(true);
    expect(matchesMarketplaceSearch({ def: git, os: "windows" } as never, "vcs")).toBe(true);
    expect(matchesMarketplaceSearch({ def: postgres, os: "windows" } as never, "postgres")).toBe(true);
  });

  it("ищет по псевдонимам", () => {
    expect(matchesMarketplaceSearch({ def: postgres, os: "windows" } as never, "psql")).toBe(true);
    expect(matchesMarketplaceSearch({ def: postgres, os: "windows" } as never, "postgres")).toBe(true);
  });

  it("ищет по описанию, notes, docs_url и source_url", () => {
    expect(matchesMarketplaceSearch({ def: postgres, os: "windows" } as never, "реляционная")).toBe(true);
    expect(matchesMarketplaceSearch({ def: postgres, os: "windows" } as never, "postgres:17")).toBe(true);
    expect(matchesMarketplaceSearch({ def: postgres, os: "windows" } as never, "postgresql.org")).toBe(true);
    expect(matchesMarketplaceSearch({ def: postgres, os: "windows" } as never, "github.com/postgres")).toBe(true);
    expect(matchesMarketplaceSearch({ def: git, os: "windows" } as never, "небывалое")).toBe(false);
  });

  it("пустой запрос — всё проходит", () => {
    expect(matchesMarketplaceSearch({ def: git, os: "windows" } as never, "   ")).toBe(true);
  });
});

// ------------------------------------------------------------
// Построение витрины
// ------------------------------------------------------------

describe("buildMarketplaceItems — работает без снапшота", () => {
  it("строит элемент для каждого определения каталога", () => {
    const items = buildMarketplaceItems(
      { git: def(), node: def({ id: "node", display: "Node.js" }) },
      null,
      "windows",
    );
    expect(items.map((i) => i.def.id)).toEqual(["git", "node"]);
  });

  it("без снапшота живые факты честно отсутствуют (null), а не выдуманы", () => {
    const items = buildMarketplaceItems({ git: def() }, null, "windows");
    expect(items[0].scan).toBeNull();
    expect(items[0].installable).toBe(true);
  });

  it("снапшот накладывает живое состояние по tool_id", () => {
    const snap = {
      tools: [scanResult("git", { state: { kind: "installed_healthy", version: "2.45" } })],
    } as unknown as EnvironmentSnapshot;
    const items = buildMarketplaceItems({ git: def() }, snap, "windows");
    expect(items[0].scan?.state.kind).toBe("installed_healthy");
  });

  it("установляемость зависит от ОС и ручной установки", () => {
    const qtOnly = def({
      id: "qt",
      sources: { windows: [{ kind: "QtOnline", id: "qt", url: null, args: [], extra_args: [], dynamic_args: false }], linux: [], macos: [] },
    });
    expect(buildMarketplaceItems({ qtOnly }, null, "windows")[0].installable).toBe(true);
    expect(buildMarketplaceItems({ qtOnly }, null, "linux")[0].installable).toBe(false);

    const android = def({ id: "android", manual_install: "Установите Android Studio" });
    expect(buildMarketplaceItems({ android }, null, "windows")[0].installable).toBe(false);
    expect(buildMarketplaceItems({ android }, null, "windows")[0].applicability).toBe("manual_only");
  });

  it("платформы берутся из заявленной availability и источников", () => {
    expect(platformsOfDefinition(def())).toEqual(["windows", "linux", "macos"]);
    const xcode = def({
      id: "xcodebuild",
      platform_availability: ["macos"],
      sources: { windows: [], linux: [], macos: [] },
    });
    expect(platformsOfDefinition(xcode)).toEqual(["macos"]);
    const winOnly = def({
      id: "w",
      sources: { windows: [{ kind: "Official", id: "w", url: null, args: [], extra_args: [], dynamic_args: false }], linux: [], macos: [] },
    });
    expect(platformsOfDefinition(winOnly)).toEqual(["windows"]);
  });

  it("контроль целостности честен: все/частично/нет/нет источников", () => {
    const src = (sha?: string): ToolDefinition["sources"]["windows"][number] => ({
      kind: "Official",
      id: "s",
      url: null,
      args: [],
      extra_args: [],
      dynamic_args: false,
      sha256: sha,
    });
    expect(checksumStatus([src("aa"), src("bb")])).toBe("all_verified");
    expect(checksumStatus([src("aa"), src()])).toBe("partial");
    expect(checksumStatus([src(), src()])).toBe("none");
    expect(checksumStatus([])).toBe("no_sources");
  });

  it("docker-альтернатива — метаданные, а не режим исполнения", () => {
    const pg = def({
      id: "postgresql",
      docker: { image: "postgres:17", notes: "альтернатива" },
    });
    const item = buildMarketplaceItems({ pg }, null, "windows")[0];
    expect(item.docker_alternative?.image).toBe("postgres:17");
    // В standalone режим исполнения — всегда host (docker не превращает
    // инструмент в «управляемый Docker»).
    expect(item.installable).toBe(true);
    expect(marketplaceExecutionCounts([item], "host")).toBe(1);
  });
});

// ------------------------------------------------------------
// Удалённые инструменты не попадают в UI
// ------------------------------------------------------------

describe("удалённые инструменты отсутствуют в витрине", () => {
  it("реальный standalone-каталог не содержит движков (unity/unreal/godot)", () => {
    const catalog = toolsJson as unknown as ToolDefinition[];
    const ids = catalog.map((t) => t.id);
    expect(ids).not.toContain("unity");
    expect(ids).not.toContain("unreal");
    expect(ids).not.toContain("godot");
    expect(ids.length).toBeGreaterThan(40);
  });

  it("витрина строится ТОЛЬКО из переданного standalone-каталога", () => {
    // Легаси-движки физически не входят в набор определений tcx_get_catalog;
    // даже если id пришёл извне — витрина его не знает, потому что строится
    // по словарю каталога, а не по снапшоту/истории.
    const items = buildMarketplaceItems(
      { git: def(), node: def({ id: "node", display: "Node.js" }) },
      {
        tools: [scanResult("unity", { state: { kind: "missing" } })],
      } as unknown as EnvironmentSnapshot,
      "windows",
    );
    expect(items.map((i) => i.def.id)).not.toContain("unity");
  });
});

// ------------------------------------------------------------
// Фильтры: семантика AND между группами, OR внутри группы
// ------------------------------------------------------------

describe("фильтры витрины", () => {
  const postgres = def({
    id: "postgresql",
    display: "PostgreSQL",
    category: "database",
    needs_admin: true,
    docker: { image: "postgres:17" },
  });
  const android = def({ id: "android", manual_install: "Установите Android Studio" });
  const git = def();

  const snapshot = {
    tools: [
      scanResult("postgresql", {
        state: { kind: "update_available", installed: "16", recommended: "17" },
        health: { state: { kind: "healthy" }, results: [] },
        provenance: { kind: "external" },
      }),
      scanResult("android", {
        state: { kind: "missing" },
        capabilities: { ...scanResult("x").capabilities, installable: false },
      }),
      scanResult("git", {
        state: { kind: "installed_healthy", version: "2.45" },
        health: { state: { kind: "degraded" }, results: [] },
      }),
    ],
  } as unknown as EnvironmentSnapshot;

  const items = buildMarketplaceItems(
    { postgresql: postgres, android, git },
    snapshot,
    "windows",
  );
  const byId = (id: string): MarketplaceItem => items.find((i) => i.def.id === id)!;

  it("OR внутри группы: несколько состояний дают объединение", () => {
    const f = withFilters({ states: ["update_available", "installed_healthy"] });
    const filtered = applyMarketplaceFilters(items, f);
    expect(filtered.map((i) => i.def.id).sort()).toEqual(["git", "postgresql"]);
  });

  it("AND между группами: состояние + категория сужают пересечением", () => {
    const f = withFilters({ states: ["update_available"], categories: ["database"] });
    const filtered = applyMarketplaceFilters(items, f);
    expect(filtered.map((i) => i.def.id)).toEqual(["postgresql"]);
  });

  it("фильтр состояния требует скан: без снапшота рантайм-группы БЕЗДЕЙСТВУЮТ", () => {
    const noScan = buildMarketplaceItems(
      { postgresql: postgres, android, git },
      null,
      "windows",
    );
    // Регрессия «персистентные фильтры другого режима прячут всю витрину»:
    // без фактов скана фильтр состояния не может отсечь ни одной карточки —
    // и не должен (поля группы скрыты в UI, предикат их игнорирует).
    const f = withFilters({ states: ["missing"] });
    const filtered = applyMarketplaceFilters(noScan, f);
    expect(filtered.map((i) => i.def.id).sort()).toEqual(["android", "git", "postgresql"]);
    expect(marketplaceStateCounts(noScan).size).toBe(0);
  });

  it("здоровье, происхождение и обновление — рантайм-фильтры", () => {
    expect(applyMarketplaceFilters(items, { ...defaultCatalogFilters(), health: ["healthy"] }).map((i) => i.def.id)).toEqual(["postgresql"]);
    expect(applyMarketplaceFilters(items, { ...defaultCatalogFilters(), provenance: ["external"] }).map((i) => i.def.id)).toEqual(["postgresql"]);
    expect(applyMarketplaceFilters(items, { ...defaultCatalogFilters(), update_only: true }).map((i) => i.def.id)).toEqual(["postgresql"]);
  });

  it("админ, ручная установка, установляемость, docker — факты каталога", () => {
    expect(applyMarketplaceFilters(items, { ...defaultCatalogFilters(), admin_only: true }).map((i) => i.def.id)).toEqual(["postgresql"]);
    expect(applyMarketplaceFilters(items, { ...defaultCatalogFilters(), manual_only: true }).map((i) => i.def.id)).toEqual(["android"]);
    expect(applyMarketplaceFilters(items, { ...defaultCatalogFilters(), installable: true }).map((i) => i.def.id).sort()).toEqual(["git", "postgresql"]);
    expect(applyMarketplaceFilters(items, { ...defaultCatalogFilters(), has_docker_alternative: true }).map((i) => i.def.id)).toEqual(["postgresql"]);
  });

  it("возможности: определение без скана, скан — при наличии", () => {
    const f = withFilters({ capabilities: ["installable"] });
    expect(applyMarketplaceFilters(items, f).map((i) => i.def.id).sort()).toEqual(["git", "postgresql"]);
    expect(capabilitiesOfDefinition(android).installable).toBe(false);
    expect(capabilitiesOfDefinition(postgres).docker_alternative_available).toBe(true);
  });

  it("режим исполнения: в standalone всё — host, docker даёт ноль кандидатов", () => {
    expect(marketplaceExecutionCounts(items, "host")).toBe(3);
    expect(marketplaceExecutionCounts(items, "docker")).toBe(0);
    expect(applyMarketplaceFilters(items, { ...defaultCatalogFilters(), execution_modes: ["docker"] })).toEqual([]);
  });

  it("счётчики соответствуют набору; нулевые группы видны как 0", () => {
    expect(marketplaceBooleanCount(items, "installable")).toBe(2);
    expect(marketplaceBooleanCount(items, "admin_only")).toBe(1);
    expect(marketplaceBooleanCount(items, "manual_only")).toBe(1);
    expect(marketplaceBooleanCount(items, "update_only")).toBe(1);
    expect(marketplaceBooleanCount(items, "has_docker_alternative")).toBe(1);
    expect(marketplaceCapabilityCounts(items, "installable")).toBe(2);
    expect(marketplaceCapabilityCounts(items, "removable")).toBe(0);
    expect(marketplaceHealthCounts(items).get("healthy")).toBe(1);
    expect(marketplaceHealthCounts(items).get("degraded")).toBe(1);
    expect(marketplaceProvenanceCounts(items).get("external")).toBe(1);
  });

  it("sourcesForPlatform возвращает источники конкретной ОС", () => {
    expect(sourcesForPlatform(def(), "linux").length).toBe(1);
    expect(sourcesForPlatform(def({ sources: { windows: [], linux: [], macos: [] } }), "windows").length).toBe(0);
  });
});

// ------------------------------------------------------------
// makeMarketplacePredicate — все группы фильтров применимы
// ------------------------------------------------------------

describe("makeMarketplacePredicate — все фильтры функциональны", () => {
  it("комбинирует каждую группу (каждая включается в решение)", () => {
    const item = {
      def: def({ id: "git", docker: { image: "x" }, needs_admin: true }),
      os: "windows",
      scan: scanResult("git", { state: { kind: "update_available", installed: "1", recommended: "2" } }),
      installable: true,
      applicability: "installable",
      sources_for_os: [],
      availability: ["windows"],
      checksum: "none",
      docker_alternative: { image: "x" },
    } as unknown as MarketplaceItem;

    const base = defaultCatalogFilters();
    expect(makeMarketplacePredicate(base)(item)).toBe(true);

    expect(makeMarketplacePredicate({ ...base, search: "zzz" })(item)).toBe(false);
    expect(makeMarketplacePredicate({ ...base, states: ["installed_healthy"] })(item)).toBe(false);
    expect(makeMarketplacePredicate({ ...base, states: ["update_available"] })(item)).toBe(true);
    expect(makeMarketplacePredicate({ ...base, health: ["healthy"] })(item)).toBe(false);
    expect(makeMarketplacePredicate({ ...base, provenance: ["external"] })(item)).toBe(false);
    expect(makeMarketplacePredicate({ ...base, admin_only: true })(item)).toBe(true);
    expect(makeMarketplacePredicate({ ...base, update_only: true })(item)).toBe(true);
    expect(makeMarketplacePredicate({ ...base, manual_only: true })(item)).toBe(false);
    expect(makeMarketplacePredicate({ ...base, installable: true })(item)).toBe(true);
    expect(makeMarketplacePredicate({ ...base, has_docker_alternative: true })(item)).toBe(true);
    expect(makeMarketplacePredicate({ ...base, categories: ["language"] })(item)).toBe(false);
  });

  it("рантайм-группы невидимы без скана (карточки не прячутся)", () => {
    const item = {
      def: def({ id: "git" }),
      os: "windows",
      scan: null,
      installable: true,
      applicability: "installable",
      sources_for_os: [],
      availability: ["windows"],
      checksum: "none",
      docker_alternative: null,
    } as unknown as MarketplaceItem;

    const base = defaultCatalogFilters();
    expect(makeMarketplacePredicate(base)(item)).toBe(true);
    expect(makeMarketplacePredicate({ ...base, states: ["installed_healthy"] })(item)).toBe(true);
    expect(makeMarketplacePredicate({ ...base, provenance: ["stack_pilot_managed"] })(item)).toBe(true);
    expect(makeMarketplacePredicate({ ...base, health: ["unhealthy"] })(item)).toBe(true);
    expect(makeMarketplacePredicate({ ...base, update_only: true })(item)).toBe(true);

    // Определение-факты продолжают фильтровать и без скана.
    expect(makeMarketplacePredicate({ ...base, categories: ["language"] })(item)).toBe(false);
    expect(makeMarketplacePredicate({ ...base, installable: true })(item)).toBe(true);
  });

  it("возможности и режим исполнения фильтруют и со скан-фактами", () => {
    const item = {
      def: def({ id: "git" }),
      os: "windows",
      scan: scanResult("git", { state: { kind: "installed_healthy", version: "1" } }),
      installable: true,
      applicability: "installable",
      sources_for_os: [],
      availability: ["windows"],
      checksum: "none",
      docker_alternative: null,
    } as unknown as MarketplaceItem;

    const base = defaultCatalogFilters();
    expect(makeMarketplacePredicate({ ...base, capabilities: ["removable"] })(item)).toBe(false);
    expect(makeMarketplacePredicate({ ...base, execution_modes: ["docker"] })(item)).toBe(false);
  });
});

// ------------------------------------------------------------
// Правдивое действие карточки (установленным — НЕ предлагать установку)
// ------------------------------------------------------------

describe("marketplaceAction — правдивое действие карточки", () => {
  function item(overrides: Partial<MarketplaceItem> = {}): MarketplaceItem {
    return {
      def: def({ id: "git" }),
      os: "windows",
      scan: null,
      installable: true,
      applicability: "installable",
      sources_for_os: [],
      availability: ["windows"],
      checksum: "none",
      docker_alternative: null,
      ...overrides,
    } as MarketplaceItem;
  }

  it("без скана предлагает локальную установку", () => {
    expect(marketplaceAction(item()).kind).toBe("install");
    expect(marketplaceAction(item()).label).toContain("Установить локально");
  });

  it("установленный инструмент НЕ предлагается ставить заново", () => {
    for (const kind of ["installed_healthy", "installed_health_unknown", "installed_unhealthy"]) {
      const act = marketplaceAction(
        item({
          scan: scanResult("git", {
            state: { kind, version: "2.45" } as ToolScanResult["state"],
          }),
        }),
      );
      expect(act.kind).toBe("installed");
      expect(act.label).toBe("Установлен");
    }
  });

  it("доступное обновление даёт отдельную кнопку «Обновить»", () => {
    const act = marketplaceAction(
      item({
        scan: scanResult("git", {
          state: { kind: "update_available", installed: "2.40", recommended: "2.45" },
        }),
      }),
    );
    expect(act.kind).toBe("update");
    expect(act.label).toBe("Обновить");
  });

  it("обновление без источника на этой ОС — НЕ кнопка (bundled-тулы вроде pip)", () => {
    const act = marketplaceAction(
      item({
        installable: false,
        applicability: "built_in",
        scan: scanResult("pip", {
          state: { kind: "update_available", installed: "24.0", recommended: "25" },
        }),
      }),
    );
    expect(act.kind).not.toBe("update");
    expect(act.kind).toBe("built_in");
  });

  it("не установлен + установляем → install; ручной → manual; нет источника → no_source", () => {
    expect(marketplaceAction(item({ scan: scanResult("git") })).kind).toBe("install");
    expect(
      marketplaceAction(item({ scan: null, applicability: "manual_only", installable: false })).kind,
    ).toBe("manual");
    expect(
      marketplaceAction(item({ scan: null, applicability: "no_source", installable: false })).kind,
    ).toBe("no_source");
  });
});