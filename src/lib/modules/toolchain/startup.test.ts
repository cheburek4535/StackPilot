// Тесты стартовых/конкурентных примитивов: коалесцирование запросов,
// «инициализация ровно один раз», stale-ответы деталей, порядок
// reconnect → старт скана. Все сценарии гонок из performance-требований.
import { describe, expect, it, vi } from "vitest";
import {
  CoalescingCall,
  LatestRequestGuard,
  OnceInitializer,
  ScanStartCoordinator,
  scanJobIsActive,
} from "./startup";
import type { ScanJobSnapshot } from "./types";

function scanJob(overrides: Partial<ScanJobSnapshot> = {}): ScanJobSnapshot {
  return {
    job_id: "tcx-job-1",
    scan_id: "tcx-scan-1",
    started_at: "a",
    updated_at: "b",
    finished_at: null,
    phase: "Tools",
    total_tools: 3,
    completed_tools: 1,
    current_tool: "git",
    running: true,
    cancel_requested: false,
    terminal: "Running",
    recovered: false,
    ...overrides,
  };
}

// ------------------------------------------------------------
// CoalescingCall — in-flight guard
// ------------------------------------------------------------

describe("CoalescingCall — повторный вызов не дублирует полёт", () => {
  it("два вызова во время полёта исполняют run ровно один раз", async () => {
    const run = vi.fn(async () => 42);
    const call = new CoalescingCall(run);

    const a = call.call();
    const b = call.call();
    expect(call.busy).toBe(true);
    expect(a).toBe(b); // тот же promise
    await expect(a).resolves.toBe(42);
    await expect(b).resolves.toBe(42);
    expect(run).toHaveBeenCalledTimes(1);
    expect(call.busy).toBe(false);
  });

  it("после завершения полёта следующий вызов исполняет run снова", async () => {
    const run = vi.fn(async () => 1);
    const call = new CoalescingCall(run);
    await call.call();
    await call.call();
    expect(run).toHaveBeenCalledTimes(2);
  });

  it("onStart — один раз на полёт; onSettle — всегда (спиннер не застревает)", async () => {
    const onStart = vi.fn();
    const onSettle = vi.fn();
    const call = new CoalescingCall(async () => 1, { onStart, onSettle });

    const a = call.call();
    const b = call.call();
    expect(onStart).toHaveBeenCalledTimes(1);
    await Promise.all([a, b]);
    expect(onSettle).toHaveBeenCalledTimes(1);
  });

  it("ошибка доходит до ВСЕХ ожидающих и сбрасывает состояние полёта", async () => {
    const onError = vi.fn();
    const call = new CoalescingCall(
      async () => {
        throw new Error("boom");
      },
      { onError },
    );

    const a = call.call();
    const b = call.call();
    await expect(a).rejects.toThrow("boom");
    await expect(b).rejects.toThrow("boom");
    expect(onError).toHaveBeenCalledTimes(1); // один реальный провал
    expect(call.busy).toBe(false); // полёт завершился — состояние сброшено
  });

  it("reset() допускает новый полёт немедленно", async () => {
    const run = vi.fn(async () => 1);
    const call = new CoalescingCall(run);
    const inFlight = call.call();
    call.reset();
    expect(call.busy).toBe(false);
    await call.call();
    await inFlight;
    expect(run).toHaveBeenCalledTimes(2);
  });
});

// ------------------------------------------------------------
// OnceInitializer — инициализация ровно один раз
// ------------------------------------------------------------

