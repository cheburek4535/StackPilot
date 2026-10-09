import { invoke } from "@tauri-apps/api/core";
import type {
  EnvironmentBinding,
  BindingDiagnostics,
  ResolvedOverlay,
  StandaloneExportResult,
  EnvironmentDiskUsage,
} from "./types";

export async function listEnvironments(): Promise<EnvironmentBinding[]> {
  return invoke<EnvironmentBinding[]>("pe_list_bindings");
}

export async function getEnvironment(bindingId: string): Promise<EnvironmentBinding> {
  return invoke<EnvironmentBinding>("pe_get_binding", { bindingId });
}

export async function saveEnvironment(binding: EnvironmentBinding): Promise<EnvironmentBinding> {
  return invoke<EnvironmentBinding>("pe_save_binding", { binding });
}

export async function deleteEnvironment(bindingId: string): Promise<void> {
  return invoke<void>("pe_delete_binding", { bindingId });
}

export async function createEnvironment(
  name?: string,
  projectPath?: string,
): Promise<EnvironmentBinding> {
  return invoke<EnvironmentBinding>("pe_create_binding", {
    name: name ?? null,
    projectPath: projectPath ?? null,
  });
}

export async function getOrCreateDefaultEnvironment(): Promise<EnvironmentBinding> {
  return invoke<EnvironmentBinding>("pe_get_or_create_default");
}

export async function findEnvironmentForProject(
  projectPath: string,
): Promise<EnvironmentBinding | null> {
  return invoke<EnvironmentBinding | null>("pe_find_binding_for_project", {
    projectPath,
  });
}

export async function bindProjectToEnvironment(
  bindingId: string,
  projectPath: string,
): Promise<EnvironmentBinding> {
  return invoke<EnvironmentBinding>("pe_bind_project", {
    bindingId,
    projectPath,
  });
}

export async function unbindProjectFromEnvironment(
  bindingId: string,
  projectPath: string,
): Promise<EnvironmentBinding> {
  return invoke<EnvironmentBinding>("pe_unbind_project", {
    bindingId,
    projectPath,
  });
}

export async function validateEnvironment(
  binding: EnvironmentBinding,
): Promise<BindingDiagnostics> {
  return invoke<BindingDiagnostics>("pe_validate_binding", { binding });
}

export async function resolveEnvironmentOverlay(
  bindingId: string,
): Promise<ResolvedOverlay> {
  return invoke<ResolvedOverlay>("pe_resolve_overlay", { bindingId });
}

export async function openEnvironmentTerminal(
  bindingId: string,
  projectPath?: string,
): Promise<void> {
  return invoke<void>("pe_open_terminal", {
    bindingId,
    projectPath: projectPath ?? null,
  });
}

export async function configureVsCodeEnvironment(
  bindingId: string,
  projectPath: string,
): Promise<void> {
  return invoke<void>("pe_configure_vscode_environment", {
    bindingId,
    projectPath,
  });
}

export async function exportStandaloneEnvironment(
  bindingId: string,
  targetDir?: string,
): Promise<StandaloneExportResult> {
  return invoke<StandaloneExportResult>("pe_export_standalone", {
    bindingId,
    targetDir: targetDir ?? null,
  });
}

export async function calculateDiskUsage(
  bindingId: string,
): Promise<EnvironmentDiskUsage> {
  return invoke<EnvironmentDiskUsage>("pe_calculate_disk_usage", { bindingId });
}

export async function cleanupSandbox(
  bindingId: string,
): Promise<EnvironmentDiskUsage> {
  return invoke<EnvironmentDiskUsage>("pe_cleanup_sandbox", { bindingId });
}


