/**
 * Run store — reactive state for V2 launch runs.
 *
 * Subscribes to backend events and maintains an idempotent map of runs.
 * Does NOT duplicate orchestration logic — the backend is authoritative.
 *
 * Usage:
 *   import { runStore } from "$lib/modules/devlauncher/runStore";
 *   // In onMount:
 *   runStore.init();
 *   // Access:
 *   runStore.currentRun; // $state
 *   // Cleanup in onDestroy:
 *   runStore.destroy();
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import * as api from "./api";
import type {
  LaunchRun,
  RunStatusPayload,
  StepStatusPayload,
  ProcessStartedPayload,
  Diagnostic,
} from "./types";
import { isRunTerminal } from "./types";

type RunStoreState = {
  /** All known runs, keyed by run_id. */
  runs: Map<string, LaunchRun>;
  /** The most recently active run (for quick access). */
  currentRunId: string | null;
  /** Whether the store has been initialized (listeners registered). */
  initialized: boolean;
};

const state: RunStoreState = {
  runs: new Map(),
  currentRunId: null,
  initialized: false,
};

const listeners: UnlistenFn[] = [];
let destroyed = false;

// ---------------------------------------------------------------------------
// Derived accessors (read-only, used by Svelte components)
// ---------------------------------------------------------------------------

/** Get a run by ID. */
export function getRunById(runId: string): LaunchRun | undefined {
  return state.runs.get(runId);
}

/** Get the current (most recent) run. */
export function getCurrentRun(): LaunchRun | null {
  if (!state.currentRunId) return null;
  return state.runs.get(state.currentRunId) ?? null;
}

/** Get all runs as an array (for listing). */
export function getAllRuns(): LaunchRun[] {
  return Array.from(state.runs.values());
}

/** Get all active (non-terminal) runs. */
export function getActiveRuns(): LaunchRun[] {
  return Array.from(state.runs.values()).filter(
    (r) => !isRunTerminal(r.status),
  );
}

/** Get step execution state for a specific step in a run. */
export function getStepState(
  runId: string,
  stepId: string,
): LaunchRun["steps"][number] | undefined {
  const run = state.runs.get(runId);
  if (!run) return undefined;
  return run.steps.find((s) => s.step_id === stepId);
}

// ---------------------------------------------------------------------------
// Mutations
// ---------------------------------------------------------------------------

/** Apply a step status update idempotently. */
function applyStepUpdate(runId: string, step: StepStatusPayload["step"]) {
  const run = state.runs.get(runId);
  if (!run) return;

  const idx = run.steps.findIndex((s) => s.step_id === step.step_id);
  if (idx >= 0) {
    run.steps[idx] = { ...step };
  } else {
    run.steps.push({ ...step });
  }
}

/** Apply a run status update. */
function applyRunStatus(runId: string, status: RunStatusPayload["status"]) {
  const run = state.runs.get(runId);
  if (!run) return;
  run.status = status;
  if (isRunTerminal(status)) {
    run.finished_at = new Date().toISOString();
  }
}

/** Store a newly created run. */
function storeRun(run: LaunchRun) {
  state.runs.set(run.run_id, { ...run, steps: run.steps.map((s) => ({ ...s })) });
  state.currentRunId = run.run_id;
}

/** Update a run with full snapshot from backend. */
function updateRun(run: LaunchRun) {
  const existing = state.runs.get(run.run_id);
  if (existing) {
    Object.assign(existing, {
      ...run,
      steps: run.steps.map((s) => ({ ...s })),
    });
  } else {
    storeRun(run);
  }
}

// ---------------------------------------------------------------------------
// Public actions
// ---------------------------------------------------------------------------

/** Create and start a run from a V2 profile.
 *  If a legacy LaunchProfile is passed, it's wrapped as a V2 profile with
 *  sequential dependencies (same ordering as the actions array). */
