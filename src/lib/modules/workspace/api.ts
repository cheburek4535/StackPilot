import { invoke } from "@tauri-apps/api/core";
import type {
  TrackedProcess,
  ProcessStatus,
  ProcessLogs,
  ProjectContext,
  SessionInfo,
  FileEntry,
  FileContent,
} from "./types";

// ===== Process commands =====

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

// ===== Project context =====

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

export async function openProjectFromPath(path: string): Promise<ProjectContext> {
  return invoke("open_project_from_path", { path });
}

// ===== Session =====

export async function getSessionInfo(): Promise<SessionInfo | null> {
  return invoke("get_session_info");
}

// ===== File explorer =====

export async function listDirectory(path: string): Promise<FileEntry[]> {
  return invoke("list_directory", { path });
}

export async function readFile(path: string): Promise<FileContent> {
  return invoke("read_file", { path });
}

export async function writeFile(path: string, content: string): Promise<void> {
  return invoke("write_file", { path, content });
}

export async function openInVSCode(path: string): Promise<void> {
  return invoke("open_in_vscode", { path });
}
