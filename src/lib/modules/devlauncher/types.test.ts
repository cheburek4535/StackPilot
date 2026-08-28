/**
 * Tests for DevLauncher V2 types and helper functions.
 */

import { describe, it, expect } from "vitest";
import {
  isV2Profile,
  stepKindIcon,
  stepKindLabel,
  stepKindSummary,
  stepStatusClass,
  runStatusClass,
  trackingQualityLabel,
  diagnosticClass,
  isRunTerminal,
  isStepTerminal,
} from "./types";
import type {
  LaunchProfile,
  LaunchProfileV2,
  StepKind,
  RunStatus,
  StepStatus,
  ProcessTrackingQuality,
  DiagnosticSeverity,
} from "./types";

describe("isV2Profile", () => {
  it("returns true for profiles with schema_version '2'", () => {
    const profile = { name: "test", description: "", project_path: null, schema_version: "2", actions: [] } as LaunchProfile;
    expect(isV2Profile(profile)).toBe(true);
  });

  it("returns false for profiles without schema_version", () => {
    const profile = { name: "test", description: "", project_path: null, actions: [] } as LaunchProfile;
    expect(isV2Profile(profile)).toBe(false);
  });

  it("returns false for profiles with other schema versions", () => {
    const profile = { name: "test", description: "", project_path: null, schema_version: "1", actions: [] } as LaunchProfile;
    expect(isV2Profile(profile)).toBe(false);
  });
});

describe("stepKindIcon", () => {
  it("returns correct icons for all step kinds", () => {
    expect(stepKindIcon({ type: "run_command", command: "npm run dev" })).toBe("▶");
    expect(stepKindIcon({ type: "run_script", script: "echo hi" })).toBe("📜");
    expect(stepKindIcon({ type: "open_application", path: "/usr/bin/code" })).toBe("⬛");
    expect(stepKindIcon({ type: "open_url", url: "http://localhost:3000" })).toBe("🌐");
    expect(stepKindIcon({ type: "wait_for_port", host: "localhost", port: 3000 })).toBe("🔌");
    expect(stepKindIcon({ type: "wait_for_url", url: "http://localhost:3000" })).toBe("⏳");
    expect(stepKindIcon({ type: "wait_for_docker" })).toBe("🐳");
    expect(stepKindIcon({ type: "delay", seconds: 5 })).toBe("⏱");
    expect(stepKindIcon({ type: "open_terminal", command: "bash" })).toBe("🖥");
    expect(stepKindIcon({ type: "open_folder", path: "." })).toBe("📁");
  });
});

describe("stepKindLabel", () => {
  it("returns correct labels", () => {
    expect(stepKindLabel({ type: "run_command", command: "npm run dev" })).toBe("Command");
    expect(stepKindLabel({ type: "wait_for_port", host: "localhost", port: 3000 })).toBe("Wait for port");
    expect(stepKindLabel({ type: "wait_for_docker" })).toBe("Wait for Docker");
  });
});

describe("stepKindSummary", () => {
  it("returns the command for run_command", () => {
    expect(stepKindSummary({ type: "run_command", command: "npm run dev" })).toBe("npm run dev");
  });

  it("returns host:port for wait_for_port", () => {
    expect(stepKindSummary({ type: "wait_for_port", host: "localhost", port: 3000 })).toBe("localhost:3000");
  });

  it("returns seconds for delay", () => {
    expect(stepKindSummary({ type: "delay", seconds: 5 })).toBe("5s");
  });
});

describe("stepStatusClass", () => {
  it("returns correct CSS classes", () => {
    expect(stepStatusClass("pending")).toBe("step-pending");
    expect(stepStatusClass("running")).toBe("step-running");
    expect(stepStatusClass("succeeded")).toBe("step-succeeded");
    expect(stepStatusClass("failed")).toBe("step-failed");
    expect(stepStatusClass("skipped")).toBe("step-skipped");
    expect(stepStatusClass("cancelled")).toBe("step-cancelled");
    expect(stepStatusClass("retrying")).toBe("step-retrying");
  });
});

describe("runStatusClass", () => {
  it("returns correct CSS classes", () => {
    expect(runStatusClass("pending")).toBe("run-pending");
    expect(runStatusClass("running")).toBe("run-running");
    expect(runStatusClass("succeeded")).toBe("run-succeeded");
    expect(runStatusClass("failed")).toBe("run-failed");
    expect(runStatusClass("cancelled")).toBe("run-cancelled");
    expect(runStatusClass("partial_success")).toBe("run-partial");
  });
});

describe("trackingQualityLabel", () => {
  it("returns correct labels", () => {
    expect(trackingQualityLabel("exact")).toBe("Exact PID tracking");
    expect(trackingQualityLabel("terminal_wrapper")).toBe("Terminal wrapper (inner PID not tracked)");
    expect(trackingQualityLabel("approximate")).toBe("Approximate (process may have been replaced)");
    expect(trackingQualityLabel("detached")).toBe("Detached (no PID tracking)");
    expect(trackingQualityLabel(null)).toBe("Unknown tracking quality");
    expect(trackingQualityLabel(undefined)).toBe("Unknown tracking quality");
  });
});

describe("diagnosticClass", () => {
  it("returns correct CSS classes", () => {
    expect(diagnosticClass("info")).toBe("diag-info");
    expect(diagnosticClass("warning")).toBe("diag-warning");
    expect(diagnosticClass("error")).toBe("diag-error");
  });
});

describe("isRunTerminal", () => {
  it("returns true for terminal run statuses", () => {
    expect(isRunTerminal("succeeded")).toBe(true);
    expect(isRunTerminal("failed")).toBe(true);
    expect(isRunTerminal("cancelled")).toBe(true);
    expect(isRunTerminal("partial_success")).toBe(true);
  });

  it("returns false for non-terminal run statuses", () => {
    expect(isRunTerminal("pending")).toBe(false);
    expect(isRunTerminal("running")).toBe(false);
  });
});

describe("isStepTerminal", () => {
  it("returns true for terminal step statuses", () => {
    expect(isStepTerminal("succeeded")).toBe(true);
    expect(isStepTerminal("failed")).toBe(true);
    expect(isStepTerminal("skipped")).toBe(true);
    expect(isStepTerminal("cancelled")).toBe(true);
  });

  it("returns false for non-terminal step statuses", () => {
    expect(isStepTerminal("pending")).toBe(false);
    expect(isStepTerminal("running")).toBe(false);
    expect(isStepTerminal("retrying")).toBe(false);
  });
});
