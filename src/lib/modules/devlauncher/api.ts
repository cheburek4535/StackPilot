import { invoke } from "@tauri-apps/api/core";
import type { LaunchProfile, LaunchAction, ActionStatus } from "./types";

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

export async function executeAction(action: LaunchAction): Promise<ActionStatus> {
  return invoke("execute_action", { action });
}

export async function analyzeProject(path: string): Promise<LaunchProfile> {
  return invoke("analyze_project", { path });
}
