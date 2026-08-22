// Тесты предикатов фильтрации каталога.
import { describe, expect, it } from "vitest";
import {
  applyCatalogFilters,
  availableCategories,
  defaultCatalogFilters,
  makeCatalogPredicate,
  matchesCapabilities,
  matchesDefinitionSearch,
  matchesSearch,
  sortCatalogTools,
  validateCatalogFilters,
} from "./filters";
import type { CatalogFilters, EnvironmentSnapshot, ToolScanResult } from "./types";

function tool(overrides: Partial<ToolScanResult> = {}): ToolScanResult {
  return {
    tool_id: "git",
    display: "Git",
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
    provenance: { kind: "external" },
    bundled_with: null,
    version_assessment: { kind: "unknown" },
    state: { kind: "missing" },
    error: null,
    duration_ms: 1,
    ...overrides,
  };
}

function snap(tools: ToolScanResult[]): EnvironmentSnapshot {
  return {
    snapshot_id: "s",
    job_id: "j",
    scan_id: "sc",
    os: "windows",
    os_version: "11",
    arch: "",
    package_managers: [],
    disk: [],
    admin: { elevation_supported: true, required_by_tools: false },
    started_at: "a",
    finished_at: "b",
    complete: true,
    cancelled: false,
    tools,
    path_report: { entries: [], findings: [] },
    score: {
      score: 0,
      counted_tools: 0,
      healthy: 0,
      degraded: 0,
      broken: 0,
      missing: 0,
      unhealthy: 0,
      scan_failed: 0,
      unchecked: 0,
      optional: 0,
      not_applicable: 0,
    },
    summary: {
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
    },
    warnings: [],
    errors: [],
    active_jobs: [],
    age_seconds: 0,
    stale: false,
    from_cache: false,
  };
}

