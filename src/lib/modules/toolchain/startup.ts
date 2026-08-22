// ============================================================
// Toolchain — стартовые/конкурентные примитивы (чистые, тестируемые)
// ============================================================
// Здесь живут ЧИСТЫЕ механизмы неблокирующего старта и защиты от гонок:
//  - CoalescingCall — in-flight guard: повторный вызов во время полёта
//    получает ТОТ ЖЕ promise (нет дублирующих IPC);
//  - OnceInitializer — «инициализация ровно один раз» (слушатели не
//    плодятся при повторном монтировании страницы);
//  - LatestRequestGuard — «применяется только ответ ПОСЛЕДНЕГО запроса»
//    (медленный ответ предыдущего выбранного инструмента не пишет
//    данные в drawer);
//  - ScanStartCoordinator — «reconnect раньше старта»: stale-результат
//    reconnect'а не может перезаписать свежезапущенный скан; старт
//    выполняется максимум один раз за поток вызовов.
//
// Функции без зависимостей от Svelte/Tauri — покрываются юнит-тестами.

import type { ScanJobSnapshot } from "./types";
import { scanIsTerminal } from "./types";

/** Скан активен, пока его терминальное состояние — Running
 *  (единая реализация с событиями: scanIsTerminal в types.ts). */
export function scanJobIsActive(job: ScanJobSnapshot): boolean {
  return !scanIsTerminal(job.terminal);
}

// ------------------------------------------------------------
// In-flight guard (коалесцирование запросов)
// ------------------------------------------------------------

export type CoalescingCallHooks<T> = {
  /** Вызывается при фактическом старте полёта (флаги загрузки). */
  onStart?: () => void;
  /** Вызывается при завершении полёта — ВСЕГДА (сброс спиннера). */
  onSettle?: () => void;
  /** Ошибка единственного реального вызова (санитизация — у вызывающего). */
  onError?: (err: unknown) => void;
};

/**
 * Коалесцирующий вызов: пока полёт не завершился, повторные call()
 * возвращают тот же promise — параллельных IPC не возникает.
 * Гарантии:
 *  - run() исполняется максимум один раз на полёт;
 *  - onSettle вызывается всегда (загрузочное состояние не застревает);
 *  - ошибка доходит до ВСЕХ ожидающих (повторный catch не нужен).
 */
export class CoalescingCall<T> {
  #promise: Promise<T> | null = null;

  constructor(
    private readonly run: () => Promise<T>,
    private readonly hooks: CoalescingCallHooks<T> = {},
  ) {}

  get busy(): boolean {
    return this.#promise !== null;
  }

  call(): Promise<T> {
    if (this.#promise) return this.#promise;
    this.hooks.onStart?.();
    const flight = this.run()
      .catch((err: unknown) => {
        this.hooks.onError?.(err);
        throw err;
      })
      .finally(() => {
        this.hooks.onSettle?.();
        this.#promise = null;
      });
    this.#promise = flight;
    return flight;
  }

  /** Принудительный сброс полёта (тесты/диагностика). */
  reset(): void {
    this.#promise = null;
  }
}

// ------------------------------------------------------------
// Инициализация ровно один раз
// ------------------------------------------------------------

/**
 * Guard «выполнить ровно один раз»: повторные run() — no-op.
 * Отражает правило ensureInitialized: слушатели каналов и стартовые
 * запросы подключаются один раз на время жизни приложения, повторное
 * монтирование страницы не плодит подписчиков.
 */
export class OnceInitializer {
  #done = false;

  /** true — функция выполнена этим вызовом; false — уже выполнялась. */
  run(fn: () => void): boolean {
    if (this.#done) return false;
    this.#done = true;
    fn();
    return true;
  }

  get initialized(): boolean {
    return this.#done;
  }

  reset(): void {
    this.#done = false;
  }
}

// ------------------------------------------------------------
// Применение ответа только последнего запроса
// ------------------------------------------------------------

/**
 * Guard детальных запросов drawer'а: начало нового запроса делает все
 * прежние ответы устаревшими. Медленный ответ ранее выбранного
 * инструмента не должен перезаписывать данные текущего.
 */
export class LatestRequestGuard {
  #latest: string | null = null;

  /** Новый запрос становится «последним». */
  begin(id: string): void {
    this.#latest = id;
  }

  /** Ответ принадлежит всё ещё актуальному запросу? */
  isLatest(id: string): boolean {
    return this.#latest === id;
  }

  get latest(): string | null {
    return this.#latest;
  }

  clear(): void {
    this.#latest = null;
  }
}

// ------------------------------------------------------------
// Координатор скана: reconnect → при необходимости старт
// ------------------------------------------------------------

/**
 * Гарантия «reconnect раньше старта»:
 *  - reconnect выполняется ровно один раз и ВСЕГДА до первого старта;
 *  - результат reconnect'а применяется ДО решения о старте, поэтому
 *    stale-терминальный скан не может перезаписать свежий запуск;
 *  - старт выполняется максимум один раз за полёт (повторные ensure()
 *    во время полёта получают тот же promise);
 *  - если после reconnect'а скан уже бежит — новый не стартует.
 */
export class ScanStartCoordinator {
  #reconnectPromise: Promise<void> | null = null;
  #inFlight: Promise<ScanJobSnapshot | null> | null = null;

  constructor(
    /** Текущее задание скана контроллера (может отсутствовать). */
    private readonly current: () => ScanJobSnapshot | null,
    /** Однократный reconnect: применяет актуальное задание бэкенда. */
    private readonly reconnect: () => Promise<void>,
    /** Старт нового скана. Вызывается только когда ничего не бежит. */
    private readonly startNew: () => Promise<ScanJobSnapshot | null>,
  ) {}

  get busy(): boolean {
    return this.#inFlight !== null;
  }

  get reconnectDone(): boolean {
    return this.#reconnectPromise !== null;
  }

  ensureRunning(): Promise<ScanJobSnapshot | null> {
    if (this.#inFlight) return this.#inFlight;
    if (!this.#reconnectPromise) {
      // Ошибка reconnect'а не блокирует старт: скан можно запустить и
      // без знания о прошлых заданиях.
      this.#reconnectPromise = this.reconnect().catch(() => {});
    }
    const flight = (async () => {
      await this.#reconnectPromise;
      const now = this.current();
      if (now && scanJobIsActive(now)) return now;
      return this.startNew();
    })().finally(() => {
      this.#inFlight = null;
    });
    this.#inFlight = flight;
    return flight;
  }
}