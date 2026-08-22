// ============================================================
// Toolchain — слой событий: дедупликация, идентичность, очистка
// ============================================================
// Гарантии слоя (контракт §4.6/§5.8):
//  - каждый слушатель возвращает функцию очистки; повторный вызов
//    очистки безопасен;
//  - ОДИН физический Tauri-listener на имя события, сколько бы
//    подписчиков ни было (ref-counting; монтирование/размонтирование
//    компонентов не плодит дубликаты);
//  - события чужих операций (stale) отбрасываются по идентичности
//    job_id/scan_id/session_id ДО вызова обработчика;
//  - терминальное событие доставляется ровно один раз
//    (createTerminalGuard), даже если бэкенд продублировал;
//  - каналы — модульные синглтоны и переживают навигацию маршрутов:
//    вернувшись на страницу, контроллер переподписывается и
//    восстанавливает состояние через getLatestScanJob()/getJob()
//    (reconnect), а не создаёт второй listener.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  JobEvent,
  JobStatus,
  ScanDoneEvent,
  ScanProgressEvent,
  ScanTerminal,
  ToolchainEvent,
  InstallPlan,
  CheckProgressEvent,
} from "./types";
import { isRecord, jobEventBelongsTo, jobEventKind, scanIsTerminal } from "./types";

/** Подключение к источнику событий. Возвращает отсоединитель. */
export type AttachFn<T> = (dispatch: (payload: T) => void) => () => void;

/** Дефолтное подключение — Tauri event API. Ошибка подключения
 * (нет Tauri-окружения, закрытое окно) не роняет подписчика. */
export function tauriAttach<T>(eventName: string): AttachFn<T> {
  return (dispatch) => {
    let off: UnlistenFn | null = null;
    let cancelled = false;
    listen<T>(eventName, (e) => dispatch(e.payload))
      .then((unlisten) => {
        if (cancelled) unlisten();
        else off = unlisten;
      })
      .catch((err) => {
        console.error(`[toolchain] не удалось подписаться на ${eventName}`, err);
      });
    return () => {
      cancelled = true;
      off?.();
    };
  };
}

let nextSubscriptionId = 1;

type Subscription<T> = {
  handler: (payload: T) => void;
  filter?: (payload: T) => boolean;
};

/**
 * Канал событий с общим подключением и ref-counting подписчиков.
 * Не зависит от Tauri: источник внедряется через AttachFn (тестируемо).
 */
export class EventChannel<T> {
  private subscriptions = new Map<number, Subscription<T>>();
  private detach: (() => void) | null = null;

  constructor(
    /** Имя события — для диагностики. */
    readonly eventName: string,
    private attach: AttachFn<T>,
  ) {}

  /**
   * Подписка. Возвращает функцию очистки (идемпотентную).
   * Обработчик не должен бросать исключения: они гасятся каналом.
   */
  subscribe(
    handler: (payload: T) => void,
    filter?: (payload: T) => boolean,
  ): () => void {
    const id = nextSubscriptionId++;
    this.subscriptions.set(id, { handler, filter });
    this.ensureAttached();
    let closed = false;
    return () => {
      if (closed) return;
      closed = true;
      this.unsubscribe(id);
    };
  }

  /** Число живых подписчиков (для тестов/диагностики). */
  get subscriberCount(): number {
    return this.subscriptions.size;
  }

  /** Принудительно отсоединить физический listener (осторожно). */
  forceDetach(): void {
    this.detach?.();
    this.detach = null;
  }

  private ensureAttached(): void {
    if (this.detach) return;
    this.detach = this.attach((payload) => this.dispatch(payload));
  }

  private unsubscribe(id: number): void {
    this.subscriptions.delete(id);
    if (this.subscriptions.size === 0) {
      // Последний подписчик ушёл — физический listener больше не нужен.
      this.detach?.();
      this.detach = null;
    }
  }

  private dispatch(payload: T): void {
    for (const sub of [...this.subscriptions.values()]) {
      if (sub.filter && !sub.filter(payload)) continue;
      try {
        sub.handler(payload);
      } catch (err) {
        console.error(`[toolchain] обработчик ${this.eventName} упал`, err);
      }
    }
  }
}

// ------------------------------------------------------------
// Модульные каналы (синглтоны — переживают навигацию маршрутов)
// ------------------------------------------------------------

