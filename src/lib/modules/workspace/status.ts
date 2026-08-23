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

export function statusLabel(status: ProcessStatus): string {
  if (status === "Running") return i18n.t("stat.running") as TranslationKey;
  if (status === "Killed") return i18n.t("stat.killed") as TranslationKey;
  if (status === "Crashed") return i18n.t("stat.crashed") as TranslationKey;
  if (typeof status === "object" && "Exited" in status) {
    return status.Exited === 0
      ? (i18n.t("stat.success") as TranslationKey)
      : (i18n.t("stat.exited", { code: status.Exited }) as TranslationKey);
  }
  return i18n.t("stat.unknown") as TranslationKey;
}

export function statusTone(status: ProcessStatus): StatusTone {
  if (status === "Running") return "lime";
  if (status === "Killed") return "red";
  if (status === "Crashed") return "red";
  if (typeof status === "object" && "Exited" in status) {
    return status.Exited === 0 ? "blue" : "amber";
  }
  return "neutral";
}

export function statusIcon(
  status: ProcessStatus,
): "play" | "check" | "x" | "alert" {
  if (status === "Running") return "play";
  if (status === "Killed") return "x";
  if (status === "Crashed") return "alert";
  if (typeof status === "object" && "Exited" in status) {
    return status.Exited === 0 ? "check" : "alert";
  }
  return "x";
}

/** True when the process is in a failed/failed-exit state (derived from real status). */
export function isProcessFailed(status: ProcessStatus): boolean {
  if (status === "Crashed") return true;
  if (status === "Killed") return true;
  if (typeof status === "object" && "Exited" in status) {
    return status.Exited !== 0;
  }
  return false;
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
