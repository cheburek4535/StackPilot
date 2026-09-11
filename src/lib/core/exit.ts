import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/**
 * Exit flow: диалог «завершить процессы перед выходом» показывается В
 * приложении (а не нативным окном). Бэкенд перехватывает закрытие окна,
 * эмитит `sp:exit-request` со списком процессов StackPilot и ждёт ответа:
 *  - resolveExitRequest(true)  — тихо завершить процессы и выйти;
 *  - resolveExitRequest(false) — выйти, оставив процессы работать;
 *  - cancelExitRequest()       — остаться в приложении (окно продолжает жить).
 */

/** Группа процесса для иконки в диалоге. */
export type ExitProcessKind =
  | "docker"
  | "node"
  | "python"
  | "jvm"
  | "compiled"
  | "shell"
  | "other";

export interface ExitAskProcess {
  pid: number;
  /** Человекочитаемый заголовок (метка шага профиля или команда). */
  title: string;
  /** Краткая командная строка (обрезана, без многострочных скриптов). */
  command: string;
  kind: ExitProcessKind;
  source: string;
  started_at: string;
}

export interface ExitAskPayload {
  processes: ExitAskProcess[];
}

export function listenExitRequest(handler: (payload: ExitAskPayload) => void): Promise<() => void> {
  return listen<ExitAskPayload>("sp:exit-request", (event) => handler(event.payload));
}

export function resolveExitRequest(terminate: boolean): Promise<void> {
  return invoke("resolve_exit_request", { terminate });
}

export function cancelExitRequest(): Promise<void> {
  return invoke("cancel_exit_request");
}