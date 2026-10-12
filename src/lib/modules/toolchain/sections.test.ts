import { describe, expect, it } from "vitest";
import {
  TOOLCHAIN_SECTIONS,
  getToolPopularityRank,
  getToolSectionId,
} from "./sections";

describe("Toolchain sections & ordering", () => {
  it("has exactly 7 canonical sections ordered from must-have down to infrastructure", () => {
    expect(TOOLCHAIN_SECTIONS).toHaveLength(7);
    const ids = TOOLCHAIN_SECTIONS.map((s) => s.id);
    expect(ids).toEqual([
      "must_have",
      "languages",
      "package_managers",
      "frameworks",
      "databases",
      "tooling",
      "infrastructure",
    ]);
  });

  it("places must-have core utilities in must_have section with correct priority", () => {
    expect(getToolSectionId("winget")).toBe("must_have");
    expect(getToolSectionId("brew")).toBe("must_have");
    expect(getToolSectionId("git")).toBe("must_have");
    expect(getToolSectionId("docker")).toBe("must_have");
    expect(getToolSectionId("vscode")).toBe("must_have");

    // Winget and Git have highest popularity ranks
    expect(getToolPopularityRank("winget")).toBeLessThan(getToolPopularityRank("curl"));
    expect(getToolPopularityRank("git")).toBeLessThan(getToolPopularityRank("tar"));
  });

  it("maps languages and runtimes correctly", () => {
    expect(getToolSectionId("node")).toBe("languages");
    expect(getToolSectionId("python")).toBe("languages");
    expect(getToolSectionId("rust")).toBe("languages");
    expect(getToolSectionId("go")).toBe("languages");
    expect(getToolSectionId("dotnet")).toBe("languages");
    expect(getToolSectionId("java")).toBe("languages");
    expect(getToolSectionId("kotlin")).toBe("languages");
    expect(getToolSectionId("zig")).toBe("languages");
  });

  it("maps package managers and build tools correctly", () => {
    expect(getToolSectionId("npm")).toBe("package_managers");
    expect(getToolSectionId("pip")).toBe("package_managers");
    expect(getToolSectionId("cargo")).toBe("package_managers");
    expect(getToolSectionId("rustc")).toBe("package_managers");
    expect(getToolSectionId("composer")).toBe("package_managers");
    expect(getToolSectionId("maven")).toBe("package_managers");
    expect(getToolSectionId("gradle")).toBe("package_managers");
    expect(getToolSectionId("cmake")).toBe("package_managers");
  });

  it("maps databases to databases section", () => {
    expect(getToolSectionId("sqlite")).toBe("databases");
    expect(getToolSectionId("postgresql")).toBe("databases");
    expect(getToolSectionId("redis")).toBe("databases");
    expect(getToolSectionId("mysql")).toBe("databases");
    expect(getToolSectionId("mongodb")).toBe("databases");
  });

  it("places Docker-preferred services (Kafka, Grafana) in the lowest infrastructure section", () => {
    expect(getToolSectionId("kafka")).toBe("infrastructure");
    expect(getToolSectionId("grafana")).toBe("infrastructure");

    const infraOrder = TOOLCHAIN_SECTIONS.find((s) => s.id === "infrastructure")?.order;
    const mustHaveOrder = TOOLCHAIN_SECTIONS.find((s) => s.id === "must_have")?.order;
    expect(infraOrder).toBe(7);
    expect(mustHaveOrder).toBe(1);
  });

  it("maps frameworks and mobile tools correctly", () => {
    expect(getToolSectionId("flutter")).toBe("frameworks");
    expect(getToolSectionId("android")).toBe("frameworks");
    expect(getToolSectionId("tauri-cli")).toBe("frameworks");
    expect(getToolSectionId("qt")).toBe("frameworks");
  });

  it("maps CLI and cloud tools to tooling section", () => {
    expect(getToolSectionId("terraform")).toBe("tooling");
    expect(getToolSectionId("firebase")).toBe("tooling");
    expect(getToolSectionId("csharprepl")).toBe("tooling");
  });

  it("falls back to category mapping for unknown tools gracefully", () => {
    expect(getToolSectionId("custom_db", "database")).toBe("databases");
    expect(getToolSectionId("new_lang", "language")).toBe("languages");
    expect(getToolSectionId("some_daemon", "middleware")).toBe("infrastructure");
    expect(getToolSectionId("random_thing")).toBe("tooling");
  });
});
