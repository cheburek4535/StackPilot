// Тесты чистой логики состояния: инкрементальный скан, merge с кэшем,
// конкурентность мутаций, stale-события заданий, история.
import { describe, expect, it } from "vitest";
import {
  applyJobEvent,
  applyScanProgress,
  gateMutation,
  identityMatches,
  isJobSucceeded,
  jobEventAppliesTo,
  jobEventLogText,
  jobIsActive,
  mergeLiveSnapshot,
  recomputeSummary,
  scanDoneAppliesTo,
  scanProgressAppliesTo,
  snapshotFreshness,
  upsertJobHistory,
  type LiveToolMap,
} from "./stateLogic";
import type {
  EnvironmentSnapshot,
  JobEvent,
  PersistedJob,
  ScanDoneEvent,
  ScanJobSnapshot,
  ScanProgressEvent,
  ToolScanResult,
} from "./types";

// ------------------------------------------------------------
// Фабрики
// ------------------------------------------------------------

const progressEvent = (overrides: Partial<ScanProgressEvent> = {}): ScanProgressEvent => ({
  job_id: "j1",
  scan_id: "s1",
  completed_count: 1,
  total_count: 2,
  tool_id: "git",
  display_name: "Git",
  icon: null,
  tool_state: "installed_healthy",
  timestamp: "t",
  error: null,
  ...overrides,
});

function toolResult(toolId: string, state: ToolScanResult["state"]): ToolScanResult {
  return {
    tool_id: toolId,
    display: toolId,
    category: "utility",
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
    state,
    error: null,
    duration_ms: 1,
  };
}

