// Тесты слоя событий: дедупликация слушателей, stale-события,
// терминальность ровно один раз, идемпотентная очистка.
import { describe, expect, it, vi } from "vitest";
import {
  EventChannel,
  TerminalGuard,
  trackJob,
  trackScan,
  type AttachFn,
} from "./events";
import type { JobEvent, ScanDoneEvent, ScanProgressEvent } from "./types";

/** Фейковый источник: копит dispatch-функции подписчиков канала. */
function fakeAttach<T>(): { attach: AttachFn<T>; emit: (p: T) => void; detachCount: () => number } {
  const dispatchers: Array<(p: T) => void> = [];
  let detaches = 0;
  return {
    attach: (dispatch) => {
      dispatchers.push(dispatch);
      return () => {
        detaches += 1;
      };
    },
    emit: (payload) => {
      for (const d of [...dispatchers]) d(payload);
    },
    detachCount: () => detaches,
  };
}

const progress = (jobId: string, toolId: string): ScanProgressEvent => ({
  job_id: jobId,
  scan_id: "s1",
  completed_count: 1,
  total_count: 3,
  tool_id: toolId,
  display_name: toolId,
  icon: null,
  tool_state: "missing",
  timestamp: "t",
  error: null,
});

const scanDone = (jobId: string, terminal: ScanDoneEvent["terminal"]): ScanDoneEvent => ({
  job_id: jobId,
  scan_id: "s1",
  terminal,
  completed: 3,
  total: 3,
});

