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

export const UI_SCALES = [
  "auto",
  "100%",
  "110%",
  "120%",
  "125%",
  "135%",
  "150%",
  "175%",
  "200%",
] as const;

export type UiScale = (typeof UI_SCALES)[number];

export const UI_SCALE_FACTORS: Record<string, number> = {
  "100%": 1.0,
  "110%": 1.1,
  "120%": 1.2,
  "125%": 1.25,
  "135%": 1.35,
  "150%": 1.5,
  "175%": 1.75,
  "200%": 2.0,
};

/**
 * Resolves the numeric scale factor from setting.
 * When set to 'auto', inspects screen/window metrics to compensate for
 * Linux Wayland / GTK3 fractional scaling where devicePixelRatio defaults to 1.
 */
export function resolveScaleFactor(scalePref?: string): number {
  if (!scalePref || scalePref === "auto") {
    if (typeof window !== "undefined") {
      const dpr = window.devicePixelRatio || 1;
      // Если это HiDPI экран (например 4K или 2K), но системный dpr равен 1
      // (частая проблема под Linux Wayland при fractional scaling 125%/150%):
      if (dpr === 1 && typeof screen !== "undefined") {
        if (screen.width >= 3200 || screen.height >= 1800) {
          return 1.5;
        }
        if (screen.width >= 2200 || screen.height >= 1300) {
          return 1.25;
        }
      }
    }
    return 1.0;
  }
  return UI_SCALE_FACTORS[scalePref] ?? 1.0;
}

/**
 * Returns the next/previous scale level for keyboard shortcuts (Ctrl + / -).
 */
export function getNextZoomLevel(current: string, direction: "in" | "out"): string {
  const manualScales = ["100%", "110%", "120%", "125%", "135%", "150%", "175%", "200%"];
  const currentIdx = manualScales.indexOf(current);
  if (currentIdx === -1) {
    const factor = resolveScaleFactor(current);
    let closestIdx = 0;
    let minDiff = 999;
    manualScales.forEach((scale, i) => {
      const diff = Math.abs((UI_SCALE_FACTORS[scale] ?? 1.0) - factor);
      if (diff < minDiff) {
        minDiff = diff;
        closestIdx = i;
      }
    });
    if (direction === "in") {
      return manualScales[Math.min(manualScales.length - 1, closestIdx + 1)];
    } else {
      return manualScales[Math.max(0, closestIdx - 1)];
    }
  }
  if (direction === "in") {
    return manualScales[Math.min(manualScales.length - 1, currentIdx + 1)];
  } else {
    return manualScales[Math.max(0, currentIdx - 1)];
  }
}

/** Applies UI-level preferences that live in AppSettings. */
export function applyUiPrefs(settings: AppSettings): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  const basePx = parseFloat(FONT_SIZES[settings.font_size] ?? FONT_SIZES.md);
  const factor = resolveScaleFactor(settings.ui_scale);
  root.style.fontSize = `${basePx * factor}px`;
  root.style.setProperty("--sp-scale-factor", factor.toString());
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