function snapshot(tools: ToolScanResult[]): EnvironmentSnapshot {
  return {
    snapshot_id: "snap-1",
    job_id: "j0",
    scan_id: "s0",
    os: "windows",
    os_version: "11",
    arch: "x86_64",
    package_managers: ["winget"],
    disk: [],
    admin: { elevation_supported: true, required_by_tools: false },
    started_at: "a",
    finished_at: "b",
    complete: true,
    cancelled: false,
    tools,
    path_report: { entries: [], findings: [] },
    score: {
      score: 50,
      counted_tools: tools.length,
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
    summary: recomputeSummary(tools),
    warnings: [],
    errors: [],
    active_jobs: [],
    age_seconds: 10,
    stale: false,
    from_cache: false,
  };
}

function job(overrides: Partial<PersistedJob> = {}): PersistedJob {
  return {
    job_id: "job-1",
    plan_id: "plan-1",
    operation: "install",
    requested_tool_ids: ["git"],
    source_choices: {},
    created_at: "c",
    started_at: "s",
    updated_at: "u",
    finished_at: null,
    status: "running",
    plan: {
      plan_id: "plan-1",
      operation: "install",
      os: "windows",
      created_at: "c",
      fingerprint: "f",
      tasks: [
        {
          task_id: "task-1",
          tool_id: "git",
          display: "Git",
          icon: null,
          action: { install_new: { target_version: null } },
          source: null,
          size_mb: 10,
          needs_admin: false,
          depends_on: [],
          path_entries: [],
          install_options: [],
          execution_mode: "host",
          status: "pending",
        },
      ],
      total_size_mb: 10,
      free_space_mb: 100,
      enough_space: true,
      needs_admin_any: false,
      capabilities: { install_execution_supported: true, elevation_supported: true },
      warnings: [],
    },
    errors: [],
    path_changes: [],
    recovered: false,
    ...overrides,
  };
}

const jobEvent = (seq: number, payload: JobEvent["payload"], jobId = "job-1"): JobEvent => ({
  job_id: jobId,
  task_id: "task-1",
  tool_id: "git",
  seq,
  timestamp: "t",
  payload,
});

// ------------------------------------------------------------
// Инкрементальный прогресс скана
// ------------------------------------------------------------

describe("applyScanProgress — инкрементальные обновления", () => {
  it("добавляет инструмент, не трогая уже полученные", () => {
    let live: LiveToolMap = {};
    live = applyScanProgress(live, progressEvent({ tool_id: "git" }));
    expect(live.git.state.kind).toBe("installed_healthy");

    live = applyScanProgress(live, progressEvent({ tool_id: "node", tool_state: "missing" }));
    expect(live.git.state.kind).toBe("installed_healthy"); // прежние данные живы
    expect(live.node.state.kind).toBe("missing");
  });

  it("событие с неизвестным kind игнорируется (таблица не ломается)", () => {
    const live = applyScanProgress({}, progressEvent({ tool_state: "PascalGarbage" }));
    expect(live.git).toBeUndefined();
  });

  it("ошибка опроса не превращается в missing (scan_failed честен)", () => {
    const live = applyScanProgress(
      {},
      progressEvent({ tool_state: "scan_failed", error: "timeout" }),
    );
    expect(live.git?.state).toEqual({ kind: "scan_failed", reason: "timeout" });
    expect(live.git?.error).toBe("timeout");
  });

  it("повтор того же варианта сохраняет payload прежнего состояния", () => {
    let live: LiveToolMap = {
      git: toolResult("git", { kind: "installed_healthy", version: "2.4" }),
    };
    live = applyScanProgress(live, progressEvent({ tool_id: "git", tool_state: "installed_healthy" }));
    expect(live.git?.state).toEqual({ kind: "installed_healthy", version: "2.4" });
  });
});

// ------------------------------------------------------------
// Merge кэша и живого среза
// ------------------------------------------------------------

describe("mergeLiveSnapshot — кэш рендерится сразу, скан ничего не стирает", () => {
  const cached = snapshot([
    toolResult("git", { kind: "installed_healthy", version: "2.4" }),
    toolResult("node", { kind: "missing" }),
  ]);

  it("без живых данных отдаётся кэш как есть (мгновенный рендер)", () => {
    const merged = mergeLiveSnapshot(cached, {});
    expect(merged).toBe(cached);
  });

  it("живые данные накладываются поверх кэша по одному инструменту", () => {
    const merged = mergeLiveSnapshot(cached, {
      node: toolResult("node", { kind: "update_available", installed: "18", recommended: "22" }),
    })!;
    expect(merged.tools.find((t) => t.tool_id === "node")?.state.kind).toBe("update_available");
    expect(merged.tools.find((t) => t.tool_id === "git")?.state.kind).toBe("installed_healthy");
    // Сводка НЕ пересчитана: покрытие неполное (смесь эпох была бы лживой).
    expect(merged.summary).toBe(cached.summary);
  });

  it("при полном покрытии сводка пересчитывается", () => {
    const merged = mergeLiveSnapshot(cached, {
      git: toolResult("git", { kind: "installed_healthy", version: "2.4" }),
      node: toolResult("node", { kind: "missing" }),
    })!;
    expect(merged.summary.missing).toBe(1);
    expect(merged.summary.installed_healthy).toBe(1);
  });

  it("null-кэш остаётся null до первого снапшота", () => {
    expect(mergeLiveSnapshot(null, { git: toolResult("git", { kind: "missing" }) })).toBeNull();
  });
});

// ------------------------------------------------------------
// Свежесть
// ------------------------------------------------------------

describe("snapshotFreshness", () => {
  it("none → live → stale по возрасту и флагу", () => {
    expect(snapshotFreshness(null)).toBe("none");
    expect(snapshotFreshness(snapshot([]))).toBe("live");
    const old = { ...snapshot([]), age_seconds: 3600, stale: true };
    expect(snapshotFreshness(old)).toBe("stale");
  });
});

// ------------------------------------------------------------
// Конкурентность мутаций
// ------------------------------------------------------------

describe("gateMutation — конфликтующие мутации блокируются", () => {
  it("активная install блокирует install/update/repair_path", () => {
    const active = job();
    expect(jobIsActive(active)).toBe(true);
    for (const op of ["install", "update", "repair_path"] as const) {
      const gate = gateMutation(active, op);
      expect(gate.allowed).toBe(false);
    }
  });

  it("read-only health_check разрешён всегда", () => {
    expect(gateMutation(job(), "health_check").allowed).toBe(true);
  });

  it("после терминального статуса мутация снова разрешена", () => {
    const done = job({ status: "succeeded", finished_at: "f" });
    expect(gateMutation(done, "install").allowed).toBe(true);
  });

  it("нет активного задания — разрешено", () => {
    expect(gateMutation(null, "install")).toEqual({ allowed: true });
  });
});

// ------------------------------------------------------------
// События задания: stale-guard и честный успех
// ------------------------------------------------------------

describe("applyJobEvent — identity, seq, терминальность", () => {
  it("событие чужого задания отбрасывается", () => {
    const base = job();
    const result = applyJobEvent(base, jobEvent(1, { progress: { line: "x" } }, "other-job"), 0);
    expect(result.applied).toBe(false);
    expect(result.job).toBe(base);
  });

  it("отставшие seq (дубликаты) не применяются", () => {
    const base = job();
    const first = applyJobEvent(base, jobEvent(5, { task_phase: { phase: "downloading" } }), 0);
    expect(first.applied).toBe(true);
    const dup = applyJobEvent(first.job, jobEvent(5, { task_phase: { phase: "installing" } }), first.lastSeq);
    expect(dup.applied).toBe(false);
    // Задача осталась в downloading, а не перезаписалась более старым событием.
    expect(first.job.plan.tasks[0].status).toEqual({ running: { phase: "downloading" } });
  });

  it("task_completed обновляет статус задачи; job_finished — статус задания", () => {
    const base = job();
    const step1 = applyJobEvent(base, jobEvent(1, { task_started: { index: 0, total: 1 } }), 0);
    const step2 = applyJobEvent(step1.job, jobEvent(2, { task_completed: { status: { succeeded: { version: "2.4" } } } }), step1.lastSeq);
    expect(step2.job.plan.tasks[0].status).toEqual({ succeeded: { version: "2.4" } });

    const step3 = applyJobEvent(step2.job, jobEvent(3, { job_finished: { status: "succeeded", errors: [] } }), step2.lastSeq);
    expect(step3.job.status).toBe("succeeded");
  });

  it("успех признаётся только от бэкенда (isJobSucceeded)", () => {
    expect(isJobSucceeded("succeeded")).toBe(true);
    expect(isJobSucceeded("partial")).toBe(true);
    expect(isJobSucceeded("running")).toBe(false);
    expect(isJobSucceeded("queued")).toBe(false);
  });

  // Регрессия «already_installed in undefined» (семейство `in`-на-undefined):
  // малиформированные ВЛОЖЕННЫЕ payload события не должны ронять
  // обработчик. Раньше task_phase.phase / task_completed.status /
  // job_finished.status читались без guard'ов.
  it("малиформированные вложенные payload не роняют applyJobEvent", () => {
    const base = job();

    const nullPhase = applyJobEvent(base, jobEvent(1, { task_phase: null } as never), 0);
    expect(nullPhase.applied).toBe(true);
    expect(nullPhase.job.plan.tasks[0].status).toEqual("pending");

    const emptyCompleted = applyJobEvent(
      base,
      jobEvent(2, { task_completed: { status: null } } as never),
      0,
    );
    expect(emptyCompleted.applied).toBe(true);

    const badFinished = applyJobEvent(
      base,
      jobEvent(3, { job_finished: null } as never),
      0,
    );
    expect(badFinished.applied).toBe(true);
    expect(badFinished.job.status).toBe("running");

    const badFinishedStatus = applyJobEvent(
      base,
      jobEvent(4, { job_finished: { status: 42, errors: "boom" } } as never),
      0,
    );
    expect(badFinishedStatus.job.status).toBe("running");
  });
});

describe("jobEventLogText — журнальная строка переживает мусор из IPC", () => {
  const ev = (payload: unknown, overrides: Partial<JobEvent> = {}): JobEvent =>
    ({ job_id: "j1", task_id: "t1", tool_id: "git", seq: 1, timestamp: "t", payload, ...overrides }) as JobEvent;

  it("валидные payload дают читаемые строки", () => {
    expect(jobEventLogText(ev({ progress: { line: "winget: ok" } }))).toBe("winget: ok");
    expect(jobEventLogText(ev({ task_started: { index: 0, total: 1 } }))).toContain("старт");
    expect(jobEventLogText(ev({ task_phase: { phase: "installing" } }))).toContain("installing");
    expect(jobEventLogText(ev({ task_completed: { status: "succeeded" } }))).toContain("succeeded");
    expect(
      jobEventLogText(ev({ task_completed: { status: { succeeded: { version: "2.4" } } } })),
    ).toContain("succeeded");
    expect(jobEventLogText(ev({ path_updated: { record: { tool_id: "git", added: ["C:\\bin"] } } }))).toContain("1 записей");
    expect(jobEventLogText(ev({ job_started: { operation: "install", total_tasks: 3 } }))).toContain("3");
  });

  it("малиформированные вложенные payload → null (не TypeError)", () => {
    expect(jobEventLogText(ev({ progress: null }))).toBeNull();
    expect(jobEventLogText(ev({ progress: {} }))).toBeNull();
    expect(jobEventLogText(ev({ task_phase: null }))).toBeNull();
    expect(jobEventLogText(ev({ task_phase: { phase: 42 } }))).toBeNull();
    expect(jobEventLogText(ev({ task_completed: null }))).toBeNull();
    expect(jobEventLogText(ev({ task_completed: { status: null } }))).toBeNull();
    expect(jobEventLogText(ev({ task_completed: { status: {} } }))).toBeNull();
    expect(jobEventLogText(ev({ path_updated: null }))).toBeNull();
    expect(jobEventLogText(ev({ path_updated: { record: null } }))).toBeNull();
    expect(jobEventLogText(ev({ path_updated: { record: { added: null } } }))).toBeNull();
    expect(jobEventLogText(ev({ job_started: null }))).toBeNull();
    expect(jobEventLogText(ev({ job_started: { total_tasks: "3" } }))).toBeNull();
    expect(jobEventLogText(ev(null))).toBeNull();
  });
});

// ------------------------------------------------------------
// История заданий
// ------------------------------------------------------------

describe("upsertJobHistory — новые сверху, без дублей, ограничена", () => {
  it("обновление существующей записи не плодит дубликат", () => {
    const a = job({ job_id: "j1" });
    const b = job({ job_id: "j2" });
    const history = upsertJobHistory([a, b], job({ job_id: "j1", status: "succeeded" }));
    expect(history.map((j) => j.job_id)).toEqual(["j1", "j2"]);
    expect(history[0].status).toBe("succeeded");
  });

  it("история ограничена лимитом", () => {
    let history: PersistedJob[] = [];
    for (let i = 0; i < 25; i += 1) {
      history = upsertJobHistory(history, job({ job_id: `j${i}` }), 20);
    }
    expect(history.length).toBe(20);
    expect(history[0].job_id).toBe("j24");
  });
});

// ------------------------------------------------------------
// Guard'ы событий на границе слушателей (stale-скан/stale-задание)
// ------------------------------------------------------------

function scanJob(overrides: Partial<ScanJobSnapshot> = {}): ScanJobSnapshot {
  return {
    job_id: "scan-1",
    scan_id: "s-1",
    started_at: "a",
    updated_at: "b",
    finished_at: null,
    phase: "Tools",
    total_tools: 3,
    completed_tools: 1,
    current_tool: "git",
    running: true,
    cancel_requested: false,
    terminal: "Running",
    recovered: false,
    ...overrides,
  };
}

describe("scanProgressAppliesTo — stale-прогресс не трогает состояние", () => {
  it("событие без текущего скана не применяется", () => {
    expect(scanProgressAppliesTo(null, progressEvent())).toBe(false);
  });

  it("прогресс ЧУЖОГО скана (старый job_id) отбрасывается", () => {
    const current = scanJob();
    expect(scanProgressAppliesTo(current, progressEvent({ job_id: "scan-old" }))).toBe(false);
  });

  it("прогресс текущего скана применяется", () => {
    expect(scanProgressAppliesTo(scanJob(), progressEvent({ job_id: "scan-1" }))).toBe(true);
  });

  it("завершённый скан поздние события не оживляет (старый прогресс после done)", () => {
    const done = scanJob({ terminal: "Completed", running: false });
    expect(scanProgressAppliesTo(done, progressEvent({ job_id: "scan-1" }))).toBe(false);
  });
});

describe("scanDoneAppliesTo — терминал только своего скана", () => {
  it("терминал чужого скана отбрасывается (новый скан не перезаписывается)", () => {
    expect(scanDoneAppliesTo(scanJob(), { job_id: "scan-0", scan_id: "s-0", terminal: "Completed", completed: 3, total: 3 })).toBe(false);
  });

  it("терминал текущего скана применяется", () => {
    const done: ScanDoneEvent = { job_id: "scan-1", scan_id: "s-1", terminal: "Completed", completed: 3, total: 3 };
    expect(scanDoneAppliesTo(scanJob(), done)).toBe(true);
  });
});

describe("jobEventAppliesTo — события чужих/старых заданий", () => {
  it("событие без текущего задания отбрасывается", () => {
    expect(jobEventAppliesTo(null, jobEvent(1, { progress: { line: "x" } }))).toBe(false);
  });

  it("событие прежнего задания (другой job_id) отбрасывается", () => {
    expect(jobEventAppliesTo(job(), jobEvent(1, { progress: { line: "x" } }, "job-old"))).toBe(false);
  });

  it("событие текущего задания применяется", () => {
    expect(jobEventAppliesTo(job(), jobEvent(1, { progress: { line: "x" } }))).toBe(true);
  });
});

// ------------------------------------------------------------
// Совместимость Project Creator: stale-события
// ------------------------------------------------------------

describe("identityMatches — stale-события не портят состояние мастера", () => {
  it("чужой session_id (хвост прежней установки) отбрасывается", () => {
    expect(identityMatches("tcxj-new", "tcxj-old")).toBe(false);
  });

  it("событие текущего запуска принимается", () => {
    expect(identityMatches("tcxj-a", "tcxj-a")).toBe(true);
  });

  it("событие без идентичности (старый бэкенд) принимается", () => {
    expect(identityMatches("tcxj-a", "")).toBe(true);
    expect(identityMatches("tcxj-a", null)).toBe(true);
    expect(identityMatches("tcxj-a", undefined)).toBe(true);
  });

  it("идентичность запуска ещё не установлена — принимается (первое событие её установит)", () => {
    expect(identityMatches(null, "tcxj-first")).toBe(true);
    expect(identityMatches("", "scan-1")).toBe(true);
    expect(identityMatches(undefined, "scan-1")).toBe(true);
  });

  it("правило одинаково защищает прогресс проверки окружения (scan_id)", () => {
    // Первое событие устанавливает scan_id...
    expect(identityMatches(null, "run-1")).toBe(true);
    // ...поздний хвост прежнего запуска отбрасывается...
    expect(identityMatches("run-1", "run-0")).toBe(false);
    // ...а события текущего запуска проходят.
    expect(identityMatches("run-1", "run-1")).toBe(true);
  });
});

// ------------------------------------------------------------
// Сводка и живой срез — регрессии «мусор из IPC не даёт NaN/краш»
// ------------------------------------------------------------

describe("recomputeSummary — зеркало бэкенда без NaN", () => {
  it("неизвестный kind состояния не даёт NaN (и не считается)", () => {
    const tools: ToolScanResult[] = [
      toolResult("a", { kind: "installed_healthy", version: "1" }),
      { ...toolResult("b", { kind: "missing" }), state: { kind: "future_kind" } as never },
      { ...toolResult("c", { kind: "missing" }), state: null as never },
    ];
    const summary = recomputeSummary(tools);
    expect(summary.installed_healthy).toBe(1);
    for (const value of Object.values(summary)) {
      expect(Number.isNaN(value)).toBe(false);
    }
  });

  it("пустой срез — все счётчики ноль", () => {
    const summary = recomputeSummary([]);
    expect(Object.values(summary).every((v) => v === 0)).toBe(true);
  });
});

describe("applyScanProgress — живая заготовка честна (не выдумывает применимость)", () => {
  it("placeholder несёт applicability unknown, а не installable", () => {
    const next = applyScanProgress({}, progressEvent({ tool_state: "installed_healthy" }));
    const live = next.git;
    expect(live).toBeDefined();
    expect(live.applicability.kind).toBe("unknown");
    // Заготовка не считается «здоровой» в формуле — версия пустая.
    expect(live.state).toEqual({ kind: "installed_healthy", version: "" });
  });
});
