// ---------------------------------------------------------------------------
// Shared DevLauncher step building & editing helpers.
//
// Single source of truth for the micro-templates used by the profile
// editor (DevLauncher profile page) and the project analyzer page.
// Pure functions only — no state, no API calls.
// ---------------------------------------------------------------------------

import type { LaunchStep, StepKind, LaunchAction, ActionType } from "./types";

export type AddTemplate =
  | { kind: "terminal_plain" }
  | { kind: "terminal_cmd" }
  | { kind: "run_command" }
  | { kind: "open_folder" }
  | { kind: "open_url" }
  | { kind: "wait_port" }
  | { kind: "delay" };

export type AddTemplateDraft = {
  command: string;
  workdir: string;
  path: string;
  url: string;
  port: string;
  host: string;
  timeout: string;
  seconds: string;
};

export function emptyAddTemplateDraft(): AddTemplateDraft {
  return {
    command: "",
    workdir: "",
    path: "",
    url: "",
    port: "8080",
    host: "127.0.0.1",
    timeout: "90",
    seconds: "5",
  };
}

/** Build a new step from a micro-template. Working dir defaults to the
 *  project root so terminals/commands open in the right place. */
export function buildStep(tpl: AddTemplate, draft: AddTemplateDraft, projectPath: string): LaunchStep {
  const id = crypto.randomUUID();
  const wd = projectPath || undefined;
  const workdir = draft.workdir.trim() || wd;
  switch (tpl.kind) {
    case "terminal_plain":
      return {
        id,
        label: "Открыть терминал",
        enabled: true,
        kind: { type: "open_terminal", command: "" },
        depends_on: [],
        working_directory: wd,
        visibility: "visible_terminal",
        execution_mode: "long_running",
        completion: { type: "process_started" },
      };
    case "terminal_cmd": {
      const command = draft.command.trim();
      return {
        id,
        label: command ? `Терминал: ${command}` : "Открыть терминал",
        enabled: true,
        kind: { type: "open_terminal", command },
        depends_on: [],
        working_directory: workdir,
        visibility: "visible_terminal",
        execution_mode: "long_running",
        completion: { type: "process_started" },
      };
    }
    case "run_command": {
      const command = draft.command.trim();
      return {
        id,
        label: `Выполнить: ${command}`,
        enabled: true,
        kind: { type: "run_command", command },
        depends_on: [],
        working_directory: workdir,
        visibility: "visible_terminal",
        execution_mode: "long_running",
        completion: { type: "process_started" },
      };
    }
    case "open_folder": {
      const path = draft.path.trim() || wd || "";
      return {
        id,
        label: "Открыть папку",
        enabled: true,
        kind: { type: "open_folder", path },
        depends_on: [],
        working_directory: path || undefined,
        completion: { type: "external_launch_accepted" },
      };
    }
    case "open_url": {
      const url = draft.url.trim();
      return {
        id,
        label: `Открыть URL: ${url}`,
        enabled: true,
        kind: { type: "open_url", url },
        depends_on: [],
        completion: { type: "external_launch_accepted" },
      };
    }
    case "wait_port": {
      const port = parseInt(draft.port, 10) || 0;
      const host = draft.host.trim() || "127.0.0.1";
      const timeout = parseInt(draft.timeout, 10) || 90;
      return {
        id,
        label: `Ожидание порта ${host}:${port}`,
        enabled: true,
        kind: { type: "wait_for_port", host, port, candidate_ports: [] },
        depends_on: [],
        timeout,
        completion: { type: "port_open", host, port, timeout_secs: timeout },
        retry_policy: { max_retries: 2, delay_ms: 2000, backoff_multiplier: 1.5 },
      };
    }
    case "delay": {
      const seconds = parseInt(draft.seconds, 10) || 5;
      return {
        id,
        label: `Пауза ${seconds} сек`,
        enabled: true,
        kind: { type: "delay", seconds },
        depends_on: [],
        timeout: seconds + 10,
        completion: { type: "delay_elapsed", seconds },
      };
    }
  }
}

/** Legacy mapping for steps produced by the micro-templates — used to
 *  display the same actions on legacy (schema v1) profiles. */
export function stepToAction(step: LaunchStep): LaunchAction {
  const kind: StepKind = step.kind;
  let action_type: ActionType;
  switch (kind.type) {
    case "run_command":
      action_type = {
        RunCommand: {
          command: kind.command,
          working_dir: step.working_directory ?? null,
          persistent:
            step.visibility === "visible_terminal" || step.execution_mode === "long_running"
              ? true
              : null,
        },
      };
      break;
    case "open_application":
      action_type = {
        OpenApplication: {
          path: kind.path,
          args: kind.args?.join(" ") ?? null,
        },
      };
      break;
    case "open_url":
      action_type = { OpenUrl: { url: kind.url } };
      break;
    case "wait_for_port":
      action_type = {
        WaitForPort: {
          host: kind.host,
          port: kind.port,
          timeout_secs: step.timeout ?? 120,
        },
      };
      break;
    case "wait_for_url":
      action_type = { WaitForUrl: { url: kind.url, timeout_secs: step.timeout ?? 120 } };
      break;
    case "delay":
      action_type = { Delay: { seconds: kind.seconds } };
      break;
    case "open_terminal":
      action_type = {
        RunCommand: {
          command: kind.command || "",
          working_dir: step.working_directory ?? null,
          persistent: true,
        },
      };
      break;
    case "run_script":
      action_type = { ExecuteScript: { script: kind.script, shell: kind.shell ?? null } };
      break;
    default:
      action_type = { Delay: { seconds: 0 } };
  }
  return {
    id: step.id,
    label: step.label,
    enabled: step.enabled,
    action_type,
  };
}

/** Cascading delete with dependency consistency:
 *  1. The step itself is removed.
 *  2. Every step that (transitively) depends on it is removed too — a
 *     dependent without its prerequisite is an invalid configuration.
 *  3. `depends_on` of surviving steps is pruned of references to removed
 *     steps — no broken references are ever persisted.
 *  Returns the new step list and the ids that were removed. */
export function deleteStepCascade(
  steps: LaunchStep[],
  stepId: string,
): { steps: LaunchStep[]; removed: string[] } {
  const removed = new Set<string>([stepId]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const step of steps) {
      if (removed.has(step.id)) continue;
      if (step.depends_on.some((dep) => removed.has(dep))) {
        removed.add(step.id);
        changed = true;
      }
    }
  }
  const next = steps
    .filter((s) => !removed.has(s.id))
    .map((s) => ({ ...s, depends_on: s.depends_on.filter((dep) => !removed.has(dep)) }));
  return { steps: next, removed: [...removed] };
}