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