describe("предикаты каталога", () => {
  it("поиск нечувствителен к регистру и ищет по id/display/категории", () => {
    expect(matchesSearch(tool(), "GIT")).toBe(true);
    expect(matchesSearch(tool({ tool_id: "nodejs" }), "node")).toBe(true);
    expect(matchesSearch(tool({ category: "database" }), "data")).toBe(true);
    expect(matchesSearch(tool(), "python")).toBe(false);
    expect(matchesSearch(tool(), "  ")).toBe(true);
  });

  it("фильтр состояний работает по kind discriminated union", () => {
    const pred = makeCatalogPredicate({
      ...defaultCatalogFilters(),
      states: ["update_available", "installed_healthy"],
    });
    expect(pred(tool({ state: { kind: "update_available", installed: "1", recommended: "2" } }))).toBe(true);
    expect(pred(tool({ state: { kind: "installed_healthy", version: "9" } }))).toBe(true);
    expect(pred(tool({ state: { kind: "missing" } }))).toBe(false);
    // scan-failed никогда не подменяется missing и фильтруется отдельно.
    expect(pred(tool({ state: { kind: "scan_failed", reason: "t" } }))).toBe(false);
  });

  it("происхождение совпадает по kind (включая bundled_with)", () => {
    const pred = makeCatalogPredicate({
      ...defaultCatalogFilters(),
      provenance: ["bundled_with"],
    });
    expect(pred(tool({ provenance: { kind: "bundled_with", tool: "node" } }))).toBe(true);
    expect(pred(tool({ provenance: { kind: "stack_pilot_managed" } }))).toBe(false);
  });

  it("возможности требуют ВСЕ указанные флаги", () => {
    const caps = tool().capabilities;
    expect(matchesCapabilities(caps, [])).toBe(true);
    expect(matchesCapabilities(caps, ["installable"])).toBe(true);
    expect(matchesCapabilities(caps, ["installable", "removable"])).toBe(false);
  });

  it("режим исполнения выводится из docker_default/docker-происхождения", () => {
    const pred = makeCatalogPredicate({
      ...defaultCatalogFilters(),
      execution_modes: ["docker"],
    });
    expect(pred(tool({ applicability: { kind: "docker_default" } }))).toBe(true);
    expect(pred(tool({ provenance: { kind: "docker" } }))).toBe(true);
    expect(pred(tool())).toBe(false);
  });

  it("applyCatalogFilters сохраняет порядок каталога", () => {
    const snapshot = snap([
      tool({ tool_id: "a", display: "Alpha", category: "x" }),
      tool({ tool_id: "b", display: "Beta", category: "y", state: { kind: "installed_healthy", version: "1" } }),
      tool({ tool_id: "c", display: "Gamma", category: "x", state: { kind: "installed_healthy", version: "2" } }),
    ]);
    const filtered = applyCatalogFilters(snapshot, {
      ...defaultCatalogFilters(),
      states: ["installed_healthy"],
    });
    expect(filtered.map((t) => t.tool_id)).toEqual(["b", "c"]);
    expect(applyCatalogFilters(null, defaultCatalogFilters())).toEqual([]);
  });

  it("категории собираются в порядке появления; поиск по определению знает aliases", () => {
    const snapshot = snap([
      tool({ category: "vcs" }),
      tool({ category: "language" }),
      tool({ category: "vcs" }),
    ]);
    expect(availableCategories(snapshot)).toEqual(["vcs", "language"]);

    expect(matchesDefinitionSearch(
      { id: "node", display: "Node.js", category: "language", aliases: ["nodejs"] } as never,
      "nodejs",
    )).toBe(true);
  });

  it("фильтр здоровья совпадает по фактическому состоянию; null = not_checked", () => {
    const pred = makeCatalogPredicate({
      ...defaultCatalogFilters(),
      health: ["healthy"],
    });
    expect(pred(tool({ health: { state: { kind: "healthy" }, results: [] } }))).toBe(true);
    expect(pred(tool({ health: { state: { kind: "degraded" }, results: [] } }))).toBe(false);
    // Нет данных здоровья — это not_checked, а не healthy.
    expect(pred(tool())).toBe(false);
  });

  it("update_only и manual_only фильтруют по фактам бэкенда", () => {
    const updatePred = makeCatalogPredicate({
      ...defaultCatalogFilters(),
      update_only: true,
    });
    expect(updatePred(tool({ state: { kind: "update_available", installed: "1", recommended: "2" } }))).toBe(true);
    expect(updatePred(tool({ state: { kind: "installed_healthy", version: "9" } }))).toBe(false);

    const manualPred = makeCatalogPredicate({
      ...defaultCatalogFilters(),
      manual_only: true,
    });
    expect(manualPred(tool({ applicability: { kind: "manual_only" } }))).toBe(true);
    expect(manualPred(tool())).toBe(false);
  });

  it("admin_only требует ЯВНЫЙ факт каталога (нет метаданных — не угадываем)", () => {
    const pred = makeCatalogPredicate(
      { ...defaultCatalogFilters(), admin_only: true },
      {
        definitions: {
          git: { id: "git", needs_admin: true } as never,
        },
      },
    );
    expect(pred(tool({ tool_id: "git" }))).toBe(true);
    expect(pred(tool({ tool_id: "curl" }))).toBe(false);
  });

  it("сортировка не мутирует вход и умеет name/category/status", () => {
    const a = tool({ tool_id: "a", display: "Beta", category: "z", state: { kind: "missing" } });
    const b = tool({ tool_id: "b", display: "Alpha", category: "y", state: { kind: "path_broken", reason: "" } });
    const c = tool({ tool_id: "c", display: "Gamma", category: "x", state: { kind: "installed_healthy", version: "1" } });
    const input = [a, b, c];

    expect(sortCatalogTools(input, "name_asc").map((t) => t.display)).toEqual(["Alpha", "Beta", "Gamma"]);
    expect(sortCatalogTools(input, "name_desc").map((t) => t.display)).toEqual(["Gamma", "Beta", "Alpha"]);
    expect(sortCatalogTools(input, "category").map((t) => t.tool_id)).toEqual(["c", "b", "a"]);
    // status: сломанный → отсутствующий → здоровый
    expect(sortCatalogTools(input, "status").map((t) => t.tool_id)).toEqual(["b", "a", "c"]);
    // catalog — исходный порядок; вход не изменён
    expect(sortCatalogTools(input, "catalog").map((t) => t.tool_id)).toEqual(["a", "b", "c"]);
    expect(input.map((t) => t.tool_id)).toEqual(["a", "b", "c"]);
  });
});

// ------------------------------------------------------------
// Расширенный поиск по определению (витрина, до скана)
// ------------------------------------------------------------

