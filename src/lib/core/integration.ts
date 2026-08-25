/**
 * Typed frontend integration layer.
 *
 * A single, typed seam between confirmed backend API calls and the UI-local
 * stores / event bus. It does NOT create backend state and does NOT mirror
 * backend state locally. It only:
 *   1. calls the existing confirmed API wrapper,
 *   2. on SUCCESS records a recent-project reference (task rule 3) and/or
 *      publishes a typed UI event (task rule 5),
 *   3. returns the backend-confirmed value untouched.
 *
 * If an API call throws, nothing is recorded and no event is emitted.
 *
 * Deliberately NOT provided here (backend limitations, contract D–G):
 * - `launchProfile` — does not exist anywhere in the codebase;
 * - any "running" flag for a project/process — only backend-confirmed
 *   `ProcessStatus` values are ever relayed;
 * - a local copy of `getCurrentProject()` / `listProfiles()` — the backend
 *   remains the single source of truth.
 */

import {
  openProjectFromPath,
  spawnProcess,
  killProcess,
  refreshProcess,
} from "$lib/modules/workspace/api";
import type {
  ProcessStatus,
  ProjectContext,
  TrackedProcess,
} from "$lib/modules/workspace/types";
import { saveProfile, buildProfileFromContext } from "$lib/modules/devlauncher/api";
import type { LaunchProfile } from "$lib/modules/devlauncher/types";
import type { WizardContext } from "$lib/modules/project_creator/types";
import { emitAppEvent } from "./events";
import { addRecentProject } from "./recent";

function now(): string {
  return new Date().toISOString();
}

/** Best-effort display name for a filesystem path (last path segment). */
export function projectNameFromPath(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : path;
}

/**
 * Opens a project via backend `openProjectFromPath`. On success records a
 * recent reference (source "open") and publishes `project-opened`. Returns
 * the backend-confirmed `ProjectContext`; the backend stays the source of
 * truth for the current Workspace.
 */
export async function openProject(path: string): Promise<ProjectContext> {
  const ctx = await openProjectFromPath(path);
  const at = now();
  addRecentProject({ path, name: projectNameFromPath(path), source: "open", at });
  emitAppEvent({
    type: "project-opened",
    projectPath: path,
    profileName: ctx.profile_name || null,
    at,
  });
  return ctx;
}

/**
 * Saves a profile via backend `saveProfile`. On success publishes
 * `profile-saved` and, when the profile carries a reliable `project_path`,
 * records a recent reference (source "profile").
 */
export async function saveProfileConfirmed(profile: LaunchProfile): Promise<void> {
  await saveProfile(profile);
  const at = now();
  emitAppEvent({
    type: "profile-saved",
    profileName: profile.name,
    projectPath: profile.project_path,
    at,
  });
  if (profile.project_path && profile.project_path.trim().length > 0) {
    addRecentProject({
      path: profile.project_path,
      name: profile.name || projectNameFromPath(profile.project_path),
      source: "profile",
      at,
    });
  }
}

/**
 * Records a successfully created project. Call ONLY after Project Creator
 * execution completed successfully (backend `start_project_execution` /
 * `project_creator:step_event` AllCompleted). Records a recent reference
 * (source "created") and publishes `project-created`.
 */
export function confirmProjectCreated(projectPath: string): void {
  const at = now();
  addRecentProject({
    path: projectPath,
    name: projectNameFromPath(projectPath),
    source: "created",
    at,
  });
  emitAppEvent({ type: "project-created", projectPath, at });
}

/**
 * Records an explicitly confirmed user opening (path verified by the user,
 * e.g. picked via the folder dialog). Records a recent reference
 * (source "confirmed") and publishes `project-opened`.
 */
export function confirmProjectOpened(projectPath: string, profileName: string | null): void {
  const at = now();
  addRecentProject({
    path: projectPath,
    name: projectNameFromPath(projectPath),
    source: "confirmed",
    at,
  });
  emitAppEvent({ type: "project-opened", projectPath, profileName, at });
}

/** Spawns a process via backend; on success publishes `process-state-changed`. */
export async function spawnProcessConfirmed(
  command: string,
  args: string[],
  label: string,
  workingDir?: string,
): Promise<TrackedProcess> {
  const proc = await spawnProcess(command, args, label, workingDir);
  emitAppEvent({
    type: "process-state-changed",
    processId: proc.id,
    status: proc.status,
    at: now(),
  });
  return proc;
}

/** Kills a process via backend; on success publishes `process-state-changed`. */
export async function killProcessConfirmed(id: string): Promise<void> {
  await killProcess(id);
  emitAppEvent({
    type: "process-state-changed",
    processId: id,
    status: "Killed",
    at: now(),
  });
}

/** Refreshes a process via backend; on success publishes `process-state-changed`. */
export async function refreshProcessConfirmed(id: string): Promise<ProcessStatus> {
  const status = await refreshProcess(id);
  emitAppEvent({
    type: "process-state-changed",
    processId: id,
    status,
    at: now(),
  });
  return status;
}

/**
 * Publishes `process-state-changed` from backend-confirmed data. The caller
 * must pass the status exactly as returned by the backend (e.g. the
 * `process-status` event payload) — the layer never invents process state,
 * so a process is only ever "running" when the backend confirms it.
 */
export function confirmProcessStateChanged(
  processId: string,
  status: ProcessStatus,
): void {
  emitAppEvent({ type: "process-state-changed", processId, status, at: now() });
}

/**
 * Publishes `toolchain-install-completed`. Call ONLY when the backend
 * confirms install completion (e.g. after handling `toolchain:install_done`
 * or after `getInstallStatus()` reports a finished session).
 */
export function confirmToolchainInstallCompleted(): void {
  emitAppEvent({ type: "toolchain-install-completed", at: now() });
}

/**
 * Builds a DevLauncher profile from WizardContext and saves it. This is the
 * seamless integration point: after Project Creator finishes execution, the
 * profile is ready in DevLauncher without filesystem analysis.
 * Returns the saved profile on success.
 */
export async function confirmProjectCreatedWithProfile(
  context: WizardContext,
): Promise<LaunchProfile> {
  const profile = await buildProfileFromContext(context);
  const at = now();
  emitAppEvent({
    type: "profile-saved",
    profileName: profile.name,
    projectPath: profile.project_path,
    at,
  });
  if (profile.project_path && profile.project_path.trim().length > 0) {
    addRecentProject({
      path: profile.project_path,
      name: profile.name || projectNameFromPath(profile.project_path),
      source: "created",
      at,
    });
  }
  return profile;
}
