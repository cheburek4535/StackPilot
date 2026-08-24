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
};

export type ActionStatus =
  | { Success: { message: string } }
  | { Failed: { error: string } }
  | { Skipped: { reason: string } };