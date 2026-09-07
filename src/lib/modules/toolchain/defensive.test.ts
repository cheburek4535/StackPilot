// ============================================================
// Защитные границы IPC: малиформированные payload'ы не роняют UI.
// Регрессия краша «Cannot use 'in' operator to search for
// 'already_installed' in undefined» и всей семьи `in`-на-undefined.
// ============================================================
import { describe, expect, it } from "vitest";
import {
  engineTaskStatusKind,
  isRecord,
  jobEventKind,
  parseToolState,
  scanStartJobId,
  statusIsOk,
  statusLabel,
  statusKind,
  taskActionKind,
  taskStateKind,
  taskStateLabel,
  type PlanTask,
} from "./types";
import {
  noopReasonLabel,
  planTaskIsActionable,
  planTaskIsNoop,
  planWarningInfo,
  taskActionLabel,
} from "./format";

/** Задача плана с частичной подменой полей (малиформация). */
function taskWith(patch: Record<string, unknown>): unknown {
  return {
    task_id: "t1",
    tool_id: "git",
    display: "Git",
    icon: null,
    action: { install_new: { target_version: "2.48" } },
    source: null,
    size_mb: 1,
    needs_admin: false,
    depends_on: [],
    path_entries: [],
    install_options: [],
    execution_mode: "host",
    status: "pending",
    ...patch,
  };
}

describe("taskActionLabel — малиформированные действия не крашат", () => {
  it("undefined action → «Неизвестное действие» (бывший краш in undefined)", () => {
    expect(taskActionLabel(taskWith({ action: undefined }) as PlanTask)).toBe(
      "Неизвестное действие",
    );
  });

  it("null action → «Неизвестное действие»", () => {
    expect(taskActionLabel(taskWith({ action: null }) as PlanTask)).toBe(
      "Неизвестное действие",
    );
  });

  it("noop БЕЗ причины → честное «без действий, причина не указана»", () => {
    const label = taskActionLabel(taskWith({ action: {} }) as PlanTask);
    // action: {} — не noop-форма; отдельный случай ниже:
    expect(label).toBe("Неизвестное действие");

    const noReason = taskActionLabel(
      taskWith({ action: { noop: undefined } }) as PlanTask,
    );
    expect(noReason).toContain("Без действий");
    expect(noReason).toContain("причина");
  });

  it("неизвестная noop-причина НЕ угадывается", () => {
    const label = taskActionLabel(
      taskWith({ action: { noop: { future_reason: {} } } }) as PlanTask,
    );
    expect(label).toContain("Без действий");
    expect(label).toContain("неизвестн");
  });

  it("легаси/строковые формы действий читаются", () => {
    expect(taskActionLabel(taskWith({ action: "repair_path" }) as PlanTask)).toBe(
      "Ремонт PATH",
    );
    expect(taskActionLabel(taskWith({ action: "health_check" }) as PlanTask)).toBe(
      "Проверка здоровья",
    );
  });

  it("валидные noop-причины подписываются с версией", () => {
    expect(
      taskActionLabel(
        taskWith({
          action: { noop: { already_installed: { version: "2.45" } } },
        }) as PlanTask,
      ),
    ).toBe("Уже установлен (2.45)");
    expect(
      taskActionLabel(
        taskWith({
          action: { noop: { update_unavailable: { version: "2.45" } } },
        }) as PlanTask,
      ),
    ).toBe("Обновление недоступно (2.45)");
    expect(taskActionLabel(taskWith({ action: { noop: "docker_managed" } }) as PlanTask)).toContain(
      "Docker",
    );
  });

  it("наследие no_op (старые записи) подписывается как noop и считается noop-задачей", () => {
    expect(
      taskActionLabel(
        taskWith({
          action: { no_op: { already_installed: { version: "2.45" } } },
        }) as PlanTask,
      ),
    ).toBe("Уже установлен (2.45)");
    expect(
      taskActionLabel(
        taskWith({
          action: { no_op: { update_unavailable: { version: "2.45" } } },
        }) as PlanTask,
      ),
    ).toBe("Обновление недоступно (2.45)");
    expect(
      planTaskIsNoop(taskWith({ action: { no_op: "docker_managed" } })),
    ).toBe(true);
    expect(
      planTaskIsActionable(taskWith({ action: { no_op: { already_installed: { version: "1" } } } })),
    ).toBe(false);
    expect(taskActionKind({ no_op: { already_installed: { version: "1" } } })).toBe("no_op");
  });

  it("полностью мусорная задача → «Неизвестное действие», не исключение", () => {
    expect(taskActionLabel(null)).toBe("Неизвестное действие");
    expect(taskActionLabel(undefined)).toBe("Неизвестное действие");
    expect(taskActionLabel("garbage" as unknown as PlanTask)).toBe(
      "Неизвестное действие",
    );
    expect(taskActionLabel({} as unknown as PlanTask)).toBe("Неизвестное действие");
    expect(
      taskActionLabel(taskWith({ action: { a: 1, b: 2 } }) as PlanTask),
    ).toBe("Неизвестное действие");
  });
});

describe("noopReasonLabel — отсутствие/незнание причины честны", () => {
  it("noop без reason и с мусорным reason не крашатся и честны", () => {
    expect(noopReasonLabel(undefined)).toContain("причина неизвестна");
    expect(noopReasonLabel(null)).toContain("причина неизвестна");
    expect(noopReasonLabel(42)).toContain("причина неизвестна");
    expect(noopReasonLabel({})).toContain("причина неизвестна");
    expect(noopReasonLabel({ who_knows: 1 })).toContain("причина неизвестна");
    expect(noopReasonLabel("weird_string")).toContain("неизвестна");
  });
});

