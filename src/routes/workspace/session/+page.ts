// ============================================================
// /workspace/session — merged into the Dashboard tab (/workspace).
import { redirect } from "@sveltejs/kit";

export function load(): never {
  redirect(308, "/workspace");
}