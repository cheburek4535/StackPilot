import { describe, expect, it } from "vitest";
import {
  EMPTY_HELP_PROGRESS,
  HELP_GRADUATED,
  helpEnabled,
  hintPrereqsMet,
  isHintVisible,
  withDid,
  withSaw,
  type HelpHintSpec,
} from "./help";

const spec: HelpHintSpec = {
  id: "hint.a",
  resolvedBy: ["action.a"],
  requiresSaw: ["page.a"],
};

describe("help progress helpers", () => {
  it("withSaw is idempotent and does not mutate", () => {
    const base = { saw: [], did: [] };
    const once = withSaw(base, "page.a");
    const twice = withSaw(once, "page.a");
    expect(once).toEqual({ saw: ["page.a"], did: [] });
    expect(twice).toBe(once);
    expect(base).toEqual({ saw: [], did: [] });
  });

  it("withDid is idempotent", () => {
    const once = withDid(EMPTY_HELP_PROGRESS, "action.a");
    expect(withDid(once, "action.a")).toBe(once);
  });
});

describe("help master switch", () => {
  it("is on for first-time users", () => {
    expect(helpEnabled(EMPTY_HELP_PROGRESS, false)).toBe(true);
  });

  it("is off after graduation unless beginner mode is on", () => {
    const graduated = withDid(EMPTY_HELP_PROGRESS, HELP_GRADUATED);
    expect(helpEnabled(graduated, false)).toBe(false);
    expect(helpEnabled(graduated, true)).toBe(true);
  });
});

describe("isHintVisible", () => {
  it("hides a hint until its prerequisites were seen", () => {
    expect(isHintVisible(EMPTY_HELP_PROGRESS, false, spec)).toBe(false);
    const seen = withSaw(EMPTY_HELP_PROGRESS, "page.a");
    expect(isHintVisible(seen, false, spec)).toBe(true);
  });

  it("auto mode hides the hint once the action is done", () => {
    const seen = withSaw(EMPTY_HELP_PROGRESS, "page.a");
    const done = withDid(seen, "action.a");
    expect(isHintVisible(done, false, spec)).toBe(false);
  });

  it("auto mode hides the hint after explicit dismissal", () => {
    const seen = withSaw(EMPTY_HELP_PROGRESS, "page.a");
    const dismissed = withDid(seen, "hint.a");
    expect(isHintVisible(dismissed, false, spec)).toBe(false);
  });

  it("beginner mode keeps the hint even after the action is done", () => {
    const done = withDid(
      withSaw(EMPTY_HELP_PROGRESS, "page.a"),
      "action.a",
    );
    expect(isHintVisible(done, true, spec)).toBe(true);
  });

  it("beginner mode still honors explicit dismissal", () => {
    const dismissed = withDid(
      withSaw(EMPTY_HELP_PROGRESS, "page.a"),
      "hint.a",
    );
    expect(isHintVisible(dismissed, true, spec)).toBe(false);
  });

  it("never shows anything after graduation with mode off", () => {
    const graduated = withDid(
      withSaw(EMPTY_HELP_PROGRESS, "page.a"),
      HELP_GRADUATED,
    );
    expect(isHintVisible(graduated, false, spec)).toBe(false);
  });
});

describe("hintPrereqsMet", () => {
  it("is met when no prerequisites are declared", () => {
    expect(hintPrereqsMet(EMPTY_HELP_PROGRESS, { id: "x" })).toBe(true);
  });
});
