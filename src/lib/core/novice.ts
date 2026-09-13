/**
 * Глобальный флаг «пользователь уже не новичок».
 *
 * Приложение должно знать в каждом модуле, общаемся ли мы с тем, кто ещё
 * ни разу не долистал конструктор Project Creator до конца. Флаг сохраняется
 * в localStorage и переживает перезапуск: после первого «просмотра» всех
 * секций страницы конструктора подсказки-блокировки больше не показываются.
 *
 * Реализован как обычный svelte/store writable (по образцу onboarding.ts) —
 * без runes, чтобы модуль безопасно импортировался из любой точки (SSR/CSR).
 */

import { writable } from "svelte/store";

const STORAGE_KEY = "stackpilot:user:experienced";

function safeReadStorage(): Storage | null {
  try {
    if (typeof window === "undefined") return null;
    // Probe access — throws in some hardened webviews.
    window.localStorage.getItem(STORAGE_KEY);
    return window.localStorage;
  } catch {
    return null;
  }
}

function load(): boolean {
  const storage = safeReadStorage();
  if (!storage) return false;
  try {
    return storage.getItem(STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

/** Глобальный флаг «юзер уже видел конструктор целиком / не новичок». */
export const userExperienced = writable<boolean>(load());

/** Отметить пользователя как опытного (вызывается после первого
 *  долистывания конструктора до конца из любого модуля). */
export function markExperienced(): void {
  userExperienced.set(true);
  const storage = safeReadStorage();
  if (!storage) return;
  try {
    storage.setItem(STORAGE_KEY, "1");
  } catch {
    /* ignore quota/private-mode errors */
  }
}

/** Сбросить флаг (для тестов / ручного переключения в dev-инструментах). */
export function resetExperienced(): void {
  userExperienced.set(false);
  const storage = safeReadStorage();
  if (!storage) return;
  try {
    storage.removeItem(STORAGE_KEY);
  } catch {
    /* ignore */
  }
}