export const toolchainChannels = {
  scanProgress: new EventChannel<ScanProgressEvent>(
    "toolchainx:scan_progress",
    tauriAttach("toolchainx:scan_progress"),
  ),
  scanDone: new EventChannel<ScanDoneEvent>(
    "toolchainx:scan_done",
    tauriAttach("toolchainx:scan_done"),
  ),
  jobEvent: new EventChannel<JobEvent>(
    "toolchainx:job_event",
    tauriAttach("toolchainx:job_event"),
  ),
  legacyTaskEvent: new EventChannel<ToolchainEvent>(
    "toolchain:task_event",
    tauriAttach("toolchain:task_event"),
  ),
  legacyInstallDone: new EventChannel<InstallPlan>(
    "toolchain:install_done",
    tauriAttach("toolchain:install_done"),
  ),
  legacyCheckProgress: new EventChannel<CheckProgressEvent>(
    "toolchain:check_progress",
    tauriAttach("toolchain:check_progress"),
  ),
} as const;

// ------------------------------------------------------------
// Идентичность и устаревшие события
// ------------------------------------------------------------

/** Фильтр прогресса скана по идентичности задания. */
export function scanProgressBelongsTo(
  event: ScanProgressEvent,
  jobId: string,
): boolean {
  return event.job_id === jobId;
}

/** Фильтр терминального события скана по идентичности задания. */
export function scanDoneBelongsTo(event: ScanDoneEvent, jobId: string): boolean {
  return event.job_id === jobId;
}

/**
 * Терминальные состояния скана (всё, кроме Running).
 * Единственная реализация — scanIsTerminal в types.ts; здесь она
 * переэкспортируется под историческим именем (вызывающие не меняются).
 */
export { scanIsTerminal as scanIsTerminalState } from "./types";

/**
 * Guard «терминальное событие — ровно один раз» на идентификатор.
 * Первый вызов для id возвращает true и запоминает его; повторы
 * (дубликат бэкенда, гонка событий со status-запросом) — false.
 */
export class TerminalGuard {
  private seen = new Set<string>();

  /** true — только при ПЕРВОМ терминальном сигнале для id. */
  firstTerminal(id: string): boolean {
    if (this.seen.has(id)) return false;
    this.seen.add(id);
    return true;
  }

  hasFired(id: string): boolean {
    return this.seen.has(id);
  }

  forget(id: string): void {
    this.seen.delete(id);
  }

  reset(): void {
    this.seen.clear();
  }
}

// ------------------------------------------------------------
// Скоупированные трекеры (удобная обёртка для компонентов)
// ------------------------------------------------------------

export type ScanTrackerHandlers = {
  onProgress?: (event: ScanProgressEvent) => void;
  onDone?: (event: ScanDoneEvent) => void;
};

/**
 * Подписка на ход КОНКРЕТНОГО скана: фильтрует чужие job_id,
 * доставляет терминал ровно один раз, возвращает очистку.
 * Безопасно вызывать многократно при монтировании/размонтировании.
 */
export function trackScan(jobId: string, handlers: ScanTrackerHandlers): () => void {
  const guard = new TerminalGuard();
  const offProgress =
    handlers.onProgress
      ? toolchainChannels.scanProgress.subscribe(handlers.onProgress, (e) =>
          scanProgressBelongsTo(e, jobId),
        )
      : null;
  const offDone = toolchainChannels.scanDone.subscribe(
    (event) => {
      if (!scanIsTerminal(event.terminal)) return;
      if (!guard.firstTerminal(`${event.job_id}:${event.terminal}`)) return;
      handlers.onDone?.(event);
    },
    (e) => scanDoneBelongsTo(e, jobId),
  );
  return () => {
    offDone();
    offProgress?.();
  };
}

export type JobFinishedInfo = { status: JobStatus; errors: string[] };

export type JobTrackerHandlers = {
  onEvent?: (event: JobEvent) => void;
  /** Вызывается ровно один раз на терминальный статус задания. */
  onFinished?: (info: JobFinishedInfo) => void;
};

/**
 * Подписка на ход КОНКРЕТНОГО задания движка: stale-события чужих
 * заданий отбрасываются, терминал доставляется ровно один раз.
 */
export function trackJob(jobId: string, handlers: JobTrackerHandlers): () => void {
  const guard = new TerminalGuard();
  return toolchainChannels.jobEvent.subscribe(
    (event) => {
      // Граница IPC: событие без читаемого payload не существует.
      const kind = jobEventKind(event);
      if (!kind) return;
      if (handlers.onEvent) handlers.onEvent(event);
      const payload: unknown = event.payload;
      if (isRecord(payload) && "job_finished" in payload) {
        const finished = payload.job_finished as
          | { status?: unknown; errors?: unknown }
          | undefined;
        const status = finished?.status;
        if (typeof status !== "string") return;
        if (!guard.firstTerminal(jobId)) return;
        handlers.onFinished?.({
          status: status as JobStatus,
          errors: Array.isArray(finished?.errors) ? (finished.errors as string[]) : [],
        });
      }
    },
    (e) => jobEventBelongsTo(e, jobId),
  );
}
