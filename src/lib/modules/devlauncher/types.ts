// ---------------------------------------------------------------------------
// Legacy types — kept for backward compatibility. DO NOT REMOVE.
// ---------------------------------------------------------------------------

export type RunCommand = {
  command: string;
  working_dir: string | null;
  /** When true, the command is long-running and opens in a native terminal. */
  persistent?: boolean | null;
};

export type OpenApplication = {
  path: string;
  args: string | null;
};

export type OpenUrl = {
  url: string;
};

export type WaitForUrl = {
  url: string;
  timeout_secs: number;
};

export type WaitForPort = {
  host: string;
  port: number;
  timeout_secs: number;
};

export type Delay = {
  seconds: number;
};

export type ExecuteScript = {
  script: string;
  shell: string | null;
};

export type ActionType =
  | { RunCommand: RunCommand }
  | { OpenApplication: OpenApplication }
  | { OpenUrl: OpenUrl }
  | { WaitForUrl: WaitForUrl }
  | { WaitForPort: WaitForPort }
  | { Delay: Delay }
  | { ExecuteScript: ExecuteScript };

export type LaunchAction = {
  id: string;
  label: string;
  enabled: boolean;
  action_type: ActionType;
};

export type PreferredIde =
  | "Vscode"
  | "Pycharm"
  | "Goland"
  | "Idea"
  | "Webstorm"
  | "Xcode"
  | "VisualStudio"
  | { Custom: string };

export type LaunchProfile = {
  name: string;
  description: string;
  project_path: string | null;
  actions: LaunchAction[];
  environment_binding_id?: string | null;
  preferred_ide?: PreferredIde | null;
  /** Present on V2 profiles returned by the backend. */
  schema_version?: string;
  /** Present on V2 profiles returned by the backend. */
  id?: string;
};

export type ActionStatus =
  | { Success: { message: string } }
  | { Failed: { error: string } }
  | { Skipped: { reason: string } };

export type FileChangeEvent = {
  path: string;
};

// ---------------------------------------------------------------------------
// V2 types — consumed by the orchestrator run-oriented lifecycle.
// These mirror the Rust models in devlauncher/models.rs.
// ---------------------------------------------------------------------------

export type StepKind =
  | { type: "run_command"; command: string }
  | { type: "run_script"; script: string; shell?: string | null }
  | { type: "open_application"; path: string; args?: string[] | null }
  | { type: "open_url"; url: string }
  | { type: "wait_for_port"; host: string; port: number; candidate_ports?: number[] }
  | { type: "wait_for_url"; url: string }
  | { type: "wait_for_docker" }
  | { type: "delay"; seconds: number }
  | { type: "open_terminal"; command: string }
  | { type: "open_folder"; path: string };

export type Visibility = "captured" | "visible_terminal" | "detached";
export type ExecutionMode = "one_shot" | "long_running";
export type FailurePolicy = "stop_run" | "skip_dependents" | "warn_and_continue";

export type CompletionPolicy =
  | { type: "exit_success" }
  | { type: "process_started" }
  | { type: "port_open"; host: string; port: number; timeout_secs: number }
  | { type: "url_ready"; url: string; timeout_secs: number }
  | { type: "delay_elapsed"; seconds: number }
  | { type: "external_launch_accepted" }
  | { type: "manual" }
  | { type: "docker_compose_up"; timeout_secs: number };

export type RetryPolicy = {
  max_retries: number;
  delay_ms?: number;
  backoff_multiplier?: number | null;
};

export type LaunchStep = {
  id: string;
  label: string;
  enabled: boolean;
  kind: StepKind;
  /** Optional in practice: the backend omits empty dependency lists from
   *  serialized profiles (serde `skip_serializing_if = "Vec::is_empty"`). */
  depends_on?: string[];
  working_directory?: string | null;
  environment?: Record<string, string> | null;
  visibility?: Visibility | null;
  execution_mode?: ExecutionMode | null;
  completion?: CompletionPolicy | null;
  failure_policy?: FailurePolicy | null;
  timeout?: number | null;
  retry_policy?: RetryPolicy | null;
  metadata?: Record<string, string> | null;
  /** Unknown fields preserved through round-trips. */
  extra?: Record<string, unknown>;
};

export type LaunchProfileV2 = {
  schema_version: string;
  id: string;
  name: string;
  description: string;
  project_root?: string | null;
  steps: LaunchStep[];
  environment_binding_id?: string | null;
  preferred_ide?: PreferredIde | null;
};

