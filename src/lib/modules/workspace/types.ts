// ProcessStatus matches the backend serde representation (snake_case,
// externally tagged struct variants).
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

export type TrackedProcess = {
  id: string;
  pid: number;
  label: string;
  status: ProcessStatus;
  started_at: string;
  duration_secs: number;
  restarts: number;
  last_error: string | null;
  session_id: string | null;
  /** True if the process runs in its own native terminal window. */
  visible?: boolean;
  /** The orchestrator run this process belongs to. */
  run_id?: string | null;
  /** The orchestrator step that spawned this process. */
  step_id?: string | null;
  /** The full command line this process was launched with. */
  command?: string | null;
  /** The working directory the process was launched in. */
  working_dir?: string | null;
  /** Tracking quality: exact, terminal_wrapper, approximate, or detached. */
  tracking_quality?: string | null;
};

export type ProcessLogs = {
  stdout_lines: string[];
  stderr_lines: string[];
};

export type ProcessOutputEvent = {
  process_id: string;
  stream: string;
  line: string;
};

export type ProcessStatusEvent = {
  process_id: string;
  status: ProcessStatus;
  error: string | null;
};

export type ProjectContext = {
  profile_name: string;
  project_path: string | null;
  description: string;
  stack: string[];
  opened_at: string;
};

export type SessionInfo = {
  started_at: string;
  duration_secs: number;
  process_count: number;
  error_count: number;
};

export type FileEntry = {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
};

export type FileContent = {
  content: string;
  language: string;
};
