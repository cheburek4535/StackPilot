/**
 * Shared presentation helpers for DevLauncher profile actions.
 *
 * Pure functions only — no state, no API calls. They describe an action's
 * union variant (`ActionType`) for display and format an `ActionStatus`
 * returned by `executeAction`.
 */

import type { ActionType, ActionStatus } from "./types";

export function actionVariant(act: ActionType): string {
  return Object.keys(act)[0];
}

export function actionIcon(act: ActionType): string {
  if ("RunCommand" in act) return "▶";
  if ("OpenUrl" in act) return "🌐";
  if ("OpenApplication" in act) return "⬛";
  if ("WaitForUrl" in act) return "⏳";
  if ("WaitForPort" in act) return "🔌";
  if ("Delay" in act) return "⏱";
  if ("ExecuteScript" in act) return "📜";
  return "?";
}

export function actionTypeLabel(act: ActionType): string {
  if ("RunCommand" in act) return "Command";
  if ("OpenUrl" in act) return "URL";
  if ("OpenApplication" in act) return "App";
  if ("WaitForUrl" in act) return "Wait URL";
  if ("WaitForPort" in act) return "Wait Port";
  if ("Delay" in act) return "Delay";
  if ("ExecuteScript" in act) return "Script";
  return "?";
}

export function actionSummary(act: ActionType): string {
  if ("RunCommand" in act) return act.RunCommand.command || "(empty)";
  if ("OpenUrl" in act) return act.OpenUrl.url || "(empty)";
  if ("OpenApplication" in act) return act.OpenApplication.path || "(empty)";
  if ("WaitForUrl" in act) return act.WaitForUrl.url || "(empty)";
  if ("WaitForPort" in act) return `${act.WaitForPort.host}:${act.WaitForPort.port}`;
  if ("Delay" in act) return `${act.Delay.seconds}s`;
  if ("ExecuteScript" in act) return act.ExecuteScript.script || "(empty)";
  return "?";
}

export function formatResult(r: ActionStatus): string {
  if ("Success" in r) return `✓ ${r.Success.message}`;
  if ("Failed" in r) return `✗ ${r.Failed.error}`;
  if ("Skipped" in r) return `— ${r.Skipped.reason}`;
  return "?";
}

export function resultClass(msg: string): string {
  if (msg.startsWith("✓")) return "ok";
  if (msg.startsWith("✗")) return "err";
  if (msg.startsWith("—")) return "skip";
  return "";
}