// ---------------------------------------------------------------------------
// Analysis draft types (backend `analyze_project_v2` output)
// ---------------------------------------------------------------------------

export type AnalysisConfidence = "high" | "medium" | "low";

export type AnalysisDiagnostic = {
  severity: DiagnosticSeverity;
  confidence: AnalysisConfidence;
  message: string;
  file?: string | null;
};

export type DraftProfile = {
  profile: LaunchProfileV2;
  diagnostics: AnalysisDiagnostic[];
};

/** Map a failure policy to a short human label. */
export function failurePolicyLabel(policy: FailurePolicy | null | undefined): string {
  switch (policy) {
    case "stop_run": return "Остановить запуск";
    case "skip_dependents": return "Пропустить зависимые";
    case "warn_and_continue": return "Продолжить";
    default: return "По умолчанию";
  }
}

/** Map a visibility mode to a short human label. */
export function visibilityLabel(v: Visibility | null | undefined): string {
  switch (v) {
    case "captured": return "В приложении";
    case "visible_terminal": return "Видимое окно";
    case "detached": return "Фоновый запуск";
    default: return "По умолчанию";
  }
}

/** Immutably apply a partial patch to a launch step. */
export function applyStepPatch(step: LaunchStep, patch: Partial<LaunchStep>): LaunchStep {
  return { ...step, ...patch };
}

// ---------------------------------------------------------------------------
// Run lifecycle types
// ---------------------------------------------------------------------------

export type RunStatus =
  | "pending"
  | "running"
  | "succeeded"
  | "failed"
  | "cancelled"
  | "partial_success";

export type StepStatus =
  | "pending"
  | "running"
  | "succeeded"
  | "failed"
  | "skipped"
  | "cancelled"
  | "retrying";

export type StepExecutionState = {
  step_id: string;
  status: StepStatus;
  process_id?: string | null;
  error?: string | null;
  retries_remaining?: number | null;
  started_at?: string | null;
  finished_at?: string | null;
};

export type LaunchRun = {
  run_id: string;
  profile_id: string;
  profile_name: string;
  status: RunStatus;
  steps: StepExecutionState[];
  created_at: string;
  finished_at?: string | null;
  cancelled: boolean;
  diagnostics: Diagnostic[];
};

// ---------------------------------------------------------------------------
// Process types (V2)
// ---------------------------------------------------------------------------

export type ProcessStatus =
  | "starting"
  | "running"
  | "ready"
  | { exited: number }
  | { exited_with_error: number }
  | "crashed"
  | "killed"
  | "timed_out"
  | "cancelled"
  | "external_launch_accepted"
  | "unknown";

export type ProcessTrackingQuality =
  | "exact"
  | "terminal_wrapper"
  | "approximate"
  | "detached";

export type ManagedProcess = {
  id: string;
  pid: number;
  label: string;
  status: ProcessStatus;
  started_at: string;
  duration_secs: number;
  restarts: number;
  last_error?: string | null;
  session_id?: string | null;
  visible: boolean;
  tracking_quality?: ProcessTrackingQuality | null;
};

// ---------------------------------------------------------------------------
// Diagnostic types
// ---------------------------------------------------------------------------

export type DiagnosticSeverity = "info" | "warning" | "error";
export type LogSource = "process" | "orchestrator" | "preflight" | "readiness" | "user";

export type Diagnostic = {
  run_id: string;
  step_id?: string | null;
  source: LogSource;
  severity: DiagnosticSeverity;
  message: string;
  timestamp: string;
};

// ---------------------------------------------------------------------------
// Log types
// ---------------------------------------------------------------------------

export type RunLogs = {
  run_id: string;
  stdout: string[];
  stderr: string[];
};

export type StepLogs = {
  run_id: string;
  step_id: string;
  process_id: string;
  stdout: string[];
  stderr: string[];
};

// ---------------------------------------------------------------------------
// Event payloads (from backend events)
// ---------------------------------------------------------------------------

export type RunStatusPayload = {
  run_id: string;
  status: RunStatus;
};

export type StepStatusPayload = {
  run_id: string;
  step: StepExecutionState;
};

export type ProcessStartedPayload = {
  run_id: string;
  step_id: string;
  process: ManagedProcess;
};

// ---------------------------------------------------------------------------
// V2 helpers
// ---------------------------------------------------------------------------

/** Check if a profile returned by the backend is a V2 profile. */
export function isV2Profile(profile: LaunchProfile): boolean {
  return profile.schema_version === "2";
}

