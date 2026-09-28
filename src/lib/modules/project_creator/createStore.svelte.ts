import { get } from "svelte/store";
import { setContext, getContext } from "svelte";
  import { onMount, onDestroy, tick } from "svelte";
  import type { Component } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import {
    getWizardTree,
    analyzeProjectTechnologies,
    selectFolder,
    startProjectExecution,
    checkFolderExists,
    getHostPlatform,
    validateProjectStack,
    getProjectExecutionSnapshot,
  } from "$lib/modules/project_creator/api";
  import { validateStack, firstError, renderWarningPairText } from "$lib/modules/project_creator/rules";
  import {
    loadCreateSession,
    saveCreateSession,
    clearCreateSession,
  } from "$lib/modules/project_creator/createSession";
  import type {
    WizardTreeData,
    ProjectTypeDef,
    LanguageDef,
    FrameworkDef,
    ToolDef,
    ProjectPreset,
    AnalysisReport,
    WizardContext,
    ExecutionPlan,
    ExecutionEvent,
    StepStatus,
    StackIssue,
  } from "$lib/modules/project_creator/types";
  import {
    checkEnvironment as tcCheckEnvironment,
    buildInstallPlan as tcBuildPlan,
    runInstall as tcRunInstall,
    abortInstall as tcAbortInstall,
    listenToolchainEvents,
    listenInstallDone,
    listenCheckProgress,
    getNewSecrets,
    getInstallStatus,
    getToolchainMetadata,
  } from "$lib/modules/toolchain/compat";
  import type {
    EnvironmentCheck,
    InstallPlan,
    ToolchainEvent,
    TaskState,
    CheckProgressEvent,
    ProjectRequirements,
    ToolRequirement,
  } from "$lib/modules/toolchain/compat";
  import { statusKind, taskStateKind, identityMatches } from "$lib/modules/toolchain/compat";
  import TechIcon from "$lib/components/TechIcon.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import { i18n, availableLocales } from "$lib/core/i18n.svelte";
  import type { TranslationKey, Locale } from "$lib/core/i18n.svelte";
  import { confirmProjectCreatedWithProfile } from "$lib/core/integration";
  import { goto } from "$app/navigation";
  import { deleteProfile } from "$lib/modules/devlauncher/api";
  import { markProfileCreated } from "$lib/modules/devlauncher/onboarding";
  import { notifyError, notifySuccess } from "$lib/core/toasts";
  import { userExperienced, markExperienced } from "$lib/core/novice";
  import {
    markHelpDid,
    markHelpGraduated,
    helpMode,
    helpProgress,
    isHintVisible,
    HELP,
    HINT_CREATE_TYPE,
    HINT_CREATE_STACK,
    HINT_CREATE_TOOLS,
    HINT_CREATE_PREVIEW,
  } from "$lib/core/help";
  import HelpHint from "$lib/components/ui/HelpHint.svelte";

export type TooltipItem =
  | { type: 'tool'; tool: ToolDef; conflictReason?: string | null }
  | {
      type: 'framework';
      fw: FrameworkDef;
      reason?: string | null;
      altInfo?: any;
      warnReason?: string | null;
    }
  | {
      type: 'language';
      lang: LanguageDef;
      side: 'backend' | 'frontend';
      blockedReason?: string | null;
      blockedDetail?: string | null;
    };

export type ActiveTooltip = {
  x: number;
  y: number;
  placement: 'top' | 'bottom';
  item: TooltipItem;
};

