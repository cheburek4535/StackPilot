// Тесты форматтеров: размеры, время, состояния, санитизация ошибок.
import { describe, expect, it } from "vitest";
import {
  capabilityLabels,
  formatAgeSeconds,
  formatBytes,
  formatRelativeTime,
  formatSizeMb,
  healthStateInfo,
  jobStatusLabel,
  phaseLabel,
  provenanceInfo,
  sanitizeErrorMessage,
  scanPhaseLabel,
  scanTerminalLabel,
  selectedSourceDescription,
  toolStateInfo,
  toolStateVersion,
  versionAssessmentInfo,
} from "./format";

describe("размеры", () => {
  it("МБ остаются МБ, тысячи переходят в ГБ", () => {
    expect(formatSizeMb(0)).toBe("0 МБ");
    expect(formatSizeMb(512)).toBe("512 МБ");
    expect(formatSizeMb(1024)).toBe("1.0 ГБ");
    expect(formatSizeMb(1536)).toBe("1.5 ГБ");
    expect(formatSizeMb(null)).toBe("—");
  });

  it("байты проходят всю лестницу единиц", () => {
    expect(formatBytes(512)).toContain("Б");
    expect(formatBytes(2048)).toContain("КБ");
    expect(formatBytes(5 * 1024 * 1024)).toContain("МБ");
    expect(formatBytes(3 * 1024 * 1024 * 1024)).toContain("ГБ");
    expect(formatBytes(-1)).toBe("—");
  });
});

describe("относительное время", () => {
  const now = new Date("2026-08-21T12:00:00Z");

  it("секунды/минуты/часы/дни", () => {
    expect(formatRelativeTime(new Date("2026-08-21T11:59:50Z"), now)).toBe("только что");
    expect(formatRelativeTime(new Date("2026-08-21T11:55:00Z"), now)).toBe("5 мин назад");
    expect(formatRelativeTime(new Date("2026-08-21T10:00:00Z"), now)).toBe("2 ч назад");
    expect(formatRelativeTime(new Date("2026-08-18T12:00:00Z"), now)).toBe("3 дн назад");
  });

  it("мусор и отсутствие данных честны", () => {
    expect(formatRelativeTime(null)).toBe("—");
    expect(formatRelativeTime("not-a-date")).toBe("—");
  });

  it("age_seconds из снапшота читаем", () => {
    expect(formatAgeSeconds(0)).toBe("только что");
    expect(formatAgeSeconds(45)).toBe("45 с назад");
    expect(formatAgeSeconds(null)).toBe("—");
  });
});

