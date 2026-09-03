// ============================================================
// /devlauncher — the legacy DevLauncher Overview duplicated the
// Workspace dashboard. Canonical route — /workspace. Redirects
// keep restored routes and deep links working.
import { redirect } from "@sveltejs/kit";

export function load(): never {
  redirect(308, "/workspace");
}