describe("OnceInitializer — повторная инициализация не плодит подписчиков", () => {
  it("второй вызов run() — no-op", () => {
    const init = new OnceInitializer();
    const fn = vi.fn();
    expect(init.run(fn)).toBe(true);
    expect(init.run(fn)).toBe(false);
    expect(init.run(fn)).toBe(false);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("имитация ре-монтирования страницы: слушатели подключены один раз", () => {
    const init = new OnceInitializer();
    const listeners = vi.fn();
    const onMount = () => {
      init.run(listeners); // ensureInitialized
    };
    onMount();
    onMount(); // навигация → обратно на /toolchain
    onMount();
    expect(listeners).toHaveBeenCalledTimes(1);
    expect(init.initialized).toBe(true);
  });

  it("reset() разрешает повторную инициализацию (пересоздание контроллера)", () => {
    const init = new OnceInitializer();
    const fn = vi.fn();
    init.run(fn);
    init.reset();
    init.run(fn);
    expect(fn).toHaveBeenCalledTimes(2);
  });
});

// ------------------------------------------------------------
// LatestRequestGuard — применяется только последний запрос
// ------------------------------------------------------------

describe("LatestRequestGuard — выбранный инструмент сменился до ответа", () => {
  it("ответ прежнего инструмента устаревает после нового выбора", () => {
    const guard = new LatestRequestGuard();
    guard.begin("git"); // выбран git, запрос ушёл
    guard.begin("node"); // выбор сменился на node
    expect(guard.isLatest("git")).toBe(false); // медленный ответ git — мимо
    expect(guard.isLatest("node")).toBe(true);
  });

  it("ответ текущего инструмента применяется", () => {
    const guard = new LatestRequestGuard();
    guard.begin("git");
    expect(guard.isLatest("git")).toBe(true);
  });

  it("clear() аннулирует все ожидающие ответы", () => {
    const guard = new LatestRequestGuard();
    guard.begin("git");
    guard.clear();
    expect(guard.isLatest("git")).toBe(false);
    expect(guard.latest).toBeNull();
  });
});

// ------------------------------------------------------------
// ScanStartCoordinator — reconnect раньше старта, старт один раз
// ------------------------------------------------------------

describe("ScanStartCoordinator — гонок скана нет", () => {
  it("повторные ensureRunning: один reconnect и один старт", async () => {
    const reconnect = vi.fn(async () => {});
    const startNew = vi.fn(async () => scanJob({ job_id: "tcx-job-new" }));
    let current: ScanJobSnapshot | null = null;
    const coord = new ScanStartCoordinator(
      () => current,
      reconnect,
      startNew,
    );

    const a = coord.ensureRunning();
    const b = coord.ensureRunning();
    expect(a).toBe(b); // коалесцирование вызовов
    await a;
    expect(reconnect).toHaveBeenCalledTimes(1);
    expect(startNew).toHaveBeenCalledTimes(1);
  });

  it("reconnect обнаружил идущий скан — новый не стартует", async () => {
    const running = scanJob({ job_id: "tcx-job-active" });
    let current: ScanJobSnapshot | null = null;
    const reconnect = vi.fn(async () => {
      current = running; // reconnect применяет задание бэкенда
    });
    const startNew = vi.fn(async () => null);
    const coord = new ScanStartCoordinator(() => current, reconnect, startNew);

    const result = await coord.ensureRunning();
    expect(result?.job_id).toBe("tcx-job-active");
    expect(startNew).not.toHaveBeenCalled();
  });

  it("reconnect всегда завершается ДО старта (stale-терминал не может перезаписать свежий запуск)", async () => {
    let current: ScanJobSnapshot | null = scanJob({ terminal: "Completed", running: false });
    let reconnectResolved = false;
    const reconnect = vi.fn(async () => {
      await new Promise((r) => setTimeout(r, 10));
      reconnectResolved = true;
      // Терминальный «хвост» прежнего скана.
      current = scanJob({ job_id: "tcx-job-reconnect", terminal: "Completed", running: false });
    });
    const startNew = vi.fn(async () => {
      // Старт обязан увидеть применённый reconnect-результат.
      expect(reconnectResolved).toBe(true);
      current = scanJob({ job_id: "tcx-job-fresh" });
      return current;
    });
    const coord = new ScanStartCoordinator(() => current, reconnect, startNew);

    await coord.ensureRunning();
    expect(startNew).toHaveBeenCalledTimes(1);
    // Поздний терминальный reconnect-результат применён ДО старта:
    // финальное задание — свежий запуск, не «хвост».
    expect(current?.job_id).toBe("tcx-job-fresh");
  });

  it("reconnect-ошибка не блокирует старт", async () => {
    const reconnect = vi.fn(async () => {
      throw new Error("no backend");
    });
    const startNew = vi.fn(async () => scanJob({ job_id: "tcx-job-x" }));
    const coord = new ScanStartCoordinator(() => null, reconnect, startNew);

    const result = await coord.ensureRunning();
    expect(result?.job_id).toBe("tcx-job-x");
  });

  it("после reconnect'а текущий скан активен — старта нет, результат — текущий", async () => {
    let current: ScanJobSnapshot | null = null;
    const coord = new ScanStartCoordinator(
      () => current,
      async () => {
        current = scanJob({ job_id: "tcx-job-running" });
      },
      vi.fn(async () => null),
    );
    const first = await coord.ensureRunning();
    expect(first?.job_id).toBe("tcx-job-running");

    // Повторный вызов после завершения: скан всё ещё активен — старта нет.
    const again = await coord.ensureRunning();
    expect(again?.job_id).toBe("tcx-job-running");
  });
});

describe("scanJobIsActive", () => {
  it("активен только пока terminal === Running", () => {
    expect(scanJobIsActive(scanJob())).toBe(true);
    expect(scanJobIsActive(scanJob({ terminal: "Completed" }))).toBe(false);
    expect(scanJobIsActive(scanJob({ terminal: "Cancelled" }))).toBe(false);
    expect(scanJobIsActive(scanJob({ terminal: "Interrupted" }))).toBe(false);
  });
});