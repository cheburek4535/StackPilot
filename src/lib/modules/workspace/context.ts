/**
 * Workspace context — reactive UI cache for the backend current project.
 *
 * Source of truth is the backend `getCurrentProject()`; this store is fed
 * ONLY from that command (there is no other writer), so it never invents or
 * shadows backend state. The workspace shell layout loads it on mount; pages
 * and the shell header subscribe to the same value so they never disagree.
 *
 * It is deliberately NOT persisted anywhere (no localStorage) and carries no
 * process/project status beyond what the backend returns.
 */

import { writable } from "svelte/store";
import { getCurrentProject, clearCurrentProject } from "./api";
import type { ProjectContext } from "./types";

export type WorkspaceContextState = {
  /** Backend-confirmed current project (null = none open). */
  project: ProjectContext | null;
  loading: boolean;
  error: string;
};

const initialState: WorkspaceContextState = {
  project: null,
  loading: true,
  error: "",
};

export const workspaceContext = writable<WorkspaceContextState>(initialState);

/** Re-reads the backend current project (idempotent, cheap IPC). */
export async function reloadWorkspaceContext(): Promise<void> {
  workspaceContext.update((s) => ({ ...s, loading: true, error: "" }));
  try {
    const project = await getCurrentProject();
    workspaceContext.set({ project, loading: false, error: "" });
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    workspaceContext.set({ project: null, loading: false, error: message });
  }
}

/** Clears the backend current project, then re-syncs the context. */
export async function clearWorkspaceProject(): Promise<void> {
  try {
    await clearCurrentProject();
  } catch {
    // the reload below still reflects whatever the backend reports
  }
  await reloadWorkspaceContext();
}
