/**
 * Recent project references — UI-local history of project paths the user
 * CONFIRMED (opened / created / saved). Persisted to localStorage as a small,
 * capped, deduplicated list of lightweight references.
 *
 * Storage semantics (contract C / task rules):
 * - A reference is written ONLY by the integration layer
 *   (`src/lib/core/integration.ts`) after a confirmed successful action:
 *   `openProjectFromPath`, successful Project Creator completion, reliable
 *   profile save, or an explicitly confirmed user opening. It is never
 *   inferred from UI intent alone.
 * - A reference is `{path, name, source, at}` — NOT a backend mirror. No
 *   project content, no "running" flag, no snapshots.
 * - The backend (`getCurrentProject`, `listProfiles`) remains the single
 *   source of truth for the current Workspace and saved profiles; this store
 *   never replaces either.
 */

import { get, writable } from "svelte/store";
import { readLocal, writeLocal, removeLocal } from "./storage";

export type RecentSource = "open" | "created" | "profile" | "confirmed";

export type RecentProjectRef = {
  path: string;
  name: string;
  source: RecentSource;
  at: string;
};

const KEY = "stackpilot:recent:v1";
const VERSION = 1;

/** Hard cap on how many recent references are kept (rule: recent limit). */
export const RECENT_PROJECTS_LIMIT = 20;

function isRef(value: unknown): value is RecentProjectRef {
  if (typeof value !== "object" || value === null) return false;
  const r = value as Record<string, unknown>;
  return (
    typeof r.path === "string" &&
    typeof r.name === "string" &&
    (r.source === "open" ||
      r.source === "created" ||
      r.source === "profile" ||
      r.source === "confirmed") &&
    typeof r.at === "string"
  );
}

function load(): RecentProjectRef[] {
  const data = readLocal<unknown>(KEY, VERSION);
  if (!Array.isArray(data)) return [];
  return data.filter(isRef).slice(0, RECENT_PROJECTS_LIMIT);
}

function persist(list: RecentProjectRef[]): void {
  writeLocal(KEY, list.slice(0, RECENT_PROJECTS_LIMIT), VERSION);
}

const initial = load();

export const recentProjects = writable<RecentProjectRef[]>(initial);

/** Adds a reference at the front (deduped by path), capped to the limit. */
export function addRecentProject(ref: RecentProjectRef): RecentProjectRef[] {
  const next = [
    ref,
    ...get(recentProjects).filter((r) => r.path !== ref.path),
  ].slice(0, RECENT_PROJECTS_LIMIT);
  recentProjects.set(next);
  persist(next);
  return next;
}

/** Removes a single reference by path. */
export function removeRecentProject(path: string): RecentProjectRef[] {
  const next = get(recentProjects).filter((r) => r.path !== path);
  recentProjects.set(next);
  persist(next);
  return next;
}

/** Clears the whole recent history. */
export function clearRecentProjects(): void {
  recentProjects.set([]);
  removeLocal(KEY);
}

/**
 * Local activity cleanup — drops references older than `maxAgeMs`.
 * Keeps only entries newer than the cutoff and re-persists the result.
 */
export function cleanupRecentProjects(maxAgeMs: number): RecentProjectRef[] {
  const cutoff = Date.now() - maxAgeMs;
  const next = get(recentProjects).filter(
    (r) => Number.isFinite(Date.parse(r.at)) && Date.parse(r.at) >= cutoff,
  );
  recentProjects.set(next);
  persist(next);
  return next;
}
