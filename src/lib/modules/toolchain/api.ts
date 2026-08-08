import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ProjectRequirements,
  EnvironmentCheck,
  EnvironmentInfo,
  ToolDefinition,
  InstallPlan,
  InstallSession,
  ToolchainEvent,
  ToolchainMetadata,
  HealthReport,
  CheckProgressEvent,
} from "./types";

export function pingToolchain(): Promise<string> {
  return invoke("ping_toolchain");
}

export function getToolDefinitions(): Promise<ToolDefinition[]> {
  return invoke("tc_get_tool_definitions");
}

export function getEnvironmentInfo(): Promise<EnvironmentInfo> {
  return invoke("tc_get_environment_info");
}

export function checkEnvironment(
  requirements: ProjectRequirements,
): Promise<EnvironmentCheck> {
  return invoke("tc_check_environment", { requirements });
}

export function buildInstallPlan(
  check: EnvironmentCheck,
  selectedToolIds?: string[],
): Promise<InstallPlan> {
  return invoke("tc_build_install_plan", { check, selectedToolIds });
}

export function runInstall(plan: InstallPlan): Promise<void> {
  return invoke("tc_run_install", { plan });
}

export function getInstallStatus(): Promise<InstallSession | null> {
  return invoke("tc_get_install_status");
}

export function abortInstall(): Promise<boolean> {
  return invoke("tc_abort_install");
}

export function getNewSecrets(): Promise<Record<string, string>> {
  return invoke("tc_take_new_secrets");
}

export function getToolchainMetadata(): Promise<ToolchainMetadata> {
  return invoke("tc_get_metadata");
}

export function getHealthReport(): Promise<HealthReport> {
  return invoke("tc_get_health_report");
}

export function listenToolchainEvents(
  handler: (event: ToolchainEvent) => void,
): Promise<UnlistenFn> {
  return listen<ToolchainEvent>("toolchain:task_event", (e) => handler(e.payload));
}

export function listenInstallDone(
  handler: (event: InstallPlan) => void,
): Promise<UnlistenFn> {
  return listen<InstallPlan>("toolchain:install_done", (e) => handler(e.payload));
}

export function listenCheckProgress(
  handler: (event: CheckProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<CheckProgressEvent>("toolchain:check_progress", (e) => handler(e.payload));
}
