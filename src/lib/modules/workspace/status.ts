/**
 * Pure presentational helpers for workspace data.
 *
 * These only re-format values that the backend already confirmed
 * (`TrackedProcess.status/duration_secs/started_at`, `SessionInfo` fields,
 * `FileEntry.size`). They never invent counts or durations.
 */

import type { ProcessStatus } from "./types";
import { i18n } from "$lib/core/i18n.svelte";
import type { TranslationKey } from "$lib/core/i18n.svelte";

export type StatusTone =
  | "neutral"
  | "violet"
  | "cyan"
  | "blue"
  | "lime"
  | "amber"
  | "red";

function isExited(status: ProcessStatus): status is { exited: number } {
  return typeof status === "object" && "exited" in status;
}

function isExitedWithError(status: ProcessStatus): status is {
  exited_with_error: number;
} {
  return typeof status === "object" && "exited_with_error" in status;
}

export function statusLabel(status: ProcessStatus): string {
  if (status === "starting") return i18n.t("devl.status_starting") as TranslationKey;
  if (status === "running") return i18n.t("devl.status_running") as TranslationKey;
  if (status === "ready") return i18n.t("devl.status_ready") as TranslationKey;
  if (status === "killed") return i18n.t("devl.status_killed") as TranslationKey;
  if (status === "crashed") return i18n.t("devl.status_crashed") as TranslationKey;
  if (status === "timed_out") return i18n.t("devl.status_timed_out") as TranslationKey;
  if (status === "cancelled") return i18n.t("devl.status_cancelled") as TranslationKey;
  if (status === "external_launch_accepted")
    return i18n.t("devl.status_external") as TranslationKey;
  if (status === "unknown") return i18n.t("devl.status_unknown") as TranslationKey;
  if (isExited(status)) {
    return status.exited === 0
      ? (i18n.t("stat.success") as TranslationKey)
      : (i18n.t("stat.exited", { code: status.exited }) as TranslationKey);
  }
  if (isExitedWithError(status)) {
    return i18n.t("stat.exited", {
      code: status.exited_with_error,
    }) as TranslationKey;
  }
  return i18n.t("devl.status_unknown") as TranslationKey;
}

export function statusTone(status: ProcessStatus): StatusTone {
  if (status === "running" || status === "starting") return "lime";
  if (status === "ready") return "cyan";
  if (status === "killed") return "red";
  if (status === "crashed") return "red";
  if (status === "timed_out") return "amber";
  if (status === "cancelled") return "violet";
  if (isExited(status)) {
    return status.exited === 0 ? "blue" : "amber";
  }
  if (isExitedWithError(status)) return "amber";
  return "neutral";
}

export function statusIcon(
  status: ProcessStatus,
): "play" | "check" | "x" | "alert" {
  if (status === "running" || status === "starting") return "play";
  if (status === "ready") return "play";
  if (status === "killed") return "x";
  if (status === "crashed") return "alert";
  if (status === "timed_out") return "alert";
  if (isExited(status)) {
    return status.exited === 0 ? "check" : "alert";
  }
  if (isExitedWithError(status)) return "alert";
  return "x";
}

/** True when the process is in a failed/failed-exit state (derived from real status). */
export function isProcessFailed(status: ProcessStatus): boolean {
  if (status === "crashed") return true;
  if (status === "killed") return true;
  if (status === "timed_out") return true;
  if (isExited(status)) {
    return status.exited !== 0;
  }
  if (isExitedWithError(status)) return true;
  return false;
}

/** True when the process is still running (non-terminal, non-exited). */
export function isProcessRunning(status: ProcessStatus): boolean {
  return (
    status === "starting" ||
    status === "running" ||
    status === "ready" ||
    status === "external_launch_accepted"
  );
}

export function formatDuration(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) return "—";
  const s = Math.floor(secs);
  if (s < 60) return i18n.t("time.dur_secs", { n: s }) as TranslationKey;
  if (s < 3600)
    return i18n.t("time.dur_min", { m: Math.floor(s / 60), s: s % 60 }) as TranslationKey;
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return i18n.t("time.dur_hour", { h, m }) as TranslationKey;
}

/** "N s/m/h/d ago" from a backend epoch-seconds timestamp. */
export function formatStarted(timestamp: string): string {
  if (!timestamp) return "—";
  const ts = Number(timestamp);
  if (!Number.isFinite(ts) || ts <= 0) return "—";
  const secs = Math.floor(Date.now() / 1000 - ts);
  if (secs < 0) return i18n.t("time.just_now") as TranslationKey;
  if (secs < 60) return i18n.t("time.secs_ago", { n: secs }) as TranslationKey;
  if (secs < 3600)
    return i18n.t("time.mins_ago", { n: Math.floor(secs / 60) }) as TranslationKey;
  if (secs < 86400)
    return i18n.t("time.hours_ago", { n: Math.floor(secs / 3600) }) as TranslationKey;
  return i18n.t("time.days_ago", { n: Math.floor(secs / 86400) }) as TranslationKey;
}

/** Local date-time from a backend epoch-seconds value. */
export function formatDateTime(epochSeconds: string | number): string {
  const ts = Number(epochSeconds);
  if (!Number.isFinite(ts) || ts <= 0) return "—";
  return new Date(ts * 1000).toLocaleString();
}

export function formatFileSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes < 1024) return i18n.t("size.bytes", { n: bytes }) as TranslationKey;
  if (bytes < 1024 * 1024)
    return i18n.t("size.kb", { n: (bytes / 1024).toFixed(1) }) as TranslationKey;
  return i18n.t("size.mb", { n: (bytes / (1024 * 1024)).toFixed(1) }) as TranslationKey;
}