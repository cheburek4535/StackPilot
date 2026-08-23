// ============================================================
// Toolchain — контроллер состояния Control Center (Svelte 5 runes)
// ============================================================
// Единственный владелец состояния модуля для будущего экрана /toolchain:
//
//  - cachedSnapshot рендерится мгновенно; live-срез накладывается
//    инкрементально и НИКОГДА не стирает полезные прежние данные;
//  - скан и мутации — задания с идентичностью; stale-события чужих
//    операций отбрасываются до изменения состояния;
//  - мутация не стартует при активной конфликтующей мутации;
//    скан во время мутации разрешён (read-only движок);
//  - успех задания признаётся ТОЛЬКО от терминального события/результата
//    бэкенда — никогда не выводится UI самостоятельно;
//  - проваленный вызов API сохраняет предыдущее валидное состояние
//    (ошибка пишется в отдельное поле, данные не трогаются);
//  - секреты в состоянии НЕ хранятся (в типах их просто нет).
//
// Файл .svelte.ts: руны ($state/$derived) компилируются Svelte 5 —
// тот же стиль, что и в компонентах приложения.

import type {
  CatalogFilters,
  EnvironmentProfile,
  EnvironmentSnapshot,
  JobEvent,
  OperationKind,
  PersistedJob,
  ScanJobSnapshot,
  ToolDefinition,
  ToolScanResult,
} from "./types";
import {
  isRecord,
  jobEventKind,
  jobStatusIsTerminal,
  operationMutatesMachine,
  scanStartJobId,
} from "./types";
import * as api from "./api";
import {
  scanIsTerminalState,
  toolchainChannels,
  TerminalGuard,
  trackJob,
} from "./events";
import {
  applyJobEvent,
  applyScanProgress,
  gateMutation,
  isJobSucceeded,
  jobEventAppliesTo,
  jobEventLogText,
  mergeLiveSnapshot,
  resolveToolState,
  scanDoneAppliesTo,
  scanProgressAppliesTo,
  snapshotFreshness,
  upsertJobHistory,
  type LiveToolMap,
  type MutationGate,
  type SnapshotFreshness,
} from "./stateLogic";
import {
  CoalescingCall,
  LatestRequestGuard,
  OnceInitializer,
  ScanStartCoordinator,
} from "./startup";
import { defaultCatalogFilters, validateCatalogFilters } from "./filters";
import { sanitizeErrorMessage } from "./format";
import { notifyError, notifyInfo, notifySuccess } from "$lib/core/toasts";
import { readLocal, writeLocal } from "$lib/core/storage";
import { i18n } from "$lib/core/i18n.svelte";
import type { TranslationKey } from "$lib/core/i18n.svelte";

// ------------------------------------------------------------
// Лёгкие UI-настройки (персистятся локально; никаких данных бэкенда)
// ------------------------------------------------------------

const PREFS_KEY = "stackpilot:toolchain:prefs";
const PREFS_VERSION = 1;

/** Ограничение журнала одного задания (строк). */
const JOB_LOG_LIMIT = 300;

export type JobLogEntry = {
  seq: number;
  tool_id: string | null;
  text: string;
  timestamp: string;
};

export type ToolchainUiPrefs = {
  mode: ToolchainMode;
  logPanelOpen: boolean;
  /** Полный набор фильтров (все группы — не только поиск/категории/состояния). */
  filters: CatalogFilters;
};

export type ToolchainMode = "build_environment" | "manage_everything" | "tool_marketplace";

const DEFAULT_PREFS: ToolchainUiPrefs = {
  mode: "manage_everything",
  logPanelOpen: false,
  filters: defaultCatalogFilters(),
};

function loadPrefs(): ToolchainUiPrefs {
  const raw = readLocal<Partial<ToolchainUiPrefs>>(PREFS_KEY, PREFS_VERSION);
  if (!raw) return { ...DEFAULT_PREFS, filters: defaultCatalogFilters() };
  // Валидация на границе хранилища: неизвестные kind'ы/типы отбрасываются,
  // а не падают и не протаскиваются в фильтры.
  const filters =
    raw.filters === undefined
      ? defaultCatalogFilters()
      : validateCatalogFilters(raw.filters);
  return {
    mode:
      raw.mode === "build_environment" || raw.mode === "tool_marketplace"
        ? raw.mode
        : "manage_everything",
    logPanelOpen: raw.logPanelOpen === true,
    filters,
  };
}

function savePrefs(prefs: ToolchainUiPrefs): void {
  writeLocal(PREFS_KEY, prefs, PREFS_VERSION);
}

// ------------------------------------------------------------
// Контроллер
// ------------------------------------------------------------

