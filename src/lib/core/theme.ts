import { getSettings } from "$lib/core/api";

export type ResolvedTheme = "dark" | "light";
export type ThemePreference = "dark" | "light" | "system";

const DEFAULT_THEME: ThemePreference = "dark";

let currentPref: ThemePreference = DEFAULT_THEME;

const systemDark =
  typeof window !== "undefined"
    ? window.matchMedia("(prefers-color-scheme: dark)")
    : null;

export function resolveTheme(pref: ThemePreference): ResolvedTheme {
  if (pref === "dark" || pref === "light") return pref;
  return systemDark?.matches ? "dark" : "light";
}

export function applyTheme(pref: ThemePreference): void {
  if (typeof document === "undefined") return;
  currentPref = pref;
  document.documentElement.dataset.theme = resolveTheme(pref);
}

/**
 * Reads the theme preference from the backend (AppSettings.theme) and
 * applies it. Falls back to the default dark design when settings are
 * unavailable (e.g. running in a plain browser without Tauri).
 */
export async function initTheme(): Promise<void> {
  try {
    const settings = await getSettings();
    applyTheme(normalizePref(settings.theme));
  } catch {
    applyTheme(DEFAULT_THEME);
  }
  systemDark?.addEventListener("change", () => {
    if (currentPref === "system") applyTheme(currentPref);
  });
}

function normalizePref(value: string): ThemePreference {
  if (value === "light" || value === "system") return value;
  return "dark";
}