// Тесты канонических discriminated unions (контракт §8) и легаси-хелперов.
import { describe, expect, it } from "vitest";
import {
  jobEventBelongsTo,
  jobStatusIsTerminal,
  operationMutatesMachine,
  parseTaskAction,
  parseToolState,
  scanStartJobId,
  statusIsOk,
  statusKind,
  taskActionIsNoop,
  taskActionKind,
  taskStateKind,
  toolStateKind,
  type JobEvent,
  type ToolState,
} from "./types";

describe("ToolState — kind-tagged discriminated union", () => {
  it("различает варианты по kind, включая unit-варианты-объекты", () => {
    const missing: ToolState = { kind: "missing" };
    const healthy: ToolState = { kind: "installed_healthy", version: "22.12.0" };
    const update: ToolState = {
      kind: "update_available",
      installed: "18.19",
      recommended: "22",
    };

    expect(toolStateKind(missing)).toBe("missing");
    expect(toolStateKind(healthy)).toBe("installed_healthy");
    expect(toolStateKind(update)).toBe("update_available");

    // Полезная нагрузка читается только после сужения по kind —
    // никакой shape-sniffing логики.
    if (update.kind === "update_available") {
      expect(update.recommended).toBe("22");
    } else {
      expect.unreachable();
    }
  });

  it("parseToolState принимает все 12 заявленных kind", () => {
    const kinds = [
      { kind: "scan_pending" },
      { kind: "scan_failed", reason: "timeout" },
      { kind: "missing" },
      { kind: "installed_healthy", version: "1" },
      { kind: "installed_health_unknown", version: "1" },
      { kind: "installed_unhealthy", version: "1" },
      { kind: "update_available", installed: "1", recommended: "2" },
      { kind: "path_broken", reason: "not on PATH" },
      { kind: "manual_install", reason: "SDK" },
      { kind: "docker_managed" },
      { kind: "built_in_system" },
      { kind: "unsupported_platform" },
    ];
    for (const raw of kinds) {
      expect(parseToolState(raw)?.kind).toBe((raw as { kind: string }).kind);
    }
  });

  it("parseToolState возвращает null для неизвестного/битого payload (не падает)", () => {
    // Легаси-кодировка (внешний тег) НЕ является валидным каноном.
    expect(parseToolState({ Installed: { version: "1" } })).toBeNull();
    expect(parseToolState({ kind: "totally_unknown_kind" })).toBeNull();
    expect(parseToolState("installed_healthy")).toBeNull(); // голая строка запрещена
    expect(parseToolState(null)).toBeNull();
    expect(parseToolState(42)).toBeNull();
    expect(parseToolState({})).toBeNull();
  });
});

describe("Легаси ToolStatus — внешне тегированный формат (совместимость)", () => {
  it("statusKind/statusIsOk различают все шесть вариантов", () => {
    expect(statusKind("Missing")).toBe("missing");
    expect(statusKind("RunInDocker")).toBe("docker");
    expect(statusKind({ Installed: { version: "1.0" } })).toBe("ok");
    expect(statusKind({ UpdateAvailable: { installed: "1", recommended: "2" } })).toBe("update");
    expect(statusKind({ ManualInstall: { reason: "sdk" } })).toBe("manual");
    expect(statusKind({ PathBroken: { reason: "no bin" } })).toBe("broken");

    expect(statusIsOk({ Installed: { version: "1.0" } })).toBe(true);
    expect(statusIsOk({ UpdateAvailable: { installed: "1", recommended: "2" } })).toBe(false);
    expect(statusIsOk("Missing")).toBe(false);
    expect(statusIsOk(undefined)).toBe(false);
  });

  it("taskStateKind покрывает легаси-состояния задач", () => {
    expect(taskStateKind("Pending")).toBe("pending");
    expect(taskStateKind({ Running: { phase: "Installing" } })).toBe("running");
    expect(taskStateKind({ Success: { version: "1" } })).toBe("success");
    expect(taskStateKind({ Failed: { error: "x" } })).toBe("failed");
    expect(taskStateKind({ Skipped: { reason: "docker" } })).toBe("skipped");
  });
});

describe("Движковые задания — идентичность и терминальность", () => {
  const event = (jobId: string): JobEvent => ({
    job_id: jobId,
    task_id: "t1",
    tool_id: "git",
    seq: 1,
    timestamp: "now",
    payload: { progress: { line: "x" } },
  });

  it("jobEventBelongsTo отбрасывает чужие задания", () => {
    expect(jobEventBelongsTo(event("j1"), "j1")).toBe(true);
    expect(jobEventBelongsTo(event("j-old"), "j1")).toBe(false);
  });

  it("только health_check не мутирует машину", () => {
    expect(operationMutatesMachine("install")).toBe(true);
    expect(operationMutatesMachine("update")).toBe(true);
    expect(operationMutatesMachine("repair_path")).toBe(true);
    expect(operationMutatesMachine("health_check")).toBe(false);
  });

  it("терминальные статусы задания: succeeded/partial/failed/cancelled/interrupted", () => {
    for (const terminal of ["succeeded", "partial", "failed", "cancelled", "interrupted"] as const) {
      expect(jobStatusIsTerminal(terminal)).toBe(true);
    }
    expect(jobStatusIsTerminal("queued")).toBe(false);
    expect(jobStatusIsTerminal("running")).toBe(false);
  });

  it("scanStartJobId извлекает задание из обоих вариантов reconnect-исхода", () => {
    const snapshot = {
      job_id: "j",
      scan_id: "s",
      started_at: "a",
      updated_at: "b",
      finished_at: null,
      phase: "Tools" as const,
      total_tools: 5,
      completed_tools: 2,
      current_tool: null,
      running: true,
      cancel_requested: false,
      terminal: "Running" as const,
      recovered: false,
    };
    expect(scanStartJobId({ Started: snapshot })?.job_id).toBe("j");
    expect(scanStartJobId({ AlreadyRunning: snapshot })?.scan_id).toBe("s");
    // Малиформация → null вместо краша.
    expect(scanStartJobId(undefined)).toBeNull();
    expect(scanStartJobId({ Started: {} })).toBeNull();
  });

  describe("движковые действия задач — канонический noop и наследие no_op", () => {
    it("канонический ключ noop разбирается во всех формах", () => {
      expect(taskActionKind({ noop: { already_installed: { version: "2.48" } } })).toBe("noop");
      expect(taskActionKind({ noop: "docker_managed" })).toBe("noop");
      expect(parseTaskAction({ noop: { already_installed: { version: "2.48" } } })).toEqual({
        noop: { already_installed: { version: "2.48" } },
      });
      expect(parseTaskAction({ noop: "docker_managed" })).toEqual({ noop: "docker_managed" });
      expect(taskActionIsNoop({ noop: { update_unavailable: { version: "1" } } })).toBe(true);
    });

    it("наследие no_op (старые персистентные записи) читается как noop", () => {
      expect(taskActionKind({ no_op: { already_installed: { version: "2.48" } } })).toBe("no_op");
      expect(parseTaskAction({ no_op: { already_installed: { version: "2.48" } } })).toEqual({
        noop: { already_installed: { version: "2.48" } },
      });
      expect(parseTaskAction({ no_op: "docker_managed" })).toEqual({ noop: "docker_managed" });
      expect(taskActionIsNoop({ no_op: { update_unavailable: { version: "1" } } })).toBe(true);
      expect(taskActionIsNoop({ no_op: "docker_managed" })).toBe(true);
    });
  });
});
