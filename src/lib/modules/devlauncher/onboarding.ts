/**
 * DevLauncher first-run hints — UI-local flags persisted to localStorage
 * (`stackpilot:devlauncher:hints:v1`), mirroring the safety pattern of
 * `core/onboarding.ts`: every read/write is guarded and malformed values
 * are treated as "not set" without throwing.
 *
 * Two flags gate the "add your first step" mini-guide on the profile page:
 * - profileCreated — set once a profile was actually created (analyze page
 *   save, Project Creator auto-profile).
 * - profileOpened — set the first time a real (non-demo) profile page is
 *   opened.
 * The guide shows only when BOTH are set and it was not shown yet.
 */

const HINTS_STORAGE_KEY = "stackpilot:devlauncher:hints:v1";
const IN_MEMORY_KEY = "__stackpilot_devlauncher_hints_in_memory__";

type PersistedHints = {
  profileCreated: boolean;
  profileOpened: boolean;
  hintShown: boolean;
};

function isPersisted(value: unknown): value is PersistedHints {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.profileCreated === "boolean" &&
    typeof record.profileOpened === "boolean" &&
    typeof record.hintShown === "boolean"
  );
}

function safeReadStorage(): Storage | null {
  try {
    if (typeof window === "undefined") return null;
    const storage = window.localStorage;
    storage.getItem(HINTS_STORAGE_KEY);
    return storage;
  } catch {
    return null;
  }
}

function readPersisted(): PersistedHints | null {
  const storage = safeReadStorage();
  if (!storage) return null;
  try {
    const raw = storage.getItem(HINTS_STORAGE_KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    return isPersisted(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function writePersisted(update: Partial<PersistedHints>): void {
  const storage = safeReadStorage();
  const current = readPersisted() ?? {
    profileCreated: false,
    profileOpened: false,
    hintShown: false,
  };
  const next: PersistedHints = { ...current, ...update };
  if (!storage) return;
  try {
    storage.setItem(HINTS_STORAGE_KEY, JSON.stringify(next));
  } catch {
    // Storage unavailable — keep the flags in memory for this session only.
    try {
      window.sessionStorage.setItem(IN_MEMORY_KEY, JSON.stringify(next));
    } catch {
      /* ignore */
    }
  }
}

function readState(): PersistedHints {
  const persisted = readPersisted();
  if (persisted) return persisted;
  try {
    const mem = window.sessionStorage.getItem(IN_MEMORY_KEY);
    if (mem) {
      const parsed: unknown = JSON.parse(mem);
      if (isPersisted(parsed)) return parsed;
    }
  } catch {
    /* ignore */
  }
  return { profileCreated: false, profileOpened: false, hintShown: false };
}

/** Call after a profile was successfully created (analyze save or the
 *  Project Creator auto-profile). Idempotent. */
export function markProfileCreated(): void {
  const state = readState();
  if (state.profileCreated) return;
  writePersisted({ profileCreated: true });
}

/** Call when a real (non-demo) profile page is opened. Idempotent. */
export function markProfileOpened(): void {
  const state = readState();
  if (state.profileOpened) return;
  writePersisted({ profileOpened: true });
}

/** Whether the first-run step guide should appear on the profile page. */
export function shouldShowTerminalHint(): boolean {
  const state = readState();
  return state.profileCreated && state.profileOpened && !state.hintShown;
}

/** Marks the guide as seen (user added the example step or dismissed it). */
export function markHintShown(): void {
  writePersisted({ hintShown: true });
}