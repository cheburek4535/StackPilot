// WSL readiness helpers — devlauncher frontend.
//
// Docker on Windows runs on the WSL2 backend. Before a docker launch the UI
// checks `devl_get_wsl_state`: if WSL is missing, the pre-flight offers an
// install dialog instead of blindly launching (Docker Desktop would just hang
// on WSL's own "press any key" install prompt). This module mirrors the
// backend event name for the install progress stream.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const WSL_INSTALL_PROGRESS_EVENT = "devlauncher:wsl-install-progress";

/** Subscribe to WSL install stage strings emitted by `devl_wsl_install`.
 *  Returns an unlisten function for teardown. */
export function subscribeWslInstallProgress(
  handler: (stage: string) => void,
): Promise<UnlistenFn> {
  return listen<string>(WSL_INSTALL_PROGRESS_EVENT, (event) => handler(event.payload));
}