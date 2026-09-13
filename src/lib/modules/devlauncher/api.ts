import { invoke } from "@tauri-apps/api/core";
import type {
  LaunchProfile,
  LaunchProfileV2,
  LaunchAction,
  ActionStatus,
  PreferredIde,
  LaunchRun,
  RunLogs,
  StepLogs,
  DraftProfile,
  DockerAuthState,
  WslState,
} from "./types";
import type { WizardContext } from "$lib/modules/project_creator/types";

export async function ping(): Promise<string> {
  return invoke("ping_rust");
}

export async function getDemoProfile(): Promise<LaunchProfile> {
  return invoke("get_demo_profile");
}

export async function listProfiles(): Promise<LaunchProfile[]> {
  return invoke("list_profiles");
}

export async function getProfile(name: string): Promise<LaunchProfile> {
  return invoke("get_profile", { name });
}

export async function saveProfile(profile: LaunchProfile): Promise<void> {
  return invoke("save_profile", { profile });
}

export async function deleteProfile(name: string): Promise<void> {
  return invoke("delete_profile", { name });
}

export async function executeAction(
  action: LaunchAction,
  opts?: { sessionId?: string; environmentBindingId?: string },
): Promise<ActionStatus> {
  return invoke("execute_action", {
    action,
    sessionId: opts?.sessionId ?? null,
    environmentBindingId: opts?.environmentBindingId ?? null,
  });
}

export async function analyzeProject(path: string): Promise<LaunchProfile> {
  return invoke("analyze_project", { path });
}

/** Analyze a project into a V2 draft profile with per-inference diagnostics. */
export async function analyzeProjectV2(path: string): Promise<DraftProfile> {
  return invoke("analyze_project_v2", { path });
}

/** Launch the preferred IDE for a project. Returns true if launched. */
export async function launchIde(
  ide: PreferredIde,
  projectPath?: string,
): Promise<boolean> {
  return invoke("launch_ide", { ide, projectPath: projectPath ?? null });
}

/** Run the entire profile: launch IDE + execute all enabled actions. */
export async function runProfile(
  profile: LaunchProfile,
  opts?: { sessionId?: string; environmentBindingId?: string },
): Promise<Array<[string, ActionStatus]>> {
  return invoke("run_profile", {
    profile,
    sessionId: opts?.sessionId ?? null,
    environmentBindingId: opts?.environmentBindingId ?? null,
  });
}

/** Builds a LaunchProfile from WizardContext and saves it — no filesystem analysis. */
export async function buildProfileFromContext(
  context: WizardContext,
): Promise<LaunchProfile> {
  return invoke("build_profile_from_context", { context });
}

/** Start watching a project directory for source file changes. */
export async function startFileWatcher(path: string): Promise<void> {
  return invoke("start_file_watcher", { path });
}

/** Stop watching the current project directory. */
export async function stopFileWatcher(): Promise<void> {
  return invoke("stop_file_watcher");
}

/** Check if file watcher is active. */
export async function isFileWatching(): Promise<boolean> {
  return invoke("is_file_watching");
}

// ---------------------------------------------------------------------------
// V2 Orchestrator API
// ---------------------------------------------------------------------------

/** Create a new run from a V2 profile. The run starts in Pending status. */
export async function createRun(profile: LaunchProfileV2): Promise<LaunchRun> {
  return invoke("create_run", { profile });
}

/** Start async execution of a Pending run. */
export async function startRun(runId: string): Promise<void> {
  return invoke("start_run", { runId });
}

/** Cancel a running or pending run. Idempotent after terminal state. */
export async function cancelRun(runId: string): Promise<void> {
  return invoke("cancel_run", { runId });
}

/** Get the current state of a run by ID. */
export async function getRun(runId: string): Promise<LaunchRun> {
  return invoke("get_run", { runId });
}

/** List all active (Pending or Running) runs. */
export async function listActiveRuns(): Promise<LaunchRun[]> {
  return invoke("list_active_runs");
}

/** Kill all running processes associated with a run. */
export async function stopRunProcesses(runId: string): Promise<string[]> {
  return invoke("stop_run_processes", { runId });
}

/** Get combined logs for all processes in a run. */
export async function getRunLogs(runId: string): Promise<RunLogs> {
  return invoke("get_run_logs", { runId });
}

/** Get logs for a specific step's process within a run. */
export async function getStepLogs(runId: string, stepId: string): Promise<StepLogs> {
  return invoke("get_step_logs", { runId, stepId });
}

/** List all runs including completed ones (history/audit). */
export async function listAllRuns(): Promise<LaunchRun[]> {
  return invoke("list_all_runs");
}

// ---------------------------------------------------------------------------
// V2 Profile persistence API
// ---------------------------------------------------------------------------

/** List all V2 profiles. */
export async function listProfilesV2(): Promise<LaunchProfileV2[]> {
  return invoke("list_profiles_v2");
}

/** Get a V2 profile by name. */
export async function getProfileV2(name: string): Promise<LaunchProfileV2> {
  return invoke("get_profile_v2", { name });
}

/** Save a V2 profile. */
export async function saveProfileV2(profile: LaunchProfileV2): Promise<void> {
  return invoke("save_profile_v2", { profile });
}

/** Delete a V2 profile by name. */
export async function deleteProfileV2(name: string): Promise<void> {
  return invoke("delete_profile_v2", { name });
}

/** Build a V2 profile from wizard context. */
export async function buildProfileV2FromContext(
  context: WizardContext,
): Promise<LaunchProfile> {
  return invoke("build_profile_v2_from_context", { context });
}

// ---------------------------------------------------------------------------
// V2 Docker first-run authorization state
// ---------------------------------------------------------------------------

/**
 * Persistent Docker authorization state:
 * - `confirmed` — a docker-involved step has reached Succeeded at least once
 *   (proves Docker Desktop's first-run sign-in/service agreement was done).
 * - `installed_via_stackpilot` — Docker was installed by the toolchain
 *   installer (not adopted externally).
 */
export async function getDockerAuthState(): Promise<DockerAuthState> {
  return invoke("devl_get_docker_auth_state");
}

/** Explicitly set the Docker authorization flag (advanced/reset use). */
export async function setDockerAuthConfirmed(confirmed: boolean): Promise<void> {
  return invoke("devl_set_docker_auth_confirmed", { confirmed });
}

// ---------------------------------------------------------------------------
// WSL readiness state + install (backend `devl_get_wsl_state` / `devl_wsl_install`)
// ---------------------------------------------------------------------------

/** Session-cached WSL state. `present=false` means Docker (WSL2 backend)
 *  cannot run until WSL is installed and the PC rebooted. */
export async function getWslState(): Promise<WslState> {
  return invoke("devl_get_wsl_state");
}

/** Install WSL from scratch + pin default version to 2. Long-running; the
 *  backend streams stages on `devlauncher:wsl-install-progress` (see the
 *  `subscribeWslInstallProgress` helper). Resolves to the reboot message. */
export async function installWsl(): Promise<string> {
  return invoke("devl_wsl_install");
}