class ToolchainController {
  // ---- снапшот: кэш + живой срез ----
  cachedSnapshot = $state<EnvironmentSnapshot | null>(null);
  /** Инкрементальные результаты идущего скана по tool_id. */
  private liveTools = $state<LiveToolMap>({});
  snapshotLoading = $state(false);
  snapshotError = $state<string | null>(null);

  /** Живой снапшот = кэш + результаты текущего скана. */
  liveSnapshot = $derived(mergeLiveSnapshot(this.cachedSnapshot, this.liveTools));
  freshness = $derived<SnapshotFreshness>(snapshotFreshness(this.liveSnapshot));

  // ---- текущий скан ----
  currentScan = $state<ScanJobSnapshot | null>(null);
  scanCancelling = $state(false);
  /** Reconnect/старт скана в полёте (фаза «подключаемся/запускаем»). */
  scanReconnecting = $state(false);

  // ---- текущее задание мутации + история сессии ----
  currentJob = $state<PersistedJob | null>(null);
  private currentJobLastSeq = 0;
  jobHistory = $state<PersistedJob[]>([]);
  jobStarting = $state(false);
  /** Reconnect истории/активного задания в полёте (фаза «восстановление»). */
  jobsReconnecting = $state(false);

  // ---- каталог ----
  catalog = $state<ToolScanResult[]>([]);
  catalogLoading = $state(false);
  catalogError = $state<string | null>(null);

  // ---- «усыновление» ручных установок (tcx_adopt_tool) ----
  /** tool_id → момент усыновления (из санитизированного view метаданных). */
  adopted = $state<Record<string, string>>({});

  // ---- метаданные каталога (описания/источники/зависимости для UI) ----
  definitions = $state<Record<string, ToolDefinition>>({});
  definitionsLoading = $state(false);
  definitionsError = $state<string | null>(null);

  // ---- Build Environment ----
  profile = $state<EnvironmentProfile | null>(null);
  profileLoading = $state(false);
  profileError = $state<string | null>(null);


  async uninstallTool(toolId: string): Promise<boolean> {
    if (confirm(i18n.t("tc.uninstall.confirm", { tool: toolId }) as string)) {
      try {
        await api.uninstallTool(toolId);
        await this.runHealthChecks([toolId]);
        return true;
      } catch (e: any) {
        notifyError(i18n.t("tc.uninstall.failed") as TranslationKey, e.toString());
        return false;
      }
    }
    return false;
  }

  // ---- UI-состояние (часть персистится) ----
  mode = $state<ToolchainMode>(DEFAULT_PREFS.mode);
  selectedToolId = $state<string | null>(null);
  drawerOpen = $state(false);
  logPanelOpen = $state(DEFAULT_PREFS.logPanelOpen);
  filters = $state<CatalogFilters>(defaultCatalogFilters());

  // ---- детали инструмента (drawer) ----
  detailsLoading = $state(false);
  detailsError = $state<string | null>(null);

  #prefsLoaded = false;
  #scanGuard = new TerminalGuard();
  #jobGuards = new Map<string, TerminalGuard>();
  #cleanups: Array<() => void> = [];
  #jobTrackerOff: (() => void) | null = null;
  #initializer = new OnceInitializer();
  /** Отложенная запись prefs при наборе поиска (не писать на каждый символ). */
  #prefsTimer: ReturnType<typeof setTimeout> | null = null;

