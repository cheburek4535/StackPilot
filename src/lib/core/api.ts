import { invoke } from "@tauri-apps/api/core";
import { initTauriMock } from "./tauriMock";
import type { AppSettings } from "./types";

if (typeof window !== "undefined") {
  initTauriMock();
}

export async function getSettings(): Promise<AppSettings> {
  return invoke("get_settings");
}

export async function updateSettings(settings: AppSettings): Promise<AppSettings> {
  return invoke("update_settings", { settings });
}

export async function resetSettings(): Promise<AppSettings> {
  return invoke("reset_settings");
}

export async function checkPath(path: string): Promise<boolean> {
  return invoke("settings_check_path", { path });
}

export async function getAppDataDir(): Promise<string> {
  return invoke("get_app_data_dir");
}
