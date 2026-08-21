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
import { jobStatusIsTerminal, operationMutatesMachine, parseToolState } from "./types";
import * as api from "./api";
import {
  scanIsTerminalState,
  toolchainChannels,
  TerminalGuard,
  trackJob,
  trackScan,
} from "./events";
import {
  applyJobEvent,
  applyScanProgress,
  gateMutation,
  isJobSucceeded,
  mergeLiveSnapshot,
  resolveToolState,
  snapshotFreshness,
  upsertJobHistory,
  type LiveToolMap,
  type MutationGate,
  type SnapshotFreshness,
} from "./stateLogic";
import { defaultCatalogFilters } from "./filters";
import { sanitizeErrorMessage } from "./format";
import { notifyError, notifyInfo, notifySuccess } from "$lib/core/toasts";
import { readLocal, writeLocal } from "$lib/core/storage";

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

/** Человекочитаемая строка журнала из типизированного события. */
function jobEventLogText(event: JobEvent): string | null {
  const p = event.payload;
  if ("progress" in p) return p.progress.line || null;
  if ("task_started" in p) return `задача ${event.task_id}: старт`;
  if ("task_phase" in p) return `задача ${event.task_id}: ${p.task_phase.phase}`;
  if ("task_completed" in p) {
    const s = p.task_completed.status;
    const kind =
      typeof s === "string" ? s : Object.keys(s)[0];
    return `задача ${event.task_id}: ${kind}`;
  }
  if ("path_updated" in p) {
    const added = p.path_updated.record.added ?? [];
    return `PATH обновлён (${added.length} записей добавлено)`;
  }
  if ("job_started" in p) return `задание запущено: задач — ${p.job_started.total_tasks}`;
  // job_finished фиксируется в статусе записи; строка не нужна.
  return null;
}

export type ToolchainUiPrefs = {
  mode: ToolchainMode;
  logPanelOpen: boolean;
  filters: Pick<CatalogFilters, "search" | "categories" | "states">;
};

export type ToolchainMode = "build_environment" | "manage_everything";

const DEFAULT_PREFS: ToolchainUiPrefs = {
  mode: "manage_everything",
  logPanelOpen: false,
  filters: { search: "", categories: [], states: [] },
};

