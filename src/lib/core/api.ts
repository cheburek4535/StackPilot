import { invoke } from "@tauri-apps/api/core";
import type { AppSettings } from "./types";

export async function getSettings(): Promise<AppSettings> {
  return invoke("get_settings");
}

export async function updateSettings(settings: AppSettings): Promise<void> {
  return invoke("update_settings", { settings });
}

export async function resetSettings(): Promise<AppSettings> {
  return invoke("reset_settings");
}
