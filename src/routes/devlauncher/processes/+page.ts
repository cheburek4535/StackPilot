// ============================================================
// /devlauncher/processes — process terminal moved into the
// Workspace Logs tab. Canonical route — /workspace/logs.
// The ?log=<process_id> deep link is preserved.
import { redirect } from "@sveltejs/kit";

export function load({ url }: { url: URL }): never {
  redirect(308, `/workspace/logs${url.search}`);
}