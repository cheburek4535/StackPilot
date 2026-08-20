/**
 * Theme / local preferences store.
 *
 * IMPORTANT split of responsibility (contract C):
 * - Theme and language are BACKEND-owned (`AppSettings.theme` /
 *   `AppSettings.language`) and are read via `getSettings()` — they are
 *   applied by `src/lib/core/theme.ts` and MUST NOT be duplicated in
 *   localStorage here.
 * - This store holds only UI-local preferences the backend does not own
 *   (sidebar collapse, content width, hint visibility, ...).
 */

import { get, writable } from "svelte/store";
import { readLocal, writeLocal, removeLocal } from "./storage";

export type ContentWidth = "narrow" | "default" | "wide";

export type LocalPreferences = {
  sidebarCollapsed: boolean;
  contentWidth: ContentWidth;
  showHints: boolean;
};

const KEY = "stackpilot:prefs:v1";
const VERSION = 1;

const DEFAULT_PREFS: LocalPreferences = {
  sidebarCollapsed: false,
  contentWidth: "default",
  showHints: true,
};

function isPrefs(value: unknown): value is LocalPreferences {
  if (typeof value !== "object" || value === null) return false;
  const p = value as Record<string, unknown>;
  return (
    typeof p.sidebarCollapsed === "boolean" &&
    (p.contentWidth === "narrow" ||
      p.contentWidth === "default" ||
      p.contentWidth === "wide") &&
    typeof p.showHints === "boolean"
  );
}

function load(): LocalPreferences {
  const data = readLocal<unknown>(KEY, VERSION);
  return isPrefs(data) ? { ...DEFAULT_PREFS, ...data } : { ...DEFAULT_PREFS };
}

export const preferences = writable<LocalPreferences>(load());

/** Applies a partial patch and persists it. Returns the new value. */
export function updatePreferences(
  patch: Partial<LocalPreferences>,
): LocalPreferences {
  const next = { ...get(preferences), ...patch };
  preferences.set(next);
  writeLocal(KEY, next, VERSION);
  return next;
}

/** Sets a single preference key. */
export function setPreference<K extends keyof LocalPreferences>(
  key: K,
  value: LocalPreferences[K],
): void {
  updatePreferences({ [key]: value } as Partial<LocalPreferences>);
}

/** Restores defaults and clears the stored value. */
export function resetPreferences(): void {
  preferences.set({ ...DEFAULT_PREFS });
  removeLocal(KEY);
}