describe("подписи состояний — неизвестное не роняет UI", () => {
  it("все 12 kind ToolState имеют подписи", () => {
    const kinds = [
      "scan_pending", "scan_failed", "missing", "installed_healthy",
      "installed_health_unknown", "installed_unhealthy", "update_available",
      "path_broken", "manual_install", "docker_managed", "built_in_system",
      "unsupported_platform",
    ] as const;
    for (const kind of kinds) {
      const state = { kind } as never;
      const info = toolStateInfo(state);
      expect(info.label).toBeTruthy();
      expect(info.label).not.toContain("Неизвестное");
    }
  });

  it("неизвестный kind → «Неизвестное состояние» с нейтральным тоном", () => {
    const info = toolStateInfo({ kind: "future_state" } as never);
    expect(info.label).toBe("Неизвестное состояние");
    expect(info.tone).toBe("neutral");
  });

  it("версия извлекается только из вариантов, которые её несут", () => {
    expect(toolStateVersion({ kind: "installed_healthy", version: "22" })).toBe("22");
    expect(toolStateVersion({ kind: "update_available", installed: "18", recommended: "22" })).toBe("18");
    expect(toolStateVersion({ kind: "missing" })).toBeNull();
  });

  it("здоровье: no_checks_defined ≠ unhealthy (честное «не знаем»)", () => {
    expect(healthStateInfo({ kind: "no_checks_defined" }).label).toContain("не заявлены");
    expect(healthStateInfo({ kind: "not_checked" }).label).toBe("Не проверялся");
    expect(healthStateInfo({ kind: "unhealthy" }).tone).toBe("red");
    expect(healthStateInfo({ kind: "failed_to_run" }).label).toBe("Не удалось проверить");
  });

  it("оценка версии различает below_min и policy_violation", () => {
    expect(versionAssessmentInfo({ kind: "below_min" }).tone).toBe("red");
    expect(versionAssessmentInfo({ kind: "policy_violation" }).label).toContain("Противоречивая");
    expect(versionAssessmentInfo({ kind: "meets_recommended" }).tone).toBe("lime");
  });

  it("происхождение называет хозяина для bundled_with", () => {
    expect(provenanceInfo({ kind: "bundled_with", tool: "node" }).label).toBe("В комплекте с node");
    expect(provenanceInfo({ kind: "stack_pilot_managed" }).label).toContain("StackPilot");
  });

  it("флаги возможностей подписываются только включённые", () => {
    const labels = capabilityLabels({
      detectable: true,
      installable: true,
      updatable: false,
      removable: false,
      repairable: false,
      health_checkable: true,
      manual_instructions_available: false,
      docker_alternative_available: false,
    });
    expect(labels).toEqual(["Обнаруживаемый", "Устанавливаемый", "Проверяемый"]);
  });

  it("статусы заданий и фазы подписаны", () => {
    expect(jobStatusLabel("succeeded").tone).toBe("lime");
    expect(jobStatusLabel("partial").label).toContain("частично");
    expect(phaseLabel("updating_path")).toContain("PATH");
    expect(phaseLabel("checking_health")).toContain("здоров");
  });
});

describe("описание источника канонического плана", () => {
  it("помечает источники без контроля целостности", () => {
    const verified = selectedSourceDescription({
      kind: "pkg_manager",
      id: "Git.Git",
      description: "winget: Git.Git",
      sha256: "abc",
      needs_admin: false,
    });
    expect(verified).toBe("winget: Git.Git");

    const unverified = selectedSourceDescription({
      kind: "official",
      id: "x",
      description: "",
      sha256: null,
      needs_admin: false,
    });
    expect(unverified).toContain("без контроля целостности");
  });
});

describe("sanitizeErrorMessage — секреты маскируются, путь укорачивается", () => {
  it("маскирует token/password-подобные значения", () => {
    const msg = sanitizeErrorMessage("invoke failed: password=Sup3rSecret! at C:\\Users\\a\\app\\main.rs:1");
    expect(msg).not.toContain("Sup3rSecret!");
    expect(msg).toContain("password=***");
  });

  it("обрезает слишком длинные сообщения", () => {
    const long = sanitizeErrorMessage("x".repeat(1000));
    expect(long.length).toBeLessThanOrEqual(300);
    expect(long.endsWith("…")).toBe(true);
  });

  it("null/объекты не ломают функцию", () => {
    expect(sanitizeErrorMessage(null)).toBeTruthy();
    expect(sanitizeErrorMessage({ code: 5 })).toBeTruthy();
    expect(sanitizeErrorMessage(new Error("boom"))).toBe("boom");
  });
});

describe("scanPhaseLabel — фазы скана", () => {
  it("переводит PascalCase-фазы бэкенда", () => {
    expect(scanPhaseLabel("Queued")).toBe("в очереди");
    expect(scanPhaseLabel("Tools")).toBe("проверка инструментов");
    expect(scanPhaseLabel("Done")).toBe("готово");
  });

  it("неизвестная фаза возвращается как есть", () => {
    expect(scanPhaseLabel("FuturePhase")).toBe("FuturePhase");
  });
});

describe("scanTerminalLabel — терминальные состояния скана", () => {
  it("переводит все терминалы", () => {
    expect(scanTerminalLabel("Completed")).toContain("завершён");
    expect(scanTerminalLabel("Partial")).toContain("частичн");
    expect(scanTerminalLabel("Cancelled")).toBe("отменён");
    expect(scanTerminalLabel("Interrupted")).toContain("перезапуском");
  });
});