export function createProjectStore() {
  
  // Toolchain: слой совместимости Project Creator (легаси-поверхность tc_*)
  
  let tree = $state<WizardTreeData | null>(null);
  let status = $state<string>("loading");
  let hostOs = $state<string>("windows");
  /** Краткое уведомление при авто-сбросе конфликтующих фреймворков */
  let dropNotice = $state<string | null>(null);
  /** Показывать ли отдельные карточки заблокированных фреймворков внутри уровня. */
  let showUnavailable = $state<Record<string, boolean>>({});
  /** То же сворачивание недоступных для «чистых» языков (стороны backend/frontend) */
  let showUnavailableLangs = $state<Record<string, boolean>>({});
  
  // ---- Ленивые секции страницы: вынесены из монолита и подгружаются
  // сразу после первого кадра (см. onMount) — первая загрузка не ждёт их. ----
  let AnalyzeMode = $state<Component<any> | null>(null);
  let PresetsMode = $state<Component<any> | null>(null);
  let EnvPanel = $state<Component<any> | null>(null);
  let ExecPanel = $state<Component<any> | null>(null);
  let DevlDialogs = $state<Component<any> | null>(null);
  let PreviewPanel = $state<Component<any> | null>(null);
  
  // ---- Mode: Constructor | Templates | Analyze ----
  let mode = $state<"constructor" | "presets" | "analyze">("constructor");
  
  // ---- Constructor context ----
  const PHASES = [i18n.t("create.phase_type"), i18n.t("create.phase_stack"), i18n.t("create.phase_review")];
  let phase = $state(0); // 0..4 — конструктор; 5 — окружение; 6 — генерация
  let selectedType = $state<ProjectTypeDef | null>(null);
  let backendLangs = $state<string[]>([]);
  let frontendLangs = $state<string[]>([]);
  /** Вручную выбранные языки сторон (без фреймворка). Авто-языки от
   *  фреймворков сюда не попадают и убираются при снятии фреймворка. */
  let manualBackendLangs = $state<string[]>([]);
  let manualFrontendLangs = $state<string[]>([]);
  let selectedFrameworks = $state<string[]>([]);
  /** Выбранный язык каждого фреймворка (fw.id → язык). Языки больше не
   *  выбираются сами по себе — они назначаются при выборе фреймворка. */
  let fwLangs = $state<Record<string, string>>({});
  let selectedTools = $state<string[]>([]);
  let testing = $state(true);
  let git = $state(true);
  let vscode = $state(true);
  
  // Живая валидация стека (зеркало бэкенда, rules.ts)
  let stackIssues = $derived<StackIssue[]>(
    tree
      ? validateStack(tree, selectedType?.id ?? null, backendLangs, frontendLangs, selectedFrameworks, selectedTools, hostOs)
      : [],
  );
  let stackError = $derived(firstError(stackIssues));
  
  /** Anchor highlights for beginner mode: the control the hint points at. */
  const stackHintVisible = $derived(
    isHintVisible(get(helpProgress), get(helpMode), HINT_CREATE_STACK),
  );
  const toolsHintVisible = $derived(
    isHintVisible(get(helpProgress), get(helpMode), HINT_CREATE_TOOLS),
  );
  const previewHintVisible = $derived(
    isHintVisible(get(helpProgress), get(helpMode), HINT_CREATE_PREVIEW),
  );
  
  /** Для новичков блокируем переход к сверке («Просмотр и создание»), пока
   *  конструктор не досмотрен до низа хотя бы раз. Флаг «опытный» глобальный
   *  (novice.ts) — после первого полного просмотра блокировка исчезает навсегда. */
  let userSeenConstructor = $state(false);
  let reviewLocked = $derived(phase === 1 && !get(userExperienced) && !userSeenConstructor);
  let scrollEl = $state<HTMLElement | null>(null);
  
  /** Проверка «долистали ли почти до самого низа». Срабатывает на прокрутку
   *  внутреннего скролл-контейнера приложения (.sp-content). Если страница не
   *  прокручивается вовсе — смысла показывать ничего нет, считаем просмотренной. */
  function checkConstructorScroll() {
    if (get(userExperienced) || userSeenConstructor) return;
    if (phase !== 1) return;
    const el = scrollEl ?? document.querySelector<HTMLElement>(".sp-content");
    if (!el) return;
    if (el.scrollHeight <= el.clientHeight + 8) {
      markConstructorSeen();
      return;
    }
    if (el.scrollTop + el.clientHeight >= el.scrollHeight - 240) {
      markConstructorSeen();
    }
  }
  
  /** Первое полное долистывание: разблокируем CTA в этой сессии и глобально
   *  помечаем пользователя «опытным» (переживает перезапуск, виден в модулях). */
  function markConstructorSeen() {
    if (userSeenConstructor) return;
    userSeenConstructor = true;
    markExperienced();
  }
  
  /** Плавный переход к последней секции стека по клику на подсказку. */
  function scrollToStackBottom() {
    const target =
      document.querySelector<HTMLElement>(".territory-tools") ??
      document.querySelector<HTMLElement>(".mega-footer");
    target?.scrollIntoView({ behavior: "smooth", block: "start" });
  }
  
  /** Тип проекта, у которого есть серверная сторона (browser-extension — нет):
   *  шаги «Backend Language» и «Backend Framework» для него скрываются. */
  let hasBackend = $derived(selectedType?.has_backend ?? true);
  
  /** Архитектурный режим выбранного стека для баннера:
   *  "integrated" — единая структура проекта (Tauri + Svelte, Qt + C++,
   *  Go + Cobra, Electron + React...), "decoupled" — два независимых проекта
   *  ./backend + ./frontend через REST/GraphQL API (NestJS + Next.js,
   *  Django + Vue, Expo + FastAPI...). null — стек ещё не определён. */
  let archMode = $derived.by<"integrated" | "decoupled" | null>(() => {
    const t = tree;
    if (!t || selectedFrameworks.length === 0) return null;
    const fws = selectedFrameworks
      .map((id) => t.frameworks.find((f) => f.id === id))
      .filter((f): f is FrameworkDef => !!f);
  
    // Легальные связки главных фреймворков (gin+cobra, axum+clap,
    // android+jetpack-compose, electron+react/vue/svelte) — единый каркас.
    if (
      t.allowed_main_pairs.some(
        (p) => selectedFrameworks.includes(p[0]) && selectedFrameworks.includes(p[1]),
      )
    ) {
      return "integrated";
    }
  
    // Универсальные фреймворки (tauri, qt) сами создают всё приложение.
    if (fws.some((f) => f.side === "either")) return "integrated";
  
    const hasBackendFw = fws.some((f) => f.side === "backend");
    const hasFrontendFw = fws.some((f) => f.side === "frontend");
    if (!hasBackendFw || !hasFrontendFw) return null;
  
    // Фронтенд — привязанный компаньон клиентской оболочки (electron→react,
    // tauri→svelte): один проект, а не два независимых.
    const linked = new Set(Object.values(linkedCompanions));
    if (fws.some((f) => f.side === "frontend" && linked.has(f.id))) return "integrated";
  
    return "decoupled";
  });
  
  // ---- README language (RU/EN, i18n) ----
  /** Язык генерируемого README (только en/ru — см. availableLocales).
   *  По умолчанию совпадает с языком интерфейса; пользователь может
   *  переключить его на финальной странице перед генерацией. */
  let readmeLocale = $state<Locale>(i18n.locale);
  /** Пользователь трогал переключатель — не следуем за сменой языка UI. */
  let readmeLocaleTouched = $state(false);
  let readmeHelpOpen = $state(false);
  
  $effect(() => {
    if (!readmeLocaleTouched) readmeLocale = i18n.locale;
  });
  
  let readmeLocaleLabel = $derived(
    availableLocales.find((l) => l.id === readmeLocale)?.index ?? readmeLocale.toUpperCase(),
  );
  
  /** Переключение языка README: идём по availableLocales (без ветвлений RU/EN). */
  function toggleReadmeLocale() {
    const ids = availableLocales.map((l) => l.id);
    const index = ids.indexOf(readmeLocale);
    readmeLocale = ids[(index + 1) % ids.length];
    readmeLocaleTouched = true;
  }
  
  /** Автоочистка backend-состояния, если серверная сторона недоступна
   *  (переключение типа проекта без бэкенда). */
  $effect(() => {
    const t = tree;
    if (!t || !selectedType) return;
    if (!hasBackend) {
      const backendFws = selectedFrameworks.filter((id) => {
        const f = t.frameworks.find((x) => x.id === id);
        return !!f && f.side === "backend";
      });
      if (backendFws.length > 0 || manualBackendLangs.length > 0) {
        if (backendFws.length > 0) {
          selectedFrameworks = selectedFrameworks.filter((id) => !backendFws.includes(id));
          const next = { ...fwLangs };
          for (const id of backendFws) delete next[id];
          fwLangs = next;
        }
        manualBackendLangs = [];
        recomputeSideLangs();
      }
    }
  });
  
  // ---- Project name & folder ----
  let projectName = $state("");
  let selectedFolder = $state<string | null>(null);
  let folderExists = $state(false);
  let folderCheckPending = $state(false);
  let showConflictDialog = $state(false);
  let conflictResolvedFolder = $state<string | null>(null);
  /** Ошибка бэкенд-валидации стека, показанная после клика по Create Project */
  let reviewError = $state<string | null>(null);
  
  /** Максимальная длина имени папки проекта: длиннее — режут файловые
   *  системы и команды создания (npm/cargo/dotnet). */
  const PROJECT_NAME_MAX = 60;
  
  /** Зарезервированные имена Windows: папку с таким именем создать нельзя
   *  (или она ведёт к системному устройству) даже с расширением. */
  const WINDOWS_RESERVED_NAMES = new Set([
    "con", "prn", "aux", "nul",
    "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9",
    "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
  ]);
  
  /** Ошибка имени НОВОЙ папки проекта (null — валидно). Проверяется только
   *  создаваемый сегмент: родительский путь может содержать кириллицу,
   *  пробелы и спецсимволы — такие папки уже существуют, и в них создавать
   *  можно. Имя же попадает в package.json, Cargo.toml, docker-compose и
   *  команды сборки, поэтому допускается лишь безопасный ASCII-набор. */
  let projectNameError = $derived.by<string | null>(() => {
    if (!projectName) return null;
    if (projectName.length > PROJECT_NAME_MAX) {
      return i18n.t("create.name_too_long", { n: PROJECT_NAME_MAX });
    }
    if (!/^[A-Za-z0-9._-]+$/.test(projectName)) {
      return i18n.t("create.name_invalid");
    }
    if (projectName.startsWith(".") || projectName.endsWith(".")) {
      return i18n.t("create.name_dots");
    }
    if (WINDOWS_RESERVED_NAMES.has(projectName.toLowerCase())) {
      return i18n.t("create.name_reserved", { name: projectName });
    }
    return null;
  });
  
  // ---- Analysis ----
  let analysisResult = $state<AnalysisReport | null>(null);
  let analysisError = $state<string | null>(null);
  let analyzing = $state(false);
  let analyzedPath = $state<string | null>(null);
  
  // ---- Execution ----
  let execPlan = $state<ExecutionPlan | null>(null);
  let execProjectPath = $state<string | null>(null);
  let execStatuses = $state<Map<number, { name: string; status: StepStatus; logs: string[] }>>(new Map());
  let execOverallStatus = $state<string>("pending");
  let execResult = $state<{ duration: number; status: string } | null>(null);
  let execError = $state<string | null>(null);
  let execLogs = $state<string[]>([]);
  let unlisten: (() => void) | null = null;
  
  /** Опциональные шаги, которые пользователь удалил в предпросмотре
   *  (git_*, vscode_*, readme...). Живут на странице, передаются в
   *  предпросмотр и в выполнение — генерация идёт по той же схеме. */
  let removedStepIds = $state<string[]>([]);
  
  // ---- Toolchain Environment ----
  let envCheck = $state<EnvironmentCheck | null>(null);
  let envChecking = $state(false);
  let envError = $state<string | null>(null);
  let envPlan = $state<InstallPlan | null>(null);
  let envInstalling = $state(false);
  let envLogs = $state<string[]>([]);
  let envTaskStates = $state<Map<string, TaskState>>(new Map());
  let envRestartHint = $state(false);
  let envInstallDone = $state(false);
  let envDownload = $state<Map<string, { received: number; total: number }>>(new Map());
  let envErrors = $state<string[]>([]);
  let envPhaseStart = $state<Map<string, number>>(new Map());
  /** Мгновенная скорость скачивания каждой задачи (байт/с), вычисляется
   *  из дельты событий tc:dl — панель показывает её рядом с прогрессом. */
  let envSpeed = $state<Map<string, number>>(new Map());
  /** Последний (received, ts) на задачу: сырьё для расчёта envSpeed. */
  let envDlPrev = $state<Map<string, { received: number; ts: number }>>(new Map());
  /** Фоновая перепроверка окружения после установки (см. autoRecheckAfterInstall):
   *  экран «установка завершена» остаётся, пока проверка не подтвердит тулы. */
  let envRechecking = $state(false);
  let newSecrets = $state<Record<string, string> | null>(null);
  let secretCopied = $state<string | null>(null);
  let unlistenTc: (() => void) | null = null;
  let unlistenTcDone: (() => void) | null = null;
  let unlistenTcCheck: (() => void) | null = null;
  let envCheckProgress = $state<CheckProgressEvent[]>([]);
  /** Идентификатор текущего запуска проверки окружения: события с чужим
   *  scan_id (поздние «хвосты» прежнего запуска) в список не попадают. */
  let envCheckScanId = $state<string | null>(null);
  let envSelectedIds = $state<Set<string>>(new Set());
  /** Docker-инструменты мастера (postgresql, redis, ...), выбранные для
   * локальной установки вместо docker-compose. Наполняется кнопкой
   * «Install locally» в опциональной секции экрана окружения. */
  let envLocalInfra = $state<Set<string>>(new Set());
  /** Общий стейт приложения: инструменты, уже установленные локально
   *  (state.json toolchain). Панды окружения показывают их «уже готовыми»
   *  и предлагают «Связать с локальным» вместо повторной установки. */
  let installedTools = $state<Set<string>>(new Set());
  
  // ---- Integration: DevLauncher profile ----
  let devlProfileCreated = $state(false);
  /** Показать ли плашку «Ниже ещё есть контент!» в диалоге: только при
   *  первом АВТО-открытии после генерации, не при ручном повторном открытии. */
  let devlShowReminder = $state(false);
  let devlProfileName = $state<string | null>(null);
  let devlProfilePath = $state<string | null>(null);
  /** Реально существует ли профиль в DevLauncher (после создания/удаления). */
  let devlProfileExists = $state(false);
  let devlConfirmCancel = $state(false);
  /** Авто-окно DevLauncher уже показывалось для текущего выполнения проекта.
   *  Сохраняется в снапшоте: повторный показ при восстановлении вкладки
   *  (переключение маршрутов → реплей событий) невозможен. */
  let devlAutoPopupShown = $state(false);
  
  // Тикер установки/проверки: раз в секунду обновляет envNow, чтобы панель
  // окружения перерисовывалась «вживую» — таймеры фаз и скорость
  // скачивания не застывают на последнем событии, а идут в реальном
  // времени до завершения операции.
  let envNow = $state(Date.now());
  let envTicker: ReturnType<typeof setInterval> | null = null;
  function startTick() {
    if (envTicker) return;
    envNow = Date.now();
    envTicker = setInterval(() => {
      envNow = Date.now();
    }, 1000);
  }
  function stopTick() {
    if (envTicker) {
      clearInterval(envTicker);
      envTicker = null;
    }
  }

  let tooltipData = $state<ActiveTooltip | null>(null);
  let tooltipTimer: ReturnType<typeof setTimeout> | null = null;
  
  function showTooltip(item: ToolDef | TooltipItem, e: MouseEvent | FocusEvent) {
    if (tooltipTimer) clearTimeout(tooltipTimer);
    const target = e.currentTarget as HTMLElement | null;
    if (!target) return;

    const normalizedItem: TooltipItem =
      'requires' in item && 'category' in item && !('type' in item)
        ? { type: 'tool', tool: item as ToolDef }
        : (item as TooltipItem);

    tooltipTimer = setTimeout(() => {
      if (!target.isConnected) return;
      const rect = target.getBoundingClientRect();
      if (!rect || (rect.width === 0 && rect.height === 0)) return;

      const tooltipWidth = 300;
      let x = rect.left + rect.width / 2;
      const minX = tooltipWidth / 2 + 12;
      const maxX = window.innerWidth - tooltipWidth / 2 - 12;
      x = Math.max(minX, Math.min(x, maxX));

      const spaceBelow = window.innerHeight - rect.bottom;
      const spaceAbove = rect.top;
      let placement: 'top' | 'bottom' = 'bottom';
      let y = rect.bottom + 8;

      if (spaceBelow < 240 && spaceAbove > spaceBelow) {
        placement = 'top';
        y = rect.top - 8;
      }

      tooltipData = { x, y, placement, item: normalizedItem };
    }, 120);
  }
  
  function hideTooltip() {
    if (tooltipTimer) {
      clearTimeout(tooltipTimer);
      tooltipTimer = null;
    }
    tooltipData = null;
  }
  
  // ----------------------------------------------------------
  // Персистентность вкладки: переключение маршрутов не убивает прогресс
  // ----------------------------------------------------------
  
  /** false, пока не восстановлено сохранённое состояние — автозейв отключён,
   *  чтобы первое срабатывание эффекта не затёрло снапшот до восстановления. */
  let persistReady = $state(false);
  /** id типа проекта из снапшота — резолвится в объект после загрузки дерева */
  let restoredTypeId: string | null = null;
  let persistTimer: ReturnType<typeof setTimeout> | null = null;
  
  /** Сериализуемое состояние вкладки. ТОЛЬКО лёгкие поля мастера: тип проекта,
   *  языки, фреймворки, инструменты, текущий шаг + мелкие строки/булевы.
   *  Тяжёлые данные (логи, execution_events, env-снапшоты) здесь не живут —
   *  они на бэкенде и восстанавливаются reSyncLiveSessions(). Это же
   *  гарантирует createSession.saveCreateSession (whitelist). */
  function buildSnapshot(): Record<string, unknown> {
    return {
      v: 1,
      mode,
      phase,
      typeId: selectedType?.id ?? null,
      backendLangs,
      frontendLangs,
      manualBackendLangs,
      manualFrontendLangs,
      selectedFrameworks,
      fwLangs,
      linkedCompanions,
      qtUiMode,
      qtWebLinked,
      selectedTools,
      testing,
      git,
      vscode,
      readmeLocale,
      projectName,
      selectedFolder,
      conflictResolvedFolder,
      folderExists,
      removedStepIds,
      envLocalInfra: [...envLocalInfra],
      envSelectedIds: [...envSelectedIds],
      envInstalling,
      envInstallDone,
      execOverallStatus,
      execProjectPath: execPlan?.project_path ?? null,
      execPlan,
      execResult,
      devlAutoPopupShown,
    };
  }
  
  function restoreSnapshot(snap: Record<string, unknown>) {
    const s = snap;
    const str = (v: unknown): string => (typeof v === "string" ? v : "");
    const bool = (v: unknown): boolean => v === true;
    const strArr = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x) => typeof x === "string") : []);
    const num = (v: unknown): number => (typeof v === "number" ? v : 0);
  
    // Анализ скрыт (см. кнопку в шапке): сохранённый режим "analyze"
    // безопасно приводим к конструктору, чтобы не показывать пустую вкладку.
    mode = (["constructor", "presets"] as const).includes(s.mode as never)
      ? (s.mode as "constructor" | "presets")
      : "constructor";
    phase = num(s.phase);
    restoredTypeId = str(s.typeId) || null;
    backendLangs = strArr(s.backendLangs);
    frontendLangs = strArr(s.frontendLangs);
    manualBackendLangs = strArr(s.manualBackendLangs);
    manualFrontendLangs = strArr(s.manualFrontendLangs);
    selectedFrameworks = strArr(s.selectedFrameworks);
    fwLangs = s.fwLangs && typeof s.fwLangs === "object" ? (s.fwLangs as Record<string, string>) : {};
    linkedCompanions =
      s.linkedCompanions && typeof s.linkedCompanions === "object"
        ? (s.linkedCompanions as Record<string, string>)
        : {};
    qtUiMode =
      s.qtUiMode && typeof s.qtUiMode === "object"
        ? (s.qtUiMode as Record<string, string>)
        : {};
    qtWebLinked =
      s.qtWebLinked && typeof s.qtWebLinked === "object"
        ? (s.qtWebLinked as Record<string, string>)
        : {};
    selectedTools = strArr(s.selectedTools);
    testing = bool(s.testing);
    git = bool(s.git);
    vscode = bool(s.vscode);
    const restoredReadmeLocale = str(s.readmeLocale) as Locale;
    if ((availableLocales.map((l) => l.id) as string[]).includes(restoredReadmeLocale)) {
      readmeLocale = restoredReadmeLocale;
      readmeLocaleTouched = true;
    }
    projectName = str(s.projectName);
    selectedFolder = str(s.selectedFolder) || null;
    conflictResolvedFolder = str(s.conflictResolvedFolder) || null;
    folderExists = bool(s.folderExists);
    removedStepIds = strArr(s.removedStepIds);
    analysisResult = (s.analysisResult as AnalysisReport | null) ?? null;
    analysisError = str(s.analysisError) || null;
    analyzedPath = str(s.analyzedPath) || null;
    execPlan = (s.execPlan as ExecutionPlan | null) ?? null;
    execProjectPath = str(s.execProjectPath) || null;
    execStatuses = new Map(
      Array.isArray(s.execStatuses)
        ? (s.execStatuses as [number, { name: string; status: StepStatus; logs: string[] }][])
        : [],
    );
    execOverallStatus = str(s.execOverallStatus) || "pending";
    execResult = (s.execResult as { duration: number; status: string } | null) ?? null;
    execError = str(s.execError) || null;
    execLogs = strArr(s.execLogs);
    devlAutoPopupShown = bool(s.devlAutoPopupShown);
    envCheck = (s.envCheck as EnvironmentCheck | null) ?? null;
    envPlan = (s.envPlan as InstallPlan | null) ?? null;
    envLogs = strArr(s.envLogs);
    envTaskStates = new Map(
      Array.isArray(s.envTaskStates) ? (s.envTaskStates as [string, TaskState][]) : [],
    );
    envRestartHint = bool(s.envRestartHint);
    envInstallDone = bool(s.envInstallDone);
    envErrors = strArr(s.envErrors);
    envDownload = new Map(
      Array.isArray(s.envDownload) ? (s.envDownload as [string, { received: number; total: number }][]) : [],
    );
    envPhaseStart = new Map(Array.isArray(s.envPhaseStart) ? (s.envPhaseStart as [string, number][]) : []);
    envSelectedIds = new Set(strArr(s.envSelectedIds));
    envLocalInfra = new Set(strArr(s.envLocalInfra));
    envCheckProgress = Array.isArray(s.envCheckProgress) ? (s.envCheckProgress as CheckProgressEvent[]) : [];
    // Секреты НАМЕРЕННО не восстанавливаются из снапшота: они не входят в
    // whitelist sessionStorage и выдаются только одноразовым getNewSecrets()
    // сразу после успешной установки (см. fetchNewSecrets).
    envInstalling = bool(s.envInstalling);
  }
  
  /** Автозейв с дебаунсом: логогенерация (установка/генерация) не спамит storage.
   *  Во время стриминга (phase 5/6) персист отключён вовсе — состояние
   *  восстанавливается из бэкенд-сессии (reSyncLiveSessions), а финальные
   *  точки (завершение установки/генерации) сами вызывают persistNow(). */
  $effect(() => {
    if (!persistReady) return;
    if (phase === 5 || phase === 6) return;
    const snap = buildSnapshot();
    if (persistTimer) clearTimeout(persistTimer);
    persistTimer = setTimeout(() => saveCreateSession(snap), 300);
  });
  
  function persistNow() {
    if (persistTimer) clearTimeout(persistTimer);
    persistTimer = null;
    if (persistReady) saveCreateSession(buildSnapshot());
  }
  
  /** Синхронизация «живых» сессий (установка окружения / генерация проекта),
   *  которые могли завершиться на бэкенде, пока вкладка была неактивна.
   *  Тяжёлые данные в sessionStorage не хранятся (см. createSession.ts) —
   *  после восстановления лёгкого снапшота состояние пересобирается отсюда:
   *  установочная сессия с бэкенда, генерация из EXECUTION_SNAPSHOT,
   *  либо обычная проверка окружения заново. */
  async function reSyncLiveSessions() {
    if (phase === 5) {
      try {
        const session = await getInstallStatus();
        if (session) {
          envPlan = session.plan;
          envTaskStates = new Map(session.plan.tasks.map((t) => [t.task_id, t.state]));
          if (session.running) {
            envInstalling = true;
            startTick();
            if (unlistenTc) unlistenTc();
            if (unlistenTcDone) unlistenTcDone();
            unlistenTc = await listenToolchainEvents(handleToolchainEvent);
            unlistenTcDone = await listenInstallDone(handleInstallDone);
          } else {
            envInstalling = false;
            envInstallDone = true;
            stopTick();
            // Секреты сессией больше не отдаются вовсе (serde skip на
            // бэкенде): единственный канал — одноразовый getNewSecrets().
            // Установка завершилась, пока вкладка была неактивна, — свежая
            // проверка в фоне переведёт поставленные тулы в «готово» без
            // ручного «Перепроверить окружение».
            void autoRecheckAfterInstall();
          }
        } else if (envInstalling) {
          envInstalling = false;
          stopTick();
        }
      } catch {
        // сессия недоступна — оставляем состояние из снапшота
      }
      if (!envInstalling) {
        // Слушатель прогресса проверки вешаем всегда — он понадобится при
        // ручном запуске. Полную проверку окружения НЕ запускаем автоматически:
        // она длится ~10 секунд и при восстановлении вкладки выглядит как
        // «загрузка» — пользователь запускает её кнопкой на экране окружения.
        try {
          if (!unlistenTcCheck) {
            unlistenTcCheck = await listenCheckProgress(handleCheckProgress);
          }
        } catch {
          // ignore
        }
      }
    }
    if (phase === 6 && execOverallStatus !== "pending") {
      try {
        const snap = await getProjectExecutionSnapshot();
        if (snap) {
          for (const ev of snap.events) handleExecEvent(ev);
          if (snap.running) {
            if (unlisten) unlisten();
            unlisten = await listen<ExecutionEvent>("project_creator:step_event", (e) => {
              handleExecEvent(e.payload);
            });
          } else if (execOverallStatus === "running") {
            execOverallStatus = "error";
            execError = i18n.t("create.interrupted");
          }
        }
      } catch {
        // снапшот недоступен — переподключаемся к событиям
        try {
          if (unlisten) unlisten();
          unlisten = await listen<ExecutionEvent>("project_creator:step_event", (e) => {
            handleExecEvent(e.payload);
          });
        } catch {
          // ignore
        }
      }
    }
  }
  
  onMount(async () => {
    // Даём первому кадру (скелетон) отрисоваться, прежде чем запускать
    // тяжёлые вычисления — пользователь сразу видит анимацию загрузки
    // вместо замёрзшего экрана.
    await tick();
  
    // Блокировка CTA для новичков: следим за прокруткой конструктора.
    scrollEl = document.querySelector<HTMLElement>(".sp-content");
    window.addEventListener("scroll", checkConstructorScroll, { capture: true, passive: true });
    checkConstructorScroll();
  
    // Вынесенные секции подгружаем сразу после первого кадра: они не входят
    // в бандл первой загрузки, но успевают загрузиться, пока пользователь
    // дойдёт до фаз 5/6 или режимов presets/analyze.
    // Анализ (AnalyzeMode) сейчас не используется и НЕ подгружается —
    // чтобы вернуть, раскомментируйте строку ниже и кнопку в шапке.
    void Promise.all([
      import("$lib/components/project-creator/PresetsMode.svelte"),
      import("$lib/components/project-creator/EnvironmentPanel.svelte"),
      import("$lib/components/project-creator/ExecutionPanel.svelte"),
      import("$lib/components/project-creator/DevLauncherDialogs.svelte"),
      import("$lib/components/project-creator/ProjectPreview.svelte"),
    ]).then(([presets, env, exec, devl, preview]) => {
      PresetsMode = presets.default;
      EnvPanel = env.default;
      ExecPanel = exec.default;
      DevlDialogs = devl.default;
      PreviewPanel = preview.default;
    });
  
    const saved = loadCreateSession();
    if (saved) {
      try {
        restoreSnapshot(saved);
      } catch (e) {
        console.error("[create] failed to restore session:", e);
      }
    }
    // Ещё один tick чтобы снапшот отрисовался перед IPC-вызовами.
    await tick();
  
    // Дерево и ОС независимы — грузим параллельно, не блокируя друг друга.
    await Promise.allSettled([
      (async () => {
        try {
          tree = await getWizardTree();
          status = tree.project_types.length > 0 ? "ready" : "empty";
          if (restoredTypeId) {
            selectedType = tree.project_types.find((pt) => pt.id === restoredTypeId) ?? null;
          }
        } catch (e) {
          status = "error";
          console.error(e);
        }
      })(),
    ]);
    // Не задерживаем первый интерактивный кадр маршрута восстановлением живых
    // сессий и проверкой toolchain: эти вызовы могут обращаться к Tauri долго.
    // После отрисовки конструктора выполняем их в фоне. ОС (getHostPlatform)
    // нужна только для блокировок фреймворков на фазе стека — тоже фон.
    queueMicrotask(async () => {
      await reSyncLiveSessions();
      await refreshInstalledTools();
      persistReady = true;
    });
    void (async () => {
      try {
        hostOs = await getHostPlatform();
      } catch (e) {
        console.error("cannot detect host OS:", e);
      }
    })();
  });
  
  onDestroy(() => {
    window.removeEventListener("scroll", checkConstructorScroll, { capture: true });
    persistNow();
    if (unlisten) unlisten();
    if (unlistenTc) unlistenTc();
    if (unlistenTcDone) unlistenTcDone();
    if (unlistenTcCheck) unlistenTcCheck();
    stopTick();
  });
  
  // ----------------------------------------------------------
  // Фреймворки
  // ----------------------------------------------------------
  
  function availableFrameworks(): FrameworkDef[] {
    if (!tree || !selectedType) return [];
    return tree.frameworks.filter(
      (f) => !f.project_types?.length || f.project_types.includes(selectedType!.id),
    ).filter((f) => !isUiVariant(f));
  }
  
  /** Уровень фреймворка для группировки в колонках:
   *  "full" — standalone-приложения (spring-boot, django, nextjs),
   *  "inplace" — лёгкие фреймворки внутрь базового проекта (fastapi, express),
   *  "side" — побочные библиотеки (aiogram, telegraf). */
  function fwLevelOf(fw: FrameworkDef): "full" | "inplace" | "side" {
    if (fw.kind === "side") return "side";
    return fw.class === "standalone" ? "full" : "inplace";
  }
  
  /** Уровни-колонки внутри сторон (порядок показа) */
  const FW_LEVELS = [
    {
      id: "full",
      title: i18n.t("create.fw_full_apps"),
      note: i18n.t("create.fw_full_apps_desc"),
    },
    {
      id: "inplace",
      title: i18n.t("create.fw_inplace"),
      note: i18n.t("create.fw_inplace_desc"),
    },
    {
      id: "side",
      title: i18n.t("create.fw_side"),
      note: i18n.t("create.fw_side_desc"),
    },
  ] as const;
  
  /** Пересчёт языков сторон: ручной выбор + языки выбранных фреймворков.
   *  Языки, добавленные только фреймворком, исчезают при его снятии —
   *  ручные остаются всегда.
   *
   *  Жёсткая консистентность: сторона с выбранным фреймворком НЕ хранит
   *  ручных языков — фреймворк диктует свой язык. Это инвариант чинится
   *  здесь же, поэтому пресеты и восстановленные снапшоты со старым
   *  (битым) состоянием (JS + TS на одной стороне) автоматически
   *  приводятся к консистентному виду. */
  function recomputeSideLangs() {
    const t = tree;
    if (!t) return;
    const backend = new Set(manualBackendLangs);
    const frontend = new Set(manualFrontendLangs);
    let backendHasFw = false;
    let frontendHasFw = false;
    for (const id of selectedFrameworks) {
      const fw = t.frameworks.find((f) => f.id === id);
      if (!fw) continue;
      const lang = fwLangs[id] ?? fw.recommended_language;
      if (!lang) continue;
      if (fw.side === "frontend") {
        frontendHasFw = true;
        frontend.add(lang);
      } else {
        backendHasFw = true;
        backend.add(lang);
      }
    }
    if (frontendHasFw && manualFrontendLangs.length > 0) manualFrontendLangs = [];
    if (backendHasFw && manualBackendLangs.length > 0) manualBackendLangs = [];
    backendLangs = [...backend];
    frontendLangs = [...frontend];
  }
  
  /** Выбор языка стороны вручную (radio): новый язык заменяет старый,
   *  повторный клик по выбранному очищает сторону. */
  function toggleLang(side: "backend" | "frontend", id: string) {
    if (side === "backend") {
      manualBackendLangs = manualBackendLangs.includes(id) ? [] : [id];
    } else {
      manualFrontendLangs = manualFrontendLangs.includes(id) ? [] : [id];
    }
    recomputeSideLangs();
  }
  
  /** Пара фреймворков объявлена легальной связкой (wizard_tree.allowed_main_pairs) */
  function isAllowedPair(a: string, b: string): boolean {
    return (
      tree?.allowed_main_pairs.some(
        (p) => (p[0] === a && p[1] === b) || (p[0] === b && p[1] === a),
      ) ?? false
    );
  }
  
  /** Фреймворк не занимает лимит «одного главного на сторону» (zig-cli) */
  function isMainLimitExempt(id: string): boolean {
    return tree?.main_limit_exempt.includes(id) ?? false;
  }
  
  /** Объяснение конфликта (conflict_notes) в обе стороны.
   *  Значения conflict_notes — i18n-ключи, переводим один раз через i18n.t(). */
  function conflictNoteOf(fw: FrameworkDef, other: FrameworkDef): string | undefined {
    const key = fw.conflict_notes?.[other.id] ?? other.conflict_notes?.[fw.id];
    if (!key) return undefined;
    return i18n.t(key as TranslationKey);
  }
  
  /** Причина, по которой фреймворк нельзя выбрать (зеркало правил rules.ts) */
  function frameworkBlockReason(fwId: string): string | null {
    return frameworkBlockInfo(fwId)?.message ?? null;
  }
  
  function unavailableFrameworks(items: FrameworkDef[]): FrameworkDef[] {
    return items.filter((fw) => frameworkBlockInfo(fw.id) !== null);
  }
  
  function availableFrameworksForDisplay(items: FrameworkDef[], levelKey: string): FrameworkDef[] {
    const unavailable = unavailableFrameworks(items);
    return showUnavailable[levelKey]
      ? items
      : items.filter((fw) => !unavailable.includes(fw));
  }
  
  function toggleUnavailable(levelKey: string) {
    showUnavailable = { ...showUnavailable, [levelKey]: !showUnavailable[levelKey] };
  }
  
  type BlockInfo = {
    message: string;
    /** Развёрнутое объяснение «почему» (строка под бейджем) */
    detail: string;
    /** Рекомендуемые альтернативы (id фреймворков той же стороны) */
    alternatives: string[];
  };
  
  /** Совместимые альтернативы для заблокированного фреймворка */
  function frameworkAlternatives(fw: FrameworkDef): string[] {
    const t = tree;
    if (!t) return [];
    const sideLangs = fw.side === "frontend" ? frontendLangs : backendLangs;
    return t.frameworks
      .filter((c) => {
        if (c.id === fw.id || selectedFrameworks.includes(c.id)) return false;
        if (c.side !== fw.side) return false;
        // Язык совместим со стороной
        const langOk =
          c.side === "either"
            ? [...backendLangs, ...frontendLangs].some((l) => c.languages.includes(l))
            : sideLangs.some((l) => c.languages.includes(l)) ||
              c.languages.includes(c.recommended_language);
        if (!langOk) return false;
        // Не конфликтует с текущим выбором и не «второй главный» на стороне
        for (const sId of selectedFrameworks) {
          const sel = t.frameworks.find((f) => f.id === sId);
          if (!sel) continue;
          if (sel.conflicts?.includes(c.id) || c.conflicts?.includes(sel.id)) return false;
          if (
            c.kind === "app" &&
            c.side !== "either" &&
            sel.kind === "app" &&
            sel.side === c.side &&
            !isAllowedPair(c.id, sel.id) &&
            !isMainLimitExempt(c.id)
          ) {
            return false;
          }
        }
        return true;
      })
      .map((c) => c.id)
      .slice(0, 4);
  }
  
  /** Структурированная причина блокировки: сообщение + объяснение + альтернативы */
  function frameworkBlockInfo(fwId: string): BlockInfo | null {
    const fw = tree?.frameworks.find((f) => f.id === fwId);
    if (!fw || selectedFrameworks.includes(fwId)) return null;
  
    // 1. Платформа
    if (fw.platforms?.length && !fw.platforms.includes(hostOs)) {
      return {
        message: i18n.t("create.block.platform_msg", { platforms: fw.platforms.join(", ") }),
        detail: i18n.t("create.block.platform_detail", {
          label: i18n.t(fw.label as TranslationKey),
          os: hostOs,
        }),
        alternatives: frameworkAlternatives(fw),
      };
    }
  
    // 2. Взаимные конфликты (conflicts + conflict_notes)
    for (const selId of selectedFrameworks) {
      const sel = tree?.frameworks.find((f) => f.id === selId);
      if (!sel) continue;
      if (sel.conflicts?.includes(fwId) || fw.conflicts?.includes(selId)) {
        const note = conflictNoteOf(fw, sel);
        const fwLabel = i18n.t(fw.label as TranslationKey);
        const selLabel = i18n.t(sel.label as TranslationKey);
        return {
          message: i18n.t("create.block.conflict_msg", { label: selLabel }),
          detail: note
            ? i18n.t("create.block.conflict_detail", { a: fwLabel, b: selLabel, note })
            : i18n.t("create.block.conflict_detail_plain", { a: fwLabel, b: selLabel }),
          alternatives: frameworkAlternatives(fw),
        };
      }
    }
  
    // 3. Универсальные (tauri/electron) против конкретной стороны
    const hasSpecific = selectedFrameworks.some((id) => {
      const f = tree?.frameworks.find((x) => x.id === id);
      return !!f && (f.side === "backend" || f.side === "frontend");
    });
    const hasEither = selectedFrameworks.some((id) => {
      const f = tree?.frameworks.find((x) => x.id === id);
      return !!f && f.side === "either";
    });
    if (fw.side === "either" && hasSpecific) {
      const fwLabel = i18n.t(fw.label as TranslationKey);
      return {
        message: i18n.t("create.block.either_vs_specific_msg"),
        detail:
          fw.id === "qt"
            ? i18n.t("create.block.either_vs_specific_detail_qt")
            : i18n.t("create.block.either_vs_specific_detail", { label: fwLabel }),
        alternatives: frameworkAlternatives(fw),
      };
    }
    if (fw.side !== "either" && hasEither) {
      const eitherFw = selectedFrameworks.find((id) => {
        const f = tree?.frameworks.find((x) => x.id === id);
        return !!f && f.side === "either";
      });
      const eitherDef = tree?.frameworks.find((x) => x.id === eitherFw);
      const eitherLabel = eitherDef ? i18n.t(eitherDef.label as TranslationKey) : "desktop";
      return {
        message: i18n.t("create.block.specific_vs_either_msg", { label: eitherLabel }),
        detail:
          eitherFw === "qt" && (fw.id === "react" || fw.id === "vue" || fw.id === "svelte")
            ? i18n.t("create.block.specific_vs_either_detail_qt")
            : i18n.t("create.block.specific_vs_either_detail", { label: eitherLabel }),
        alternatives: frameworkAlternatives(fw),
      };
    }
  
    // 4. Один главный фреймворк на сторону (с исключениями allowed_main_pairs/main_limit_exempt)
    if (fw.side === "backend" || fw.side === "frontend") {
      if (fw.kind === "app" && !isMainLimitExempt(fw.id)) {
        const sameSide = selectedFrameworks.find((id) => {
          const f = tree?.frameworks.find((x) => x.id === id);
          return !!f && f.kind === "app" && f.side === fw.side && !isAllowedPair(fw.id, id);
        });
        if (sameSide) {
          const selDef = tree?.frameworks.find((x) => x.id === sameSide);
          const selLabel = selDef ? i18n.t(selDef.label as TranslationKey) : sameSide;
          const sideLabel = i18n
            .t(fw.side === "backend" ? "create.backend" : "create.frontend")
            .toLowerCase();
          return {
            message: i18n.t("create.block.one_main_msg", { side: sideLabel }),
            detail: i18n.t("create.block.one_main_detail", {
              a: selLabel,
              b: i18n.t(fw.label as TranslationKey),
            }),
            alternatives: frameworkAlternatives(fw),
          };
        }
      }
      // 5. Язык
      const langs = fw.side === "backend" ? backendLangs : frontendLangs;
      if (langs.length > 0 && !fw.languages.some((l) => langs.includes(l))) {
        const langNames = fw.languages.map((l) => langLabel(l));
        const sideLabel = i18n
          .t(fw.side === "backend" ? "create.backend" : "create.frontend")
          .toLowerCase();
        return {
          message: i18n.t("create.block.lang_msg", { langs: langNames.join(", ") }),
          detail: i18n.t("create.block.lang_detail", {
            side: sideLabel,
            langs: langNames.join(i18n.t("create.block.lang_sep")),
            label: i18n.t(fw.label as TranslationKey),
          }),
          alternatives: frameworkAlternatives(fw),
        };
      }
    }
    return null;
  }
  
  /** Предупреждение (не блокировка): Phoenix LiveView + тяжёлый SPA,
   *  два full-stack фреймворка, backend + Electron (warning_pairs).
   *  Плейсхолдеры {a}/{b}/{a_lang} подставляются label'ами. */
  function frameworkWarnReason(fwId: string): string | null {
    const t = tree;
    if (!t || !selectedFrameworks.includes(fwId)) return null;
    for (const wp of t.warning_pairs ?? []) {
      if ((wp.b === fwId && selectedFrameworks.includes(wp.a)) || (wp.a === fwId && selectedFrameworks.includes(wp.b))) {
        const a = t.frameworks.find((f) => f.id === wp.a);
        const b = t.frameworks.find((f) => f.id === wp.b);
        if (!a || !b) continue;
        return renderWarningPairText(t, wp.reason, a, b);
      }
    }
    return null;
  }
  
  /** Жёсткий сброс ручных языков стороны при выборе фреймворка: у стороны
   *  теперь есть привязанный язык фреймворка, поэтому старый массив
   *  (frontend_lang/backend_lang) очищается ДО записи нового — в стейте
   *  никогда не окажется двух языков одной стороны (JS + TS из-за Vue). */
  function resetSideLangsForFramework(fw: FrameworkDef) {
    if (fw.side === "frontend") {
      manualFrontendLangs = [];
    } else if (fw.side === "backend") {
      manualBackendLangs = [];
    } else {
      manualFrontendLangs = [];
      manualBackendLangs = [];
    }
  }
  
  /** Клик по карточке фреймворка: выбрать (или открыть настройки, если выбран).
   *  Мультиязычные фреймворки открывают попап выбора языка; однозназычные —
   *  выбираются сразу с их языком; повторный клик снимает одиночные. */
  function clickFramework(id: string) {
    const fw = tree?.frameworks.find((f) => f.id === id);
    if (!fw) return;
    if (selectedFrameworks.includes(id)) {
      if (fw.languages.length > 1 || companionOptions(fw).length > 0) {
        openFwPopup(id);
      } else {
        removeFramework(id);
      }
      return;
    }
    if (frameworkBlockReason(id)) return;
    const dropConflicts = fw.conflicts ?? [];
    const dropped = selectedFrameworks.filter((f) => dropConflicts.includes(f));
    if (dropped.length > 0) {
      const note = dropped
        .map((x) => {
          const cfw = tree?.frameworks.find((f) => f.id === x);
          if (!cfw) return x;
          const cnote = conflictNoteOf(fw, cfw);
          const label = i18n.t(cfw.label as TranslationKey);
          return cnote ? `${label} (${cnote})` : label;
        })
        .join(", ");
      dropNotice = i18n.t("create.drop_notice", { note });
      window.setTimeout(() => (dropNotice = null), 6000);
    }
    if (companionOptions(fw).length > 0 && !linkedCompanions[id]) {
      const first = companionOptions(fw)[0];
      addCompanion(fw.id, first.id);
    }
    selectedFrameworks = [...selectedFrameworks.filter((f) => !dropConflicts.includes(f)), id];
    fwLangs = { ...fwLangs, [id]: fwLangs[id] ?? fw.recommended_language };
    resetSideLangsForFramework(fw);
    recomputeSideLangs();
    if (fw.languages.length > 1 || companionOptions(fw).length > 0) {
      openFwPopup(id);
    }
  }
  
  /** Сразу добавить рекомендованный фреймворк (как обычный клик по карточке) */
  function applyRecommendedFramework(id: string) {
    clickFramework(id);
  }
  
  // ----------------------------------------------------------
  // Попап настройки фреймворка (язык + подфреймворк)
  // ----------------------------------------------------------
  
  /** id фреймворка с открытым попапом (null = закрыт) */
  let fwPopup = $state<string | null>(null);
  /** Черновой выбор языка в попапе */
  let popupLang = $state<string | null>(null);
  /** Черновой выбор подфреймворка (для tauri-подобных: фронтовая часть) */
  let popupCompanion = $state<string | null>(null);
  /** Черновой выбор языка подфреймворка */
  let popupCompanionLang = $state<string | null>(null);
  /** Черновой выбор UI-технологии Qt в попапе (id режима из qt_ui_options) */
  let popupQtUi = $state<string | null>(null);
  /** Черновой выбор веб-фреймворка внутри Qt WebEngine */
  let popupWebFw = $state<string | null>(null);
  /** Связки «владелец → подфреймворк» (tauri → svelte): удаление владельца тянет подфреймворк */
  let linkedCompanions = $state<Record<string, string>>({});
  /** Выбранная UI-технология Qt (id режима из qt_ui_options) для каждого qt */
  let qtUiMode = $state<Record<string, string>>({});
  /** Веб-фреймворк внутри Qt WebEngine (qt → react/vue/svelte) */
  let qtWebLinked = $state<Record<string, string>>({});
  
  /** Фреймворк — UI-вариант другого фреймворка (qt → qt-qml/qt-widgets/...).
   *  Такие не показываются как самостоятельные карточки — живут в попапе владельца. */
  function isUiVariant(fw: FrameworkDef): boolean {
    if (!tree) return false;
    return tree.frameworks.some((f) => (f.qt_ui_options ?? []).some((m) => m.id === fw.id));
  }
  
  /** Подфреймворки для side="either" (tauri): совместимые фронтовые приложения.
   *  Сознательное ограничение: десктопные оболочки работают только со
   *  Svelte/Vue/React — полнофреймворковые Next/Nuxt/SvelteKit в них не живут. */
  const EITHER_COMPANIONS = new Set(["svelte", "vue", "react"]);
  
  function companionOptions(fw: FrameworkDef): FrameworkDef[] {
    if (!tree) return [];
    // Фреймворки с собственным UI-стеком (Qt): технологии UI из qt_ui_options,
    // а не фронтовые приложения чужого стека. Порядок — как в qt_ui_options.
    if (fw.qt_ui_options?.length) {
      const modeIds = fw.qt_ui_options.map((m) => m.id);
      return modeIds
        .map((id) => tree!.frameworks.find((c) => c.id === id))
        .filter((d): d is FrameworkDef => !!d);
    }
    // Data-driven: fw.companions из wizard_tree (tauri → svelte/vue/react,
    // electron → react/vue/svelte). Фолбэк для either-фреймворков — прежний
    // фиксированный список.
    const ids = fw.companions?.length
      ? fw.companions
      : fw.side === "either"
        ? [...EITHER_COMPANIONS]
        : [];
    if (!ids.length) return [];
    const compat = tree.frameworks.filter(
      (c) =>
        ids.includes(c.id) &&
        c.side === "frontend" &&
        c.kind === "app" &&
        !(fw.conflicts ?? []).includes(c.id) &&
        (!c.project_types?.length || !selectedType || c.project_types.includes(selectedType.id)),
    );
    const recommendedIds = (fw.recommends ?? []).map((r) => r.framework);
    return [...compat.filter((c) => recommendedIds.includes(c.id)), ...compat.filter((c) => !recommendedIds.includes(c.id))];
  }
  
  function langLabel(id: string): string {
    const label = tree?.languages.find((l) => l.id === id)?.label ?? id;
    return i18n.t(label as TranslationKey);
  }
  
  /** Языки, доступные для «чистого» backend-выбора (без фреймворка).
   *  Категория "both" (kotlin, dart, csharp, swift) тоже доступна — вандальные
   *  языки без фреймворка. Ограничены project_language_map выбранного типа
   *  проекта: язык должен входить в разрешённый список (browser-extension →
   *  только TS/JS, без Go). */
  function backendCandidates(): LanguageDef[] {
    if (!tree) return [];
    const allowed = selectedType ? tree.project_language_map[selectedType.id] : null;
    let langs = tree.languages.filter(
      (l) => l.category === "backend" || l.category === "both",
    );
    if (allowed) langs = langs.filter((l) => allowed.includes(l.id));
    return langs;
  }
  
  /** Языки для «чистого» frontend-выбора (включая чистый HTML/CSS/JS).
   *  «both»-языки (TypeScript, JavaScript, Dart, Kotlin, Swift, C#) тоже
   *  доступны фронтенду: один и тот же язык может стоять и на бэкенде, и на
   *  фронтенде (полноценный full-stack). Ограничены project_language_map
   *  выбранного типа проекта. */
  function frontendCandidates(): LanguageDef[] {
    if (!tree) return [];
    const allowed = selectedType ? tree.project_language_map[selectedType.id] : null;
    let langs = tree.languages.filter(
      (l) => l.category === "frontend" || l.category === "static" || l.category === "both",
    );
    if (allowed) langs = langs.filter((l) => allowed.includes(l.id));
    return langs;
  }
  
  /** Причина, по которой чистый язык нельзя выбрать (сторона уже занята).
   *  Сторона передаётся явно: «both»-язык присутствует в обеих колонках, и
   *  блокировка должна считаться по колонке, а не по category языка. */
  function languageBlockReason(lang: LanguageDef, side: "backend" | "frontend"): string | null {
    const active = side === "frontend" ? frontendLangs : backendLangs;
    if (active.includes(lang.id)) return null;
    if (active.length > 0) return i18n.t("create.already_on_side", { list: active.map((l) => langLabel(l)).join(", ") });
    return null;
  }
  
  /** Развёрнутое объяснение блокировки чистого языка (подпись под бейджем) */
  function languageBlockDetail(lang: LanguageDef): string | null {
    return null;
  }
  
  /** Кандидаты стороны для «чистых» языков (backend/frontend). */
  function langSideCandidates(side: "backend" | "frontend"): LanguageDef[] {
    return side === "backend" ? backendCandidates() : frontendCandidates();
  }
  
  /** Недоступные в данный момент чистые языки стороны (blocked-карточки). */
  function blockedLanguages(side: "backend" | "frontend"): LanguageDef[] {
    return langSideCandidates(side).filter((l) => languageBlockReason(l, side) !== null);
  }
  
  /** Видимые чистые языки стороны: пока не развёрнуто — только доступные.
   *  Недоступные прячем за компактной карточкой-сводкой, как у фреймворков. */
  function visibleLanguages(side: "backend" | "frontend"): LanguageDef[] {
    const items = langSideCandidates(side);
    return showUnavailableLangs[side]
      ? items
      : items.filter((l) => languageBlockReason(l, side) === null);
  }
  
  function toggleUnavailableLangs(side: "backend" | "frontend") {
    showUnavailableLangs = { ...showUnavailableLangs, [side]: !showUnavailableLangs[side] };
  }
  
  /** Список языков фреймворка одной строкой ("TypeScript, JavaScript") */
  function fwLangsLabel(fw: FrameworkDef): string {
    return fw.languages.map((l) => langLabel(l)).join(", ");
  }
  
  /** Сводка выбранных языков для чипа на карточке ("TypeScript" / "TypeScript + JavaScript") */
  function fwLangSummary(fw: FrameworkDef): string {
    if (!selectedFrameworks.includes(fw.id)) return "";
    if (fw.qt_ui_options?.length) {
      const modeId = qtUiMode[fw.id] ?? fw.qt_ui_options[0].id;
      const mode = fw.qt_ui_options.find((m) => m.id === modeId);
      const parts = mode ? [i18n.t(mode.label as TranslationKey)] : [];
      if (modeId === "qt-webengine" && qtWebLinked[fw.id]) {
        const wf = tree?.frameworks.find((f) => f.id === qtWebLinked[fw.id]);
        if (wf) parts.push(i18n.t(wf.label as TranslationKey));
      }
      return parts.join(" + ");
    }
    const own = langLabel(fwLangs[fw.id] ?? fw.recommended_language);
    const cid = linkedCompanions[fw.id];
    if (cid) {
      const cfw = tree?.frameworks.find((f) => f.id === cid);
      const cl = langLabel(fwLangs[cid] ?? cfw?.recommended_language ?? "");
      return `${own} + ${cl}`;
    }
    return own;
  }
  
  function addCompanion(ownerId: string, cid: string) {
    if (selectedFrameworks.includes(cid)) return;
    const cfw = tree?.frameworks.find((f) => f.id === cid);
    if (!cfw) return;
    selectedFrameworks = [...selectedFrameworks, cid];
    linkedCompanions = { ...linkedCompanions, [ownerId]: cid };
    if (!fwLangs[cid]) fwLangs = { ...fwLangs, [cid]: cfw.recommended_language };
  }
  
  function openFwPopup(id: string) {
    const fw = tree?.frameworks.find((f) => f.id === id);
    if (!fw) return;
    popupLang = fwLangs[id] ?? fw.recommended_language;
    popupQtUi = null;
    popupWebFw = null;
    if (fw.qt_ui_options?.length) {
      popupQtUi = qtUiMode[id] ?? fw.qt_ui_options[0].id;
      popupWebFw = qtWebLinked[id] ?? null;
      popupCompanion = null;
      popupCompanionLang = null;
    } else {
      const opts = companionOptions(fw);
      if (opts.length > 0) {
        const linked = linkedCompanions[id];
        const selected = opts.find((o) => o.id === linked) ?? opts[0];
        popupCompanion = selected.id;
        popupCompanionLang =
          fwLangs[selected.id] && selected.languages.includes(fwLangs[selected.id])
            ? fwLangs[selected.id]
            : selected.recommended_language;
      } else {
        popupCompanion = null;
        popupCompanionLang = null;
      }
    }
    fwPopup = id;
  }
  
  /** Применить черновики попапа (Done) — меню закрывается только явно */
  function applyFwPopup() {
    const fw = tree?.frameworks.find((f) => f.id === fwPopup);
    if (!fw) {
      fwPopup = null;
      return;
    }
  
    if (fw.qt_ui_options?.length) {
      applyQtUiPopup(fw);
      recomputeSideLangs();
      fwPopup = null;
      return;
    }
  
    if (popupLang && fw.languages.includes(popupLang)) {
      // Смена языка фреймворка: старый язык стороны убирается ДО записи
      // нового — в стейте никогда не окажется двух языков одной стороны
      // (JS → Vue(JS) → TS: сначала снимаем JS, потом добавляем TS).
      const prevLang = fwLangs[fw.id];
      if (prevLang && prevLang !== popupLang) {
        manualBackendLangs = manualBackendLangs.filter((l) => l !== prevLang);
        manualFrontendLangs = manualFrontendLangs.filter((l) => l !== prevLang);
      }
      fwLangs = { ...fwLangs, [fw.id]: popupLang };
    }
    if (companionOptions(fw).length > 0) {
      const prev = linkedCompanions[fw.id];
      if (prev && prev !== popupCompanion) {
        selectedFrameworks = selectedFrameworks.filter((x) => x !== prev);
        const nl = { ...linkedCompanions };
        delete nl[fw.id];
        linkedCompanions = nl;
        const nf = { ...fwLangs };
        delete nf[prev];
        fwLangs = nf;
      }
      if (popupCompanion && !selectedFrameworks.includes(popupCompanion)) {
        addCompanion(fw.id, popupCompanion);
      }
      if (popupCompanion && popupCompanionLang) {
        const cfw = tree?.frameworks.find((f) => f.id === popupCompanion);
        if (cfw && cfw.languages.includes(popupCompanionLang)) {
          const prevCompanionLang = fwLangs[popupCompanion];
          if (prevCompanionLang && prevCompanionLang !== popupCompanionLang) {
            manualBackendLangs = manualBackendLangs.filter((l) => l !== prevCompanionLang);
            manualFrontendLangs = manualFrontendLangs.filter((l) => l !== prevCompanionLang);
          }
          fwLangs = { ...fwLangs, [popupCompanion]: popupCompanionLang };
        }
      }
    }
    recomputeSideLangs();
    fwPopup = null;
  }
  
  /** Применить выбор UI-технологии Qt (qt_ui_options) и веб-фреймворка WebEngine */
  function applyQtUiPopup(fw: FrameworkDef) {
    const modeId = popupQtUi;
    const mode = fw.qt_ui_options?.find((m) => m.id === modeId);
    if (!mode || !modeId) return;
  
    qtUiMode = { ...qtUiMode, [fw.id]: modeId };
  
    // Сменить вариант: снять старый (и его веб-фреймворк), поставить новый
    const prev = linkedCompanions[fw.id];
    if (prev && prev !== modeId) {
      selectedFrameworks = selectedFrameworks.filter((x) => x !== prev);
      const nl = { ...linkedCompanions };
      delete nl[fw.id];
      linkedCompanions = nl;
      const nf = { ...fwLangs };
      delete nf[prev];
      fwLangs = nf;
    }
    if (modeId && !selectedFrameworks.includes(modeId)) {
      addCompanion(fw.id, modeId);
    }
  
    // Веб-фреймворк внутри WebEngine
    const webIds = mode.web_framework_options ?? [];
    if (webIds.length > 0) {
      const wf = popupWebFw ?? qtWebLinked[fw.id] ?? webIds[0];
      const prevWf = qtWebLinked[fw.id];
      if (prevWf && prevWf !== wf) {
        selectedFrameworks = selectedFrameworks.filter((x) => x !== prevWf);
        const nf = { ...fwLangs };
        delete nf[prevWf];
        fwLangs = nf;
      }
      if (wf && !selectedFrameworks.includes(wf)) addCompanion(modeId, wf);
      qtWebLinked = { ...qtWebLinked, [fw.id]: wf };
    } else if (qtWebLinked[fw.id]) {
      const wf = qtWebLinked[fw.id];
      selectedFrameworks = selectedFrameworks.filter((x) => x !== wf);
      const nf = { ...fwLangs };
      delete nf[wf];
      fwLangs = nf;
      const nq = { ...qtWebLinked };
      delete nq[fw.id];
      qtWebLinked = nq;
    }
  }
  
  function cancelFwPopup() {
    fwPopup = null;
  }
  
  /** Удалить фреймворк вместе со связанным подфреймворком */
  function removeFramework(id: string) {
    const prev = linkedCompanions[id];
    const toRemove = new Set([id]);
    if (prev) toRemove.add(prev);
    // Qt WebEngine: тянем и веб-фреймворк
    const webFw = qtWebLinked[id];
    if (webFw) toRemove.add(webFw);
    selectedFrameworks = selectedFrameworks.filter((x) => !toRemove.has(x));
    const nl = { ...linkedCompanions };
    delete nl[id];
    for (const r of toRemove) delete nl[r];
    linkedCompanions = nl;
    const nq = { ...qtWebLinked };
    delete nq[id];
    qtWebLinked = nq;
    const nf = { ...fwLangs };
    for (const r of toRemove) delete nf[r];
    fwLangs = nf;
    recomputeSideLangs();
    fwPopup = null;
  }
  
  /** Подтверждение полной очистки выбранных технологий */
  let confirmClearStack = $state(false);
  
  /** Сбросить выбранные технологии (фреймворки, компаньоны, инструменты),
   *  оставив тип проекта и языки на месте. */
  function clearStack() {
    selectedFrameworks = [];
    fwLangs = {};
    linkedCompanions = {};
    selectedTools = [];
    qtUiMode = {};
    qtWebLinked = {};
    dropNotice = null;
    fwPopup = null;
    confirmClearStack = false;
  }
  
  // ----------------------------------------------------------
  // Инструменты
  // ----------------------------------------------------------
  
  /** Тул совместим с текущим стеком: его язык присутствует в проекте */
  function toolFitsStack(t: ToolDef): boolean {
    if (t.for_languages.length === 0) return true;
    const langs = allSelectedLangs();
    return t.for_languages.some((l) => langs.includes(l));
  }
  
  /** Все тулы wizard_tree, применимые к текущему стеку.
   *  Единственный жёсткий фильтр — язык (pytest без python не предлагаем).
   *  npm скрыт намеренно: для JS/TS-стеков он обязателен и приходит через
   *  required_tools фреймворков, для остальных бесполезен — как отдельный
   *  опциональный тул ему в UI не место. */
  function availableTools(): ToolDef[] {
    if (!tree) return [];
    return tree.tools.filter((t) => t.id !== "npm");
  }
  
  /** Максимум бейджей «рекомендуется» на карточках инструментов (2–5) */
  const MAX_RECOMMENDED_BADGES = 3;
  
  /** id инструментов, которые получают бейдж «рекомендуется»: не весь набор
   *  рекомендаций стека, а небольшой приоритетный список (до MAX_RECOMMENDED_BADGES),
   *  чтобы не засорять карточки. Сначала — тулы из framework_tool_map выбранных
   *  фреймворков (в порядке их выбора), затем — рекомендации типа проекта
   *  (etl → airflow/clickhouse/grafana). */
  function recommendedBadgeIds(): string[] {
    if (!tree) return [];
    const seen = new Set<string>();
    const ordered: string[] = [];
    const tm = tree.framework_tool_map ?? {};
    for (const fwId of selectedFrameworks) {
      for (const tid of tm[fwId] ?? []) {
        if (seen.has(tid)) continue;
        seen.add(tid);
        ordered.push(tid);
      }
    }
    for (const t of tree.tools) {
      if (seen.has(t.id)) continue;
      if (
        t.for_project_types.length > 0 &&
        selectedType &&
        t.for_project_types.includes(selectedType.id)
      ) {
        seen.add(t.id);
        ordered.push(t.id);
      }
    }
    return ordered.slice(0, MAX_RECOMMENDED_BADGES);
  }
  
  function isDockerForced(): boolean {
    return selectedTools.some((tid) => {
      const t = tree?.tools.find((x) => x.id === tid);
      return t?.requires_docker ?? false;
    });
  }
  
  function dockerEnabled(): boolean {
    return isDockerForced() || selectedTools.includes("docker");
  }

  /** Возвращает причину недоступности инструмента в текущей связке или null, если инструмент доступен. */
  function toolConflictReason(toolId: string): string | null {
    if (!tree) return null;
    const tool = tree.tools.find((t) => t.id === toolId);
    if (!tool) return null;

    // 1. Конфликт с уже выбранными фреймворками
    for (const fwId of selectedFrameworks) {
      const fw = tree.frameworks.find((f) => f.id === fwId);
      if (!fw) continue;
      if (fw.tool_conflicts?.includes(toolId)) {
        return i18n.t("create.tool_unavailable_with", { name: i18n.t(fw.label as TranslationKey) });
      }
    }

    // 2. Инструмент привязан к конкретным фреймворкам (например, angular-cli только для angular)
    if (tool.for_frameworks && tool.for_frameworks.length > 0) {
      const hasMatchingFw = selectedFrameworks.some((fid) => tool.for_frameworks?.includes(fid));
      if (!hasMatchingFw) {
        if (selectedFrameworks.length > 0) {
          const firstSelectedFw = tree.frameworks.find((f) => f.id === selectedFrameworks[0]);
          const name = firstSelectedFw ? i18n.t(firstSelectedFw.label as TranslationKey) : selectedFrameworks[0];
          return i18n.t("create.tool_unavailable_with", { name });
        } else {
          const targetLabels = tool.for_frameworks
            .map((fid) => {
              const f = tree!.frameworks.find((x) => x.id === fid);
              return f ? i18n.t(f.label as TranslationKey) : fid;
            })
            .join(", ");
          return i18n.t("create.tool_requires_framework", { name: targetLabels });
        }
      }
    }

    // 3. Требование по языкам (например, pytest требует Python, biome/vitest требуют TypeScript/JavaScript, gcc требует C/C++)
    if (tool.for_languages && tool.for_languages.length > 0) {
      const langs = allSelectedLangs();
      const hasLang = tool.for_languages.some((l) => langs.includes(l));
      if (!hasLang) {
        const langNames = tool.for_languages
          .map((lid) => {
            const l = tree!.languages.find((x) => x.id === lid);
            return l ? l.label : lid;
          })
          .join(" / ");
        return i18n.t("create.tool_requires_lang", { name: langNames });
      }
    }

    // 4. Ограничение по типу проекта
    if (tool.for_project_types && tool.for_project_types.length > 0 && selectedType) {
      if (!tool.for_project_types.includes(selectedType.id)) {
        return i18n.t("create.tool_unsupported_project_type");
      }
    }

    // 5. Конфликт с уже выбранными инструментами
    for (const existingId of selectedTools) {
      if (existingId === toolId) continue;
      const existing = tree.tools.find((t) => t.id === existingId);
      if (!existing) continue;

      // 5a. Прямой взаимный конфликт в conflicts[]
      if (tool.conflicts.includes(existingId) || existing.conflicts.includes(toolId)) {
        return i18n.t("create.tool_unavailable_with", { name: i18n.t(existing.label as TranslationKey) });
      }

      // 5b. Пересечение ответственности с политикой exclusive (например, gradle и maven, postgresql и mysql, prisma и drizzle)
      if (tool.responsibility && existing.responsibility && tool.responsibility === existing.responsibility) {
        const policyA = tool.alternative_policy ?? "allow";
        const policyB = existing.alternative_policy ?? "allow";
        if (policyA === "exclusive" || policyB === "exclusive") {
          return i18n.t("create.tool_unavailable_with", { name: i18n.t(existing.label as TranslationKey) });
        }
      }
    }

    // 6. Зависимость инструмента заблокирована
    for (const reqId of tool.requires) {
      const reqReason = toolConflictReason(reqId);
      if (reqReason) {
        return reqReason;
      }
    }

    return null;
  }

  function toggleTool(id: string) {
    const tool = tree?.tools.find((t) => t.id === id);
    if (!tool) return;

    if (selectedTools.includes(id)) {
      const dependents = tree!.tools.filter((t) => t.requires.includes(id));
      const toRemove = new Set([id, ...dependents.map((d) => d.id)]);
      selectedTools = selectedTools.filter((t) => !toRemove.has(t));
    } else {
      // Заблокированный конфликтом инструмент нельзя выбрать
      if (toolConflictReason(id)) return;
      for (const conflictId of tool.conflicts) {
        if (selectedTools.includes(conflictId)) {
          selectedTools = selectedTools.filter((t) => t !== conflictId);
        }
      }
      for (const existingId of selectedTools) {
        const existing = tree!.tools.find((t) => t.id === existingId);
        if (existing?.conflicts.includes(id)) {
          selectedTools = selectedTools.filter((t) => t !== existingId);
        }
      }
      const toAdd = new Set([id]);
      for (const reqId of tool.requires) {
        if (!selectedTools.includes(reqId)) {
          toAdd.add(reqId);
        }
      }
      if (tool.requires_docker && !selectedTools.includes("docker") && !toAdd.has("docker")) {
        toAdd.add("docker");
      }
      selectedTools = [...selectedTools, ...toAdd];
    }
  }
  
  const TOOL_CATEGORIES: { id: string; label: string }[] = [
    { id: "database", label: i18n.t("create.tool_category.databases") },
    { id: "cache", label: i18n.t("create.tool_category.caches") },
    { id: "messaging", label: i18n.t("create.tool_category.messaging") },
    { id: "observability", label: i18n.t("create.tool_category.observability") },
    { id: "testing", label: i18n.t("create.tool_category.testing") },
    { id: "tooling", label: i18n.t("create.tool_category.tooling") },
    { id: "container", label: i18n.t("create.tool_category.containers") },
    { id: "orchestration", label: i18n.t("create.tool_category.orchestration") },
    { id: "etl", label: i18n.t("create.tool_category.etl") },
    { id: "baas", label: i18n.t("create.tool_category.baas") },
    { id: "infra", label: i18n.t("create.tool_category.infrastructure") },
    { id: "cli", label: i18n.t("create.tool_category.cli") },
  ];
  
  // ----------------------------------------------------------
  // Шаблоны
  // ----------------------------------------------------------
  
  function applyPreset(p: ProjectPreset) {
    const t = tree;
    if (!t) return;
    selectedType = t.project_types.find((pt) => pt.id === p.stack.project_type) ?? null;
    const nextFwLangs: Record<string, string> = {};
    for (const fid of p.stack.frameworks) {
      const fw = t.frameworks.find((f) => f.id === fid);
      if (!fw) continue;
      const sideLang = fw.side === "frontend" ? p.stack.frontend_lang : p.stack.backend_lang;
      nextFwLangs[fid] = sideLang && fw.languages.includes(sideLang) ? sideLang : fw.recommended_language;
    }
    fwLangs = nextFwLangs;
    selectedFrameworks = [...p.stack.frameworks];
    selectedTools = [...p.stack.tools];
    testing = p.stack.features.testing;
    git = p.stack.features.git;
    vscode = p.stack.features.vscode;
    manualBackendLangs = p.stack.backend_lang ? [p.stack.backend_lang] : [];
    manualFrontendLangs = p.stack.frontend_lang ? [p.stack.frontend_lang] : [];
    recomputeSideLangs();
    mode = "constructor";
    phase = 2;
  }
  
  // ----------------------------------------------------------
  // Анализ существующего проекта
  // ----------------------------------------------------------
  
  async function runAnalysis() {
    const folder = await selectFolder();
    if (!folder) return;
    analyzedPath = folder;
    analyzing = true;
    analysisError = null;
    analysisResult = null;
    try {
      analysisResult = await analyzeProjectTechnologies(folder);
    } catch (e) {
      analysisError = String(e);
    } finally {
      analyzing = false;
    }
  }
  
  function applyAnalysis() {
    if (!analysisResult || !tree) return;
    selectedType = tree.project_types.find((pt) =>
      analysisResult!.project_type_hints.some((h) => pt.label.toLowerCase().includes(h.toLowerCase()))
    ) ?? null;
    const langIds = analysisResult.detected_technologies
      .filter((t) => tree!.languages.some((l) => l.id === t.name.toLowerCase()))
      .map((t) => t.name.toLowerCase());
    const firstLang = tree.languages.find((l) => l.id === langIds[0]);
    if (firstLang?.category === "frontend" || firstLang?.category === "static") {
      manualFrontendLangs = langIds.slice(0, 1);
      manualBackendLangs = [];
    } else {
      manualBackendLangs = langIds.slice(0, 1);
      manualFrontendLangs = langIds.length > 1 ? [langIds[1]] : [];
    }
    recomputeSideLangs();
    selectedFrameworks = [];
    fwLangs = {};
    selectedTools = analysisResult.detected_technologies
      .filter((t) => tree!.tools.some((tl) => tl.id === t.name.toLowerCase() || tl.label === t.name))
      .map((t) => t.name.toLowerCase());
    if (analysisResult.has_docker && !selectedTools.includes("docker")) {
      selectedTools = [...selectedTools, "docker"];
    }
    testing = analysisResult.has_tests;
    git = analysisResult.has_git;
    mode = "constructor";
    phase = 2;
  }
  
  // ----------------------------------------------------------
  // Фазы и навигация
  // ----------------------------------------------------------
  
  function goPhase(p: number) {
    if (p >= 0 && p <= 6) phase = p;
  }
  
  function back() {
    if (phase > 0) phase--;
  }
  
  function selectType(t: ProjectTypeDef) {
    markHelpDid(HELP.createTypeSelected);
    selectedType = t;
    backendLangs = [];
    frontendLangs = [];
    manualBackendLangs = [];
    manualFrontendLangs = [];
    selectedFrameworks = [];
    fwLangs = {};
    selectedTools = [];
    phase = 1;
  }
  
  // ----------------------------------------------------------
  // Папка и конфликт
  // ----------------------------------------------------------
  
  function effectiveProjectPath(): string | null {
    if (!selectedFolder || !projectName) return null;
    const folder = conflictResolvedFolder ?? projectName;
    return `${selectedFolder}/${folder}`;
  }
  
  async function pickProjectFolder() {
    const folder = await selectFolder();
    if (!folder) return;
    selectedFolder = folder;
    conflictResolvedFolder = null;
    await checkProjectFolder();
  }
  
  async function onProjectNameInput() {
    conflictResolvedFolder = null;
    folderExists = false;
    if (selectedFolder && projectName) {
      await checkProjectFolder();
    }
  }
  
  /** Номер последнего запущенного запроса к checkFolderExists: устаревшие
   *  ответы (пользователь успел переименовать проект) игнорируются. */
  let folderCheckSeq = 0;
  
  async function checkProjectFolder() {
    const path = effectiveProjectPath();
    if (!path) return;
    const seq = ++folderCheckSeq;
    folderCheckPending = true;
    try {
      const exists = await checkFolderExists(path);
      if (seq !== folderCheckSeq) return;
      folderExists = exists;
    } catch {
      if (seq !== folderCheckSeq) return;
      folderExists = false;
    } finally {
      if (seq === folderCheckSeq) folderCheckPending = false;
    }
  }
  
  async function resolveFolderConflict(action: "overwrite" | "auto-rename" | "cancel") {
    showConflictDialog = false;
    if (action === "cancel") return;
    if (action === "overwrite") {
      conflictResolvedFolder = null;
      await goToEnvironment();
      return;
    }
    if (action === "auto-rename") {
      let counter = 2;
      let testName = `${projectName}-${counter}`;
      while (selectedFolder && (await checkFolderExists(`${selectedFolder}/${testName}`))) {
        counter++;
        testName = `${projectName}-${counter}`;
      }
      conflictResolvedFolder = testName;
      folderExists = false;
    }
  }
  
  // ----------------------------------------------------------
  // Окружение
  // ----------------------------------------------------------
  
  function allSelectedLangs(): string[] {
    return [...backendLangs, ...frontendLangs];
  }
  
  function buildRequirements(): ProjectRequirements {
    return {
      languages: allSelectedLangs(),
      frameworks: selectedFrameworks,
      tools: selectedTools,
      local_infra_tools: [...envLocalInfra],
      git_init: git,
      vscode_config: vscode,
      docker: dockerEnabled(),
    };
  }
  
  async function runEnvironmentCheck(silent = false) {
    envChecking = !silent;
    envError = null;
    envCheckProgress = [];
    envCheckScanId = null;
    try {
      const fresh = await tcCheckEnvironment(buildRequirements());
      envCheck = fresh;
      envSelectedIds = new Set(
        (fresh?.requirements ?? [])
          .filter((r) => statusKind(r.status) !== "ok" && statusKind(r.status) !== "manual")
          .map((r) => r.tool_id),
      );
    } catch (e) {
      console.error("[env] ОШИБКА проверки:", e);
      envError = String(e) || i18n.t("create.env_check_unknown_error");
    } finally {
      envChecking = false;
    }
  }
  
  async function goToEnvironment() {
    phase = 5;
    envCheck = null;
    envPlan = null;
    envLogs = [];
    envTaskStates = new Map();
    envInstalling = false;
    envInstallDone = false;
    envErrors = [];
    envRestartHint = false;
    envDownload = new Map();
    envSpeed = new Map();
    envDlPrev = new Map();
    envRechecking = false;
    envPhaseStart = new Map();
    stopTick();
    envCheckProgress = [];
    envCheckScanId = null;
    if (unlistenTcCheck) unlistenTcCheck();
    unlistenTcCheck = await listenCheckProgress(handleCheckProgress);
    await refreshInstalledTools();
    await runEnvironmentCheck();
  }
  
  function handleCheckProgress(event: CheckProgressEvent) {
    // События без scan_id (старый бэкенд) принимаются как раньше; с
    // scan_id — только от текущего запуска проверки: поздние события
    // прежнего запуска не должны подменять свежий прогресс.
    if (!identityMatches(envCheckScanId, event.scan_id ?? null)) return;
    if (event.scan_id && !envCheckScanId) envCheckScanId = event.scan_id;
    envCheckProgress = [...envCheckProgress, event];
  }
  
  function allMissingTools(): ToolRequirement[] {
    return (
      envCheck?.requirements.filter(
        (r) => statusKind(r.status) !== "ok" && statusKind(r.status) !== "manual",
      ) ?? []
    );
  }
  
  function toggleEnvTool(toolId: string) {
    const next = new Set(envSelectedIds);
    if (next.has(toolId)) next.delete(toolId);
    else next.add(toolId);
    envSelectedIds = next;
  }
  
  function selectAllEnvTools() {
    envSelectedIds = new Set(allMissingTools().map((r) => r.tool_id));
  }
  
  /** Общий стейт приложения: какие инструменты уже установлены локально
   *  (state.json toolchain). Панды окружения читают его, чтобы не предлагать
   *  «Install locally» для уже установленных тулов. */
  async function refreshInstalledTools() {
    try {
      const meta = await getToolchainMetadata();
      installedTools = new Set(Object.keys(meta.tools));
    } catch {
      // стейт недоступен — считаем, что ничего не установлено
    }
  }
  
  /** Опциональный docker-инструмент (postgresql, redis, ...) пользователь
   * решил ставить ЛОКАЛЬНО вместо docker-compose: добавляем его в
   * envLocalInfra и перезапускаем проверку — теперь тул обычное требование
   * (Missing → установка), а из docker-compose проекта он исключится. */
  async function optInLocalInfra(toolId: string) {
    const next = new Set(envLocalInfra);
    next.add(toolId);
    envLocalInfra = next;
    await recheckEnvironment();
  }
  
  /** Вернуть docker-инструмент обратно в docker-compose (отменить выбор
   *  локальной установки): убираем из envLocalInfra и перезапускаем
   *  проверку — тул снова станет опциональным (RunInDocker). */
  async function revertLocalInfra(toolId: string) {
    const next = new Set(envLocalInfra);
    next.delete(toolId);
    envLocalInfra = next;
    await recheckEnvironment();
  }
  
  async function startInstall() {
    if (!envCheck) return;
    markHelpDid(HELP.envInstallStarted);
    envInstalling = true;
    envInstallDone = false;
    envError = null;
    envLogs = [];
    envErrors = [];
    envTaskStates = new Map();
    envDownload = new Map();
    envSpeed = new Map();
    envDlPrev = new Map();
    envPhaseStart = new Map();
    startTick();
    try {
      envPlan = await tcBuildPlan(envCheck, [...envSelectedIds]);
    } catch (e) {
      envError = String(e);
      envInstalling = false;
      return;
    }
    try {
      if (unlistenTc) unlistenTc();
      if (unlistenTcDone) unlistenTcDone();
      unlistenTc = await listenToolchainEvents(handleToolchainEvent);
      unlistenTcDone = await listenInstallDone(handleInstallDone);
      await tcRunInstall(envPlan);
      // Бэкенд канонизирует план в задание движка и генерирует НОВЫЙ
      // session_id (= job_id): события установки несут именно его.
      // Синхронизируем план из авторитетной сессии, чтобы фильтр
      // «чужих/поздних» событий сравнивал правильные идентификаторы.
      const session = await getInstallStatus();
      if (session && session.plan.session_id) envPlan = session.plan;
    } catch (e) {
      envError = String(e);
      envInstalling = false;
      stopTick();
    }
  }
  
  /** Событие принадлежит текущей операции? Чистое правило живёт в
   *  stateLogic (identityMatches, покрыто тестами): пустые идентификаторы
   *  пропускаются — поведение мастера не меняется; известные чужие
   *  session_id/scan_id отбрасываются до попадания в стейт. */
  function eventBelongsToCurrentRun(eventSessionId: string | undefined): boolean {
    return identityMatches(envPlan?.session_id ?? null, eventSessionId ?? null);
  }
  
  function handleToolchainEvent(event: ToolchainEvent) {
    if (!eventBelongsToCurrentRun(event.session_id)) return;
    const t = event.event_type;
    if (t === "TaskStarted") {
      envTaskStates.set(event.task_id, { Running: { phase: "Downloading" } });
      envTaskStates = new Map(envTaskStates);
      envPhaseStart.set(event.task_id, Date.now());
      envPhaseStart = new Map(envPhaseStart);
    }
    if (typeof t !== "object" || !t || Array.isArray(t)) return;
    if ("TaskPhaseChanged" in t) {
      envTaskStates.set(event.task_id, { Running: { phase: t.TaskPhaseChanged.phase } });
      envTaskStates = new Map(envTaskStates);
      envPhaseStart.set(event.task_id, Date.now());
      envPhaseStart = new Map(envPhaseStart);
    }
    if ("TaskProgress" in t) {
      const line = t.TaskProgress.line;
      const dl = line.match(/^tc:dl (\d+) (-?\d+)$/);
      if (dl) {
        const received = Number(dl[1]);
        const total = Number(dl[2]);
        // Скорость = дельта байтов между событиями tc:dl (приходят часто).
        const prev = envDlPrev.get(event.task_id);
        const ts = Date.now();
        let speed = 0;
        if (prev && ts > prev.ts && received >= prev.received) {
          speed = (received - prev.received) / ((ts - prev.ts) / 1000);
        }
        envSpeed.set(event.task_id, speed);
        envDlPrev.set(event.task_id, { received, ts });
        envDownload.set(event.task_id, { received, total });
        envDownload = new Map(envDownload);
        envSpeed = new Map(envSpeed);
        envDlPrev = new Map(envDlPrev);
      } else if (line.startsWith("tc:error ")) {
        envErrors = [...envErrors.slice(-199), `${event.tool_id}: ${line.slice("tc:error ".length)}`];
        envLogs = [...envLogs.slice(-2999), line];
      } else {
        envLogs = [...envLogs.slice(-2999), line];
      }
    }
    if ("TaskCompleted" in t) {
      envTaskStates.set(event.task_id, t.TaskCompleted.state);
      envTaskStates = new Map(envTaskStates);
      envSpeed.delete(event.task_id);
      envSpeed = new Map(envSpeed);
    }
  }
  
  function handleInstallDone(plan: InstallPlan) {
    if (!eventBelongsToCurrentRun(plan.session_id)) return;
    for (const task of plan.tasks) {
      envTaskStates.set(task.task_id, task.state);
    }
    envTaskStates = new Map(envTaskStates);
    // Финальный план несёт фактический session_id запуска — принимаем
    // его как авторитетный для возможных поздних событий этой установки.
    if (plan.session_id) envPlan = plan;
    envInstalling = false;
    envInstallDone = true;
    stopTick();
    const installedCount = plan.tasks.filter((t) => taskStateKind(t.state) === "success").length;
    if (installedCount > 0) envRestartHint = true;
    // Обновляем «уже установлено локально» — только что поставленные тулы
    // сразу уходят в общий стейт и перестают предлагаться к установке.
    refreshInstalledTools();
    persistNow();
    fetchNewSecrets();
    // Свежая проверка окружения в фоне: только что поставленные тулы
    // переходят в «готово» без ручного «Перепроверить окружение».
    // Провал проверки не ломает итог установки — остаётся экран
    // завершения с кнопкой перепроверки.
    void autoRecheckAfterInstall();
  }
  
  /** Фоновая перепроверка окружения после установки: экран «установка
   *  завершена» остаётся видимым, пока проверка не подтвердит свежие
   *  тулы (спиннер «Проверяю окружение…»), затем мастер переключается
   *  на обновлённый список с готовыми статусами. */
  async function autoRecheckAfterInstall() {
    envRechecking = true;
    envError = null;
    envCheckProgress = [];
    envCheckScanId = null;
    try {
      const fresh = await tcCheckEnvironment(buildRequirements());
      if (!fresh) return;
      envCheck = fresh;
      envSelectedIds = new Set(
        (fresh.requirements ?? [])
          .filter((r) => statusKind(r.status) !== "ok" && statusKind(r.status) !== "manual")
          .map((r) => r.tool_id),
      );
      // Свежая проверка прошла — экран «завершено» сменяется списком,
      // где поставленные тулы уже «готовы».
      envInstallDone = false;
    } catch (e) {
      console.error("[env] авто-перепроверка после установки:", e);
    } finally {
      envRechecking = false;
      persistNow();
    }
  }
  
  async function recheckEnvironment() {
    envInstallDone = false;
    envPlan = null;
    envErrors = [];
    await runEnvironmentCheck(true);
  }
  
  async function fetchNewSecrets() {
    try {
      const secrets = await getNewSecrets();
      if (Object.keys(secrets).length > 0) newSecrets = secrets;
    } catch {
      // ignore
    }
  }
  
  async function copySecret(key: string, value: string) {
    try {
      await navigator.clipboard.writeText(value);
      secretCopied = key;
      setTimeout(() => { secretCopied = null; }, 1500);
    } catch {
      // ignore
    }
  }
  
  async function cancelInstall() {
    try {
      await tcAbortInstall();
    } catch {
      // ignore
    }
    envInstalling = false;
    stopTick();
    // Свежая проверка в фоне: тулы, успевшие установиться до отмены,
    // сразу помечаются «готово» — список не врёт после прерывания.
    await runEnvironmentCheck(true);
  }
  
  // ----------------------------------------------------------
  // Генерация
  // ----------------------------------------------------------
  
  async function confirmAll() {
    const path = effectiveProjectPath();
    if (!path || !selectedFolder) return;
    if (projectNameError) return;
    reviewError = null;
  
    try {
      const issues = await validateProjectStack(
        selectedType?.id ?? null,
        backendLangs,
        frontendLangs,
        selectedFrameworks,
        selectedTools,
      );
      const blocking = issues.filter((i) => i.severity === "Error");
      if (blocking.length > 0) {
        // Показываем ошибки бэкенд-валидации (нормализация, конфликты,
        // duplicate write paths), которых нет во фронтенд-зеркале rules.ts.
        // Раньше клик молча гасился — кнопка выглядела «сломанной».
        // Если бэкенд вернул message_key — переводим через i18n.
        const first = blocking[0];
        reviewError =
          first.message_key && first.args
            ? i18n.t(first.message_key as TranslationKey, first.args)
            : first.message;
        return;
      }
    } catch (e) {
      reviewError = i18n.t("create.validation_failed", { err: String(e) });
      return;
    }
  
    folderCheckPending = true;
    const seq = ++folderCheckSeq;
    try {
      const exists = await checkFolderExists(path);
      if (seq !== folderCheckSeq) return;
      if (exists) {
        folderExists = true;
        folderCheckPending = false;
        showConflictDialog = true;
        return;
      }
    } catch {
      // ignore, proceed anyway
    }
    if (seq !== folderCheckSeq) return;
    folderCheckPending = false;
  
    markHelpDid(HELP.createStackConfirmed);
    await goToEnvironment();
  }
  
  /** Ответы мастера для движка: выбранная UI-технология Qt и веб-фреймворк
   *  WebEngine (ключи qt_ui / qt_web_framework). */
  function buildAnswers(): Record<string, string[]> {
    const answers: Record<string, string[]> = {};
    for (const fwId of selectedFrameworks) {
      const fw = tree?.frameworks.find((f) => f.id === fwId);
      if (!fw?.qt_ui_options?.length) continue;
      const modeId = qtUiMode[fwId];
      if (modeId) {
        answers["qt_ui"] = [modeId];
        if (qtWebLinked[fwId]) answers["qt_web_framework"] = [qtWebLinked[fwId]];
      }
      break;
    }
    return answers;
  }
  
  /** Построить WizardContext для предпросмотра (без запуска выполнения). */
  function buildWizardContext(): WizardContext {
    return {
      project_path: null,
      project_name: projectName,
      is_existing: false,
      project_type: selectedType?.id ?? null,
      languages: allSelectedLangs(),
      backend_languages: backendLangs,
      frontend_languages: frontendLangs,
      frameworks: selectedFrameworks,
      tools: selectedTools,
      local_infra_tools: [...envLocalInfra],
      features: [],
      infrastructure: [],
      docker: dockerEnabled(),
      testing,
      ci: false,
      git_init: git,
      vscode_config: vscode,
      answers: buildAnswers(),
      readme_locale: readmeLocale,
    };
  }
  
  async function doCreateProject() {
    const path = effectiveProjectPath();
    if (!path || !selectedFolder || !selectedType) return;
  
    markHelpDid(HELP.envContinued);
    const ctx: WizardContext = {
      project_path: null,
      project_name: conflictResolvedFolder ?? projectName,
      is_existing: false,
      project_type: selectedType.id,
      languages: allSelectedLangs(),
      backend_languages: backendLangs,
      frontend_languages: frontendLangs,
      frameworks: selectedFrameworks,
      tools: selectedTools,
      local_infra_tools: [...envLocalInfra],
      features: [],
      infrastructure: [],
      docker: dockerEnabled(),
      testing,
      ci: false,
      git_init: git,
      vscode_config: vscode,
      answers: buildAnswers(),
      readme_locale: readmeLocale,
    };
  
    phase = 6;
    execPlan = null;
    execProjectPath = path;
    execStatuses = new Map();
    execLogs = [];
    execOverallStatus = "running";
    execResult = null;
    execError = null;
    devlAutoPopupShown = false;
    persistNow();
  
    if (unlisten) unlisten();
    unlisten = await listen<ExecutionEvent>("project_creator:step_event", (e) => {
      handleExecEvent(e.payload);
    });
  
    try {
      execPlan = await startProjectExecution(ctx, path, removedStepIds);
    } catch (err) {
      execError = String(err);
      execOverallStatus = "error";
    }
  }
  
  async function handleExecEvent(event: ExecutionEvent) {
    const idx = event.step_index;
  
    if (!execStatuses.has(idx)) {
      execStatuses.set(idx, { name: event.step_name, status: "Running", logs: [] });
    }
    const entry = execStatuses.get(idx)!;
    entry.name = event.step_name;
  
    const t = event.event_type;
    if (t && typeof t === "object" && !Array.isArray(t)) {
      if ("StepProgress" in t) {
        const p = (t as Record<string, { stdout?: string; stderr?: string }>).StepProgress;
        const line = p?.stdout || p?.stderr || "";
        if (line) {
          // Ограничиваем накопление: длинные генерации (npm install, cargo build)
          // льют тысячи строк — держим последние 300 строк шага и 3000 всего.
          entry.logs = [...entry.logs.slice(-299), line];
          execLogs = [...execLogs.slice(-2999), line];
        }
      }
      if ("StepCompleted" in t) {
        const c = (t as Record<string, { status: StepStatus; duration_ms: number }>).StepCompleted;
        if (c) {
          entry.status = c.status;
          if (typeof c.status === "object" && c.status !== null && "Failed" in c.status) {
            const errMsg = (c.status as { Failed: { error: string } }).Failed.error;
            if (errMsg) {
              entry.logs = [...entry.logs.slice(-299), `ERROR: ${errMsg}`];
              execLogs = [...execLogs.slice(-2999), `ERROR: ${errMsg}`];
            }
          }
        }
      }
      if ("AllCompleted" in t) {
        const a = (t as Record<string, { result: { total_duration_ms: number; overall: unknown } }>).AllCompleted;
        execOverallStatus = "done";
        execResult = { duration: a?.result?.total_duration_ms ?? 0, status: JSON.stringify(a?.result?.overall) };
        persistNow();
        // Build DevLauncher profile from wizard context (seamless integration).
        // Контекст из плана НЕ содержит project_path (фронтенд шлёт его
        // отдельным аргументом), поэтому подставляем реальный путь — иначе
        // профиль получит пустой project_path и затрёт чужой профиль.
        //
        // Окно DevLauncher открывается ТОЛЬКО при полностью успешном создании
        // (overall === "Success"): провал любого шага (PartialFailure/Aborted)
        // не должен показывать «профиль создан». И только один раз за
        // выполнение — реплей событий при возврате на вкладку не открывает
        // уже показанное/закрытое окно заново.
        const createdSuccessfully = a?.result?.overall === "Success";
        if (createdSuccessfully) markHelpGraduated();
        if (createdSuccessfully && !devlAutoPopupShown && execPlan?.context) {
          const ctx = { ...execPlan.context, project_path: execPlan.project_path };
          try {
            const profile = await confirmProjectCreatedWithProfile(ctx);
            devlProfileCreated = true;
            devlShowReminder = true;
            devlProfileName = profile.name;
            devlProfilePath = profile.project_path;
            devlProfileExists = true;
            markProfileCreated();
          } catch {
            // Non-critical: profile creation failed, user can still use VS Code
            devlProfileCreated = false;
            devlShowReminder = false;
            devlProfileExists = false;
          }
          devlAutoPopupShown = true;
          persistNow();
        }
      }
      if ("Error" in t) {
        const err = (t as Record<string, { message: string }>).Error;
        execError = err?.message ?? String(t);
        execOverallStatus = "error";
        persistNow();
      }
    }
    execStatuses = new Map(execStatuses);
  }
  
  async function openInVSCode() {
    const targetPath = (execPlan?.project_path || execProjectPath || effectiveProjectPath())?.toString().trim();
    if (!targetPath) {
      notifyError(i18n.t("create.open_vscode") as TranslationKey, "No project path available");
      return;
    }
    try {
      await invoke("open_in_vscode", { path: targetPath });
      notifySuccess("VS Code", i18n.t("create.open_vscode") as TranslationKey);
    } catch (error) {
      notifyError(i18n.t("create.open_vscode") as TranslationKey, String(error));
    }
  }
  
  function cancelExecution() {
    execOverallStatus = "cancelled";
    persistNow();
  }
  
  // ---- DevLauncher integration dialog handlers ----
  function openDevLauncher() {
    if (devlProfileName) {
      goto(`/devlauncher/profiles/${encodeURIComponent(devlProfileName)}`);
    }
    devlProfileCreated = false;
    devlShowReminder = false;
  }
  
  /** OK / крестик — только закрывают окно; профиль остаётся в DevLauncher. */
  function dismissProfileOk() {
    devlProfileCreated = false;
    devlShowReminder = false;
  }
  
  /** Маленькая кнопка на финальной странице: открывает то же окно.
   *  Профиль создаётся ТОЛЬКО если его ещё нет — не дублируем, не заменяем. */
  async function reopenDevlDialog() {
    if (!devlProfileExists) {
      if (!execPlan?.context) return;
      const ctx = { ...execPlan.context, project_path: execPlan.project_path };
      try {
        const profile = await confirmProjectCreatedWithProfile(ctx);
        devlProfileName = profile.name;
        devlProfilePath = profile.project_path;
        devlProfileExists = true;
        markProfileCreated();
      } catch {
        return;
      }
    }
    devlProfileCreated = true;
    devlShowReminder = false;
  }
  
  function cancelProfile() {
    devlConfirmCancel = true;
  }
  
  function confirmCancelProfile() {
    if (devlProfileName) {
      deleteProfile(devlProfileName).catch(() => {});
    }
    devlProfileCreated = false;
    devlShowReminder = false;
    devlProfileName = null;
    devlProfilePath = null;
    devlProfileExists = false;
    devlConfirmCancel = false;
  }
  
  function dismissCancelConfirm() {
    devlConfirmCancel = false;
  }
  
  function resetAll() {
    selectedType = null;
    backendLangs = [];
    frontendLangs = [];
    manualBackendLangs = [];
    manualFrontendLangs = [];
    selectedFrameworks = [];
    fwLangs = {};
    linkedCompanions = {};
    selectedTools = [];
    testing = true;
    git = true;
    vscode = true;
    tooltipData = null;
    execProjectPath = null;
    envCheck = null;
    envPlan = null;
    envLogs = [];
    envTaskStates = new Map();
    envInstalling = false;
    envError = null;
    envRestartHint = false;
    envCheckProgress = [];
    envInstallDone = false;
    envErrors = [];
    envDownload = new Map();
    envPhaseStart = new Map();
    envSelectedIds = new Set();
    envLocalInfra = new Set();
    newSecrets = null;
    secretCopied = null;
    projectName = "";
    selectedFolder = null;
    conflictResolvedFolder = null;
    folderExists = false;
    phase = 0;
    mode = "constructor";
    devlProfileCreated = false;
    devlProfileName = null;
    devlProfilePath = null;
    devlProfileExists = false;
    devlConfirmCancel = false;
    devlAutoPopupShown = false;
    clearCreateSession();
  }
  
  return {
    get tree() { return tree; },
    set tree(v) { tree = v; },
    get status() { return status; },
    set status(v) { status = v; },
    get hostOs() { return hostOs; },
    set hostOs(v) { hostOs = v; },
    get dropNotice() { return dropNotice; },
    set dropNotice(v) { dropNotice = v; },
    get showUnavailable() { return showUnavailable; },
    set showUnavailable(v) { showUnavailable = v; },
    get showUnavailableLangs() { return showUnavailableLangs; },
    set showUnavailableLangs(v) { showUnavailableLangs = v; },
    get AnalyzeMode() { return AnalyzeMode; },
    set AnalyzeMode(v) { AnalyzeMode = v; },
    get PresetsMode() { return PresetsMode; },
    set PresetsMode(v) { PresetsMode = v; },
    get EnvPanel() { return EnvPanel; },
    set EnvPanel(v) { EnvPanel = v; },
    get ExecPanel() { return ExecPanel; },
    set ExecPanel(v) { ExecPanel = v; },
    get DevlDialogs() { return DevlDialogs; },
    set DevlDialogs(v) { DevlDialogs = v; },
    get PreviewPanel() { return PreviewPanel; },
    set PreviewPanel(v) { PreviewPanel = v; },
    get mode() { return mode; },
        set mode(v) { mode = v; },
        get phase() { return phase; },
    set phase(v) { phase = v; },
    get selectedType() { return selectedType; },
    set selectedType(v) { selectedType = v; },
    get backendLangs() { return backendLangs; },
    set backendLangs(v) { backendLangs = v; },
    get frontendLangs() { return frontendLangs; },
    set frontendLangs(v) { frontendLangs = v; },
    get manualBackendLangs() { return manualBackendLangs; },
    set manualBackendLangs(v) { manualBackendLangs = v; },
    get manualFrontendLangs() { return manualFrontendLangs; },
    set manualFrontendLangs(v) { manualFrontendLangs = v; },
    get selectedFrameworks() { return selectedFrameworks; },
    set selectedFrameworks(v) { selectedFrameworks = v; },
    get fwLangs() { return fwLangs; },
    set fwLangs(v) { fwLangs = v; },
    get selectedTools() { return selectedTools; },
    set selectedTools(v) { selectedTools = v; },
    get testing() { return testing; },
    set testing(v) { testing = v; },
    get git() { return git; },
    set git(v) { git = v; },
    get vscode() { return vscode; },
    set vscode(v) { vscode = v; },
    get stackIssues() { return stackIssues; },
    set stackIssues(v) { stackIssues = v; },
    get stackError() { return stackError; },
    set stackError(v) { stackError = v; },
    get stackHintVisible() { return stackHintVisible; },
        get toolsHintVisible() { return toolsHintVisible; },
        get previewHintVisible() { return previewHintVisible; },
        get userSeenConstructor() { return userSeenConstructor; },
    set userSeenConstructor(v) { userSeenConstructor = v; },
    get reviewLocked() { return reviewLocked; },
    set reviewLocked(v) { reviewLocked = v; },
    get scrollEl() { return scrollEl; },
    set scrollEl(v) { scrollEl = v; },
    get checkConstructorScroll() { return checkConstructorScroll; },
        get markConstructorSeen() { return markConstructorSeen; },
        get scrollToStackBottom() { return scrollToStackBottom; },
        get hasBackend() { return hasBackend; },
    set hasBackend(v) { hasBackend = v; },
    get archMode() { return archMode; },
    set archMode(v) { archMode = v; },
    get readmeLocale() { return readmeLocale; },
    set readmeLocale(v) { readmeLocale = v; },
    get readmeLocaleTouched() { return readmeLocaleTouched; },
    set readmeLocaleTouched(v) { readmeLocaleTouched = v; },
    get readmeHelpOpen() { return readmeHelpOpen; },
    set readmeHelpOpen(v) { readmeHelpOpen = v; },
    get readmeLocaleLabel() { return readmeLocaleLabel; },
    set readmeLocaleLabel(v) { readmeLocaleLabel = v; },
    get toggleReadmeLocale() { return toggleReadmeLocale; },
        get projectName() { return projectName; },
    set projectName(v) { projectName = v; },
    get selectedFolder() { return selectedFolder; },
    set selectedFolder(v) { selectedFolder = v; },
    get folderExists() { return folderExists; },
    set folderExists(v) { folderExists = v; },
    get folderCheckPending() { return folderCheckPending; },
    set folderCheckPending(v) { folderCheckPending = v; },
    get showConflictDialog() { return showConflictDialog; },
    set showConflictDialog(v) { showConflictDialog = v; },
    get conflictResolvedFolder() { return conflictResolvedFolder; },
    set conflictResolvedFolder(v) { conflictResolvedFolder = v; },
    get reviewError() { return reviewError; },
    set reviewError(v) { reviewError = v; },
    get projectNameError() { return projectNameError; },
    set projectNameError(v) { projectNameError = v; },
    get analysisResult() { return analysisResult; },
    set analysisResult(v) { analysisResult = v; },
    get analysisError() { return analysisError; },
    set analysisError(v) { analysisError = v; },
    get analyzing() { return analyzing; },
    set analyzing(v) { analyzing = v; },
    get analyzedPath() { return analyzedPath; },
    set analyzedPath(v) { analyzedPath = v; },
    get execPlan() { return execPlan; },
    set execPlan(v) { execPlan = v; },
    get execProjectPath() { return execProjectPath; },
    set execProjectPath(v) { execProjectPath = v; },
    get execStatuses() { return execStatuses; },
    set execStatuses(v) { execStatuses = v; },
    get execOverallStatus() { return execOverallStatus; },
    set execOverallStatus(v) { execOverallStatus = v; },
    get execResult() { return execResult; },
    set execResult(v) { execResult = v; },
    get execError() { return execError; },
    set execError(v) { execError = v; },
    get execLogs() { return execLogs; },
    set execLogs(v) { execLogs = v; },
    get unlisten() { return unlisten; },
    set unlisten(v) { unlisten = v; },
    get removedStepIds() { return removedStepIds; },
    set removedStepIds(v) { removedStepIds = v; },
    get envCheck() { return envCheck; },
    set envCheck(v) { envCheck = v; },
    get envChecking() { return envChecking; },
    set envChecking(v) { envChecking = v; },
    get envError() { return envError; },
    set envError(v) { envError = v; },
    get envPlan() { return envPlan; },
    set envPlan(v) { envPlan = v; },
    get envInstalling() { return envInstalling; },
    set envInstalling(v) { envInstalling = v; },
    get envLogs() { return envLogs; },
    set envLogs(v) { envLogs = v; },
    get envTaskStates() { return envTaskStates; },
    set envTaskStates(v) { envTaskStates = v; },
    get envRestartHint() { return envRestartHint; },
    set envRestartHint(v) { envRestartHint = v; },
    get envInstallDone() { return envInstallDone; },
    set envInstallDone(v) { envInstallDone = v; },
    get envDownload() { return envDownload; },
    set envDownload(v) { envDownload = v; },
    get envErrors() { return envErrors; },
    set envErrors(v) { envErrors = v; },
    get envPhaseStart() { return envPhaseStart; },
    set envPhaseStart(v) { envPhaseStart = v; },
    get envSpeed() { return envSpeed; },
    set envSpeed(v) { envSpeed = v; },
    get envDlPrev() { return envDlPrev; },
    set envDlPrev(v) { envDlPrev = v; },
    get envRechecking() { return envRechecking; },
    set envRechecking(v) { envRechecking = v; },
    get newSecrets() { return newSecrets; },
    set newSecrets(v) { newSecrets = v; },
    get secretCopied() { return secretCopied; },
    set secretCopied(v) { secretCopied = v; },
    get unlistenTc() { return unlistenTc; },
    set unlistenTc(v) { unlistenTc = v; },
    get unlistenTcDone() { return unlistenTcDone; },
    set unlistenTcDone(v) { unlistenTcDone = v; },
    get unlistenTcCheck() { return unlistenTcCheck; },
    set unlistenTcCheck(v) { unlistenTcCheck = v; },
    get envCheckProgress() { return envCheckProgress; },
    set envCheckProgress(v) { envCheckProgress = v; },
    get envCheckScanId() { return envCheckScanId; },
    set envCheckScanId(v) { envCheckScanId = v; },
    get envSelectedIds() { return envSelectedIds; },
    set envSelectedIds(v) { envSelectedIds = v; },
    get envLocalInfra() { return envLocalInfra; },
    set envLocalInfra(v) { envLocalInfra = v; },
    get installedTools() { return installedTools; },
    set installedTools(v) { installedTools = v; },
    get devlProfileCreated() { return devlProfileCreated; },
    set devlProfileCreated(v) { devlProfileCreated = v; },
    get devlShowReminder() { return devlShowReminder; },
    set devlShowReminder(v) { devlShowReminder = v; },
    get devlProfileName() { return devlProfileName; },
    set devlProfileName(v) { devlProfileName = v; },
    get devlProfilePath() { return devlProfilePath; },
    set devlProfilePath(v) { devlProfilePath = v; },
    get devlProfileExists() { return devlProfileExists; },
    set devlProfileExists(v) { devlProfileExists = v; },
    get devlConfirmCancel() { return devlConfirmCancel; },
    set devlConfirmCancel(v) { devlConfirmCancel = v; },
    get devlAutoPopupShown() { return devlAutoPopupShown; },
    set devlAutoPopupShown(v) { devlAutoPopupShown = v; },
    get envNow() { return envNow; },
    set envNow(v) { envNow = v; },
    get envTicker() { return envTicker; },
    set envTicker(v) { envTicker = v; },
    get startTick() { return startTick; },
        get stopTick() { return stopTick; },
        get tooltipData() { return tooltipData; },
    set tooltipData(v) { tooltipData = v; },
    get tooltipTimer() { return tooltipTimer; },
    set tooltipTimer(v) { tooltipTimer = v; },
    get showTooltip() { return showTooltip; },
        get hideTooltip() { return hideTooltip; },
        get persistReady() { return persistReady; },
    set persistReady(v) { persistReady = v; },
    get restoredTypeId() { return restoredTypeId; },
    set restoredTypeId(v) { restoredTypeId = v; },
    get persistTimer() { return persistTimer; },
    set persistTimer(v) { persistTimer = v; },
    get buildSnapshot() { return buildSnapshot; },
        get restoreSnapshot() { return restoreSnapshot; },
        get persistNow() { return persistNow; },
        get availableFrameworks() { return availableFrameworks; },
        get fwLevelOf() { return fwLevelOf; },
        get recomputeSideLangs() { return recomputeSideLangs; },
        get toggleLang() { return toggleLang; },
        get isAllowedPair() { return isAllowedPair; },
        get isMainLimitExempt() { return isMainLimitExempt; },
        get conflictNoteOf() { return conflictNoteOf; },
        get frameworkBlockReason() { return frameworkBlockReason; },
        get unavailableFrameworks() { return unavailableFrameworks; },
        get availableFrameworksForDisplay() { return availableFrameworksForDisplay; },
        get toggleUnavailable() { return toggleUnavailable; },
        get frameworkAlternatives() { return frameworkAlternatives; },
        get frameworkBlockInfo() { return frameworkBlockInfo; },
        get frameworkWarnReason() { return frameworkWarnReason; },
        get resetSideLangsForFramework() { return resetSideLangsForFramework; },
        get clickFramework() { return clickFramework; },
        get applyRecommendedFramework() { return applyRecommendedFramework; },
        get fwPopup() { return fwPopup; },
    set fwPopup(v) { fwPopup = v; },
    get popupLang() { return popupLang; },
    set popupLang(v) { popupLang = v; },
    get popupCompanion() { return popupCompanion; },
    set popupCompanion(v) { popupCompanion = v; },
    get popupCompanionLang() { return popupCompanionLang; },
    set popupCompanionLang(v) { popupCompanionLang = v; },
    get popupQtUi() { return popupQtUi; },
    set popupQtUi(v) { popupQtUi = v; },
    get popupWebFw() { return popupWebFw; },
    set popupWebFw(v) { popupWebFw = v; },
    get linkedCompanions() { return linkedCompanions; },
    set linkedCompanions(v) { linkedCompanions = v; },
    get qtUiMode() { return qtUiMode; },
    set qtUiMode(v) { qtUiMode = v; },
    get qtWebLinked() { return qtWebLinked; },
    set qtWebLinked(v) { qtWebLinked = v; },
    get isUiVariant() { return isUiVariant; },
        get companionOptions() { return companionOptions; },
        get langLabel() { return langLabel; },
        get backendCandidates() { return backendCandidates; },
        get frontendCandidates() { return frontendCandidates; },
        get languageBlockReason() { return languageBlockReason; },
        get languageBlockDetail() { return languageBlockDetail; },
        get langSideCandidates() { return langSideCandidates; },
        get blockedLanguages() { return blockedLanguages; },
        get visibleLanguages() { return visibleLanguages; },
        get toggleUnavailableLangs() { return toggleUnavailableLangs; },
        get fwLangsLabel() { return fwLangsLabel; },
        get fwLangSummary() { return fwLangSummary; },
        get addCompanion() { return addCompanion; },
        get openFwPopup() { return openFwPopup; },
        get applyFwPopup() { return applyFwPopup; },
        get applyQtUiPopup() { return applyQtUiPopup; },
        get cancelFwPopup() { return cancelFwPopup; },
        get removeFramework() { return removeFramework; },
        get confirmClearStack() { return confirmClearStack; },
    set confirmClearStack(v) { confirmClearStack = v; },
    get clearStack() { return clearStack; },
        get toolFitsStack() { return toolFitsStack; },
        get availableTools() { return availableTools; },
        get recommendedBadgeIds() { return recommendedBadgeIds; },
        get isDockerForced() { return isDockerForced; },
        get dockerEnabled() { return dockerEnabled; },
        get toolConflictReason() { return toolConflictReason; },
        get toggleTool() { return toggleTool; },
        get applyPreset() { return applyPreset; },
        get applyAnalysis() { return applyAnalysis; },
        get goPhase() { return goPhase; },
        get back() { return back; },
        get selectType() { return selectType; },
        get effectiveProjectPath() { return effectiveProjectPath; },
        get folderCheckSeq() { return folderCheckSeq; },
    set folderCheckSeq(v) { folderCheckSeq = v; },
    get allSelectedLangs() { return allSelectedLangs; },
        get buildRequirements() { return buildRequirements; },
        get handleCheckProgress() { return handleCheckProgress; },
        get allMissingTools() { return allMissingTools; },
        get toggleEnvTool() { return toggleEnvTool; },
        get selectAllEnvTools() { return selectAllEnvTools; },
        get eventBelongsToCurrentRun() { return eventBelongsToCurrentRun; },
        get handleToolchainEvent() { return handleToolchainEvent; },
        get handleInstallDone() { return handleInstallDone; },
        get buildAnswers() { return buildAnswers; },
        get buildWizardContext() { return buildWizardContext; },
        get cancelExecution() { return cancelExecution; },
        get openDevLauncher() { return openDevLauncher; },
        get dismissProfileOk() { return dismissProfileOk; },
        get cancelProfile() { return cancelProfile; },
        get confirmCancelProfile() { return confirmCancelProfile; },
        get dismissCancelConfirm() { return dismissCancelConfirm; },
                get reopenDevlDialog() { return reopenDevlDialog; },
        get handleExecEvent() { return handleExecEvent; },
        get cancelInstall() { return cancelInstall; },
        get revertLocalInfra() { return revertLocalInfra; },
        get pickProjectFolder() { return pickProjectFolder; },
        get openInVSCode() { return openInVSCode; },
        get onProjectNameInput() { return onProjectNameInput; },
        get copySecret() { return copySecret; },
        get recheckEnvironment() { return recheckEnvironment; },
        get runEnvironmentCheck() { return runEnvironmentCheck; },
        get goToEnvironment() { return goToEnvironment; },
        get runAnalysis() { return runAnalysis; },
        get resolveFolderConflict() { return resolveFolderConflict; },
        get autoRecheckAfterInstall() { return autoRecheckAfterInstall; },
        get startInstall() { return startInstall; },
        get checkProjectFolder() { return checkProjectFolder; },
        get confirmAll() { return confirmAll; },
        get doCreateProject() { return doCreateProject; },
        get reSyncLiveSessions() { return reSyncLiveSessions; },
        get fetchNewSecrets() { return fetchNewSecrets; },
        get refreshInstalledTools() { return refreshInstalledTools; },
        get optInLocalInfra() { return optInLocalInfra; },
        get FW_LEVELS() { return FW_LEVELS; },
        get TOOL_CATEGORIES() { return TOOL_CATEGORIES; },
        get PHASES() { return PHASES; },
        get HINT_CREATE_TYPE() { return HINT_CREATE_TYPE; },
        get HINT_CREATE_STACK() { return HINT_CREATE_STACK; },
        get HINT_CREATE_TOOLS() { return HINT_CREATE_TOOLS; },
        get HINT_CREATE_PREVIEW() { return HINT_CREATE_PREVIEW; },
        get HelpHint() { return HelpHint; },
        get resetAll() { return resetAll; },
      };
}

const STORE_KEY = Symbol("projectStore");
export function setProjectStore(store: ReturnType<typeof createProjectStore>) {
  setContext(STORE_KEY, store);
}
export function getProjectStore() {
  return getContext<ReturnType<typeof createProjectStore>>(STORE_KEY);
}