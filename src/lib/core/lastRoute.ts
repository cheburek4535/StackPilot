/**
 * Last route — the most recent UI route the user visited, persisted to
 * localStorage so the app can restore context on restart. UI-local only:
 * a route string is never presented as backend state.
 *
 * Written by the shell on every navigation (see AppShell.svelte);
 * the store initializes from localStorage at module load.
 */

import { writable } from "svelte/store";
import { readLocal, writeLocal, removeLocal } from "./storage";

const KEY = "stackpilot:last-route:v1";
const VERSION = 1;

function readLastRoute(): string | null {
  const value = readLocal<unknown>(KEY, VERSION);
  return typeof value === "string" && value.startsWith("/") ? value : null;
}

export const lastRoute = writable<string | null>(readLastRoute());

/** Records a visited route (must be an absolute pathname). */
export function rememberRoute(pathname: string): void {
  if (!pathname || !pathname.startsWith("/")) return;
  lastRoute.set(pathname);
  writeLocal(KEY, pathname, VERSION);
}

/** Forgets the stored route. */
export function clearLastRoute(): void {
  lastRoute.set(null);
  removeLocal(KEY);
}
