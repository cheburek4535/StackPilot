// Cached platform capabilities for the DevLauncher UI.
//
// The backend reports OS, shells, Docker status and the terminal emulator it
// opens for visible steps. Windows-only guidance (the WSL install dialog, the
// "Docker Desktop may ask you to sign in" note) must be gated on these facts
// instead of rendering on every platform.

import { getPlatformCapabilities } from "./api";
import type { PlatformCapabilities } from "./types";

/** In-flight/finished capability probe, shared by every caller. */
let pending: Promise<PlatformCapabilities> | null = null;

/** Load platform capabilities (cached for the session; a failure is not
 *  cached, so a transient IPC error can be retried). */
export function loadPlatformCapabilities(): Promise<PlatformCapabilities> {
  pending ??= getPlatformCapabilities().catch((e) => {
    pending = null;
    throw e;
  });
  return pending;
}

/** Drop the cached probe (used by tests). */
export function resetPlatformCapabilities(): void {
  pending = null;
}

/** Windows-only features (WSL dialogs) are gated on this. When the probe has
 *  not finished yet the answer is `false`: showing a "reboot your PC" dialog
 *  on Linux is worse than showing nothing. */
export function isWindowsHost(caps: PlatformCapabilities | null): boolean {
  return caps?.os === "windows";
}

/** macOS host check. `false` until the probe resolves. */
export function isMacosHost(caps: PlatformCapabilities | null): boolean {
  return caps?.os === "macos";
}

/** Whether Docker Desktop may show its own first-run prompts (sign-in,
 *  terms). Only meaningful where Docker Desktop is the container runtime. */
export function usesDockerDesktop(caps: PlatformCapabilities | null): boolean {
  return caps?.docker_desktop === true;
}