export async function launchRun(
  profile: import("./types").LaunchProfileV2 | import("./types").LaunchProfile,
): Promise<LaunchRun> {
  // Convert legacy profile to V2 if needed.
  const v2 = toV2Profile(profile);

  // Launch the preferred IDE up front (when configured on the profile) so
  // the user sees their editor open with the project immediately — the
  // orchestrator run covers the remaining steps.
  if (v2.preferred_ide && v2.project_root) {
    try {
      await api.launchIde(v2.preferred_ide, v2.project_root);
    } catch {
      // Non-critical — the profile may also carry an explicit
      // open_application step for the IDE.
    }
  }

  const run = await api.createRun(v2);
  storeRun(run);
  // Start async execution — backend emits events from here on.
  await api.startRun(run.run_id);
  // Return the updated run state (events may have fired already).
  try {
    return await api.getRun(run.run_id);
  } catch {
    return run;
  }
}

/** Convert a legacy LaunchProfile to LaunchProfileV2. */
function toV2Profile(
  profile: import("./types").LaunchProfileV2 | import("./types").LaunchProfile,
): import("./types").LaunchProfileV2 {
  // If already V2, return as-is.
  if ("steps" in profile && Array.isArray(profile.steps)) {
    return profile as import("./types").LaunchProfileV2;
  }

  // Legacy profile: convert actions to V2 steps with sequential dependencies.
  const legacy = profile as import("./types").LaunchProfile;
  const steps: import("./types").LaunchStep[] = [];
  let prevId: string | null = null;

  for (const action of legacy.actions) {
    const stepId = action.id;
    const dependsOn: string[] = prevId ? [prevId] : [];

    steps.push({
      id: stepId,
      label: action.label,
      enabled: action.enabled,
      kind: actionTypeToStepKind(action.action_type),
      depends_on: dependsOn,
    });
    prevId = stepId;
  }

  return {
    schema_version: "2",
    id: legacy.id ?? `legacy-${legacy.name}`,
    name: legacy.name,
    description: legacy.description,
    project_root: legacy.project_path,
    steps,
    environment_binding_id: legacy.environment_binding_id,
    preferred_ide: legacy.preferred_ide,
  };
}

/** Convert a legacy ActionType to a V2 StepKind. */
function actionTypeToStepKind(
  actionType: import("./types").ActionType,
): import("./types").StepKind {
  if ("RunCommand" in actionType) {
    return { type: "run_command", command: actionType.RunCommand.command };
  }
  if ("OpenUrl" in actionType) {
    return { type: "open_url", url: actionType.OpenUrl.url };
  }
  if ("OpenApplication" in actionType) {
    return {
      type: "open_application",
      path: actionType.OpenApplication.path,
      args: actionType.OpenApplication.args ? actionType.OpenApplication.args.split(/\s+/) : null,
    };
  }
  if ("WaitForUrl" in actionType) {
    return { type: "wait_for_url", url: actionType.WaitForUrl.url };
  }
  if ("WaitForPort" in actionType) {
    return {
      type: "wait_for_port",
      host: actionType.WaitForPort.host,
      port: actionType.WaitForPort.port,
    };
  }
  if ("Delay" in actionType) {
    return { type: "delay", seconds: actionType.Delay.seconds };
  }
  if ("ExecuteScript" in actionType) {
    return {
      type: "run_script",
      script: actionType.ExecuteScript.script,
      shell: actionType.ExecuteScript.shell,
    };
  }
  return { type: "run_command", command: "" };
}

/** Cancel an active run. Idempotent — safe to call after terminal state. */
export async function cancelCurrentRun(): Promise<void> {
  if (!state.currentRunId) return;
  try {
    await api.cancelRun(state.currentRunId);
  } catch {
    // Already cancelled or terminal — safe to ignore.
  }
}

/** Stop all processes in a run without cancelling the run. */
export async function stopProcesses(runId: string): Promise<void> {
  try {
    await api.stopRunProcesses(runId);
  } catch {
    // Non-critical.
  }
}

/** Fetch and store a specific run (for remount recovery). */
export async function fetchRun(runId: string): Promise<LaunchRun | null> {
  try {
    const run = await api.getRun(runId);
    updateRun(run);
    return run;
  } catch {
    return null;
  }
}