  // ---- in-flight guard'ы (коалесцирование запросов) ----
  /** Единый полёт чтения снапшота: параллельных IPC не бывает. */
  #snapshotInFlight: Promise<void> | null = null;
  /** Сколько foreground-ожидающих ждут текущий полёт (для ошибки/спиннера). */
  #snapshotForeground = 0;
  /** Единый полёт reconnect сканов (getLatestScanJob — один раз). */
  #scanReconnectPromise: Promise<void> | null = null;
  /** Единый полёт reconnect заданий/истории. */
  #jobsReconnectInFlight: Promise<void> | null = null;
  /** Единый полёт метаданных усыновления. */
  #adoptedCall = new CoalescingCall<void>(async () => {
    try {
      const meta = await api.getToolchainMetadata();
      this.adopted = meta.adopted ?? {};
    } catch {
      /* отсутствие метаданных не блокирует UI: считаем набор пустым */
    }
  });
  /** Единый полёт загрузки каталога. */
  #definitionsCall = new CoalescingCall<void>(
    async () => {
      const defs = await api.getCatalog();
      const map: Record<string, ToolDefinition> = {};
      for (const d of defs) map[d.id] = d;
      this.definitions = map;
    },
    {
      onStart: () => {
        this.definitionsLoading = true;
        this.definitionsError = null;
      },
      onSettle: () => {
        this.definitionsLoading = false;
      },
      onError: (err) => {
        this.definitionsError = sanitizeErrorMessage(err);
      },
    },
  );
  /** Координатор скана: reconnect раньше старта, старт максимум один. */
  #scanCoordinator = new ScanStartCoordinator(
    () => this.currentScan,
    () => this.#reconnectScanOnce(),
    () => this.#startNewScan(),
  );
  /** Stale-guard деталей drawer'а: применяется только последний ответ. */
  #detailsGuard = new LatestRequestGuard();
  /** Полёты деталей по tool_id (коалесцирование в пределах инструмента). */
  #detailsInFlight = new Map<string, Promise<ToolScanResult | null>>();
  /** Инструменты, для которых уже идёт health-check. */
  #healthInFlight = new Set<string>();

  // ==========================================================
  // Инициализация / подписки
  // ==========================================================

  /**
   * Вызывать из onMount страницы (идемпотентно): подключает слушатели
   * ОДИН раз на приложение, восстанавливает состояние после навигации
   * (reconnect к идущему скан/заданию), грузит кэш мгновенно.
   * Возвращает функцию очистки подписок уровня страницы (пустую —
   * каналы живут на время приложения).
   *
   * Фазы разделены и не блокируют друг друга: снапшот (кэш), каталог,
   * метаданные и reconnect идут параллельными полётами; страница
   * интерактивна сразу.
   */
  ensureInitialized(): () => void {
    if (!this.#prefsLoaded) {
      const prefs = loadPrefs();
      this.mode = prefs.mode;
      this.logPanelOpen = prefs.logPanelOpen;
      this.filters = { ...defaultCatalogFilters(), ...prefs.filters };
      this.#prefsLoaded = true;
    }

    // Повторный вызов (ре-монтирование страницы, второй контроллер) —
    // no-op: слушатели и стартовые запросы ровно один раз.
    this.#initializer.run(() => {
      this.#attachListeners();
      void this.refreshSnapshot();
      void this.reconnectJobs();
      void this.ensureDefinitions();
      void this.refreshAdopted();
    });
    return () => {
      /* каналы — синглтоны; страница ничего не отсоединяет */
    };
  }

  #attachListeners(): void {
    // Прогресс скана: инкрементально, только события ТЕКУЩЕГО задания;
    // завершённый скан поздние события не оживляют.
    this.#cleanups.push(
      toolchainChannels.scanProgress.subscribe((event) => {
        if (!scanProgressAppliesTo(this.currentScan, event)) return;
        const scan = this.currentScan!;
        this.liveTools = applyScanProgress(this.liveTools, event);
        this.currentScan = {
          ...scan,
          completed_tools: Math.max(scan.completed_tools, event.completed_count),
          total_tools: event.total_count || scan.total_tools,
          current_tool: event.tool_id,
          updated_at: event.timestamp,
        };
      }),
    );

    // Терминал скана: ровно один раз; живой срез замещается финальным
    // снапшотом (частичные данные больше не «как финальные»).
    this.#cleanups.push(
      toolchainChannels.scanDone.subscribe((event) => {
        if (!scanDoneAppliesTo(this.currentScan, event)) return;
        const scan = this.currentScan!;
        if (!this.#scanGuard.firstTerminal(`${event.job_id}:${event.terminal}`)) return;
        this.currentScan = {
          ...scan,
          running: false,
          cancel_requested: false,
          terminal: event.terminal,
          completed_tools: event.completed,
          total_tools: event.total,
          finished_at: new Date().toISOString(),
        };
        // Финальный снапшот авторитетен: инкрементальные фрагменты
        // этого скана стираются до его получения.
        this.liveTools = {};
        void this.refreshSnapshot({ background: true });
        if (event.terminal === "Cancelled") {
          notifyInfo(i18n.t("tc.scan.cancelled") as TranslationKey, i18n.t("tc.scan.partial_results") as TranslationKey);
        } else if (event.terminal === "Failed" || event.terminal === "Interrupted") {
          notifyError(i18n.t("tc.scan.crashed") as TranslationKey, i18n.t("tc.scan.state", { state: event.terminal }) as TranslationKey);
        }
      }),
    );

    // События заданий движка: stale-guard по identity+seq.
    this.#cleanups.push(
      toolchainChannels.jobEvent.subscribe((event) => this.#handleJobEvent(event)),
    );
  }

  #handleJobEvent(event: JobEvent): void {
    // Граница IPC: событие обязано быть читаемым (payload — объект ровно
    // с одним известным ключом), иначе отбрасываем до любых `in`-проверок.
    const payloadKind = jobEventKind(event);
    if (!payloadKind) return;
    const job = this.currentJob;
    // Чужое/устаревшее задание (в т.ч. «хвост» прежней операции после
    // reconnect к новому заданию) — игнорируем до изменения состояния.
    if (!job || !jobEventAppliesTo(job, event)) return;

    const guard = this.#jobGuards.get(job.job_id) ?? new TerminalGuard();
    this.#jobGuards.set(job.job_id, guard);

    this.#appendJobLog(event);

    const applied = applyJobEvent(job, event, this.currentJobLastSeq);
    if (!applied.applied) return; // дубликат/отставшее событие
    this.currentJobLastSeq = applied.lastSeq;
    this.currentJob = applied.job;

    if (payloadKind === "job_finished") {
      // Сужение через isRecord+in: payload из IPC может быть любой формы.
      const payload: unknown = event.payload;
      if (!isRecord(payload) || !("job_finished" in payload)) return;
      const finished = payload.job_finished as
        | { status?: unknown; errors?: unknown }
        | undefined;
      const status = finished?.status;
      if (typeof status !== "string") return;
      // Терминал ровно один раз; успех — только от бэкенда.
      if (!guard.firstTerminal(`${job.job_id}:finished`)) return;
      this.#finalizeCurrentJob(
        status as PersistedJob["status"],
        Array.isArray(finished?.errors) ? (finished.errors as string[]) : [],
      );
    }
  }

  // ---- журнал событий заданий (для панели активности и drawer'а) ----

  /** Строка журнала: технические детали живут в expandable-областях UI. */
  jobLogs = $state<Record<string, JobLogEntry[]>>({});

  #appendJobLog(event: JobEvent): void {
    const text = jobEventLogText(event);
    if (!text) return;
    const entry: JobLogEntry = {
      seq: event.seq,
      tool_id: event.tool_id || null,
      text,
      timestamp: event.timestamp,
    };
    const existing = this.jobLogs[event.job_id] ?? [];
    const next = [...existing, entry].slice(-JOB_LOG_LIMIT);
    this.jobLogs = { ...this.jobLogs, [event.job_id]: next };
  }

  jobLogFor(jobId: string): JobLogEntry[] {
    return this.jobLogs[jobId] ?? [];
  }

  #finalizeCurrentJob(status: PersistedJob["status"], errors: string[]): void {
    const job = this.currentJob;
    if (job) {
      const finalRecord: PersistedJob = {
        ...job,
        status,
        errors,
        finished_at: job.finished_at ?? new Date().toISOString(),
      };
      this.jobHistory = upsertJobHistory(this.jobHistory, finalRecord);
      this.currentJob = finalRecord;
    }
    // Журналы храним только для заданий, которые ещё видны в истории
    // или активны — память не течёт с ростом числа операций.
    const keep = new Set<string>(this.jobHistory.map((j) => j.job_id));
    if (this.currentJob) keep.add(this.currentJob.job_id);
    const logs = { ...this.jobLogs };
    for (const id of Object.keys(logs)) {
      if (!keep.has(id)) delete logs[id];
    }
    this.jobLogs = logs;

    const opLabel = job ? job.operation : (i18n.t("tc.job.default_label") as string);
    if (isJobSucceeded(status)) {
      notifySuccess(i18n.t("tc.job.finished", { op: opLabel }) as TranslationKey, errors.length ? i18n.t("tc.job.errors", { n: errors.length }) as TranslationKey : undefined);
    } else if (status === "cancelled") {
      notifyInfo(i18n.t("tc.job.cancelled_toast") as TranslationKey);
    } else if (status === "failed" || status === "interrupted") {
      notifyError(i18n.t("tc.job.failed", { op: opLabel }) as TranslationKey, errors[0] ? sanitizeErrorMessage(errors[0]) : undefined);
    }
    // Данные машины изменились — обновляем снапшот в фоне.
    void this.refreshSnapshot({ background: true });
    // После успешной мутации — точечная read-only перепроверка затронутых
    // инструментов, чтобы карточки/витрина показали свежий факт сразу.
    if (
      job &&
      isJobSucceeded(status) &&
      operationMutatesMachine(job.operation) &&
      job.requested_tool_ids.length > 0
    ) {
      void this.runHealthChecks(job.requested_tool_ids);
    }
  }

  /** Reconnect после навигации/перезапуска: идущий скан и задание.
   *  Идемпотентен: повторные вызовы во время полёта получают тот же
   *  результат — параллельных getLatestScanJob/listJobs не бывает. */
  async reconnectJobs(): Promise<void> {
    if (this.#jobsReconnectInFlight) return this.#jobsReconnectInFlight;
    this.jobsReconnecting = true;
    this.#jobsReconnectInFlight = (async () => {
      // Скан — общая цепочка с ensureScanRunning (один getLatestScanJob).
      await this.#reconnectScanOnce();
      try {
        const jobs = await api.listJobs();
        const active = jobs.find((j) => !jobStatusIsTerminal(j.status));
        if (active) this.#adoptJob(active);
        this.jobHistory = jobs.slice(0, 20);
      } catch {
        /* история недоступна — не критично */
      }
    })().finally(() => {
      this.jobsReconnecting = false;
      this.#jobsReconnectInFlight = null;
    });
    return this.#jobsReconnectInFlight;
  }

  // ==========================================================
  // Снапшот и сканы
  // ==========================================================

  /**
   * Перечитывает снапшот. Провал НЕ затирает предыдущие данные:
   * ошибка пишется в snapshotError, кэш остаётся как был.
   * Повторные вызовы во время полёта коалесцируются в один IPC.
   */
  async refreshSnapshot(options: { background?: boolean } = {}): Promise<void> {
    const foreground = !options.background;
    if (this.#snapshotInFlight) {
      if (foreground) {
        // Явный повторный запрос «в лицо»: индикатор и ошибка важны.
        this.snapshotLoading = true;
        this.snapshotError = null;
        this.#snapshotForeground += 1;
        try {
          await this.#snapshotInFlight;
        } finally {
          this.#releaseSnapshotWait();
        }
      } else {
        await this.#snapshotInFlight;
      }
      return;
    }
    if (foreground) {
      this.snapshotLoading = true;
      this.snapshotError = null;
      this.#snapshotForeground += 1;
    }
    const flight = (async () => {
      try {
        const snap = await api.getEnvironmentSnapshot();
        if (snap) this.#adoptSnapshot(snap);
      } catch (err) {
        // Предыдущее валидное состояние сохранено намеренно; ошибка видна
        // только если её ждали «в лицо» (не фоновое обновление).
        if (this.#snapshotForeground > 0) {
          this.snapshotError = sanitizeErrorMessage(err);
        }
      } finally {
        this.#snapshotInFlight = null;
        if (foreground) this.#releaseSnapshotWait();
      }
    })();
    this.#snapshotInFlight = flight;
    return flight;
  }

  #releaseSnapshotWait(): void {
    this.#snapshotForeground -= 1;
    if (this.#snapshotForeground <= 0) {
      this.#snapshotForeground = 0;
      this.snapshotLoading = false;
    }
  }

  #adoptSnapshot(snapshot: EnvironmentSnapshot): void {
    this.cachedSnapshot = snapshot;
    // Живой срез валиден только для того же запуска скана.
    if (snapshot.scan_id && this.currentScan && snapshot.scan_id === this.currentScan.scan_id) {
      // оставляем инкрементальные данные текущего скана
    }
  }

  /** Автоскан при входе (контракт §4.2): reconnect или новый запуск.
   *  Гонок нет: reconnect всегда раньше старта, повторные вызовы во
   *  время полёта коалесцируются, один и тот же скан не стартует дважды. */
  async ensureScanRunning(): Promise<ScanJobSnapshot | null> {
    const active = this.currentScan;
    if (active && !scanIsTerminalState(active.terminal)) {
      return active; // уже идёт — reconnect не нужен
    }
    this.scanReconnecting = true;
    try {
      return await this.#scanCoordinator.ensureRunning();
    } finally {
      this.scanReconnecting = false;
    }
  }

  /** Reconnect скана (общая цепочка старта): применяет актуальное задание.
   *  Коалесцируется: пока полёт жив, повторные вызовы получают тот же
   *  результат — один getLatestScanJob на всю цепочку запуска. */
  #reconnectScanOnce(): Promise<void> {
    if (!this.#scanReconnectPromise) {
      this.#scanReconnectPromise = (async () => {
        try {
          const latestScan = await api.getLatestScanJob();
          if (latestScan) this.#adoptScan(latestScan);
        } catch {
          /* нет данных о сканах — не ошибка UI */
        }
      })().finally(() => {
        this.#scanReconnectPromise = null;
      });
    }
    return this.#scanReconnectPromise;
  }

  /** Старт нового скана — только когда после reconnect ничего не бежит. */
  async #startNewScan(): Promise<ScanJobSnapshot | null> {
    try {
      const outcome = await api.startScan();
      // Guard вместо `"Started" in outcome`: outcome из IPC может быть
      // малиформированным — тогда честный null, а не краш.
      const job = scanStartJobId(outcome);
      if (!job) {
        this.snapshotError = i18n.t("tc.backend.unreadable") as TranslationKey;
        return null;
      }
      this.#adoptScan(job);
      return job;
    } catch (err) {
      this.snapshotError = sanitizeErrorMessage(err);
      return null;
    }
  }

  #adoptScan(job: ScanJobSnapshot): void {
    this.currentScan = job;
    this.scanCancelling = false;
    this.#scanGuard.reset();
    if (job.running) this.liveTools = {}; // новый прогон — чистый срез
  }

  /** Отмена скана: состояние не очищается до терминала. */
  async cancelCurrentScan(): Promise<boolean> {
    const scan = this.currentScan;
    if (!scan || !scan.running || this.scanCancelling) return false;
    this.scanCancelling = true;
    try {
      await api.cancelScan(scan.job_id);
      this.currentScan = { ...scan, cancel_requested: true };
      return true;
    } catch (err) {
      notifyError(i18n.t("tc.scan.cancel_failed") as TranslationKey, sanitizeErrorMessage(err));
      return false;
    } finally {
      this.scanCancelling = false;
    }
  }

  // ==========================================================
  // Задания мутации (install/update/repair_path)
  // ==========================================================

  /** Проверка конкурентности без старта (для disabled-состояний кнопок). */
  mutationGate(operation: OperationKind): MutationGate {
    return gateMutation(this.currentJob, operation);
  }

  get mutationInProgress(): boolean {
    const job = this.currentJob;
    return !!job && !jobStatusIsTerminal(job.status) && operationMutatesMachine(job.operation);
  }

  /**
   * Старт задания из ограниченного запроса. Конфликтующая мутация
   * блокирует старт ДО обращения к бэкенду.
   */
  async startMutation(request: Parameters<typeof api.startJob>[0]): Promise<string | null> {
    const gate = this.mutationGate(request.operation);
    if (!gate.allowed) {
      notifyError(i18n.t("tc.job.start_failed") as TranslationKey, gate.reason);
      return null;
    }
    this.jobStarting = true;
    try {
      const jobId = await api.startJob(request);
      // Запись подтянется событиями; сразу ставим каркас с identity.
      this.#adoptJob({
        job_id: jobId,
        plan_id: "",
        operation: request.operation,
        requested_tool_ids: (request.tools ?? []).map((t) => t.tool_id),
        source_choices: {},
        created_at: new Date().toISOString(),
        started_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
        finished_at: null,
        status: "queued",
        plan: {
          plan_id: "",
          operation: request.operation,
          os: "",
          created_at: new Date().toISOString(),
          fingerprint: "",
          tasks: [],
          total_size_mb: 0,
          free_space_mb: 0,
          enough_space: true,
          needs_admin_any: false,
          capabilities: { install_execution_supported: true, elevation_supported: true },
          warnings: [],
        },
        errors: [],
        path_changes: [],
        recovered: false,
      });
      return jobId;
    } catch (err) {
      // Провал старта не меняет существующее состояние заданий.
      notifyError(i18n.t("tc.job.start_failed") as TranslationKey, sanitizeErrorMessage(err));
      return null;
    } finally {
      this.jobStarting = false;
    }
  }

  #adoptJob(job: PersistedJob): void {
    if (this.currentJob?.job_id === job.job_id) {
      // Та же запись (reconnect/повторное усыновление): трекер и guard
      // уже подписаны — дублирующих слушателей не создаём.
      this.currentJob = job;
      return;
    }
    this.currentJob = job;
    this.currentJobLastSeq = 0;
    this.#jobGuards.set(job.job_id, new TerminalGuard());
    // Подписка трекера на конкретное задание (терминал ровно один раз).
    this.#jobTrackerOff?.();
    const off = trackJob(job.job_id, {
      onFinished: ({ status, errors }) => {
        if (this.#jobTrackerOff === off) this.#jobTrackerOff = null;
        // Дублирующая защита от гонки channel-subscribe vs handleJobEvent:
        // finalize выполнится только если ещё не финализировали.
        const guard = this.#jobGuards.get(job.job_id);
        if (guard && !guard.hasFired(`${job.job_id}:finished`)) {
          guard.firstTerminal(`${job.job_id}:finished`);
          this.#finalizeCurrentJob(status, errors);
        }
      },
    });
    this.#jobTrackerOff = off;
  }

  /** Отмена текущего задания. */
  async cancelCurrentJob(): Promise<boolean> {
    const job = this.currentJob;
    if (!job || jobStatusIsTerminal(job.status)) return false;
    try {
      await api.cancelJob(job.job_id);
      return true;
    } catch (err) {
      notifyError(i18n.t("tc.job.cancel_failed") as TranslationKey, sanitizeErrorMessage(err));
      return false;
    }
  }

  /**
   * Повтор терминального задания: бэкенд строит план заново от свежих
   * фактов; новое задание становится текущим. Провал старта не трогает
   * существующее состояние.
   */
  async retryJob(jobId: string): Promise<string | null> {
    try {
      const newId = await api.retryJob(jobId);
      const job = await api.getJob(newId);
      if (job) this.#adoptJob(job);
      notifyInfo(i18n.t("tc.job.retry_started") as TranslationKey, i18n.t("tc.job.new_job", { id: newId }) as TranslationKey);
      return newId;
    } catch (err) {
      notifyError(i18n.t("tc.job.retry_failed") as TranslationKey, sanitizeErrorMessage(err));
      return null;
    }
  }

  // ==========================================================
  // Каталог / детали инструмента / здоровье
  // ==========================================================

  /**
   * Метаданные state.json (санитизированный view, без секретов):
   * нужны для факта «усыновлён/наблюдается» в drawer'е.
   * Коалесцируется: повторные вызовы во время полёта — один IPC.
   */
  refreshAdopted(): Promise<void> {
    return this.#adoptedCall.call();
  }

  /**
   * Явное «усыновить» найденную ручную установку: track-метка в
   * state.json (НЕ «установлено StackPilot»). После успеха набор
   * усыновлений перечитывается с бэкенда.
   */
  async adoptTool(toolId: string): Promise<boolean> {
    try {
      await api.adoptTool(toolId);
      await this.refreshAdopted();
      notifySuccess(i18n.t("tc.tool.tracked") as TranslationKey, i18n.t("tc.tool.tracked_desc", { tool: toolId }) as TranslationKey);
      return true;
    } catch (err) {
      notifyError(i18n.t("tc.tool.track_failed") as TranslationKey, sanitizeErrorMessage(err));
      return false;
    }
  }

  isAdopted(toolId: string): boolean {
    return Object.prototype.hasOwnProperty.call(this.adopted, toolId);
  }

  /** Каталог из снапшота; если его нет — честная загрузка деталей.
   *  Читает через ЕДИНЫЙ полёт снапшота — дублирующего IPC не бывает. */
  async loadCatalog(): Promise<void> {
    if (this.catalog.length > 0 || this.catalogLoading) return;
    this.catalogLoading = true;
    try {
      await this.refreshSnapshot();
      const snap = this.cachedSnapshot;
      if (snap) {
        this.catalog = snap.tools;
      } else {
        this.catalogError = this.snapshotError ?? (i18n.t("tc.snapshot.missing") as TranslationKey);
      }
    } finally {
      this.catalogLoading = false;
    }
  }

  /**
   * Метаданные каталога (описания, источники, зависимости). Грузятся один
   * раз; провал сохраняет прежнее состояние и виден рядом с данными.
   * Коалесцируется: параллельных tcx_get_catalog не бывает.
   */
  ensureDefinitions(): Promise<void> {
    if (Object.keys(this.definitions).length > 0) return Promise.resolve();
    // Ошибка пишется в definitionsError (виден retry-экран); отклонение
    // наружу не уходит — fire-and-forget-вызовы не роняют консоль.
    return this.#definitionsCall.call().catch(() => {});
  }

  definitionFor(toolId: string): ToolDefinition | null {
    return this.definitions[toolId] ?? null;
  }

  /** Живые детали одного инструмента (drawer). Провал сохраняет старое.
   *  Race-safe: применяется только ответ ПОСЛЕДНЕГО выбранного инструмента
   *  (медленный ответ прежнего выбора не перезаписывает drawer), а в
   *  пределах одного инструмента запросы коалесцируются. */
  refreshToolDetails(toolId: string): Promise<ToolScanResult | null> {
    this.#detailsGuard.begin(toolId);
    const existing = this.#detailsInFlight.get(toolId);
    if (existing) {
      // Запрос для этого инструмента уже в полёте — ждём его же.
      return existing.then((result) => {
        if (result && this.#detailsGuard.isLatest(toolId)) {
          this.liveTools = { ...this.liveTools, [toolId]: result };
        }
        return result;
      });
    }
    this.detailsLoading = true;
    this.detailsError = null;
    const flight = api
      .getToolDetails(toolId)
      .then((result) => {
        // Stale-guard: если выбор сменился, ответ прежнего инструмента
        // не пишется в состояние (drawer читает только текущий).
        if (this.#detailsGuard.isLatest(toolId)) {
          this.liveTools = { ...this.liveTools, [toolId]: result };
        }
        return result;
      })
      .catch((err) => {
        if (this.#detailsGuard.isLatest(toolId)) {
          this.detailsError = sanitizeErrorMessage(err);
          notifyError(i18n.t("tc.details.update_failed") as TranslationKey, this.detailsError);
        }
        return null;
      })
      .finally(() => {
        this.#detailsInFlight.delete(toolId);
        if (this.#detailsGuard.isLatest(toolId)) this.detailsLoading = false;
      });
    this.#detailsInFlight.set(toolId, flight);
    return flight;
  }

  /** Health-check выбранных инструментов (read-only).
   *  Коалесцирование в пределах инструмента: параллельных проверок
   *  одного id не бывает. */
  async runHealthChecks(toolIds: string[]): Promise<ToolScanResult[]> {
    const pending = toolIds.filter((id) => !this.#healthInFlight.has(id));
    if (pending.length === 0) return [];
    for (const id of pending) this.#healthInFlight.add(id);
    try {
      const results = await api.runHealthChecks(pending);
      const next = { ...this.liveTools };
      for (const r of results) next[r.tool_id] = r;
      this.liveTools = next;
      return results;
    } catch (err) {
      notifyError(i18n.t("tc.health.check_failed") as TranslationKey, sanitizeErrorMessage(err));
      return [];
    } finally {
      for (const id of pending) this.#healthInFlight.delete(id);
    }
  }

  // ==========================================================
  // Build Environment
  // ==========================================================

  async loadProfile(requirements: EnvironmentProfile["requirements"]): Promise<EnvironmentProfile | null> {
    this.profileLoading = true;
    this.profileError = null;
    try {
      const profile = await api.resolveEnvironmentProfile(requirements);
      this.profile = profile;
      return profile;
    } catch (err) {
      // Прежний профиль сохраняется; ошибка видна рядом.
      this.profileError = sanitizeErrorMessage(err);
      return null;
    } finally {
      this.profileLoading = false;
    }
  }

  // ==========================================================
  // Marketplace (витрина инструментов)
  // ==========================================================

  /** ОС витрины: снапшот, иначе лёгкий read-only опрос окружения. */
  marketplaceOs = $state<string | null>(null);
  marketplaceOsLoading = $state(false);
  marketplaceOsError = $state<string | null>(null);

  /** Определяет текущую платформу для витрины. Работает БЕЗ снапшота.
   *  Провал опроса окружения — видимая ошибка с повтором, а не вечный
   *  экран «Определяем платформу…». */
  async ensureMarketplacePlatform(): Promise<string | null> {
    const fromSnapshot = this.liveSnapshot?.os;
    if (fromSnapshot) {
      this.marketplaceOs = fromSnapshot;
      this.marketplaceOsError = null;
      return fromSnapshot;
    }
    if (this.marketplaceOsLoading) return this.marketplaceOs;
    this.marketplaceOsLoading = true;
    try {
      const info = await api.getEnvironmentInfo();
      this.marketplaceOs = info?.os ?? null;
      this.marketplaceOsError = info?.os ? null : (i18n.t("tc.platform.not_reported") as TranslationKey);
    } catch (err) {
      this.marketplaceOs = null;
      this.marketplaceOsError = sanitizeErrorMessage(err);
    } finally {
      this.marketplaceOsLoading = false;
    }
    return this.marketplaceOs;
  }

  // ==========================================================
  // UI-действия
  // ==========================================================

  setMode(mode: ToolchainMode): void {
    this.mode = mode;
    this.persistPrefs();
  }

  toggleDrawer(open?: boolean): void {
    this.drawerOpen = open ?? !this.drawerOpen;
    if (!this.drawerOpen) this.selectedToolId = null;
  }

  selectTool(toolId: string | null): void {
    this.selectedToolId = toolId;
    this.drawerOpen = toolId !== null;
  }

  toggleLogPanel(open?: boolean): void {
    this.logPanelOpen = open ?? !this.logPanelOpen;
    this.persistPrefs();
  }

  setFilters(patch: Partial<CatalogFilters>): void {
    this.filters = { ...this.filters, ...patch };
    // Поиск меняется на каждый символ — запись настроек откладывается;
    // структурные фильтры (клики) сохраняются сразу.
    if ("search" in patch && Object.keys(patch).length === 1) {
      if (this.#prefsTimer) clearTimeout(this.#prefsTimer);
      this.#prefsTimer = setTimeout(() => {
        this.#prefsTimer = null;
        this.persistPrefs();
      }, 600);
    } else {
      if (this.#prefsTimer) {
        clearTimeout(this.#prefsTimer);
        this.#prefsTimer = null;
      }
      this.persistPrefs();
    }
  }

  resetFilters(): void {
    if (this.#prefsTimer) {
      clearTimeout(this.#prefsTimer);
      this.#prefsTimer = null;
    }
    this.filters = defaultCatalogFilters();
    this.persistPrefs();
  }

  persistPrefs(): void {
    savePrefs({
      mode: this.mode,
      logPanelOpen: this.logPanelOpen,
      filters: { ...this.filters },
    });
  }

  // ==========================================================
  // Производные данные для UI
  // ==========================================================

  toolById(toolId: string): ToolScanResult | null {
    return resolveToolState(this.cachedSnapshot, this.liveTools, toolId);
  }
}

/** Синглтон контроллера: переживает навигацию маршрутов. */
export const toolchain = new ToolchainController();
