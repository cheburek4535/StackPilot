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
};