/** Fetch all active runs from the backend (for remount recovery). */
export async function recoverActiveRuns(): Promise<LaunchRun[]> {
  try {
    const runs = await api.listActiveRuns();
    for (const run of runs) {
      updateRun(run);
    }
    return runs;
  } catch {
    return [];
  }
}

/** Fetch ALL runs (including finished) from the backend for history display. */
export async function recoverAllRuns(): Promise<LaunchRun[]> {
  try {
    const runs = await api.listAllRuns();
    for (const run of runs) {
      updateRun(run);
    }
    return runs;
  } catch {
    return [];
  }
}

/** Get the most recent run for a profile (by profile name), if any. */
export function getLatestRunForProfile(
  profileName: string,
): LaunchRun | null {
  let latest: LaunchRun | null = null;
  for (const run of state.runs.values()) {
    if (run.profile_name !== profileName) continue;
    if (!latest || run.created_at > latest.created_at) {
      latest = run;
    }
  }
  return latest;
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/**
 * Initialize the store: register event listeners and recover active runs.
 * Safe to call multiple times (idempotent).
 */
export async function init(): Promise<void> {
  if (state.initialized || destroyed) return;
  state.initialized = true;

  // Register backend event listeners
  const regRunCreated = await listen<LaunchRun>(
    "devlauncher:run-created",
    (e) => {
      storeRun(e.payload);
    },
  );
  listeners.push(regRunCreated);

  const regRunStatus = await listen<RunStatusPayload>(
    "devlauncher:run-status-changed",
    (e) => {
      applyRunStatus(e.payload.run_id, e.payload.status);
    },
  );
  listeners.push(regRunStatus);

  const regStepStatus = await listen<StepStatusPayload>(
    "devlauncher:step-status-changed",
    (e) => {
      if (e.payload?.step?.step_id) {
        applyStepUpdate(e.payload.run_id, e.payload.step);
      }
    },
  );
  listeners.push(regStepStatus);

  const regProcessStarted = await listen<ProcessStartedPayload>(
    "devlauncher:process-started",
    (e) => {
      // Store the process_id in the step execution state.
      const run = state.runs.get(e.payload.run_id);
      if (run) {
        const step = run.steps.find(
          (s) => s.step_id === e.payload.step_id,
        );
        if (step) {
          step.process_id = e.payload.process.id;
        }
      }
    },
  );
  listeners.push(regProcessStarted);

  const regDiagnostic = await listen<Diagnostic>(
    "devlauncher:diagnostic",
    (e) => {
      const run = state.runs.get(e.payload.run_id);
      if (run) {
        // Append diagnostic if not already present (idempotent by timestamp).
        const exists = run.diagnostics.some(
          (d) =>
            d.timestamp === e.payload.timestamp &&
            d.message === e.payload.message,
        );
        if (!exists) {
          run.diagnostics.push(e.payload);
        }
      }
    },
  );
  listeners.push(regDiagnostic);

  const regRunFinished = await listen<LaunchRun>(
    "devlauncher:run-finished",
    (e) => {
      updateRun(e.payload);
    },
  );
  listeners.push(regRunFinished);

  // Recover any active runs from a previous session (remount/reload).
  // Also recover finished runs so run history survives tab switches.
  await Promise.all([recoverActiveRuns(), recoverAllRuns()]);
}

/**
 * Destroy the store: remove all event listeners and reset state.
 * Safe to re-init afterwards (e.g. after navigating back to a tab).
 */
export function destroy(): void {
  destroyed = true;
  for (const fn of listeners) {
    fn();
  }
  listeners.length = 0;
  state.initialized = false;
  state.runs.clear();
  state.currentRunId = null;
  // Allow a later init() to re-register listeners (tab switches).
  destroyed = false;
}

// ---------------------------------------------------------------------------
// Expose a Svelte 5–compatible store shape for component use.
// Components import individual functions, not a reactive class — this keeps
// the store lightweight and avoids class-based reactivity edge cases.
// ---------------------------------------------------------------------------
