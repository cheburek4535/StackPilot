import { invoke } from "@tauri-apps/api/core";
import type { LaunchProfile, LaunchAction, ActionStatus, PreferredIde } from "./types";

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