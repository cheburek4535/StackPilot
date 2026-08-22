// Тесты прозрачной оценки здоровья окружения (score breakdown).
// Никакой «обязательный набор»: формула считает только применимые
// инструменты; непроверенные/неприменимые/bundled не штрафуют.
import { describe, expect, it } from "vitest";
import {
  SCORE_FORMULA_TEXT,
  classifyScoreTool,
  scoreBreakdown,
} from "./score";
import type { EnvironmentSnapshot, ToolScanResult } from "./types";

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
    provenance: { kind: "unknown" },
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

describe("scoreBreakdown — здоровье окружения, а не «обязательный набор»", () => {
  it("null-снапшот — пустая разбивка", () => {
    const b = scoreBreakdown(null);
    expect(b.score).toBe(0);
    expect(b.applicable).toBe(0);
    expect(b.contributions).toEqual([]);
  });

  it("все здоровые — 100 из 100, все применимые", () => {
    const b = scoreBreakdown(
      snap([
        tool({ tool_id: "a", state: { kind: "installed_healthy", version: "1" } }),
        tool({ tool_id: "b", state: { kind: "installed_healthy", version: "2" } }),
      ]),
    );
    expect(b.score).toBe(100);
    expect(b.applicable).toBe(2);
    expect(b.healthy).toBe(2);
  });

  it("отсутствующий снижает пропорционально; никакого «required»-счёта", () => {
    const b = scoreBreakdown(
      snap([
        tool({ tool_id: "a", state: { kind: "installed_healthy", version: "1" } }),
        tool({ tool_id: "b", state: { kind: "missing" } }),
      ]),
    );
    expect(b.score).toBe(50);
    expect(b.applicable).toBe(2);
    expect(b.missing).toBe(1);
    // Поле «обязательных» в разбивке нет и быть не может.
    expect(Object.keys(b)).not.toContain("required");
    expect(SCORE_FORMULA_TEXT).not.toMatch(/обязательн/i);
  });

  it("обновление — деградация (50), не поломка", () => {
    const b = scoreBreakdown(
      snap([
        tool({ tool_id: "a", state: { kind: "installed_healthy", version: "2" } }),
        tool({
          tool_id: "b",
          state: { kind: "update_available", installed: "1", recommended: "2" },
        }),
      ]),
    );
    expect(b.score).toBe(75);
    expect(b.degraded).toBe(1);
  });

  it("сломанные и нездоровые — 0 очков, но в знаменателе", () => {
    const b = scoreBreakdown(
      snap([
        tool({ tool_id: "a", state: { kind: "path_broken", reason: "x" } }),
        tool({ tool_id: "b", state: { kind: "installed_unhealthy", version: "1" } }),
      ]),
    );
    expect(b.score).toBe(0);
    expect(b.unhealthy).toBe(2);
    expect(b.applicable).toBe(2);
  });

  it("непроверенные НЕ штрафуют и не входят в знаменатель", () => {
    const b = scoreBreakdown(
      snap([
        tool({ tool_id: "h", state: { kind: "installed_healthy", version: "1" } }),
        tool({ tool_id: "u", state: { kind: "installed_health_unknown", version: "1" } }),
        tool({ tool_id: "f", state: { kind: "scan_failed", reason: "timeout" } }),
        tool({ tool_id: "p", state: { kind: "scan_pending" } }),
      ]),
    );
    expect(b.score).toBe(100); // неизвестное ≠ плохо
    expect(b.applicable).toBe(1);
    expect(b.unchecked).toBe(3);
  });

  it("неприменимые платформы и ручные инструменты вне формулы", () => {
    const b = scoreBreakdown(
      snap([
        tool({
          tool_id: "x",
          applicability: { kind: "unsupported_on_platform" },
          state: { kind: "unsupported_platform" },
        }),
        tool({ tool_id: "m", state: { kind: "manual_install", reason: "вручную" } }),
        tool({ tool_id: "d", state: { kind: "docker_managed" } }),
        tool({ tool_id: "s", state: { kind: "built_in_system" } }),
      ]),
    );
    expect(b.score).toBe(0);
    expect(b.applicable).toBe(0);
    expect(b.not_applicable).toBe(4);
  });

  it("bundled-дети, обеспеченные родителем, не штрафуют (excluded)", () => {
    const b = scoreBreakdown(
      snap([
        tool({ tool_id: "node", state: { kind: "installed_healthy", version: "22" } }),
        tool({ tool_id: "npm", bundled_with: "node", state: { kind: "missing" } }),
      ]),
    );
    expect(b.score).toBe(100);
    expect(b.excluded_bundled).toBe(1);
    expect(b.applicable).toBe(1);
  });

  it("docker-рекомендация — метаданные: сама по себе ничего не исключает", () => {
    // Инструмент с docker-альтернативой, но НЕ установленный локально —
    // в standalone он хост-требование: честный 0 очков (а не «управляется
    // Docker» и не «неприменим»).
    const b = scoreBreakdown(
      snap([
        tool({
          tool_id: "pg",
          state: { kind: "missing" },
          capabilities: { ...tool().capabilities, docker_alternative_available: true },
        }),
      ]),
    );
    expect(b.score).toBe(0);
    expect(b.applicable).toBe(1);
    expect(b.missing).toBe(1);

    // А вот состояние docker_managed (реальный факт скана) — вне формулы.
    const docker = scoreBreakdown(snap([tool({ tool_id: "pg", state: { kind: "docker_managed" } })]));
    expect(docker.score).toBe(0);
    expect(docker.applicable).toBe(0);
    expect(docker.not_applicable).toBe(1);
  });

  it("вклад в поповере совпадает с классификацией по каждому инструменту", () => {
    const b = scoreBreakdown(
      snap([
        tool({ tool_id: "a", state: { kind: "installed_healthy", version: "1" } }),
        tool({ tool_id: "b", state: { kind: "missing" } }),
      ]),
    );
    const byId = Object.fromEntries(b.contributions.map((c) => [c.tool_id, c]));
    expect(byId.a.contribution).toBe(100);
    expect(byId.a.counted).toBe(true);
    expect(byId.b.contribution).toBe(0);
    expect(byId.b.counted).toBe(true);
  });

  it("classifyScoreTool тотален для всех состояний (ничего не падает)", () => {
    const cases = [
      { kind: "scan_pending" },
      { kind: "scan_failed", reason: "x" },
      { kind: "missing" },
      { kind: "installed_healthy", version: "1" },
      { kind: "installed_health_unknown", version: "1" },
      { kind: "installed_unhealthy", version: "1" },
      { kind: "update_available", installed: "1", recommended: "2" },
      { kind: "path_broken", reason: "x" },
      { kind: "manual_install", reason: "x" },
      { kind: "docker_managed" },
      { kind: "built_in_system" },
      { kind: "unsupported_platform" },
    ] as const;
    for (const state of cases) {
      const c = classifyScoreTool(tool({ state }));
      expect(typeof c.counted).toBe("boolean");
      expect([0, 50, 100]).toContain(c.contribution);
    }
  });

  // Регрессия «оценка растёт до финала скана»: «живая» заготовка идущего
  // скана (installed_healthy с пустой версией) не должна притворяться
  // здоровым инструментом (100) — фактов ещё нет.
  it("живые заготовки скана без версии не засчитываются как здоровые", () => {
    const placeholder = classifyScoreTool(
      tool({ state: { kind: "installed_healthy", version: "" } }),
    );
    expect(placeholder.counted).toBe(false);
    expect(placeholder.contribution).toBe(0);
    expect(placeholder.bucket).toBe("unchecked");

    const placeholderUpdate = classifyScoreTool(
      tool({ state: { kind: "update_available", installed: "", recommended: "" } }),
    );
    expect(placeholderUpdate.counted).toBe(false);

    // Настоящий факт (версия есть) считается как раньше.
    const real = classifyScoreTool(tool({ state: { kind: "installed_healthy", version: "2.45" } }));
    expect(real.counted).toBe(true);
    expect(real.contribution).toBe(100);
  });

  it("нечитаемый инструмент из IPC не роняет разбивку", () => {
    const broken = scoreBreakdown({
      ...snap([]),
      tools: [null as never, { tool_id: "x" } as never, tool({})],
    });
    expect(broken.score).toBeGreaterThanOrEqual(0);
    expect(broken.applicable).toBe(1);
  });
});