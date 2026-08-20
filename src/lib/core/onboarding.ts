/**
 * Onboarding foundation — first-run detection, skip / complete,
 * and a reopen entry point. UI-local state persisted to localStorage
 * (`stackpilot:onboarding:v1`); never presented as backend-confirmed.
 *
 * Safety: every localStorage read/write is guarded; malformed values are
 * treated as "not seen" without throwing.
 */

import { writable } from "svelte/store";
import { ONBOARDING_STORAGE_KEY } from "./app";

export type OnboardingStatus = "done" | "skipped";

type PersistedOnboarding = {
  status: OnboardingStatus;
  at: string;
};

export type OnboardingState = {
  /** True while the overlay is visible (in-session). */
  open: boolean;
  /** True on the very first run (no persisted record). */
  firstRun: boolean;
  status: OnboardingStatus | null;
};

const IN_MEMORY_KEY = "__stackpilot_onboarding_in_memory__";

function isPersisted(value: unknown): value is PersistedOnboarding {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return (
    (record.status === "done" || record.status === "skipped") &&
    typeof record.at === "string"
  );
}

function safeReadStorage(): Storage | null {
  try {
    if (typeof window === "undefined") return null;
    const storage = window.localStorage;
    // Probe access — throws in some hardened webviews.
    storage.getItem(ONBOARDING_STORAGE_KEY);
    return storage;
  } catch {
    return null;
  }
}

function readPersisted(): PersistedOnboarding | null {
  const storage = safeReadStorage();
  if (!storage) return null;
  try {
    const raw = storage.getItem(ONBOARDING_STORAGE_KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    return isPersisted(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function writePersisted(status: OnboardingStatus): void {
  const storage = safeReadStorage();
  if (!storage) return;
  try {
    const value: PersistedOnboarding = { status, at: new Date().toISOString() };
    storage.setItem(ONBOARDING_STORAGE_KEY, JSON.stringify(value));
  } catch {
    // Storage unavailable — keep state in memory for this session only.
    try {
      window.sessionStorage.setItem(IN_MEMORY_KEY, status);
    } catch {
      /* ignore */
    }
  }
}

function initialStatus(): OnboardingStatus | null {
  const persisted = readPersisted();
  if (persisted) return persisted.status;
  try {
    const mem = window.sessionStorage.getItem(IN_MEMORY_KEY);
    if (mem === "done" || mem === "skipped") return mem;
  } catch {
    /* ignore */
  }
  return null;
}

const initial = initialStatus();

export const onboarding = writable<OnboardingState>({
  open: false,
  firstRun: initial === null,
  status: initial,
});

export function showOnboarding(): void {
  onboarding.update((state) => ({ ...state, open: true }));
}

export function hideOnboarding(): void {
  onboarding.update((state) => ({ ...state, open: false }));
}

export function completeOnboarding(): void {
  writePersisted("done");
  onboarding.set({ open: false, firstRun: false, status: "done" });
}

export function skipOnboarding(): void {
  writePersisted("skipped");
  onboarding.set({ open: false, firstRun: false, status: "skipped" });
}

/** Reopens onboarding after completion (reopen entry point). */
export function reopenOnboarding(): void {
  showOnboarding();
}