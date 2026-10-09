export type IsolationMode = "global" | "isolated";

export interface ToolOverride {
  executable_path?: string | null;
  version?: string | null;
  path_entries?: string[];
  env_vars?: Record<string, string>;
}

export interface BindingWarning {
  subject: string;
  message: string;
}

export interface BindingDiagnostics {
  warnings: BindingWarning[];
  host_compatible: boolean;
}

export interface EnvironmentBinding {
  schema_version: number;
  binding_id: string;
  name?: string | null;
  isolation_mode: IsolationMode;
  is_default: boolean;
  description?: string | null;
  icon?: string | null;
  color?: string | null;
  project_path?: string | null;
  bound_projects: string[];
  env_dir?: string | null;
  tool_overrides: Record<string, ToolOverride>;
  managed_path_entries: string[];
  env_vars: Record<string, string>;
  env_vars_remove: string[];
  preferred_ide?: string | null;
  preferred_ide_args?: string[] | null;
  created_at: string;
  updated_at: string;
}

export interface ResolvedOverlay {
  binding_id: string;
  path_prepend: string[];
  vars_set: Record<string, string>;
  vars_remove: string[];
  diagnostics: string[];
}

export interface StandaloneExportResult {
  target_dir: string;
  created_files: string[];
}

export interface EnvironmentDiskUsage {
  binding_id: string;
  size_bytes: number;
  size_display: string;
  path?: string | null;
}


