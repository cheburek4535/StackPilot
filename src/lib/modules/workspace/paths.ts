/**
 * Frontend-only path helpers for the workspace file explorer.
 *
 * These manipulate path STRINGS only — the backend contract is untouched
 * (listDirectory/readFile/writeFile all take a plain path string, and the
 * Rust side accepts both `/` and `\` separators on Windows). They make the
 * UI behave correctly on Windows (drive-root breadcrumbs, parent navigation)
 * without any backend change.
 */

const SEP = /[\\/]/;
const WINDOWS_DRIVE = /^[A-Za-z]:$/;

/** Normalizes separators to forward slashes for display/splitting. */
export function normalizePath(path: string): string {
  return path.replace(/\\/g, "/");
}

/** Splits a path into segments, tolerating both separators. */
export function pathSegments(path: string): string[] {
  return normalizePath(path)
    .split(SEP)
    .filter((p) => p.length > 0);
}

/**
 * Breadcrumbs for a filesystem path. Each crumb carries a clickable path that
 * is a valid directory for listDirectory().
 *   "C:\Users\app" -> [{C:, "C:/"}, {Users, "C:/Users"}, {app, "C:/Users/app"}]
 *   "/home/app"    -> [{/, "/"}, {home, "/home"}, {app, "/home/app"}]
 *   "src/lib"      -> [{src, "src"}, {lib, "src/lib"}]
 */
export function toBreadcrumbs(
  path: string,
): { label: string; path: string }[] {
  const normalized = normalizePath(path);
  const crumbs: { label: string; path: string }[] = [];

  if (normalized.startsWith("/")) {
    crumbs.push({ label: "/", path: "/" });
  }

  let acc = normalized.startsWith("/") ? "/" : "";

  for (const part of pathSegments(normalized)) {
    // Windows drive root: `C:` -> `C:/` (a real directory).
    if (WINDOWS_DRIVE.test(part)) {
      const root = `${part}/`;
      crumbs.push({ label: part, path: root });
      acc = root;
      continue;
    }
    if (acc && !acc.endsWith("/")) acc += "/";
    acc = `${acc}${part}`;
    crumbs.push({ label: part, path: acc });
  }

  return crumbs;
}

/**
 * The parent directory of a path (best-effort, string based).
 * Returns null when there is no navigable parent.
 *   "C:/Users/app"  -> "C:/Users"
 *   "C:/Users"      -> "C:/"   (drive root)
 *   "C:/"           -> null
 *   "/home/app"     -> "/home"
 *   "/"             -> null
 */
export function parentPath(path: string): string | null {
  const normalized = normalizePath(path);
  if (WINDOWS_DRIVE.test(normalized)) return null;
  if (normalized === "/") return null;

  const trimmed = normalized.replace(/\/+$/, "");
  if (!trimmed) return null;

  const idx = trimmed.lastIndexOf("/");
  if (idx === -1) return null;

  const parent = trimmed.slice(0, idx) || "/";
  if (WINDOWS_DRIVE.test(parent)) return `${parent}/`;
  return parent;
}
