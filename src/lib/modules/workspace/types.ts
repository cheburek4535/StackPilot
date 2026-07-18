export type ProcessStatus =
  | "Running"
  | { Exited: number }
  | "Killed"
  | "Crashed";

export type TrackedProcess = {
  id: string;
  pid: number;
  label: string;
  status: ProcessStatus;
  started_at: string;
  duration_secs: number;
  restarts: number;
  last_error: string | null;
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
