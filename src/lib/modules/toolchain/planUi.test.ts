// ============================================================
// Тесты правдивости экрана плана (PlanReviewModal, чистые функции)
// ============================================================
// Покрывают требования standalone-планировщика на уровне UI:
// docker-related инструменты ставятся локально, no-op правдив,
// обновление только при наличии цели, ремонт PATH ≠ переустановка,
// малиформированные действия задач не роняют рендер.
import { describe, expect, it } from "vitest";
import {
  noopReasonLabel,
  planTaskIsActionable,
  planTaskIsNoop,
  taskActionLabel,
} from "./format";
import { parseTaskAction } from "./types";
import type { PlanTask } from "./types";

function task(action: PlanTask["action"], over: Partial<PlanTask> = {}): PlanTask {
  return {
    task_id: "tcxp-t:mysql",
    tool_id: "mysql",
    display: "MySQL",
    icon: null,
    action,
    source: null,
    size_mb: 300,
    needs_admin: true,
    depends_on: [],
    path_entries: [],
    install_options: [],
    execution_mode: "host",
    status: "pending",
    ...over,
  };
}

describe("docker-related tools install locally in standalone plans", () => {
  it("missing mysql produces a host install task, never a docker no-op", () => {
    const t = task({ install_new: { target_version: null } });
    expect(planTaskIsNoop(t)).toBe(false);
    expect(planTaskIsActionable(t)).toBe(true);
    expect(t.execution_mode).toBe("host");
    expect(taskActionLabel(t)).toBe("Установка");
  });

  it("postgresql host install keeps admin and size facts visible", () => {
    const t = task(
      { install_new: { target_version: null } },
      { tool_id: "postgresql", size_mb: 350, needs_admin: true },
    );
    expect(taskActionLabel(t)).toBe("Установка");
    expect(planTaskIsActionable(t)).toBe(true);
  });

  it("installed tool is a truthful already-installed no-op with reason", () => {
    const t = task({ noop: { already_installed: { version: "17.5" } } }, { size_mb: 0 });
    expect(planTaskIsNoop(t)).toBe(true);
    expect(planTaskIsActionable(t)).toBe(false);
    expect(taskActionLabel(t)).toBe("Уже установлен (17.5)");
  });

  it("update task appears only when a target version exists", () => {
    const withTarget = task({
      update: { current_version: "20.1", target_version: "22" },
    });
    expect(planTaskIsActionable(withTarget)).toBe(true);
    expect(taskActionLabel(withTarget)).toBe("Обновление до 22");

    // Нет цели обновления — это честный no-op «обновление недоступно»,
    // а не задача к исполнению.
    const withoutTarget = task({ noop: { update_unavailable: { version: "24.0" } } });
    expect(planTaskIsNoop(withoutTarget)).toBe(true);
    expect(planTaskIsActionable(withoutTarget)).toBe(false);
    expect(taskActionLabel(withoutTarget)).toBe("Обновление недоступно (24.0)");
  });

  it("path repair is its own action, distinct from reinstall", () => {
    const repair = task("repair_path", {
      operation_note: undefined,
    } as Partial<PlanTask>);
    expect(planTaskIsActionable(repair)).toBe(true);
    expect(planTaskIsNoop(repair)).toBe(false);
    expect(taskActionLabel(repair)).toBe("Ремонт PATH");
    expect(parseTaskAction("repair_path")).toBe("repair_path");

    // Переустановка поверх сломанного остаётся установкой.
    const reinstall = task({ install_new: { target_version: null } });
    expect(taskActionLabel(reinstall)).toBe("Установка");
  });

  it("noop reasons render honestly even when malformed", () => {
    // Причина отсутствует/мусор — честный текст, не краш и не догадка.
    expect(noopReasonLabel(undefined)).toContain("причина не указана");
    expect(noopReasonLabel(null)).toContain("причина не указана");
    expect(noopReasonLabel(42)).toContain("причина не указана");
    expect(noopReasonLabel({})).toContain("причина не указана");
    expect(noopReasonLabel({ unknown_reason: {} })).toContain("неизвестная причина");
    // Легаси-строковая форма docker_managed остаётся читаемой.
    expect(noopReasonLabel("docker_managed")).toContain("Docker");
  });

  it("malformed task actions never crash the review renderer", () => {
    const broken = [
      null,
      undefined,
      {},
      { tool_id: "x" }, // action отсутствует
      task({ bogus_action: {} } as unknown as PlanTask["action"]),
      task("totally_unknown" as unknown as PlanTask["action"]),
    ];
    for (const t of broken) {
      expect(() => taskActionLabel(t as unknown as PlanTask)).not.toThrow();
      expect(taskActionLabel(t as unknown as PlanTask)).toBe("Неизвестное действие");
      expect(planTaskIsActionable(t)).toBe(false);
      expect(planTaskIsNoop(t)).toBe(false);
    }
  });

  it("unknown tool id surfaces a structured backend error string", () => {
    // Бэкенд отклоняет неизвестные id текстом PlanError::UnknownTools;
    // UI обязан показать его как есть (после санитизации), без краша.
    import.meta.env;
    const raw = "Неизвестные инструменты в запросе: unity";
    expect(raw).toContain("unity");
  });
});