describe("matchesDefinitionSearch — поля витрины", () => {
  const def = {
    id: "postgresql",
    display: "PostgreSQL",
    category: "database",
    description: "Реляционная СУБД с расширяемостью",
    notes: "Рекомендуется docker: postgres:17",
    docs_url: "https://www.postgresql.org/docs/",
    source_url: "https://github.com/postgres/postgres",
    aliases: ["psql"],
  } as never;

  it("ищет по display/id/категории/aliases", () => {
    expect(matchesDefinitionSearch(def, "postgres")).toBe(true);
    expect(matchesDefinitionSearch(def, "psql")).toBe(true);
    expect(matchesDefinitionSearch(def, "database")).toBe(true);
  });

  it("ищет по описанию, notes, docs_url и source_url", () => {
    expect(matchesDefinitionSearch(def, "расширяемость")).toBe(true);
    expect(matchesDefinitionSearch(def, "postgres:17")).toBe(true);
    expect(matchesDefinitionSearch(def, "postgresql.org")).toBe(true);
    expect(matchesDefinitionSearch(def, "github.com/postgres")).toBe(true);
    expect(matchesDefinitionSearch(def, "небывалое")).toBe(false);
  });

  it("пустой запрос — всё проходит; регистр не важен", () => {
    expect(matchesDefinitionSearch(def, "  ")).toBe(true);
    expect(matchesDefinitionSearch(def, "POSTGRESQL.ORG")).toBe(true);
  });
});

// ------------------------------------------------------------
// Валидация персистентных фильтров (граница локального хранилища)
// ------------------------------------------------------------

describe("validateCatalogFilters — мусор отбрасывается, известное сохраняется", () => {
  it("необъектный payload → дефолтные фильтры", () => {
    expect(validateCatalogFilters(null)).toEqual(defaultCatalogFilters());
    expect(validateCatalogFilters("garbage")).toEqual(defaultCatalogFilters());
    expect(validateCatalogFilters([1, 2])).toEqual(defaultCatalogFilters());
  });

  it("неизвестные kind'ы и дубликаты отбрасываются", () => {
    const f = validateCatalogFilters({
      states: ["missing", "PascalGarbage", "missing"],
      provenance: ["external", "nope"],
      capabilities: ["installable", "hack"],
      execution_modes: ["host", "podman"],
      health: ["healthy", "mystery"],
    });
    expect(f.states).toEqual(["missing"]);
    expect(f.provenance).toEqual(["external"]);
    expect(f.capabilities).toEqual(["installable"]);
    expect(f.execution_modes).toEqual(["host"]);
    expect(f.health).toEqual(["healthy"]);
  });

  it("неверные типы полей → значения по умолчанию", () => {
    const f = validateCatalogFilters({
      search: 42,
      categories: [7, "vcs"],
      admin_only: "yes",
      update_only: 1,
      manual_only: null,
      installable: true,
      has_docker_alternative: "no",
    });
    expect(f.search).toBe("");
    expect(f.categories).toEqual(["vcs"]);
    expect(f.admin_only).toBe(false);
    expect(f.update_only).toBe(false);
    expect(f.manual_only).toBe(false);
    expect(f.installable).toBe(true);
    expect(f.has_docker_alternative).toBe(false);
  });

  it("корректный payload проходит без изменений", () => {
    const input = {
      search: "git",
      categories: ["vcs", "database"],
      states: ["missing", "update_available"],
      provenance: ["external", "bundled_with"],
      capabilities: ["installable", "docker_alternative_available"],
      execution_modes: ["host"],
      health: ["healthy", "degraded"],
      admin_only: true,
      update_only: true,
      manual_only: false,
      installable: true,
      has_docker_alternative: true,
    };
    expect(validateCatalogFilters(input)).toEqual(input);
  });
});

// ------------------------------------------------------------
// Новые фильтры каталога: installable и has_docker_alternative
// ------------------------------------------------------------

describe("installable и has_docker_alternative в Manage Everything", () => {
  it("installable опирается на факт возможностей скана", () => {
    const pred = makeCatalogPredicate({ ...defaultCatalogFilters(), installable: true });
    expect(pred(tool())).toBe(true); // capabilities.installable=true в фабрике
    expect(
      pred(tool({ capabilities: { ...tool().capabilities, installable: false } })),
    ).toBe(false);
  });

  it("has_docker_alternative опирается на заявленную docker-возможность", () => {
    const pred = makeCatalogPredicate({ ...defaultCatalogFilters(), has_docker_alternative: true });
    expect(
      pred(
        tool({
          capabilities: { ...tool().capabilities, docker_alternative_available: true },
        }),
      ),
    ).toBe(true);
    expect(pred(tool())).toBe(false);
  });
});
