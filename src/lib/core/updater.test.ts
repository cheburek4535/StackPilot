import { describe, it, expect } from "vitest";
import { compareVersions, extractVersion, isNewerVersion } from "./updater.svelte";

describe("updater: version parsing and comparison", () => {
  it("extracts version strings properly", () => {
    expect(extractVersion("v1.4.0")).toBe("1.4.0");
    expect(extractVersion("V2.0.1")).toBe("2.0.1");
    expect(extractVersion("1.4.0")).toBe("1.4.0");
    expect(extractVersion("v1.5.0-beta.1")).toBe("1.5.0-beta.1");
    expect(extractVersion("StackPilot-v1.4.2")).toBe("1.4.2");
    expect(extractVersion("")).toBe("0.0.0");
  });

  it("compares versions correctly", () => {
    // Greater
    expect(compareVersions("v1.4.1", "1.4.0")).toBe(1);
    expect(compareVersions("1.5.0", "1.4.0")).toBe(1);
    expect(compareVersions("2.0.0", "1.4.0")).toBe(1);
    expect(compareVersions("1.4.10", "1.4.2")).toBe(1);

    // Equal
    expect(compareVersions("v1.4.0", "1.4.0")).toBe(0);
    expect(compareVersions("1.4.0", "1.4.0")).toBe(0);
    expect(compareVersions("v1.4", "1.4.0")).toBe(0);

    // Smaller
    expect(compareVersions("v1.3.9", "1.4.0")).toBe(-1);
    expect(compareVersions("1.4.0", "1.4.1")).toBe(-1);
    expect(compareVersions("1.4.0", "2.0.0")).toBe(-1);
  });

  it("handles prerelease comparisons", () => {
    // A stable release is newer than a prerelease of the same version
    expect(compareVersions("1.4.0", "1.4.0-rc1")).toBe(1);
    expect(compareVersions("1.4.0-rc1", "1.4.0")).toBe(-1);

    // Higher core version with prerelease is still newer than lower stable version
    expect(compareVersions("1.5.0-beta.1", "1.4.0")).toBe(1);
  });

  it("correctly identifies if a version is strictly newer", () => {
    expect(isNewerVersion("v1.4.1", "1.4.0")).toBe(true);
    expect(isNewerVersion("v1.4.0", "1.4.0")).toBe(false);
    expect(isNewerVersion("v1.3.5", "1.4.0")).toBe(false);
    expect(isNewerVersion("v2.0.0", "1.4.0")).toBe(true);
  });
});