function loadPrefs(): ToolchainUiPrefs {
  const raw = readLocal<Partial<ToolchainUiPrefs>>(PREFS_KEY, PREFS_VERSION);
  if (!raw) return { ...DEFAULT_PREFS };
  return {
    mode: raw.mode === "build_environment" ? "build_environment" : "manage_everything",
    logPanelOpen: raw.logPanelOpen === true,
    filters: {
      search: typeof raw.filters?.search === "string" ? raw.filters.search : "",
      categories: Array.isArray(raw.filters?.categories) ? raw.filters.categories : [],
      states: Array.isArray(raw.filters?.states) ? raw.filters.states : [],
    },
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

  // ---- текущее задание мутации + история сессии ----
  currentJob = $state<PersistedJob | null>(null);
  private currentJobLastSeq = 0;
  jobHistory = $state<PersistedJob[]>([]);
  jobStarting = $state(false);

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

  // ---- UI-состояние (часть персистится) ----
  mode = $state<ToolchainMode>(DEFAULT_PREFS.mode);
  selectedToolId = $state<string | null>(null);
  drawerOpen = $state(false);
  logPanelOpen = $state(DEFAULT_PREFS.logPanelOpen);
  filters = $state<CatalogFilters>(defaultCatalogFilters());

  #prefsLoaded = false;
  #scanGuard = new TerminalGuard();
  #jobGuards = new Map<string, TerminalGuard>();
  #cleanups: Array<() => void> = [];
  #jobTrackerOff: (() => void) | null = null;
  #initialized = false;
  /** Отложенная запись prefs при наборе поиска (не писать на каждый символ). */
  #prefsTimer: ReturnType<typeof setTimeout> | null = null;

  // ==========================================================
  // Инициализация / подписки
  // ==========================================================

  /**
   * Вызывать из onMount страницы (идемпотентно): подключает слушатели
   * ОДИН раз на приложение, восстанавливает состояние после навигации
   * (reconnect к идущему скану/заданию), грузит кэш мгновенно.
   * Возвращает функцию очистки подписок уровня страницы (пустую —
   * каналы живут на время приложения).
   */
  ensureInitialized(): () => void {
    if (!this.#prefsLoaded) {
      const prefs = loadPrefs();
      this.mode = prefs.mode;
      this.logPanelOpen = prefs.logPanelOpen;
      this.filters = { ...defaultCatalogFilters(), ...prefs.filters };
      this.#prefsLoaded = true;
    }

    if (!this.#initialized) {
      this.#initialized = true;
      this.#attachListeners();
      void this.refreshSnapshot();
      void this.reconnectJobs();
      void this.ensureDefinitions();
      void this.refreshAdopted();
    }
    return () => {
      /* каналы — синглтоны; страница ничего не отсоединяет */
    };
  }

  #attachListeners(): void {
    // Прогресс скана: инкрементально, только события ТЕКУЩЕГО задания.
    this.#cleanups.push(
      toolchainChannels.scanProgress.subscribe((event) => {
        if (!this.currentScan || event.job_id !== this.currentScan.job_id) return;
        this.liveTools = applyScanProgress(this.liveTools, event);
        this.currentScan = {
          ...this.currentScan,
          completed_tools: Math.max(this.currentScan.completed_tools, event.completed_count),
          total_tools: event.total_count || this.currentScan.total_tools,
          current_tool: event.tool_id,
          updated_at: event.timestamp,
        };
      }),
    );

    // Терминал скана: ровно один раз; снапшот перечитываем с бэкенда.
    this.#cleanups.push(
      toolchainChannels.scanDone.subscribe((event) => {
        if (!this.currentScan || event.job_id !== this.currentScan.job_id) return;
        if (!this.#scanGuard.firstTerminal(`${event.job_id}:${event.terminal}`)) return;
        this.currentScan = {
          ...this.currentScan,
          running: false,
          cancel_requested: false,
          terminal: event.terminal,
          completed_tools: event.completed,
          total_tools: event.total,
          finished_at: new Date().toISOString(),
        };
        void this.refreshSnapshot({ background: true });
        if (event.terminal === "Cancelled") {
          notifyInfo("Скан отменён", "Частичные результаты сохранены");
        } else if (event.terminal === "Failed" || event.terminal === "Interrupted") {
          notifyError("Скан завершился аварийно", `Состояние: ${event.terminal}`);
        }
      }),
    );

    // События заданий движка: stale-guard по identity+seq.
    this.#cleanups.push(
      toolchainChannels.jobEvent.subscribe((event) => this.#handleJobEvent(event)),
    );
  }

  #handleJobEvent(event: JobEvent): void {
    const job = this.currentJob;
    if (!job || event.job_id !== job.job_id) return; // чужое задание — игнорируем

    const guard = this.#jobGuards.get(job.job_id) ?? new TerminalGuard();
    this.#jobGuards.set(job.job_id, guard);

    this.#appendJobLog(event);

    const applied = applyJobEvent(job, event, this.currentJobLastSeq);
    if (!applied.applied) return; // дубликат/отставшее событие
    this.currentJobLastSeq = applied.lastSeq;
    this.currentJob = applied.job;

    if ("job_finished" in event.payload && jobStatusIsTerminal(event.payload.job_finished.status)) {
      // Терминал ровно один раз; успех — только от бэкенда.
      if (!guard.firstTerminal(`${job.job_id}:finished`)) return;
      const status = event.payload.job_finished.status;
      this.#finalizeCurrentJob(status, event.payload.job_finished.errors);
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

    const opLabel = job ? job.operation : "задание";
    if (isJobSucceeded(status)) {
      notifySuccess(`Задание «${opLabel}» завершено`, errors.length ? `${errors.length} ошибок` : undefined);
    } else if (status === "cancelled") {
      notifyInfo("Задание отменено");
    } else if (status === "failed" || status === "interrupted") {
      notifyError(`Задание «${opLabel}» не выполнено`, errors[0] ? sanitizeErrorMessage(errors[0]) : undefined);
    }
    // Данные машины изменились — обновляем снапшот в фоне.
    void this.refreshSnapshot({ background: true });
  }

  /** Reconnect после навигации/перезапуска: идущий скан и задание. */
  async reconnectJobs(): Promise<void> {
    try {
      const latestScan = await api.getLatestScanJob();
      if (latestScan && !scanIsTerminalState(latestScan.terminal)) {
        this.#adoptScan(latestScan);
      } else if (latestScan) {
        this.currentScan = latestScan;
      }
    } catch {
      /* нет данных о сканах — не ошибка UI */
    }
    try {
      const jobs = await api.listJobs();
      const active = jobs.find((j) => !jobStatusIsTerminal(j.status));
      if (active) this.#adoptJob(active);
      this.jobHistory = jobs.slice(0, 20);
    } catch {
      /* история недоступна — не критично */
    }
  }

  // ==========================================================
  // Снапшот и сканы
  // ==========================================================

  /**
   * Перечитывает снапшот. Провал НЕ затирает предыдущие данные:
   * ошибка пишется в snapshotError, кэш остаётся как был.
   */
  async refreshSnapshot(options: { background?: boolean } = {}): Promise<void> {
    if (options.background) {
      try {
        const snap = await api.getEnvironmentSnapshot();
        if (snap) this.#adoptSnapshot(snap);
      } catch {
        /* фоновое обновление молча сохраняет прежнее состояние */
      }
      return;
    }
    this.snapshotLoading = true;
    this.snapshotError = null;
    try {
      const snap = await api.getEnvironmentSnapshot();
      if (snap) this.#adoptSnapshot(snap);
    } catch (err) {
      // Предыдущее валидное состояние сохранено намеренно.
      this.snapshotError = sanitizeErrorMessage(err);
    } finally {
      this.snapshotLoading = false;
    }
  }

  #adoptSnapshot(snapshot: EnvironmentSnapshot): void {
    this.cachedSnapshot = snapshot;
    // Живой срез валиден только для того же запуска скана.
    if (snapshot.scan_id && this.currentScan && snapshot.scan_id === this.currentScan.scan_id) {
      // оставляем инкрементальные данные текущего скана
    } else {
      this.liveTools = {};
    }
  }

  /** Автоскан при входе (контракт §4.2): reconnect или новый запуск. */
  async ensureScanRunning(): Promise<ScanJobSnapshot | null> {
    if (this.currentScan && !scanIsTerminalState(this.currentScan.terminal)) {
      return this.currentScan; // уже идёт — reconnect не нужен
    }
    try {
      const outcome = await api.startScan();
      const job =
        "Started" in outcome ? outcome.Started : outcome.AlreadyRunning;
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
      notifyError("Не удалось отменить скан", sanitizeErrorMessage(err));
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
      notifyError("Задание не запущено", gate.reason);
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
      notifyError("Не удалось запустить задание", sanitizeErrorMessage(err));
      return null;
    } finally {
      this.jobStarting = false;
    }
  }

  #adoptJob(job: PersistedJob): void {
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
      notifyError("Не удалось отменить задание", sanitizeErrorMessage(err));
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
      notifyInfo("Повтор задания запущен", `Новое задание: ${newId}`);
      return newId;
    } catch (err) {
      notifyError("Не удалось повторить задание", sanitizeErrorMessage(err));
      return null;
    }
  }

  // ==========================================================
  // Каталог / детали инструмента / здоровье
  // ==========================================================

  /**
   * Метаданные state.json (санитизированный view, без секретов):
   * нужны для факта «усыновлён/наблюдается» в drawer'е.
   */
  async refreshAdopted(): Promise<void> {
    try {
      const meta = await api.getToolchainMetadata();
      this.adopted = meta.adopted ?? {};
    } catch {
      /* отсутствие метаданных не блокирует UI: считаем набор пустым */
    }
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
      notifySuccess("Инструмент отслеживается", `${toolId}: помечен как наблюдаемый`);
      return true;
    } catch (err) {
      notifyError("Не удалось взять под наблюдение", sanitizeErrorMessage(err));
      return false;
    }
  }

  isAdopted(toolId: string): boolean {
    return Object.prototype.hasOwnProperty.call(this.adopted, toolId);
  }

  /** Каталог из снапшота; если его нет — честная загрузка деталей. */
  async loadCatalog(): Promise<void> {
    if (this.catalog.length > 0 || this.catalogLoading) return;
    this.catalogLoading = true;
    this.catalogError = null;
    try {
      const snap = await api.getEnvironmentSnapshot();
      if (snap) {
        this.cachedSnapshot = snap;
        this.catalog = snap.tools;
      } else {
        this.catalogError = "Снапшот отсутствует — выполните первый скан";
      }
    } catch (err) {
      this.catalogError = sanitizeErrorMessage(err);
    } finally {
      this.catalogLoading = false;
    }
  }

  /**
   * Метаданные каталога (описания, источники, зависимости). Грузятся один
   * раз; провал сохраняет прежнее состояние и виден рядом с данными.
   */
  async ensureDefinitions(): Promise<void> {
    if (Object.keys(this.definitions).length > 0 || this.definitionsLoading) return;
    this.definitionsLoading = true;
    this.definitionsError = null;
    try {
      const defs = await api.getCatalog();
      const map: Record<string, ToolDefinition> = {};
      for (const d of defs) map[d.id] = d;
      this.definitions = map;
    } catch (err) {
      this.definitionsError = sanitizeErrorMessage(err);
    } finally {
      this.definitionsLoading = false;
    }
  }

  definitionFor(toolId: string): ToolDefinition | null {
    return this.definitions[toolId] ?? null;
  }

  /** Живые детали одного инструмента (drawer). Провал сохраняет старое. */
  async refreshToolDetails(toolId: string): Promise<ToolScanResult | null> {
    try {
      const result = await api.getToolDetails(toolId);
      this.liveTools = { ...this.liveTools, [toolId]: result };
      return result;
    } catch (err) {
      notifyError("Не удалось обновить инструмент", sanitizeErrorMessage(err));
      return null;
    }
  }

  /** Health-check выбранных инструментов (read-only). */
  async runHealthChecks(toolIds: string[]): Promise<ToolScanResult[]> {
    try {
      const results = await api.runHealthChecks(toolIds);
      const next = { ...this.liveTools };
      for (const r of results) next[r.tool_id] = r;
      this.liveTools = next;
      return results;
    } catch (err) {
      notifyError("Проверка здоровья не удалась", sanitizeErrorMessage(err));
      return [];
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
      filters: {
        search: this.filters.search,
        categories: this.filters.categories,
        states: this.filters.states,
      },
    });
  }

  // ==========================================================
  // Производные данные для UI
  // ==========================================================

  toolById(toolId: string): ToolScanResult | null {
    return resolveToolState(this.cachedSnapshot, this.liveTools, toolId);
  }

  /** Состояние инструмента, разобранное безопасно (для событий со строкой). */
  static parseState(raw: string): ReturnType<typeof parseToolState> {
    return parseToolState(raw);
  }
}

/** Синглтон контроллера: переживает навигацию маршрутов. */
export const toolchain = new ToolchainController();