describe("planTaskIsActionable / planTaskIsNoop — guard'ы вместо `in`", () => {
  it("малиформация не считается ни actionable, ни noop", () => {
    for (const broken of [
      null,
      undefined,
      {},
      taskWith({ action: undefined }),
      taskWith({ action: null }),
      taskWith({ action: { a: 1, b: 2 } }),
    ]) {
      expect(planTaskIsActionable(broken)).toBe(false);
      expect(planTaskIsNoop(broken)).toBe(false);
    }
  });

  it("noop любой валидной формы — noop, но НЕ actionable", () => {
    expect(planTaskIsNoop(taskWith({ action: { noop: "docker_managed" } }))).toBe(true);
    expect(
      planTaskIsActionable(taskWith({ action: { noop: { already_installed: { version: "1" } } } })),
    ).toBe(false);
  });

  it("установочные действия actionable", () => {
    expect(planTaskIsActionable(taskWith({ action: { install_new: { target_version: null } } }))).toBe(true);
    expect(planTaskIsActionable(taskWith({ action: { update: { current_version: "1", target_version: "2" } } }))).toBe(true);
    expect(planTaskIsActionable(taskWith({ action: "repair_path" }))).toBe(true);
  });
});

describe("planWarningInfo — тотальная функция над предупреждениями плана", () => {
  it("все известные варианты разбираются", () => {
    const unverified = planWarningInfo({
      unverified_source: { tool_id: "git", source_id: "winget" },
    });
    expect(unverified.tone).toBe("amber");
    expect(unverified.text).toContain("git");

    const admin = planWarningInfo({ admin_required: { tool_id: "docker" } });
    expect(admin.text).toContain("docker");

    const reinstall = planWarningInfo({ reinstall_on_broken: { tool_id: "node" } });
    expect(reinstall.tone).toBe("red");
    expect(reinstall.text).toContain("node");
  });

  it("малиформация даёт «неизвестное предупреждение», а не краш на .tool_id", () => {
    for (const broken of [null, undefined, 42, {}, [], { who: 1 }, { unverified_source: null }]) {
      const info = planWarningInfo(broken);
      expect(info.text).toBeTruthy();
      expect(["amber", "red"]).toContain(info.tone);
    }
    // Отсутствующие вложенные поля подставляют «?» вместо краша.
    expect(planWarningInfo({ admin_required: {} }).text).toContain("?");
  });
});

describe("легаси-форматтеры состояний задач переживают мусор", () => {
  it("taskStateKind/taskStateLabel на undefined/null/объектах без ключей", () => {
    for (const garbage of [undefined, null, 42, {}, { Running: {} }, { Success: {} }, { Failed: {} }, { Skipped: {} }, { Future: {} }]) {
      expect(() => taskStateKind(garbage)).not.toThrow();
      expect(() => taskStateLabel(garbage)).not.toThrow();
      expect(taskStateLabel(garbage)).toBeTruthy();
    }
    expect(taskStateKind(undefined)).toBe("pending");
    expect(taskStateLabel({ Failed: {} })).toContain("Ошибка");
    expect(taskStateLabel({ Skipped: {} })).toContain("Пропущено");
    expect(taskStateLabel({ Running: { phase: "FuturePhase" } })).toBe("Выполняется…");
  });

  it("statusKind/statusLabel/statusIsOk переживают мусор (Project Creator)", () => {
    for (const garbage of [undefined, null, 42, {}, { Installed: {} }, { PathBroken: {} }]) {
      expect(() => statusKind(garbage)).not.toThrow();
      expect(() => statusLabel(garbage)).not.toThrow();
      expect(() => statusIsOk(garbage)).not.toThrow();
    }
    expect(statusLabel({ Installed: {} })).toBe("✓ Установлен");
    expect(statusLabel(null)).toBe("Неизвестное состояние");
    expect(statusKind({ Future: { x: 1 } })).toBe("missing");
  });
});

describe("движковые guard'ы типов", () => {
  it("parseToolState отбрасывает малиформацию (уже покрыто), isRecord строг", () => {
    expect(parseToolState(undefined)).toBeNull();
    expect(isRecord([])).toBe(false);
    expect(isRecord(null)).toBe(false);
    expect(isRecord({})).toBe(true);
  });

  it("taskActionKind различает виды или честный null", () => {
    expect(taskActionKind("repair_path")).toBe("repair_path");    expect(taskActionKind("nope")).toBeNull();
    expect(taskActionKind(undefined)).toBeNull();
    expect(taskActionKind({})).toBeNull();
    expect(taskActionKind({ install_new: { target_version: null } })).toBe("install_new");
    expect(taskActionKind({ noop: "docker_managed" })).toBe("noop");
  });

  it("scanStartJobId возвращает null вместо краша на чужой форме", () => {
    expect(scanStartJobId(undefined)).toBeNull();
    expect(scanStartJobId({})).toBeNull();
    expect(scanStartJobId({ Started: {} })).toBeNull();
    expect(scanStartJobId({ AlreadyRunning: null })).toBeNull();
  });

  it("jobEventKind/engineTaskStatusKind безопасны", () => {
    expect(jobEventKind(undefined)).toBeNull();
    expect(jobEventKind({ payload: null })).toBeNull();
    expect(jobEventKind({ payload: {} })).toBeNull();
    expect(engineTaskStatusKind(undefined)).toBe("unknown");
    expect(engineTaskStatusKind(null)).toBe("unknown");
  });
});
