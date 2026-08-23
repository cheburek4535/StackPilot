/**
 * Shared presentation helpers for DevLauncher profile actions.
 *
 * Pure functions only — no state, no API calls. They describe an action's
 * union variant (`ActionType`) for display and format an `ActionStatus`
 * returned by `executeAction`.
 */

import type { ActionType, ActionStatus } from "./types";
import { i18n } from "$lib/core/i18n.svelte";
import type { TranslationKey } from "$lib/core/i18n.svelte";

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
  if ("RunCommand" in act) return i18n.t("act.command") as TranslationKey;
  if ("OpenUrl" in act) return i18n.t("act.url") as TranslationKey;
  if ("OpenApplication" in act) return i18n.t("act.app") as TranslationKey;
  if ("WaitForUrl" in act) return i18n.t("act.wait_url") as TranslationKey;
  if ("WaitForPort" in act) return i18n.t("act.wait_port") as TranslationKey;
  if ("Delay" in act) return i18n.t("act.delay") as TranslationKey;
  if ("ExecuteScript" in act) return i18n.t("act.script") as TranslationKey;
  return i18n.t("act.unknown") as TranslationKey;
}

export function actionSummary(act: ActionType): string {
  if ("RunCommand" in act) return act.RunCommand.command || (i18n.t("act.empty") as TranslationKey);
  if ("OpenUrl" in act) return act.OpenUrl.url || (i18n.t("act.empty") as TranslationKey);
  if ("OpenApplication" in act) return act.OpenApplication.path || (i18n.t("act.empty") as TranslationKey);
  if ("WaitForUrl" in act) return act.WaitForUrl.url || (i18n.t("act.empty") as TranslationKey);
  if ("WaitForPort" in act) return `${act.WaitForPort.host}:${act.WaitForPort.port}`;
  if ("Delay" in act) return `${act.Delay.seconds}s`;
  if ("ExecuteScript" in act) return act.ExecuteScript.script || (i18n.t("act.empty") as TranslationKey);
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
