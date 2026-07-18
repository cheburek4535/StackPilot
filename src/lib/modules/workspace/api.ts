import { invoke } from "@tauri-apps/api/core";
import type {
  TrackedProcess,
  ProcessStatus,
  ProcessLogs,
  ProjectContext,
  SessionInfo,
} from "./types";

export async function spawnProcess(
  command: string,
  args: string[],
  label: string,
  workingDir?: string,
): Promise<TrackedProcess> {
  return invoke("spawn_process", {
    command,
    args,
    label,
    workingDir: workingDir ?? null,
  });
}

export async function listProcesses(): Promise<TrackedProcess[]> {
  return invoke("list_processes");
}

export async function killProcess(id: string): Promise<void> {
  return invoke("kill_process", { id });
}

export async function refreshProcess(id: string): Promise<ProcessStatus> {
  return invoke("refresh_process", { id });
}

export async function getProcessLogs(id: string): Promise<ProcessLogs> {
  return invoke("get_process_logs", { id });
}

// --- Project context ---

export async function setCurrentProject(
  profileName: string,
  projectPath: string | null,
  description: string,
  stack: string[],
): Promise<ProjectContext> {
  return invoke("set_current_project", {
    profileName,
    projectPath,
    description,
    stack,
  });
}

export async function getCurrentProject(): Promise<ProjectContext | null> {
  return invoke("get_current_project");
}

export async function clearCurrentProject(): Promise<void> {
  return invoke("clear_current_project");
}

export async function getSessionInfo(): Promise<SessionInfo | null> {
  return invoke("get_session_info");
}
