/**
 * Global notification foundation (toasts).
 *
 * The shell owns a single notification surface (ToastRegion renders
 * `toasts`); domain pages feed it through `pushToast`. No domain logic
 * lives here — this is UI-local state only.
 */

import { writable } from "svelte/store";

export type ToastKind = "info" | "success" | "warning" | "error";

export type Toast = {
  id: number;
  kind: ToastKind;
  title: string;
  message?: string;
  /** 0 = sticky (dismissed manually). */
  durationMs: number;
};

export type ToastInput = {
  kind?: ToastKind;
  title: string;
  message?: string;
  durationMs?: number;
};

let nextId = 1;

export const toasts = writable<Toast[]>([]);

export function pushToast(input: ToastInput): number {
  const toast: Toast = {
    id: nextId++,
    kind: input.kind ?? "info",
    title: input.title,
    message: input.message,
    durationMs: input.durationMs ?? (input.kind === "error" ? 8000 : 5000),
  };
  toasts.update((list) => [...list, toast]);
  return toast.id;
}

export function dismissToast(id: number): void {
  toasts.update((list) => list.filter((t) => t.id !== id));
}

export function clearToasts(): void {
  toasts.set([]);
}

export function notifyInfo(title: string, message?: string): number {
  return pushToast({ kind: "info", title, message });
}

export function notifySuccess(title: string, message?: string): number {
  return pushToast({ kind: "success", title, message });
}

export function notifyWarning(title: string, message?: string): number {
  return pushToast({ kind: "warning", title, message });
}

export function notifyError(title: string, message?: string): number {
  return pushToast({ kind: "error", title, message });
}