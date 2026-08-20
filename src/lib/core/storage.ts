/**
 * Storage primitives for UI-local persistence.
 *
 * Values written here are UI-local ONLY: they are never presented as
 * backend-confirmed facts, and they never hold secrets, process logs,
 * execution snapshots, or large payloads.
 *
 * Safety guarantees:
 * - safe JSON parse: every read goes through `safeJsonParse` and never throws;
 * - schema version: every value is wrapped in `{ v, data }` so a future format
 *   change can migrate or drop stale data instead of corrupting it;
 * - graceful fallback: when localStorage is unavailable (plain browser,
 *   hardened webview, quota exceeded) reads return null and writes are
 *   silently dropped — callers must not throw.
 */

export const STORAGE_SCHEMA_VERSION = 1;

export type VersionedValue<T> = {
  v: number;
  data: T;
};

/** Safe JSON parse — never throws. Returns null for empty or malformed input. */
export function safeJsonParse<T>(raw: string | null): T | null {
  if (raw === null || raw === undefined) return null;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

/** Wraps data with the schema version so it can be round-tripped via `decodeVersioned`. */
export function encodeVersioned<T>(
  data: T,
  version: number = STORAGE_SCHEMA_VERSION,
): string {
  return JSON.stringify({ v: version, data });
}

/**
 * Decodes a versioned value. Returns null when the raw string is missing,
 * malformed, or carries a different schema version (stale → treated as absent).
 */
export function decodeVersioned<T>(
  raw: string | null,
  version: number = STORAGE_SCHEMA_VERSION,
): T | null {
  const parsed = safeJsonParse<VersionedValue<T>>(raw);
  if (!parsed || typeof parsed !== "object") return null;
  if (parsed.v !== version) return null;
  return parsed.data;
}

/** Returns a usable Storage handle, or null when localStorage is unavailable. */
export function getLocalStorage(): Storage | null {
  try {
    if (typeof window === "undefined") return null;
    const storage = window.localStorage;
    // Probe access — throws in some hardened webviews.
    storage.getItem("__stackpilot_probe__");
    return storage;
  } catch {
    return null;
  }
}

/** Reads a versioned value; null when unavailable / malformed / stale. */
export function readLocal<T>(
  key: string,
  version: number = STORAGE_SCHEMA_VERSION,
): T | null {
  const storage = getLocalStorage();
  if (!storage) return null;
  try {
    return decodeVersioned<T>(storage.getItem(key), version);
  } catch {
    return null;
  }
}

/** Writes a versioned value; returns false when storage is unavailable. */
export function writeLocal<T>(
  key: string,
  data: T,
  version: number = STORAGE_SCHEMA_VERSION,
): boolean {
  const storage = getLocalStorage();
  if (!storage) return false;
  try {
    storage.setItem(key, encodeVersioned(data, version));
    return true;
  } catch {
    return false;
  }
}

/** Removes a key; no-op when storage is unavailable. */
export function removeLocal(key: string): void {
  const storage = getLocalStorage();
  if (!storage) return;
  try {
    storage.removeItem(key);
  } catch {
    /* ignore */
  }
}
