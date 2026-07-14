// ============================================================
// Типы данных DevLauncher (TypeScript)
//
// Эти типы должны совпадать с Rust-структурами из models.rs.
// Tauri автоматически преобразует JSON, поэтому структура
// данных на фронтенде и бэкенде должна быть одинаковой.
//
// Rust-перечисление (enum) в JSON выглядит так:
//   {"VariantName": {field1: value1, field2: value2}}
// TypeScript-аналог — discriminated union (объединение типов).
// ============================================================

// ---- ActionType ----

export type RunCommand = {
  command: string;
  working_dir: string | null;
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

// Объединение всех вариантов ActionType.
// Внимание: ключ объекта — это название варианта из Rust.
export type ActionType =
  | { RunCommand: RunCommand }
  | { OpenApplication: OpenApplication }
  | { OpenUrl: OpenUrl }
  | { WaitForUrl: WaitForUrl }
  | { WaitForPort: WaitForPort }
  | { Delay: Delay }
  | { ExecuteScript: ExecuteScript };

// ---- LaunchAction ----

export type LaunchAction = {
  id: string;
  label: string;
  enabled: boolean;
  action_type: ActionType;
};

// ---- LaunchProfile ----

export type LaunchProfile = {
  name: string;
  description: string;
  project_path: string | null;
  actions: LaunchAction[];
};

// ---- ActionStatus ----

export type ActionStatus =
  | { Success: { message: string } }
  | { Failed: { error: string } }
  | { Skipped: { reason: string } };

// ---- Settings ----

export type PreferredApp = {
  name: string;
  path: string;
  args: string | null;
};

export type AppSettings = {
  vscode_path: string;
  browser_path: string;
  terminal: string;
  theme: string;
  language: string;
  auto_save_profiles: boolean;
  preferred_apps: PreferredApp[];
};
