/**
 * Local UI activity — lightweight, capped records of user/UI activity
 * (e.g. "view:workspace", "pref:theme-opened"). Used for UI-local concerns
 * like idle detection or "welcome back" heuristics.
 *
 * Storage semantics:
 * - Records are tiny (`{key, at}`); never logs, snapshots, or payloads.
 * - The list is capped (`ACTIVITY_LIMIT`) and deduplicated per key.
 * - `cleanupActivity` provides the local-activity cleanup: drop entries
 *   older than a cutoff.
 */

import { get, writable } from "svelte/store";
import { readLocal, writeLocal, removeLocal } from "./storage";

export type ActivityRecord = {
  key: string;
  at: string;
};

const KEY = "stackpilot:activity:v1";
const VERSION = 1;

/** Hard cap on how many activity records are kept. */
export const ACTIVITY_LIMIT = 50;

function isRecord(value: unknown): value is ActivityRecord {
  if (typeof value !== "object" || value === null) return false;
  const r = value as Record<string, unknown>;
  return typeof r.key === "string" && typeof r.at === "string";
}

function load(): ActivityRecord[] {
  const data = readLocal<unknown>(KEY, VERSION);
  if (!Array.isArray(data)) return [];
  return data.filter(isRecord).slice(0, ACTIVITY_LIMIT);
}

function persist(list: ActivityRecord[]): void {
  writeLocal(KEY, list.slice(0, ACTIVITY_LIMIT), VERSION);
}

const initial = load();

export const activity = writable<ActivityRecord[]>(initial);

/** Records an activity (deduped by key, bumped to the front, capped). */
export function recordActivity(key: string): void {
  const at = new Date().toISOString();
  const next = [
    { key, at },
    ...get(activity).filter((r) => r.key !== key),
  ].slice(0, ACTIVITY_LIMIT);
  activity.set(next);
  persist(next);
}

/** Clears all activity records. */
export function clearActivity(): void {
  activity.set([]);
  removeLocal(KEY);
}

/** Local activity cleanup — drops records older than `maxAgeMs`. */
export function cleanupActivity(maxAgeMs: number): ActivityRecord[] {
  const cutoff = Date.now() - maxAgeMs;
  const next = get(activity).filter(
    (r) => Number.isFinite(Date.parse(r.at)) && Date.parse(r.at) >= cutoff,
  );
  activity.set(next);
  persist(next);
  return next;
}
