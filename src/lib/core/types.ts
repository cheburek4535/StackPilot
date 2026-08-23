export type PreferredApp = {
  name: string;
  path: string;
  args: string | null;
};

export type AiSettings = {
  enabled: boolean;
  provider: string;
  base_url: string;
  api_key: string;
  model: string;
  temperature: number;
  max_tokens: number;
  timeout_secs: number;
  system_prompt: string;
  page_context: boolean;
};

export type PersonalSettings = {
  name: string;
  username: string;
  email: string;
};

export type AppSettings = {
  vscode_path: string;
  browser_path: string;
  terminal: string;
  theme: string;
  language: string;
  auto_save_profiles: boolean;
  preferred_apps: PreferredApp[];

  auto_save: boolean;
  font_size: string;
  reduced_motion: boolean;
  show_interface_hints: boolean;
  accent_color: string;
  restore_last_route: boolean;
  confirm_before_reset: boolean;

  personal: PersonalSettings;
  ai: AiSettings;
};