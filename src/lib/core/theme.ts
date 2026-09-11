import { getSettings } from "$lib/core/api";
import type { AppSettings } from "$lib/core/types";

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

// ===== Accent color =====

export const ACCENT_PRESETS: Record<string, string> = {
  violet: "#8b5cf6",
  blue: "#3b82f6",
  green: "#22c55e",
  orange: "#ff8a1f",
  red: "#ef4444",
  cyan: "#06b6d4",
};

const ACCENT_DEFAULT = ACCENT_PRESETS.orange;

function clamp(n: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, n));
}

function parseHex(value: string): { r: number; g: number; b: number } | null {
  const hex = value.trim().replace(/^#/, "");
  if (!/^[0-9a-fA-F]{6}$/.test(hex)) return null;
  const n = parseInt(hex, 16);
  return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255 };
}

function rgb(r: number, g: number, b: number, a?: number): string {
  return a === undefined ? `rgb(${r},${g},${b})` : `rgba(${r},${g},${b},${a})`;
}

/**
 * Applies an accent color app-wide. Accepts a preset key ("violet",
 * "green", ...) or a "#rrggbb" hex value. Only the accent tokens are
 * overridden — the rest of the design system is untouched.
 */
export function applyAccentColor(value: string): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  const raw = ACCENT_PRESETS[value] ?? value;
  const parsed = parseHex(raw);
  if (!parsed) {
    const fallback = parseHex(ACCENT_DEFAULT)!;
    root.style.setProperty("--sp-accent", rgb(fallback.r, fallback.g, fallback.b));
    root.style.setProperty(
      "--sp-accent-strong",
      rgb(fallback.r, fallback.g, fallback.b),
    );
    root.style.setProperty(
      "--sp-accent-soft",
      rgb(fallback.r, fallback.g, fallback.b, 0.09),
    );
    root.style.setProperty(
      "--sp-accent-border",
      rgb(fallback.r, fallback.g, fallback.b, 0.22),
    );
    root.style.setProperty(
      "--sp-accent-glow",
      rgb(fallback.r, fallback.g, fallback.b, 0.35),
    );
    root.style.setProperty(
      "--sp-shadow-accent",
      `0 0 0 1px rgb(${fallback.r},${fallback.g},${fallback.b},0.3), 0 4px 24px rgb(${fallback.r},${fallback.g},${fallback.b},0.22)`,
    );
    root.style.setProperty(
      "--sp-focus-ring",
      `0 0 0 2px var(--sp-bg-0), 0 0 0 4px rgb(${fallback.r},${fallback.g},${fallback.b})`,
    );
    return;
  }
  const strong = {
    r: Math.round(parsed.r * 0.82),
    g: Math.round(parsed.g * 0.82),
    b: Math.round(parsed.b * 0.82),
  };
  root.style.setProperty("--sp-accent", rgb(parsed.r, parsed.g, parsed.b));
  root.style.setProperty("--sp-accent-strong", rgb(strong.r, strong.g, strong.b));
  root.style.setProperty("--sp-accent-soft", rgb(parsed.r, parsed.g, parsed.b, 0.09));
  root.style.setProperty(
    "--sp-accent-border",
    rgb(parsed.r, parsed.g, parsed.b, 0.22),
  );
  root.style.setProperty(
    "--sp-accent-glow",
    rgb(parsed.r, parsed.g, parsed.b, 0.35),
  );
  root.style.setProperty(
    "--sp-shadow-accent",
    `0 0 0 1px rgb(${parsed.r},${parsed.g},${parsed.b},0.3), 0 4px 24px rgb(${parsed.r},${parsed.g},${parsed.b},0.22)`,
  );
  root.style.setProperty(
    "--sp-focus-ring",
    `0 0 0 2px var(--sp-bg-0), 0 0 0 4px rgb(${parsed.r},${parsed.g},${parsed.b})`,
  );
}

// ===== UI preferences =====

const FONT_SIZES: Record<string, string> = { sm: "15px", md: "16px", lg: "17px" };

/** Applies UI-level preferences that live in AppSettings. */
export function applyUiPrefs(settings: AppSettings): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  root.style.fontSize = FONT_SIZES[settings.font_size] ?? FONT_SIZES.md;
  root.classList.toggle("sp-reduced-motion", settings.reduced_motion);
  root.classList.toggle("sp-hints-off", !settings.show_interface_hints);
  applyAccentColor(settings.accent_color);
}

/**
 * Reads the theme preference from the backend (AppSettings.theme) and
 * applies it together with accent color and UI prefs. Falls back to the
 * default dark design when settings are unavailable (e.g. running in a
 * plain browser without Tauri).
 */
export async function initTheme(): Promise<void> {
  try {
    const settings = await getSettings();
    applyTheme(normalizePref(settings.theme));
    applyUiPrefs(settings);
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