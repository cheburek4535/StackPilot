import { invoke } from "@tauri-apps/api/core";
import type { TrackedProcess, ProcessStatus } from "./types";

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