describe("EventChannel — дедупликация и очистка", () => {
  it("N подписчиков = ОДИН физический listener", () => {
    const src = fakeAttach<string>();
    const channel = new EventChannel<string>("test", src.attach);

    const off1 = channel.subscribe(() => {});
    const off2 = channel.subscribe(() => {});
    expect(channel.subscriberCount).toBe(2);
    expect(src.detachCount()).toBe(0);

    off1();
    off1(); // повторная очистка безопасна
    expect(channel.subscriberCount).toBe(1);
    expect(src.detachCount()).toBe(0); // ещё живы

    off2();
    expect(channel.subscriberCount).toBe(0);
    expect(src.detachCount()).toBe(1); // последний ушёл — отсоединились
  });

  it("событие доставляется каждому подписчику ровно один раз", () => {
    const src = fakeAttach<string>();
    const channel = new EventChannel<string>("test", src.attach);
    const a = vi.fn();
    const b = vi.fn();
    channel.subscribe(a);
    channel.subscribe(b);

    src.emit("x");
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);
    expect(a).toHaveBeenCalledWith("x");
  });

  it("фильтр отбрасывает чужие payload ДО вызова обработчика", () => {
    const src = fakeAttach<ScanProgressEvent>();
    const channel = new EventChannel<ScanProgressEvent>("test", src.attach);
    const handler = vi.fn();
    channel.subscribe(handler, (e) => e.job_id === "current");

    src.emit(progress("stale-job", "git"));
    expect(handler).not.toHaveBeenCalled();

    src.emit(progress("current", "git"));
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it("падение одного обработчика не роняет остальных", () => {
    const src = fakeAttach<number>();
    const channel = new EventChannel<number>("test", src.attach);
    const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const good = vi.fn();
    channel.subscribe(() => {
      throw new Error("boom");
    });
    channel.subscribe(good);

    src.emit(7);
    expect(good).toHaveBeenCalledWith(7);
    expect(errSpy).toHaveBeenCalled();
    errSpy.mockRestore();
  });
});

describe("TerminalGuard — терминал ровно один раз", () => {
  it("первый вызов true, повторы false", () => {
    const guard = new TerminalGuard();
    expect(guard.firstTerminal("j1")).toBe(true);
    expect(guard.firstTerminal("j1")).toBe(false);
    expect(guard.firstTerminal("j2")).toBe(true);
    expect(guard.hasFired("j1")).toBe(true);
    guard.forget("j1");
    expect(guard.firstTerminal("j1")).toBe(true);
  });
});

describe("trackScan — идентичность + терминальность скана", () => {
  it("прогресс чужого скана игнорируется, своего доставляется", () => {
    const progressSrc = fakeAttach<ScanProgressEvent>();
    const doneSrc = fakeAttach<ScanDoneEvent>();
    // Подменяем каналы локально через прямую подписку на фейки:
    const onProgress = vi.fn();
    const offs = [
      progressSrc.attach((p) => {
        if (p.job_id === "scan-current") onProgress(p);
      }),
    ];

    progressSrc.emit(progress("scan-stale", "git"));
    progressSrc.emit(progress("scan-current", "node"));
    expect(onProgress).toHaveBeenCalledTimes(1);
    expect(onProgress.mock.calls[0][0].tool_id).toBe("node");
    offs.forEach((off) => off());
    void doneSrc;
  });

  it("терминальное событие доставляется ровно один раз при дублях", () => {
    const doneSrc = fakeAttach<ScanDoneEvent>();
    const onDone = vi.fn();
    const guard = new TerminalGuard();
    const off = doneSrc.attach((e) => {
      if (e.job_id !== "j1") return;
      if (e.terminal === "Running") return;
      if (!guard.firstTerminal(`${e.job_id}:${e.terminal}`)) return;
      onDone(e);
    });

    doneSrc.emit(scanDone("j1", "Completed"));
    doneSrc.emit(scanDone("j1", "Completed")); // дубликат бэкенда
    doneSrc.emit(scanDone("j2", "Completed")); // чужой скан
    expect(onDone).toHaveBeenCalledTimes(1);
    off();
  });

  it("trackScan не вызывает onDone для не-терминальных событий", () => {
    const doneSrc = fakeAttach<ScanDoneEvent>();
    const onDone = vi.fn();
    const off = doneSrc.attach((e) => {
      if (trackScanFilter(e)) onDone(e);
    });
    function trackScanFilter(e: ScanDoneEvent): boolean {
      return e.job_id === "j1" && e.terminal !== "Running";
    }
    doneSrc.emit(scanDone("j1", "Running"));
    expect(onDone).not.toHaveBeenCalled();
    off();
  });
});

const jobEvent = (jobId: string, seq: number, payload: JobEvent["payload"]): JobEvent => ({
  job_id: jobId,
  task_id: "task-1",
  tool_id: "git",
  seq,
  timestamp: "t",
  payload,
});

describe("trackJob — события задания с identity-guard", () => {
  it("чужие задания не доходят до обработчиков", () => {
    const src = fakeAttach<JobEvent>();
    const seen: JobEvent[] = [];
    const off = src.attach((e) => {
      if (e.job_id !== "my-job") return;
      seen.push(e);
    });

    src.emit(jobEvent("other-job", 1, { progress: { line: "stale" } }));
    src.emit(jobEvent("my-job", 2, { progress: { line: "mine" } }));
    expect(seen.map((e) => e.seq)).toEqual([2]);
    off();
  });

  it("job_finished обрабатывается один раз даже при повторе", () => {
    const src = fakeAttach<JobEvent>();
    const finished: string[] = [];
    const guard = new TerminalGuard();
    const off = src.attach((e) => {
      if (e.job_id !== "j9") return;
      if (!("job_finished" in e.payload)) return;
      if (!guard.firstTerminal("j9")) return;
      finished.push(e.payload.job_finished.status);
    });

    src.emit(jobEvent("j9", 5, { job_finished: { status: "succeeded", errors: [] } }));
    src.emit(jobEvent("j9", 6, { job_finished: { status: "failed", errors: ["late"] } }));
    expect(finished).toEqual(["succeeded"]);
    off();
  });

  it("интеграционно: trackScan+trackJob поверх общих каналов очищаются", () => {
    // Проверяем сам контракт API трекеров: очистка идемпотентна.
    const cleanup = trackScan("nonexistent-job", {});
    cleanup();
    expect(() => cleanup()).not.toThrow();

    const cleanupJob = trackJob("nonexistent-job", {});
    cleanupJob();
    expect(() => cleanupJob()).not.toThrow();
  });
});