/** Derive a StepKind display icon. */
export function stepKindIcon(kind: StepKind): string {
  switch (kind.type) {
    case "run_command": return "▶";
    case "run_script": return "📜";
    case "open_application": return "⬛";
    case "open_url": return "🌐";
    case "wait_for_port": return "🔌";
    case "wait_for_url": return "⏳";
    case "wait_for_docker": return "🐳";
    case "delay": return "⏱";
    case "open_terminal": return "🖥";
    case "open_folder": return "📁";
    default: return "?";
  }
}

/** Derive a StepKind display label. */
export function stepKindLabel(kind: StepKind): string {
  switch (kind.type) {
    case "run_command": return "Command";
    case "run_script": return "Script";
    case "open_application": return "Application";
    case "open_url": return "URL";
    case "wait_for_port": return "Wait for port";
    case "wait_for_url": return "Wait for URL";
    case "wait_for_docker": return "Wait for Docker";
    case "delay": return "Delay";
    case "open_terminal": return "Terminal";
    case "open_folder": return "Folder";
    default: return "Unknown";
  }
}

/** Derive a StepKind summary text. */
export function stepKindSummary(kind: StepKind): string {
  switch (kind.type) {
    case "run_command": return kind.command;
    case "run_script": return kind.script;
    case "open_application": return kind.path;
    case "open_url": return kind.url;
    case "wait_for_port": {
      const extras = kind.candidate_ports?.length
        ? ` (+${kind.candidate_ports.join(",")})`
        : "";
      return `${kind.host}:${kind.port}${extras}`;
    }
    case "wait_for_url": return kind.url;
    case "wait_for_docker": return "daemon";
    case "delay": return `${kind.seconds}s`;
    case "open_terminal": return kind.command || "plain terminal";
    case "open_folder": return kind.path;
    default: return "";
  }
}

/** Map StepStatus to a CSS class name. */
export function stepStatusClass(status: StepStatus): string {
  switch (status) {
    case "pending": return "step-pending";
    case "running": return "step-running";
    case "succeeded": return "step-succeeded";
    case "failed": return "step-failed";
    case "skipped": return "step-skipped";
    case "cancelled": return "step-cancelled";
    case "retrying": return "step-retrying";
    default: return "";
  }
}

/** Map RunStatus to a CSS class name. */
export function runStatusClass(status: RunStatus): string {
  switch (status) {
    case "pending": return "run-pending";
    case "running": return "run-running";
    case "succeeded": return "run-succeeded";
    case "failed": return "run-failed";
    case "cancelled": return "run-cancelled";
    case "partial_success": return "run-partial";
    default: return "";
  }
}

/** User-facing run status. The three terminal outcomes are unambiguous:
 * - `succeeded` — everything passed;
 * - `partial_success` — non-critical steps failed and were skipped, the
 *   run continued and services are up;
 * - `failed` — a critical step failed, the run was stopped and its
 *   processes terminated.
 */
export function runStatusLabel(status: RunStatus): string {
  switch (status) {
    case "pending": return "Ожидает запуска";
    case "running": return "Выполняется…";
    case "succeeded": return "Успешно";
    case "failed": return "Провален (остановлен)";
    case "cancelled": return "Отменён";
    case "partial_success": return "Запущен с ошибками";
    default: return status;
  }
}

/** Map ProcessTrackingQuality to a human-readable label. */
export function trackingQualityLabel(q: ProcessTrackingQuality | null | undefined): string {
  switch (q) {
    case "exact": return "Exact PID tracking";
    case "terminal_wrapper": return "Terminal wrapper (inner PID not tracked)";
    case "approximate": return "Approximate (process may have been replaced)";
    case "detached": return "Detached (no PID tracking)";
    default: return "Unknown tracking quality";
  }
}

/** Map DiagnosticSeverity to a CSS class name. */
export function diagnosticClass(severity: DiagnosticSeverity): string {
  switch (severity) {
    case "info": return "diag-info";
    case "warning": return "diag-warning";
    case "error": return "diag-error";
    default: return "";
  }
}

/** Check if a run is in a terminal state (completed or cancelled). */
export function isRunTerminal(status: RunStatus): boolean {
  return status === "succeeded" || status === "failed" || status === "cancelled" || status === "partial_success";
}

/** Check if a step is in a terminal state. */
export function isStepTerminal(status: StepStatus): boolean {
  return status === "succeeded" || status === "failed" || status === "skipped" || status === "cancelled";
}
