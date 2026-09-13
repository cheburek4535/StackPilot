// Docker first-run authorization helpers — devlauncher frontend.
//
// Mirrors the backend's notion of a "docker step": a step can only succeed
// once Docker Desktop's first-run authorization (sign in / service agreement)
// is complete. These helpers let the UI detect docker runs and point the user
// at "open Docker Desktop and authorize" before/after a launch.

import type { LaunchProfileV2, LaunchStep } from "./types";
import { isDockerStep, profileHasDockerSteps } from "./types";

export { isDockerStep, profileHasDockerSteps };

/** True when the run's failed step is docker-involved (steps marker of the
 *  "authorize in Docker Desktop" failure) for a given profile. */
export function isDockerStepId(
  stepId: string,
  profile: LaunchProfileV2 | null | undefined,
): boolean {
  if (!profile) return false;
  const step = profile.steps.find((s) => s.id === stepId);
  return step ? isDockerStep(step) : false;
}

/** Ids of all enabled docker-involved steps in a profile. */
export function dockerStepIds(profile: LaunchProfileV2 | null | undefined): string[] {
  if (!profile) return [];
  return profile.steps
    .filter((s): s is LaunchStep => s.enabled && isDockerStep(s))
    .map((s) => s.id);
}