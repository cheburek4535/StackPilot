// ============================================================
// /devlauncher/profiles — profile triggers moved into the Workspace
// dashboard. Canonical route — /workspace (Dashboard tab).
import { redirect } from "@sveltejs/kit";

export function load(): never {
  redirect(308, "/workspace");
}