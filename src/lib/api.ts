// ============================================================
// API-слой для вызова Rust-команд.
//
// Все вызовы `invoke()` собраны здесь, чтобы страницы
// (svelte-компоненты) не зависели от @tauri-apps/api/core
// напрямую.
//
// Каждая функция:
//   • принимает типизированные параметры
//   • вызывает Rust-команду через invoke()
//   • возвращает Promise с типизированным результатом
// ============================================================

import { invoke } from "@tauri-apps/api/core";
import type { LaunchProfile, LaunchAction, ActionStatus, AppSettings } from "./types";

// ===== Системные =====

/** Проверить связь с Rust-бэкендом */
export async function ping(): Promise<string> {
  return invoke("ping_rust");
}

/** Получить демо-профиль для ознакомления */
export async function getDemoProfile(): Promise<LaunchProfile> {
  return invoke("get_demo_profile");
}

// ===== ProfileManager =====

/** Получить список сохранённых профилей */
export async function listProfiles(): Promise<LaunchProfile[]> {
  return invoke("list_profiles");
}

/** Получить профиль по имени */
export async function getProfile(name: string): Promise<LaunchProfile> {
  return invoke("get_profile", { name });
}

/** Сохранить профиль (создать или перезаписать) */
export async function saveProfile(profile: LaunchProfile): Promise<void> {
  return invoke("save_profile", { profile });
}

/** Удалить профиль по имени */
export async function deleteProfile(name: string): Promise<void> {
  return invoke("delete_profile", { name });
}

// ===== LaunchEngine =====

/** Выполнить одно действие */
export async function executeAction(action: LaunchAction): Promise<ActionStatus> {
  return invoke("execute_action", { action });
}

// ===== Settings =====

/** Получить настройки приложения */
export async function getSettings(): Promise<AppSettings> {
  return invoke("get_settings");
}

/** Обновить настройки приложения */
export async function updateSettings(settings: AppSettings): Promise<void> {
  return invoke("update_settings", { settings });
}

/** Сбросить настройки на значения по умолчанию */
export async function resetSettings(): Promise<AppSettings> {
  return invoke("reset_settings");
}

// ===== Analyzer =====

/** Проанализировать папку проекта и получить предложенный профиль */
export async function analyzeProject(path: string): Promise<LaunchProfile> {
  return invoke("analyze_project", { path });
}
