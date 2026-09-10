<script lang="ts">
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
// Toolchain: слой совместимости Project Creator (легаси-поверхность tc_*)
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
import {
  buildReadme,
  type ReadmeInput,
  type ReadmeNamedItem,
} from "$lib/modules/project_creator/readme";
import { confirmProjectCreatedWithProfile } from "$lib/core/integration";
import { goto } from "$app/navigation";
import { deleteProfile } from "$lib/modules/devlauncher/api";
import { notifyError } from "$lib/core/toasts";

let tree = $state<WizardTreeData | null>(null);
let status = $state<string>("loading");
let hostOs = $state<string>("windows");
/** Краткое уведомление при авто-сбросе конфликтующих фреймворков */
let dropNotice = $state<string | null>(null);
/** Показывать ли отдельные карточки заблокированных фреймворков внутри уровня. */
let showUnavailable = $state<Record<string, boolean>>({});

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

/** Элемент README: id + i18n-ключ подписи (из wizard_tree). */
function readmeItems(
  ids: string[],
  defs: { id: string; label: string }[] | undefined,
): ReadmeNamedItem[] {
  return ids.map((id) => ({
    id,
    labelKey: defs?.find((d) => d.id === id)?.label ?? id,
  }));
}

/** Локализованный README.md: собирается из i18n-ключей на выбранном языке.
 *  Переводчик с явной локалью — код не содержит ветвлений RU/EN. */
let readmeContent = $derived.by(() => {
  const input: ReadmeInput = {
    projectName: conflictResolvedFolder ?? projectName,
    projectType: selectedType ? { id: selectedType.id, labelKey: selectedType.label } : null,
    backendLanguages: readmeItems(backendLangs, tree?.languages),
    frontendLanguages: readmeItems(frontendLangs, tree?.languages),
    frameworks: readmeItems(selectedFrameworks, tree?.frameworks),
    tools: readmeItems(selectedTools, tree?.tools),
    localInfraTools: readmeItems([...envLocalInfra], tree?.tools),
    architecture: archMode ?? "unknown",
    features: {
      docker: dockerEnabled(),
      testing,
      git,
      vscode,
      ci: false,
    },
  };
  return buildReadme(input, (key, vars) => i18n.translateIn(readmeLocale, key, vars));
});

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
let envTick = $state(0);
let tickTimer: ReturnType<typeof setInterval> | null = null;
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

function startTick() {
  if (tickTimer) return;
  tickTimer = setInterval(() => {
    envTick += 1;
  }, 1000);
}

function stopTick() {
  if (tickTimer) {
    clearInterval(tickTimer);
    tickTimer = null;
  }
}

let tooltipData = $state<{ x: number; y: number; tool: ToolDef } | null>(null);
let tooltipTimer: ReturnType<typeof setTimeout> | null = null;

function showTooltip(tool: ToolDef, e: MouseEvent | FocusEvent) {
  if (tooltipTimer) clearTimeout(tooltipTimer);
  // currentTarget жив только во время диспатча события — захватываем элемент
  // синхронно, а rect берём при показе (позиция может устареть из-за скролла).
  const target = e.currentTarget as HTMLElement | null;
  tooltipTimer = setTimeout(() => {
    const rect = target?.getBoundingClientRect();
    if (!rect) return;
    // Тултип у самой карточки, с привязкой к вьюпорту, чтобы не уезжал за край.
    const x = Math.max(8, Math.min(rect.left, window.innerWidth - 260));
    const y = Math.max(8, Math.min(rect.bottom + 8, window.innerHeight - 140));
    tooltipData = { x, y, tool };
  }, 300);
}

function hideTooltip() {
  if (tooltipTimer) clearTimeout(tooltipTimer);
  tooltipTimer = null;
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
 *  Ограничены project_language_map выбранного типа проекта. */
function frontendCandidates(): LanguageDef[] {
  if (!tree) return [];
  const allowed = selectedType ? tree.project_language_map[selectedType.id] : null;
  let langs = tree.languages.filter((l) => l.category === "frontend" || l.category === "static");
  if (allowed) langs = langs.filter((l) => allowed.includes(l.id));
  return langs;
}

/** Причина, по которой чистый язык нельзя выбрать (сторона уже занята). */
function languageBlockReason(lang: LanguageDef): string | null {
  const side = lang.category === "static" ? "frontend" : lang.category;
  const active = side === "frontend" ? frontendLangs : backendLangs;
  if (active.includes(lang.id)) return null;
  if (active.length > 0) return i18n.t("create.already_on_side", { list: active.map((l) => langLabel(l)).join(", ") });
  return null;
}

/** Развёрнутое объяснение блокировки чистого языка (подпись под бейджем) */
function languageBlockDetail(lang: LanguageDef): string | null {
  return null;
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
  return tree.tools.filter((t) => t.id !== "npm" && toolFitsStack(t));
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

function toggleTool(id: string) {
  const tool = tree?.tools.find((t) => t.id === id);
  if (!tool) return;

  if (selectedTools.includes(id)) {
    const dependents = tree!.tools.filter((t) => t.requires.includes(id));
    const toRemove = new Set([id, ...dependents.map((d) => d.id)]);
    selectedTools = selectedTools.filter((t) => !toRemove.has(t));
  } else {
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
  envInstalling = true;
  envInstallDone = false;
  envError = null;
  envLogs = [];
  envTaskStates = new Map();
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
      envDownload.set(event.task_id, { received: Number(dl[1]), total: Number(dl[2]) });
      envDownload = new Map(envDownload);
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
}

// ----------------------------------------------------------
// Генерация
// ----------------------------------------------------------

async function confirmAll() {
  const path = effectiveProjectPath();
  if (!path || !selectedFolder) return;
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
    readme_content: readmeContent,
  };
}

async function doCreateProject() {
  const path = effectiveProjectPath();
  if (!path || !selectedFolder || !selectedType) return;

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
    readme_content: readmeContent,
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
      if (createdSuccessfully && !devlAutoPopupShown && execPlan?.context) {
        const ctx = { ...execPlan.context, project_path: execPlan.project_path };
        try {
          const profile = await confirmProjectCreatedWithProfile(ctx);
          devlProfileCreated = true;
          devlShowReminder = true;
          devlProfileName = profile.name;
          devlProfilePath = profile.project_path;
          devlProfileExists = true;
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
  if (!execPlan?.project_path) return;
  try {
    await invoke("open_in_vscode", { path: execPlan.project_path });
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
</script>

<div class="wizard">
  <h1>{i18n.t("create.title") as TranslationKey}</h1>

  {#if status === "loading"}
    <div class="skeleton-wrap" aria-busy="true" aria-label={i18n.t("create.init") as TranslationKey}>
      <div class="skeleton-header">
        <div class="skeleton-bar skeleton-bar--title"></div>
      </div>
      <div class="skeleton-mode-switch">
        <div class="skeleton-pill"></div>
        <div class="skeleton-pill"></div>
        <div class="skeleton-pill"></div>
      </div>
      <div class="skeleton-builder">
        <div class="skeleton-phases">
          <div class="skeleton-phase"></div>
          <div class="skeleton-phase"></div>
          <div class="skeleton-phase"></div>
        </div>
        <div class="skeleton-content">
          <div class="skeleton-bar skeleton-bar--subtitle"></div>
          <div class="skeleton-grid">
            <div class="skeleton-card"></div>
            <div class="skeleton-card"></div>
            <div class="skeleton-card"></div>
            <div class="skeleton-card"></div>
          </div>
        </div>
        <div class="skeleton-sidebar">
          <div class="skeleton-bar skeleton-bar--sidebar"></div>
          <div class="skeleton-bar skeleton-bar--sidebar-short"></div>
        </div>
      </div>
    </div>
  {:else if status === "error"}
    <p class="error">{i18n.t("create.init_failed") as TranslationKey}</p>
  {:else if status === "empty"}
    <p class="muted">{i18n.t("create.no_types") as TranslationKey}</p>
  {:else}

    <div class="mode-switch">
      <button class="mode-btn" class:active={mode === "constructor"} onclick={() => { mode = "constructor"; }}>{i18n.t("create.mode.constructor") as TranslationKey}</button>
      <button class="mode-btn" class:active={mode === "presets"} onclick={() => { mode = "presets"; }}>{i18n.t("create.mode.templates") as TranslationKey}</button>
      <!-- Анализ временно скрыт: чтобы вернуть — раскомментируйте кнопку и
           строку AnalyzeMode в Promise.all в onMount. -->
      <!-- <button class="mode-btn" class:active={mode === "analyze"} onclick={() => { mode = "analyze"; }}>{i18n.t("create.mode.analyze") as TranslationKey}</button> -->
    </div>

    {#if mode === "analyze"}
      {#if AnalyzeMode}
        <AnalyzeMode
          {analyzing}
          {analyzedPath}
          {analysisError}
          {analysisResult}
          onrun={runAnalysis}
          onapply={applyAnalysis}
        />
      {/if}

    {:else if mode === "presets"}
      {#if PresetsMode}
        <PresetsMode {tree} onapply={applyPreset} />
      {/if}

    {:else}

      {#if phase === 5 || phase === 6}
        <!-- ================================================================
             Environment check & install
             ================================================================ -->
        {#if phase === 5}
          {#if EnvPanel}
            <EnvPanel
              {tree}
              {envCheck}
              {envChecking}
              {envCheckProgress}
              {envSelectedIds}
              {envLocalInfra}
              {envPlan}
              {envInstalling}
              {envInstallDone}
              {envErrors}
              {envLogs}
              {envTaskStates}
              {envRestartHint}
              {envDownload}
              {envPhaseStart}
              {envError}
              {newSecrets}
              {secretCopied}
              {installedTools}
              ontoggleEnvTool={toggleEnvTool}
              onselectAll={selectAllEnvTools}
              onoptInLocalInfra={optInLocalInfra}
              onrevertLocalInfra={revertLocalInfra}
              onstartInstall={startInstall}
              oncancelInstall={cancelInstall}
              onrecheck={recheckEnvironment}
              oncheck={() => runEnvironmentCheck()}
              oncontinue={doCreateProject}
              onback={back}
              onbackReview={() => (phase = 2)}
              oncopySecret={copySecret}
              ondismissSecrets={() => (newSecrets = null)}
            />
          {/if}
        {/if}

        <!-- ================================================================
             Execution
             ================================================================ -->
        {#if phase === 6}
          {#if ExecPanel}
            <ExecPanel
              {execPlan}
              {execProjectPath}
              {execStatuses}
              {execOverallStatus}
              {execResult}
              {execError}
              {execLogs}
              {devlProfileExists}
              oncancel={cancelExecution}
              onreset={resetAll}
              onopenvscode={openInVSCode}
              onreopendevl={reopenDevlDialog}
            />
          {/if}
        {/if}
      {:else}

      <!-- ================================================================
           Конструктор: слева фазы + содержимое, справа панель контекста
           ================================================================ -->
      <div class="builder">
        <div class="builder-left">
          <div class="phase-nav">
            {#each PHASES as name, i}
              <button
                class="phase-item"
                class:active={i === phase}
                class:done={i < phase}
                onclick={() => goPhase(i)}
              >
                <span class="phase-circle">{i < phase ? "✓" : i + 1}</span>
                <span class="phase-label">{name}</span>
              </button>
            {/each}
          </div>

          {#snippet archBanner()}
            {#if archMode}
              <div class="arch-banner arch-{archMode}" role="status">
                <div class="arch-body">
                  <p class="arch-title">
                    {archMode === "integrated"
                      ? (i18n.t("create.mode_integrated") as TranslationKey)
                      : (i18n.t("create.mode_decoupled") as TranslationKey)}
                  </p>
                  <p class="arch-text">
                    {archMode === "integrated"
                      ? (i18n.t("create.mode_integrated_desc") as TranslationKey)
                      : (i18n.t("create.mode_decoupled_desc") as TranslationKey)}
                  </p>
                  <p class="arch-examples">
                    {archMode === "integrated"
                      ? (i18n.t("create.mode_integrated_ex") as TranslationKey)
                      : (i18n.t("create.mode_decoupled_ex") as TranslationKey)}
                  </p>
                </div>
              </div>
            {/if}
          {/snippet}

          <!-- Phase 0: Project Type -->
          {#if phase === 0}
            <p class="prompt">{i18n.t("create.what_building") as TranslationKey}</p>
            <div class="card-grid type-grid">
              {#each tree!.project_types as pt}
                <button class="card" onclick={() => selectType(pt)}>
                  <TechIcon icon={pt.icon} alt={i18n.t(pt.label as TranslationKey)} size="xl" />
                  <h3>{i18n.t(pt.label as TranslationKey)}</h3>
                  <p>{i18n.t(pt.description as TranslationKey)}</p>
                </button>
              {/each}
            </div>
          {/if}

          <!-- Phase 1: Stack & Tools — одна скролл-страница -->
          {#if phase === 1}
            {@const fws = availableFrameworks()}
            {@const backendFws = fws.filter((f) => f.side === "backend")}
            {@const frontendFws = fws.filter((f) => f.side === "frontend")}
            {@const eitherFws = fws.filter((f) => f.side === "either")}

            {#snippet fwCard(fw: FrameworkDef)}
              {@const reason = frameworkBlockReason(fw.id)}
              {@const altInfo = reason !== null ? frameworkBlockInfo(fw.id) : null}
              {@const warnReason = frameworkWarnReason(fw.id)}
              {@const summary = fwLangSummary(fw)}
              <div class="fw-card-wrap">
                <button
                  class="card"
                  class:selected={selectedFrameworks.includes(fw.id)}
                  class:blocked={reason !== null}
                  title={altInfo?.detail}
                  onclick={() => clickFramework(fw.id)}
                >
                  {#if selectedFrameworks.includes(fw.id)}
                    <span class="card-check" aria-hidden="true">✓</span>
                  {/if}
                  <TechIcon icon={fw.icon} alt={i18n.t(fw.label as TranslationKey)} size="lg" />
                  <h3>{i18n.t(fw.label as TranslationKey)}</h3>
                  <p>{i18n.t(fw.description as TranslationKey)}</p>
                  {#if selectedFrameworks.includes(fw.id) && summary}
                    <span class="fw-lang-chip selected">{summary}</span>
                  {:else}
                    <span class="fw-lang-chip">{fwLangsLabel(fw)}</span>
                  {/if}
                  {#if fw.languages.length > 1}
                    <span class="fw-lang-multi">{i18n.t("create.choose_language") as TranslationKey}</span>
                  {/if}
                  {#if warnReason !== null}
                    <span class="warn-badge" title={warnReason}>⚠ {warnReason}</span>
                  {/if}
                  {#if reason}
                    <span class="conflict-badge">{reason}</span>
                    {#if altInfo?.detail}
                      <span class="conflict-detail">{altInfo.detail}</span>
                    {/if}
                    {#if altInfo?.alternatives?.length}
                      <span class="conflict-alts">
                        <span class="conflict-alts-label">{i18n.t("create.instead_of") as TranslationKey}</span>
                        {#each altInfo.alternatives as altId}
                          {@const altFw = tree?.frameworks.find((f) => f.id === altId)}
                          {#if altFw}
                            <span
                              role="button"
                              tabindex="0"
                              class="alt-chip"
                              onclick={(e) => {
                                e.stopPropagation();
                                clickFramework(altId);
                              }}
                              onkeydown={(e) => {
                                if (e.key === "Enter" || e.key === " ") {
                                  e.preventDefault();
                                  e.stopPropagation();
                                  clickFramework(altId);
                                }
                              }}
                            >
                              {i18n.t(altFw.label as TranslationKey)}
                            </span>
                          {/if}
                        {/each}
                      </span>
                    {/if}
                  {/if}
                </button>
                {#if fwPopup === fw.id}
                  <div class="fw-popup">
                    <p class="popup-title">{i18n.t(fw.label as TranslationKey)}</p>
                    {#if fw.qt_ui_options?.length}
                      {@const selectedMode = fw.qt_ui_options.find((m) => m.id === popupQtUi) ?? fw.qt_ui_options[0]}
                      {@const webDefs = (selectedMode.web_framework_options ?? [])
                        .map((wid) => tree?.frameworks.find((f) => f.id === wid))
                        .filter((d): d is FrameworkDef => !!d)}
                      <p class="popup-label">{i18n.t("create.ui_technology") as TranslationKey}</p>
                      <div class="popup-list">
                        {#each fw.qt_ui_options as mode}
                          <button
                            class="popup-opt stack"
                            class:selected={popupQtUi === mode.id}
                            onclick={() => (popupQtUi = mode.id)}
                          >
                            <span class="popup-opt-label">
                              {i18n.t(mode.label as TranslationKey)}
                              {#if (mode.web_framework_options ?? []).length > 0}
                                <span class="popup-opt-tag">{i18n.t("create.web_ui") as TranslationKey}</span>
                              {/if}
                            </span>
                            <span class="popup-opt-desc">{i18n.t(mode.description as TranslationKey)}</span>
                          </button>
                        {/each}
                      </div>
                      {#if webDefs.length > 0}
                        <p class="popup-label">{i18n.t("create.web_frontend") as TranslationKey}</p>
                        <div class="popup-list">
                          {#each webDefs as wf}
                            <button
                              class="popup-opt stack"
                              class:selected={popupWebFw === wf.id}
                              onclick={() => (popupWebFw = wf.id)}
                            >
                              <span class="popup-opt-label">{i18n.t(wf.label as TranslationKey)}</span>
                              <span class="popup-opt-desc">{i18n.t(wf.description as TranslationKey)}</span>
                            </button>
                          {/each}
                        </div>
                      {/if}
                    {:else if companionOptions(fw).length > 0}
                      <p class="popup-label">{i18n.t("create.frontend_framework") as TranslationKey}</p>
                      <div class="popup-list">
                        {#each companionOptions(fw) as c, ci}
                          <button
                            class="popup-opt"
                            class:selected={popupCompanion === c.id}
                            onclick={() => {
                              popupCompanion = c.id;
                              popupCompanionLang = c.recommended_language;
                            }}
                          >
                            <span>{i18n.t(c.label as TranslationKey)}</span>
                            {#if ci === 0}
                              <span class="star">{i18n.t("create.recommended") as TranslationKey}</span>
                            {/if}
                          </button>
                        {/each}
                      </div>
                      {#if popupCompanion}
                        {@const cfw = tree?.frameworks.find((f) => f.id === popupCompanion)}
                        {#if cfw}
                          <p class="popup-label">{i18n.t("create.frontend_language") as TranslationKey}</p>
                          <div class="popup-list">
                            {#each cfw.languages as l}
                              <button
                                class="popup-opt"
                                class:selected={popupCompanionLang === l}
                                onclick={() => (popupCompanionLang = l)}
                              >
                                <span>{langLabel(l)}</span>
                                {#if l === cfw.recommended_language}
                                  <span class="star">{i18n.t("create.recommended") as TranslationKey}</span>
                                {/if}
                              </button>
                            {/each}
                          </div>
                        {/if}
                      {/if}
                    {:else}
                      <p class="popup-label">{i18n.t("create.language") as TranslationKey}</p>
                      <div class="popup-list">
                        {#each fw.languages as l}
                          <button
                            class="popup-opt"
                            class:selected={popupLang === l}
                            onclick={() => (popupLang = l)}
                          >
                            <span>{langLabel(l)}</span>
                            {#if l === fw.recommended_language}
                              <span class="star">{i18n.t("create.recommended") as TranslationKey}</span>
                            {/if}
                          </button>
                        {/each}
                      </div>
                    {/if}
                    <div class="popup-actions">
                      <button class="btn-primary btn-xs" onclick={applyFwPopup}>{i18n.t("create.done") as TranslationKey}</button>
                      <button class="btn-secondary btn-xs" onclick={cancelFwPopup}>{i18n.t("create.cancel") as TranslationKey}</button>
                      {#if selectedFrameworks.includes(fw.id)}
                        <button class="btn-remove" onclick={() => removeFramework(fw.id)}>{i18n.t("create.remove") as TranslationKey}</button>
                      {/if}
                    </div>
                  </div>
                {/if}
              </div>
            {/snippet}

            <p class="prompt">{i18n.t("create.stack_tools") as TranslationKey}</p>
            {@render archBanner()}
            {#if selectedFrameworks.length > 0 || selectedTools.length > 0}
              <button
                class="btn-clear-stack"
                title={i18n.t("create.clear_stack_title") as TranslationKey}
                onclick={() => (confirmClearStack = true)}
              >
                {i18n.t("create.clear_stack") as TranslationKey}
              </button>
            {/if}
            {#if dropNotice}
              <p class="notice-bar" role="status">{dropNotice}</p>
            {/if}
            <p class="hint">
              {i18n.t("create.fw_hint") as TranslationKey}
            </p>

            {#if confirmClearStack}
              <div class="clear-overlay" onclick={() => (confirmClearStack = false)}>
                <div class="clear-dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
                  <h3>{i18n.t("create.clear_confirm_title") as TranslationKey}</h3>
                  <p>
                    {i18n.t("create.clear_confirm_body") as TranslationKey}
                  </p>
                  <div class="clear-actions">
                    <button class="btn-primary" onclick={() => clearStack()}>{i18n.t("create.clear_yes") as TranslationKey}</button>
                    <button class="btn-back" onclick={() => (confirmClearStack = false)}>{i18n.t("create.cancel") as TranslationKey}</button>
                  </div>
                </div>
              </div>
            {/if}

            {#snippet fwLevel(title: string, items: FrameworkDef[], note: string, levelKey: string)}
              {@const unavailable = unavailableFrameworks(items)}
              {@const visibleItems = availableFrameworksForDisplay(items, levelKey)}
              <details class="fw-level" open>
                <summary>
                  <TechIcon alt="" size="sm" />
                  <span class="fw-level-title">{title}</span>
                  <span class="fw-level-count">{items.length}</span>
                </summary>
                {#if unavailable.length > 0}
                  <div class="fw-level-toolbar">
                    <button
                      type="button"
                      class="fw-level-action"
                      onclick={() => toggleUnavailable(levelKey)}
                    >
                      {showUnavailable[levelKey] ? (i18n.t("create.hide_unavailable") as TranslationKey) : (i18n.t("create.show_unavailable") as TranslationKey)}
                    </button>
                  </div>
                {/if}
                <p class="fw-level-note">{note}</p>
                <div class="card-grid fw-grid">
                  {#each visibleItems as fw}
                    {@render fwCard(fw)}
                  {/each}
                  {#if unavailable.length > 0 && !showUnavailable[levelKey]}
                    <button
                      type="button"
                      class="unavailable-summary"
                      onclick={() => toggleUnavailable(levelKey)}
                    >
                      <strong>{i18n.t("create.unavailable_count", { n: unavailable.length }) as TranslationKey}</strong>
                      <span>{i18n.t("create.unavailable_desc") as TranslationKey}</span>
                    </button>
                  {/if}
                </div>
              </details>
            {/snippet}

            {#snippet territory(side: string, title: string, desc: string, items: FrameworkDef[], langs: string[])}
              <section class="territory territory-{side}">
                <header class="territory-head">
                  <TechIcon alt="" size="md" />
                  <div class="territory-title-wrap">
                    <h3 class="territory-title">{title}</h3>
                    <p class="territory-desc">{desc}</p>
                  </div>
                  <div class="territory-meta">
                    {#each langs as l}
                      <span class="territory-lang-chip">{langLabel(l)}</span>
                    {/each}
                    <span class="territory-count">{items.length}</span>
                  </div>
                </header>
                <div class="territory-body">
                  {#each FW_LEVELS as lvl}
                    {@const lvlItems = items.filter((f) => fwLevelOf(f) === lvl.id)}
                    {#if lvlItems.length > 0}
                      {@render fwLevel(lvl.title, lvlItems, lvl.note, `${side}-${lvl.id}`)}
                    {/if}
                  {/each}
                </div>
              </section>
            {/snippet}

            {#if hasBackend && backendFws.length > 0}
              {@render territory(
                "backend",
                i18n.t("create.terr_backend") as TranslationKey,
                i18n.t("create.terr_backend_desc") as TranslationKey,
                backendFws,
                backendLangs,
              )}
            {/if}
            {#if frontendFws.length > 0}
              {@render territory(
                "frontend",
                i18n.t("create.terr_frontend") as TranslationKey,
                i18n.t("create.terr_frontend_desc") as TranslationKey,
                frontendFws,
                frontendLangs,
              )}
            {/if}
            {#if eitherFws.length > 0}
              {@render territory(
                "either",
                i18n.t("create.terr_standalone") as TranslationKey,
                i18n.t("create.terr_standalone_desc") as TranslationKey,
                eitherFws,
                [],
              )}
            {/if}
            {#if !hasBackend}
              <p class="hint backendless-note">
                {i18n.t("create.no_backend_note") as TranslationKey}
              </p>
            {/if}
            {#if fws.length === 0}
              <p class="muted">{i18n.t("create.no_frameworks") as TranslationKey}</p>
            {/if}

            <!-- Языки без фреймворков (необязательно): чистый стек или поддержка -->
            <section class="territory territory-langs">
              <header class="territory-head">
                <TechIcon alt="" size="md" />
                <div class="territory-title-wrap">
                  <h3 class="territory-title">{i18n.t("create.plain_languages") as TranslationKey}</h3>
                  <p class="territory-desc">
                    {i18n.t("create.plain_languages_desc") as TranslationKey}
                  </p>
                </div>
              </header>
              <div class="territory-body">
                <div class="lang-sides">
                  {#if hasBackend && backendCandidates().length > 0}
                    <div class="lang-side">
                      <p class="lang-side-title">{i18n.t("create.backend_language") as TranslationKey}</p>
                      <p class="hint-sm">{i18n.t("create.one_per_side") as TranslationKey}</p>
                      <div class="card-grid lang-grid">
                        {#each backendCandidates() as lang}
                          {@const blockedReason = languageBlockReason(lang)}
                          {@const blockedDetail = languageBlockDetail(lang)}
                          <button
                            class="card"
                            class:selected={backendLangs.includes(lang.id)}
                            class:blocked={blockedReason !== null}
                            disabled={blockedReason !== null}
                            title={blockedDetail ?? undefined}
                            onclick={() => toggleLang("backend", lang.id)}
                          >
                            {#if backendLangs.includes(lang.id)}
                              <span class="card-check" aria-hidden="true">✓</span>
                            {/if}
                            <TechIcon icon={lang.icon} alt={i18n.t(lang.label as TranslationKey)} size="lg" />
                            <h3>{i18n.t(lang.label as TranslationKey)}</h3>
                            {#if backendLangs.includes(lang.id)}
                              <span class="fw-lang-chip selected">{i18n.t("create.active") as TranslationKey}</span>
                            {/if}
                            {#if blockedReason}
                              <span class="conflict-badge">{blockedReason}</span>
                            {/if}
                            {#if blockedDetail}
                              <span class="conflict-detail">{blockedDetail}</span>
                            {/if}
                          </button>
                        {/each}
                      </div>
                    </div>
                  {/if}
                  {#if frontendCandidates().length > 0}
                    <div class="lang-side">
                      <p class="lang-side-title">{i18n.t("create.frontend_language2") as TranslationKey}</p>
                      <p class="hint-sm">{i18n.t("create.one_per_side") as TranslationKey}</p>
                      <div class="card-grid lang-grid">
                        {#each frontendCandidates() as lang}
                          {@const blockedReason = languageBlockReason(lang)}
                          <button
                            class="card"
                            class:selected={frontendLangs.includes(lang.id)}
                            class:blocked={blockedReason !== null}
                            disabled={blockedReason !== null}
                            onclick={() => toggleLang("frontend", lang.id)}
                          >
                            {#if frontendLangs.includes(lang.id)}
                              <span class="card-check" aria-hidden="true">✓</span>
                            {/if}
                            <TechIcon icon={lang.icon} alt={i18n.t(lang.label as TranslationKey)} size="lg" />
                            <h3>{i18n.t(lang.label as TranslationKey)}</h3>
                            {#if lang.category === "static"}
                              <p>{i18n.t("create.plain_html") as TranslationKey}</p>
                            {/if}
                            {#if frontendLangs.includes(lang.id)}
                              <span class="fw-lang-chip selected">{i18n.t("create.active") as TranslationKey}</span>
                            {/if}
                            {#if blockedReason}
                              <span class="conflict-badge">{blockedReason}</span>
                            {/if}
                          </button>
                        {/each}
                      </div>
                    </div>
                  {/if}
                  {#if !hasBackend}
                    <p class="hint backendless-note">
                      {i18n.t("create.backend_lang_skipped") as TranslationKey}
                    </p>
                  {/if}
                </div>
              </div>
            </section>

            <!-- Инструменты и фичи -->
            <section class="territory territory-tools">
              <header class="territory-head">
                <TechIcon alt="" size="md" />
                <div class="territory-title-wrap">
                  <h3 class="territory-title">{i18n.t("create.tools_features") as TranslationKey}</h3>
                  <p class="territory-desc">{i18n.t("create.tools_features_desc") as TranslationKey}</p>
                </div>
                <span class="territory-count">{i18n.t("create.selected_count", { n: selectedTools.length }) as TranslationKey}</span>
              </header>
              <div class="territory-body">
                {#each TOOL_CATEGORIES as cat}
                  {@const catTools = availableTools().filter((t) => t.category === cat.id)}
                  {#if catTools.length > 0}
                    <div class="tool-group">
                      <p class="tool-cat-title">
                        <TechIcon alt="" size="xs" />
                        {cat.label}
                        <span class="tool-cat-count">{catTools.length}</span>
                      </p>
                      <div class="tool-menu">
                        {#each catTools as tool}
                          <button
                            class="tool-item"
                            class:selected={selectedTools.includes(tool.id)}
                            onclick={() => toggleTool(tool.id)}
                            onmouseenter={(e) => showTooltip(tool, e)}
                            onmouseleave={hideTooltip}
                            onfocus={(e) => showTooltip(tool, e)}
                            onblur={hideTooltip}
                          >
                            <TechIcon icon={tool.icon} alt={i18n.t(tool.label as TranslationKey)} size="md" />
                            <span class="tool-item-text">
                              <span class="tool-item-name">{i18n.t(tool.label as TranslationKey)}</span>
                              <span class="tool-item-desc">{i18n.t(tool.description as TranslationKey)}</span>
                            </span>
                            <span class="tool-item-badges">
                              {#if recommendedBadgeIds().includes(tool.id)}
                                <span class="tool-item-badge rec">{i18n.t("create.recommended") as TranslationKey}</span>
                              {/if}
                              {#if tool.requires_docker}
                                <span class="tool-item-badge docker"><TechIcon icon="docker.svg" alt="" size="xs" /> {i18n.t("create.docker_badge") as TranslationKey}</span>
                              {/if}
                              {#if tool.conflicts.length > 0}
                                <span class="tool-item-badge conflict">
                                  {i18n.t("create.conflicts_count", { n: tool.conflicts.length }) as TranslationKey}
                                </span>
                              {/if}
                              {#if selectedTools.includes(tool.id)}
                                <span class="tool-item-check">✓</span>
                              {/if}
                            </span>
                          </button>
                        {/each}
                      </div>
                    </div>
                  {/if}
                {/each}

                {#if tooltipData}
                  <div class="tooltip" style="left: {tooltipData.x}px; top: {tooltipData.y}px;">
                    <strong>{i18n.t(tooltipData.tool.label as TranslationKey)}</strong>
                    <p>{i18n.t(tooltipData.tool.description as TranslationKey)}</p>
                    {#if tooltipData.tool.requires.length > 0}
                      <p class="tt-req">{i18n.t("create.requires", { list: tooltipData.tool.requires.join(", ") }) as TranslationKey}</p>
                    {/if}
                    {#if tooltipData.tool.conflicts.length > 0}
                      <p class="tt-conf">{i18n.t("create.conflicts_with", { list: tooltipData.tool.conflicts.join(", ") }) as TranslationKey}</p>
                    {/if}
                    {#if tooltipData.tool.requires_docker}
                      <p class="tt-docker"><TechIcon icon="docker.svg" alt="" size="xs" /> {i18n.t("create.requires_docker") as TranslationKey}</p>
                    {/if}
                  </div>
                {/if}

                <div class="features-panel">
                  <p class="group-label">{i18n.t("create.features") as TranslationKey}</p>
                  <label class="feature-toggle">
                    <input type="checkbox" bind:checked={testing} />
                    <span>{i18n.t("create.feature.testing") as TranslationKey}</span>
                  </label>
                  <label class="feature-toggle">
                    <input type="checkbox" bind:checked={git} />
                    <span>{i18n.t("create.feature.git") as TranslationKey}</span>
                  </label>
                  <label class="feature-toggle">
                    <input type="checkbox" bind:checked={vscode} />
                    <span>{i18n.t("create.feature.vscode") as TranslationKey}</span>
                  </label>
                  <label class="feature-toggle">
                    <input type="checkbox" checked={dockerEnabled()} disabled />
                    <span>{i18n.t("create.feature.docker") as TranslationKey}{isDockerForced() ? ` ${i18n.t("create.feature.docker_forced") as TranslationKey}` : ""}</span>
                  </label>
                </div>
              </div>
            </section>

            <!-- Липкий футер: сводка + переход к финальной сверке -->
            <div class="mega-footer">
              <span class="mega-summary">
                {#if allSelectedLangs().length > 0}
                  <span class="mega-langs">{allSelectedLangs().map((l) => langLabel(l)).join(" · ")}</span>
                {/if}
                {i18n.t("create.summary_count", { f: selectedFrameworks.length, t: selectedTools.length }) as TranslationKey}
                {#if stackError}
                  <span class="mega-error">⚠ {stackError}</span>
                {/if}
              </span>
              <button class="btn-primary" onclick={() => (phase = 2)} disabled={!!stackError}>
                {i18n.t("create.review_create") as TranslationKey}
              </button>
            </div>

            <p class="grow-note">
              {i18n.t("create.catalog_grows_note") as TranslationKey}
            </p>
          {/if}

          <!-- Phase 2: Review -->
          {#if phase === 2}
            <p class="prompt">{i18n.t("create.review_create_short") as TranslationKey}</p>
            {@render archBanner()}

            <div class="project-name-section">
              <label class="pn-label" for="project-name">{i18n.t("create.project_name") as TranslationKey}</label>
              <input
                id="project-name"
                class="pn-input"
                type="text"
                placeholder={i18n.t("create.project_name_ph") as TranslationKey}
                bind:value={projectName}
                oninput={onProjectNameInput}
              />
              <div class="folder-row">
                <button class="btn-select-folder" onclick={pickProjectFolder}>
                  📁 {selectedFolder ? (i18n.t("create.change_folder") as TranslationKey) : (i18n.t("create.select_dest") as TranslationKey)}
                </button>
                {#if selectedFolder}
                  <span class="folder-path" title={selectedFolder}>{selectedFolder}</span>
                {/if}
              </div>
              {#if selectedFolder && projectName}
                <div class="path-preview">
                  <span class="pp-label">{i18n.t("create.full_path") as TranslationKey}</span>
                  <code class="pp-path">{effectiveProjectPath()}</code>
                  {#if folderCheckPending}
                    <span class="pp-checking">{i18n.t("create.checking") as TranslationKey}</span>
                  {:else if folderExists}
                    <span class="pp-exists">{i18n.t("create.folder_exists") as TranslationKey}</span>
                  {/if}
                </div>
              {/if}
            </div>

            <!-- README language: компактный переключатель RU/EN + пояснение -->
            <div class="readme-row">
              <span class="readme-label">{i18n.t("create.readme.toggle_label") as TranslationKey}</span>
              <button
                type="button"
                class="readme-switch"
                role="switch"
                aria-checked={readmeLocale === availableLocales[1].id}
                aria-label={i18n.t("create.readme.toggle_aria") as TranslationKey}
                onclick={toggleReadmeLocale}
              >
                <span class="readme-switch-knob"></span>
              </button>
              <span class="readme-lang">{readmeLocaleLabel}</span>
              <span class="readme-help-wrap">
                <button
                  type="button"
                  class="readme-help"
                  aria-label={i18n.t("create.readme.toggle_hint") as TranslationKey}
                  onmouseenter={() => (readmeHelpOpen = true)}
                  onmouseleave={() => (readmeHelpOpen = false)}
                  onfocus={() => (readmeHelpOpen = true)}
                  onblur={() => (readmeHelpOpen = false)}
                >
                  <Icon name="help" size={14} />
                </button>
                {#if readmeHelpOpen}
                  <span class="readme-tip" role="tooltip">
                    {i18n.t("create.readme.toggle_hint") as TranslationKey}
                  </span>
                {/if}
              </span>
            </div>

            <div class="preview-section">
              {#if PreviewPanel}
                <PreviewPanel
                  {selectedType}
                  {backendLangs}
                  {frontendLangs}
                  {selectedFrameworks}
                  {selectedTools}
                  {envLocalInfra}
                  {testing}
                  {git}
                  {vscode}
                  {readmeLocale}
                  {readmeContent}
                  projectName={projectName || ""}
                  projectFolder={selectedFolder || ""}
                  bind:removedStepIds
                />
              {/if}
            </div>

            {#if showConflictDialog}
              <div class="conflict-overlay" onclick={() => { showConflictDialog = false; }}>
                <div class="conflict-dialog" onclick={(e) => e.stopPropagation()}>
                  <h3>{i18n.t("create.folder_exists") as TranslationKey}</h3>
                  <p>
                    {i18n.t("create.folder_conflict_body") as TranslationKey}
                  </p>
                  <div class="conflict-actions">
                    <button class="btn-primary" onclick={() => resolveFolderConflict('overwrite')}>
                      {i18n.t("create.overwrite") as TranslationKey}
                    </button>
                    <button class="btn-secondary" onclick={() => resolveFolderConflict('auto-rename')}>
                      {i18n.t("create.auto_rename", { name: projectName }) as TranslationKey}
                    </button>
                    <button class="btn-back" onclick={() => resolveFolderConflict('cancel')}>
                      {i18n.t("create.use_other_name") as TranslationKey}
                    </button>
                  </div>
                </div>
              </div>
            {/if}

            {#if reviewError}
              <p class="review-error" role="alert">⛔ {i18n.t(reviewError as TranslationKey)}</p>
            {/if}
            {#if stackError && (!projectName || !selectedFolder)}
              <p class="review-hint">⚠ {stackError}</p>
            {/if}

            <div class="btn-row">
              <button class="btn-back" onclick={back}>{i18n.t("create.back") as TranslationKey}</button>
              <button
                class="btn-primary create-btn"
                disabled={!projectName || !selectedFolder || stackError !== null}
                title={stackError ?? undefined}
                onclick={confirmAll}
              >
                {i18n.t("create.create_project") as TranslationKey}
              </button>
            </div>
            {#if !projectName || !selectedFolder}
              <p class="review-hint">
                {!projectName ? (i18n.t("create.enter_name") as TranslationKey) : (i18n.t("create.select_folder_first") as TranslationKey)}
                {stackError ? ` · ${stackError}` : ""}
              </p>
            {/if}
          {/if}
        </div>

        <!-- ============ Панель контекста (правая колонка) ============ -->
        <aside class="builder-context">
          <p class="ctx-title">{i18n.t("create.your_stack") as TranslationKey}</p>

          {#if stackIssues.length > 0}
            <div class="stack-issues">
              {#each stackIssues as issue}
                {@const isError = issue.severity !== "Warning"}
                {@const text = issue.message_key ? i18n.t(issue.message_key as TranslationKey, issue.args) : issue.message}
                <div class="stack-issue" class:error={isError} class:warning={!isError}>
                  <span class="icon">{isError ? "⛔" : "⚠️"}</span>
                  <span class="message">{text}</span>
                  {#if issue.args?.recommendation}
                    <div class="recommendation">💡 {issue.args.recommendation}</div>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}

          <div class="ctx-group">
            <div class="ctx-row">
              <span class="ctx-label">{i18n.t("create.project_type") as TranslationKey}</span>
              <span class="ctx-value">{selectedType ? (i18n.t(selectedType.label as TranslationKey)) : "—"}</span>
              <button class="btn-change" onclick={() => goPhase(0)}>{i18n.t("create.change") as TranslationKey}</button>
            </div>
            <div class="ctx-row">
              <span class="ctx-label">{i18n.t("create.backend") as TranslationKey}</span>
              <span class="ctx-value">
                {hasBackend
                  ? (backendLangs.length > 0 ? backendLangs.map((l) => langLabel(l)).join(", ") : (i18n.t("create.none") as TranslationKey))
                  : (i18n.t("create.na_no_backend") as TranslationKey)}
              </span>
              {#if hasBackend}
                <button class="btn-change" onclick={() => goPhase(1)}>{i18n.t("create.change") as TranslationKey}</button>
              {/if}
            </div>
            <div class="ctx-row">
              <span class="ctx-label">{i18n.t("create.frontend") as TranslationKey}</span>
              <span class="ctx-value">{frontendLangs.length > 0 ? frontendLangs.map((l) => langLabel(l)).join(", ") : (i18n.t("create.none") as TranslationKey)}</span>
              <button class="btn-change" onclick={() => goPhase(1)}>{i18n.t("create.change") as TranslationKey}</button>
            </div>
            <div class="ctx-row">
              <span class="ctx-label">{i18n.t("create.frameworks") as TranslationKey}</span>
              <span class="ctx-value">
                {selectedFrameworks.length > 0
                  ? selectedFrameworks
                      .map((id) => {
                        const f = tree?.frameworks.find((x) => x.id === id);
                        return f ? i18n.t(f.label as TranslationKey) : id;
                      })
                      .join(", ")
                  : (i18n.t("create.none") as TranslationKey)}
              </span>
              <button class="btn-change" onclick={() => goPhase(1)}>{i18n.t("create.change") as TranslationKey}</button>
            </div>
            <div class="ctx-row">
              <span class="ctx-label">{i18n.t("create.tools") as TranslationKey}</span>
              <span class="ctx-value">
                {selectedTools.length > 0
                  ? selectedTools
                      .map((id) => {
                        const t = tree?.tools.find((x) => x.id === id);
                        return t ? i18n.t(t.label as TranslationKey) : id;
                      })
                      .join(", ")
                  : (i18n.t("create.none") as TranslationKey)}
              </span>
              <button class="btn-change" onclick={() => goPhase(1)}>{i18n.t("create.change") as TranslationKey}</button>
            </div>
            <div class="ctx-row">
              <span class="ctx-label">{i18n.t("create.features") as TranslationKey}</span>
              <span class="ctx-value">
                {[
                  git && i18n.t("create.feature.git"),
                  testing && i18n.t("create.feature.testing"),
                  vscode && i18n.t("create.feature.vscode"),
                  dockerEnabled() && i18n.t("create.feature.docker"),
                ].filter(Boolean).join(", ") || (i18n.t("create.none") as TranslationKey)}
              </span>
              <button class="btn-change" onclick={() => goPhase(1)}>{i18n.t("create.change") as TranslationKey}</button>
            </div>
          </div>

          {#if phase !== 2}
            <button
              class="ctx-cta"
              onclick={() => goPhase(2)}
              disabled={!!stackError}
              title={stackError ?? undefined}
            >
              {i18n.t("create.review_create") as TranslationKey}
            </button>
          {/if}
        </aside>
      </div>
      {/if}
    {/if}
  {/if}
</div>

<!-- DevLauncher integration: dialogs (profile created / cancel profile) -->
{#if DevlDialogs}
  <DevlDialogs
    created={devlProfileCreated}
    showReminder={devlShowReminder}
    confirmCancel={devlConfirmCancel}
    onopen={openDevLauncher}
    onclose={dismissProfileOk}
    oncancel={cancelProfile}
    onconfirmcancel={confirmCancelProfile}
    ondismisscancel={dismissCancelConfirm}
  />
{/if}

<style>
.wizard { max-width: 1280px; margin: 0 auto; padding: 2rem; }
.muted { color: var(--sp-text-3); }
.error { color: var(--sp-danger); }
.mode-switch { display: flex; gap: 0; margin-bottom: 1.5rem; border-radius: 8px; overflow: hidden; border: 1px solid var(--sp-border-strong); width: fit-content; }
.mode-btn { padding: 0.5rem 1.25rem; cursor: pointer; border: none; background: var(--sp-bg-1); color: var(--sp-text-2); font-size: 0.9rem; }
.mode-btn.active { background: var(--sp-accent-soft); color: #fff; font-weight: 600; }
.prompt { font-size: 1.4rem; font-weight: 700; margin-bottom: 0.4rem; }
.hint { color: var(--sp-text-3); margin-bottom: 1.5rem; font-size: 0.95rem; }
.backendless-note { border-left: 3px solid var(--sp-accent-strong); padding: 0.35rem 0.75rem; background: var(--sp-bg-1); margin: 0.75rem 0; }

/* ---- Конструктор: две колонки ---- */
.builder { display: grid; grid-template-columns: 1fr 320px; gap: 1.5rem; align-items: start; }
.builder-left { min-width: 0; }
.builder-context {
  position: sticky;
  top: 1rem;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--sp-border);
  border-radius: 12px;
  background: var(--sp-bg-1);
  padding: 0.85rem;
}
.ctx-title {
  font-weight: 700;
  font-size: 0.75rem;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  margin: 0 0 0.25rem;
  padding-bottom: 0.6rem;
  color: var(--sp-text-3);
  border-bottom: 1px solid var(--sp-border-faint);
}
.ctx-group { display: flex; flex-direction: column; }
.ctx-row { display: flex; align-items: flex-start; gap: 0.5rem; padding: 0.5rem 0; border-bottom: 1px solid var(--sp-border-faint); }
.ctx-row:last-child { border-bottom: none; }
.ctx-label {
  flex: 0 0 88px;
  font-weight: 600;
  color: var(--sp-text-3);
  font-size: 0.72rem;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  padding-top: 0.15rem;
}
.ctx-value { flex: 1; font-size: 0.82rem; color: var(--sp-text-1); overflow-wrap: anywhere; }
.btn-change {
  background: none;
  border: none;
  color: var(--sp-accent);
  padding: 0.1rem 0.15rem;
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.72rem;
  flex: 0 0 auto;
  opacity: 0.75;
}
.btn-change:hover { color: var(--sp-accent-strong); text-decoration: underline; opacity: 1; }
.ctx-cta {
  margin-top: 0.85rem;
  width: 100%;
  background: var(--sp-accent-strong);
  color: #fff;
  border: none;
  border-radius: 8px;
  padding: 0.6rem 1rem;
  font-weight: 600;
  font-size: 0.88rem;
  cursor: pointer;
  box-shadow: var(--sp-shadow-1);
  transition: background 0.15s, box-shadow 0.15s;
}
.ctx-cta:hover { background: var(--sp-accent); box-shadow: var(--sp-shadow-accent); }
.ctx-cta:disabled { opacity: 0.5; cursor: not-allowed; }

/* ---- Фазы (slim stepper) ---- */
.phase-nav { display: flex; align-items: center; gap: 0; margin-bottom: 1.75rem; flex-wrap: wrap; }
.phase-item {
  display: flex;
  align-items: center;
  gap: 0.45rem;
  background: none;
  border: none;
  cursor: pointer;
  color: var(--sp-text-3);
  font-size: 0.8rem;
  padding: 0.3rem 0.75rem;
}
.phase-item + .phase-item::before {
  content: "";
  width: 26px;
  height: 1px;
  background: var(--sp-border-strong);
  margin-right: 0.75rem;
  flex-shrink: 0;
}
.phase-item:hover { color: var(--sp-text-2); }
.phase-item.active { color: #fff; }
.phase-item.active .phase-circle { background: var(--sp-accent-strong); color: #fff; box-shadow: 0 0 0 3px var(--sp-accent-soft); }
.phase-item.done .phase-circle { background: var(--sp-success); color: #0c0d11; }
.phase-circle {
  width: 22px;
  height: 22px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--sp-bg-2);
  border: 1px solid var(--sp-border);
  font-weight: 700;
  font-size: 0.7rem;
  transition: background 0.15s, border-color 0.15s;
}
.phase-label { font-weight: 600; }

/* ---- Стороны (Stack) ---- */
.side-section { min-width: 0; }

/* ---- Баннер архитектуры (Integrated / Decoupled) ---- */
.arch-banner {
  display: flex;
  align-items: flex-start;
  gap: 0.75rem;
  border: 1px solid var(--sp-border);
  border-radius: 10px;
  padding: 0.7rem 0.9rem;
  margin: 0 0 1rem;
  background: var(--sp-bg-1);
}
.arch-integrated { border-color: rgba(132, 204, 22, 0.35); background: rgba(132, 204, 22, 0.05); }
.arch-decoupled { border-color: var(--sp-accent-border); background: var(--sp-accent-soft); }
.arch-body { flex: 1; min-width: 0; }
.arch-title { margin: 0 0 0.15rem; font-size: 0.85rem; font-weight: 700; color: var(--sp-text-1); }
.arch-integrated .arch-title { color: var(--sp-success); }
.arch-decoupled .arch-title { color: var(--sp-accent); }
.arch-text { margin: 0; font-size: 0.82rem; color: var(--sp-text-2); line-height: 1.35; }
.arch-examples { margin: 0.2rem 0 0; font-size: 0.72rem; color: var(--sp-text-3); }

/* ---- Уровни фреймворков (сворачиваемые колонки) ---- */
.fw-level {
  border: 1px solid var(--sp-border);
  border-radius: 10px;
  background: var(--sp-bg-2);
  margin-bottom: 0.75rem;
}
.fw-level > summary {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.55rem 0.9rem;
  cursor: pointer;
  list-style: none;
  user-select: none;
  border-bottom: 1px solid transparent;
  border-radius: 10px 10px 0 0;
}
.fw-level > summary::-webkit-details-marker { display: none; }
.fw-level > summary::before {
  content: "▸";
  color: var(--sp-accent-strong);
  font-size: 0.8rem;
  transition: transform 0.15s;
}
.fw-level[open] > summary::before { transform: rotate(90deg); }
.fw-level[open] > summary { border-bottom-color: var(--sp-border); }
.fw-level > summary:hover { background: rgba(255, 255, 255, 0.03); }
.fw-level-title { font-weight: 700; font-size: 0.9rem; color: var(--sp-text-1); }
.fw-level-toolbar { display: flex; justify-content: flex-end; padding: 0.45rem 0.9rem 0; }
.fw-level-action { margin-left: 0.5rem; border: 1px solid var(--sp-accent-border); border-radius: 6px; padding: 0.2rem 0.45rem; background: transparent; color: var(--sp-text-2); cursor: pointer; font-size: 0.68rem; white-space: nowrap; }
.fw-level-action:hover { border-color: var(--sp-accent-strong); color: #fff; }
.fw-level-count {
  margin-left: auto;
  font-size: 0.72rem;
  color: var(--sp-text-3);
  background: var(--sp-bg-2);
  border-radius: 999px;
  padding: 0.1rem 0.55rem;
}
.fw-level-note { margin: 0; padding: 0.4rem 0.9rem 0.6rem; font-size: 0.75rem; color: var(--sp-text-3); }
.fw-level .fw-grid { margin-bottom: 0; padding: 0.9rem; padding-top: 0.2rem; }

/* ---- Карточки (unified selectable card) ---- */
.card-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: 0.75rem; margin-bottom: 1.5rem; }
.card {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.4rem;
  padding: 1rem;
  border: 1px solid var(--sp-border);
  border-radius: 10px;
  background: var(--sp-bg-1);
  cursor: pointer;
  transition: border-color 0.15s, background 0.15s, box-shadow 0.15s;
  text-align: center;
  color: var(--sp-text-1);
}
.card:hover { border-color: var(--sp-accent-border); background: var(--sp-bg-2); }
.card.selected {
  border-color: var(--sp-accent);
  background: var(--sp-accent-soft);
  box-shadow: var(--sp-shadow-accent);
}
.card.blocked { opacity: 0.4; cursor: not-allowed; border-color: var(--sp-border); background: var(--sp-bg-1); }
.card.blocked:hover { border-color: var(--sp-border); background: var(--sp-bg-1); }
.card-check {
  position: absolute;
  top: 0.5rem;
  right: 0.5rem;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: var(--sp-accent-strong);
  color: #fff;
  font-size: 0.7rem;
  font-weight: 700;
  box-shadow: var(--sp-shadow-1);
}

/* Утилиты блокировки (клиентская оболочка): карточка видна, но явно
   недоступна — «не молча некликабельна», с подписью и тултипом. */
.opacity-50 { opacity: 0.5; }
.opacity-40 { opacity: 0.4; }
.grayscale { filter: grayscale(1); }
.cursor-not-allowed { cursor: not-allowed; }
.opacity-50:hover,
.opacity-40:hover,
.grayscale:hover { border-color: var(--sp-border-strong); background: var(--sp-bg-1); }
.card h3 { margin: 0; font-size: 0.95rem; }
.card p { margin: 0; font-size: 0.78rem; color: var(--sp-text-3); }
.type-grid { grid-template-columns: repeat(4, 1fr); }
.fw-grid { grid-template-columns: repeat(4, 1fr); grid-auto-rows: 1fr; align-items: stretch; }
.fw-grid .card { min-height: 200px; height: 100%; box-sizing: border-box; }
.fw-grid .card p { flex: 1; }
.unavailable-summary { min-height: 200px; display: flex; flex-direction: column; justify-content: center; gap: 0.5rem; padding: 1rem; border: 1px dashed var(--sp-border-strong); border-radius: 10px; background: var(--sp-bg-1); color: var(--sp-text-2); cursor: pointer; text-align: center; }
.unavailable-summary:hover { border-color: var(--sp-accent-strong); color: var(--sp-text-1); background: var(--sp-bg-2); }
.unavailable-summary strong { color: var(--sp-text-1); font-size: 0.9rem; }
.unavailable-summary span { color: var(--sp-text-3); font-size: 0.75rem; }
.fw-grid .card .fw-lang-chip,
.fw-grid .card .fw-lang-multi,
.fw-grid .card .conflict-badge { margin-top: 0.3rem; }
.conflict-badge { display: block; font-size: 0.7rem; color: var(--sp-danger); margin-top: 0.25rem; }
.conflict-detail { display: block; font-size: 0.68rem; color: var(--sp-danger); margin-top: 0.15rem; line-height: 1.25; }
.warn-badge { display: block; font-size: 0.68rem; color: var(--sp-warning); background: rgba(245, 158, 11, 0.12); border: 1px solid rgba(245, 158, 11, 0.35); border-radius: 6px; padding: 0.1rem 0.45rem; margin-top: 0.25rem; }
.conflict-alts { display: flex; flex-wrap: wrap; gap: 0.3rem; align-items: center; margin-top: 0.35rem; }
.conflict-alts-label { font-size: 0.68rem; color: var(--sp-text-3); }
.alt-chip {
  display: inline-block;
  font-size: 0.68rem;
  color: var(--sp-accent);
  background: var(--sp-accent-soft);
  border: 1px solid var(--sp-accent-border);
  border-radius: 999px;
  padding: 0.1rem 0.5rem;
  cursor: pointer;
  transition: all 0.15s;
}
.alt-chip:hover { background: var(--sp-accent-border); color: #fff; }
.alt-chip:focus-visible { outline: 2px solid var(--sp-accent-strong); }
.notice-bar { display: block; font-size: 0.78rem; color: var(--sp-warning); background: rgba(245, 158, 11, 0.12); border: 1px solid rgba(245, 158, 11, 0.35); border-radius: 8px; padding: 0.45rem 0.7rem; margin-bottom: 0.8rem; }
.fw-lang-chip { display: inline-block; font-size: 0.72rem; color: var(--sp-accent); background: var(--sp-accent-soft); border: 1px solid var(--sp-accent-border); padding: 0.15rem 0.5rem; border-radius: 999px; margin-top: 0.3rem; }
.fw-lang-chip.selected { color: var(--sp-success); background: rgba(132, 204, 22, 0.12); border-color: rgba(132, 204, 22, 0.4); }
.fw-lang-multi { display: block; font-size: 0.68rem; color: var(--sp-accent); margin-top: 0.15rem; }

/* ---- Попап настройки фреймворка ---- */
.fw-card-wrap { position: relative; }
.fw-popup {
  position: absolute;
  top: calc(100% + 6px);
  left: 0;
  z-index: 50;
  width: 260px;
  max-width: 90vw;
  background: var(--sp-bg-2);
  border: 1px solid var(--sp-accent-strong);
  border-radius: 10px;
  padding: 0.8rem;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
}
.popup-title { margin: 0 0 0.5rem; font-size: 0.85rem; font-weight: 700; color: #fff; }
.popup-label { margin: 0.5rem 0 0.3rem; font-size: 0.72rem; color: var(--sp-text-2); text-transform: uppercase; letter-spacing: 0.04em; }
.popup-list { display: flex; flex-direction: column; gap: 0.3rem; max-height: 160px; overflow-y: auto; }
.popup-opt {
  display: flex; justify-content: space-between; align-items: center; gap: 0.5rem;
  background: var(--sp-bg-2); border: 1px solid var(--sp-accent-border); color: var(--sp-text-1);
  padding: 0.45rem 0.6rem; border-radius: 6px; cursor: pointer; font-size: 0.82rem; text-align: left;
}
.popup-opt:hover { border-color: var(--sp-accent-strong); }
.popup-opt.selected { border-color: var(--sp-accent-strong); background: var(--sp-accent-soft); color: #fff; }
.popup-opt.stack { flex-direction: column; align-items: stretch; gap: 0.2rem; }
.popup-opt-label { display: flex; align-items: center; gap: 0.45rem; font-weight: 600; }
.popup-opt-desc { font-size: 0.7rem; color: var(--sp-text-3); line-height: 1.35; font-weight: 400; }
.popup-opt.selected .popup-opt-desc { color: var(--sp-text-2); }
.popup-opt-tag {
  font-size: 0.62rem;
  color: var(--sp-info);
  background: rgba(6, 182, 212, 0.12);
  border: 1px solid rgba(6, 182, 212, 0.35);
  border-radius: 999px;
  padding: 0.05rem 0.45rem;
  font-weight: 600;
}
.star { color: var(--sp-warning); font-size: 0.72rem; white-space: nowrap; }
.popup-actions { display: flex; gap: 0.4rem; margin-top: 0.7rem; align-items: center; flex-wrap: wrap; }
.btn-xs { padding: 0.3rem 0.7rem; font-size: 0.78rem; }
.btn-remove { background: none; border: 1px solid rgba(239, 68, 68, 0.35); color: var(--sp-danger); padding: 0.3rem 0.7rem; border-radius: 6px; cursor: pointer; font-size: 0.78rem; }
.btn-remove:hover { background: rgba(239, 68, 68, 0.12); }

/* ---- Липкий футер страницы стека ---- */
.mega-footer {
  position: sticky;
  bottom: 0;
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 1rem;
  margin-top: 1.25rem;
  padding: 0.75rem 1rem;
  background: var(--sp-glass-strong);
  border: 1px solid var(--sp-border);
  border-radius: 10px;
  backdrop-filter: blur(4px);
  z-index: 40;
}
.mega-summary { font-size: 0.85rem; color: var(--sp-text-2); display: flex; align-items: center; gap: 0.6rem; flex-wrap: wrap; }
.mega-langs {
  font-size: 0.78rem;
  color: var(--sp-accent);
  background: var(--sp-accent-soft);
  border: 1px solid var(--sp-accent-border);
  padding: 0.15rem 0.55rem;
  border-radius: 999px;
}
.mega-error { color: var(--sp-danger); font-size: 0.78rem; }
.hint-sm { font-size: 0.75rem; color: var(--sp-text-3); margin: 0 0 0.5rem; }
.tauri-note { display: block; margin-top: 0.5rem; font-size: 0.85rem; color: var(--sp-accent-strong); }

/* ---- Территории стека ---- */
.territory {
  position: relative;
  border: 1px solid var(--sp-border);
  border-radius: 12px;
  background: var(--sp-bg-1);
  margin-bottom: 1rem;
}
.territory-head {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.7rem 1rem;
  background: var(--sp-bg-2);
  border-bottom: 1px solid var(--sp-border);
  border-radius: 11px 11px 0 0;
}
.territory-title-wrap { flex: 1; min-width: 0; }
.territory-title { margin: 0; font-size: 0.95rem; font-weight: 700; color: var(--sp-text-1); }
.territory-desc { margin: 0.15rem 0 0; font-size: 0.78rem; color: var(--sp-text-3); }
.territory-meta { display: flex; align-items: center; gap: 0.35rem; flex-wrap: wrap; }
.territory-lang-chip {
  font-size: 0.72rem;
  color: var(--sp-accent);
  background: var(--sp-accent-soft);
  border: 1px solid var(--sp-accent-border);
  padding: 0.15rem 0.5rem;
  border-radius: 999px;
}
.territory-count {
  font-size: 0.72rem;
  color: var(--sp-text-3);
  background: var(--sp-bg-2);
  border-radius: 999px;
  padding: 0.15rem 0.55rem;
}
.territory-body { padding: 0.9rem; }
.territory-body .fw-level { margin-bottom: 0.5rem; }
.territory-body .fw-level:last-child { margin-bottom: 0; }
.territory-body .card-grid { margin-bottom: 0.25rem; }

/* ---- Чистые языки (plain language picker) ---- */
.lang-sides { display: grid; grid-template-columns: 1fr 1fr; gap: 1.25rem; }
@media (max-width: 900px) { .lang-sides { grid-template-columns: 1fr; } }
.lang-side-title { margin: 0 0 0.15rem; font-size: 0.85rem; font-weight: 700; color: var(--sp-text-1); }
.lang-grid { grid-template-columns: repeat(auto-fill, minmax(170px, 1fr)); margin-bottom: 0; }

/* ---- Инструменты ---- */
.tool-group {
  margin-bottom: 1.5rem;
  padding-top: 1.2rem;
  border-top: 1px solid var(--sp-border-strong);
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}
.tool-group:first-child {
  border-top: none;
  padding-top: 0;
}
.tool-cat-title {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  margin: 0 0 0.5rem;
  font-size: 0.8rem;
  font-weight: 700;
  color: var(--sp-text-2);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}
.tool-cat-count {
  font-size: 0.68rem;
  color: var(--sp-text-3);
  background: var(--sp-bg-2);
  border-radius: 999px;
  padding: 0.1rem 0.5rem;
  font-weight: 600;
}
.tool-menu { display: grid; grid-template-columns: repeat(2, 1fr); gap: 0.5rem; }
.tool-item {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  width: 100%;
  min-width: 0;
  background: var(--sp-bg-2);
  border: 1px solid var(--sp-border-strong);
  border-radius: 9px;
  padding: 0.6rem 0.8rem;
  cursor: pointer;
  text-align: left;
  color: var(--sp-text-1);
  transition: border-color 0.15s, background 0.15s, transform 0.15s;
}
.tool-item:hover { border-color: var(--sp-accent-strong); background: var(--sp-bg-2); transform: translateY(-1px); }
.tool-item.selected { border-color: var(--sp-accent-strong); background: var(--sp-accent-soft); box-shadow: inset 0 0 0 1px var(--sp-accent-strong); }
.tool-item-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 0.1rem; }
.tool-item-name { font-size: 0.88rem; font-weight: 600; }
.tool-item-desc { font-size: 0.74rem; color: var(--sp-text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.tool-item-badges { display: flex; align-items: center; justify-content: flex-end; gap: 0.35rem; flex: 0 0 auto; flex-wrap: wrap; }
.tool-item-badge { font-size: 0.68rem; padding: 0.12rem 0.45rem; border-radius: 999px; white-space: nowrap; }
.tool-item-badge.docker { color: var(--sp-info); background: rgba(6, 182, 212, 0.12); border: 1px solid rgba(6, 182, 212, 0.35); }
.tool-item-badge.conflict { color: var(--sp-danger); background: rgba(239, 68, 68, 0.12); border: 1px solid rgba(239, 68, 68, 0.35); }
.tool-item-badge.rec { color: var(--sp-warning); background: rgba(245, 158, 11, 0.12); border: 1px solid rgba(245, 158, 11, 0.35); }
.tool-item-check { color: var(--sp-accent-strong); font-weight: 700; font-size: 0.95rem; }
.group-label { font-size: 0.85rem; font-weight: 600; color: var(--sp-text-3); text-transform: uppercase; letter-spacing: 0.04em; }
.tooltip {
  position: fixed;
  background: var(--sp-bg-1);
  border: 1px solid var(--sp-accent-strong);
  border-radius: 8px;
  padding: 0.6rem 0.9rem;
  font-size: 0.8rem;
  max-width: 240px;
  z-index: 999;
  pointer-events: none;
  color: var(--sp-text-2);
  opacity: 0.94;
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.45);
  animation: tooltip-in 0.14s ease-out;
}
@keyframes tooltip-in {
  from { opacity: 0; transform: translateY(3px); }
  to { opacity: 0.94; transform: translateY(0); }
}
.tooltip strong { color: #fff; }
.tt-req, .tt-conf, .tt-docker { margin: 0.2rem 0; font-size: 0.75rem; }
.features-panel { margin-bottom: 1.5rem; display: flex; flex-wrap: wrap; gap: 1rem; align-items: center; }
.feature-toggle { display: flex; align-items: center; gap: 0.4rem; cursor: pointer; font-size: 0.9rem; }
.feature-toggle input { accent-color: var(--sp-accent-strong); }

/* ---- Кнопки ---- */
.btn-row { display: flex; gap: 0.75rem; margin-top: 1.5rem; flex-wrap: wrap; }.btn-back { background: none; border: 1px solid var(--sp-border-strong); color: var(--sp-text-3); padding: 0.4rem 0.9rem; border-radius: 6px; cursor: pointer; font-size: 0.85rem; }
.btn-back:hover { border-color: var(--sp-accent-strong); color: #fff; }
.btn-primary { background: var(--sp-accent-strong); color: #fff; padding: 0.6rem 1.5rem; border-radius: 8px; border: none; cursor: pointer; font-weight: 600; font-size: 0.95rem; }
.btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
.btn-secondary { background: var(--sp-accent-soft); color: var(--sp-text-2); padding: 0.6rem 1.5rem; border-radius: 8px; border: 1px solid var(--sp-border-strong); cursor: pointer; font-size: 0.95rem; }
.create-btn { font-size: 1.1rem; padding: 0.75rem 2rem; }
.review-error { color: var(--sp-danger); font-weight: 600; font-size: 0.9rem; margin: 1rem 0 0; }
.review-hint { color: var(--sp-text-3); font-size: 0.8rem; margin: 0.5rem 0 0; }

/* ---- Проблемы стека ---- */
.stack-issues { border: 1px solid var(--sp-border-strong); border-radius: 10px; padding: 0.8rem 1rem; margin-bottom: 0.75rem; background: var(--sp-bg-2); }
.stack-issue { display: flex; flex-wrap: wrap; gap: 0.3rem 0.4rem; align-items: flex-start; margin: 0.4rem 0; font-size: 0.8rem; line-height: 1.35; }
.stack-issue.error { color: var(--sp-danger); }
.stack-issue.warning { color: var(--sp-warning); }
.stack-issue .recommendation { flex-basis: 100%; margin-top: 0.15rem; font-size: 0.9em; font-style: italic; color: var(--sp-text-3); }

/* ---- Summary ---- */
.project-name-section { border: 1px solid var(--sp-border-strong); border-radius: 10px; padding: 1.25rem; margin-bottom: 1rem; background: var(--sp-bg-1); }
.preview-section { border: 1px solid var(--sp-border-strong); border-radius: 10px; padding: 1rem; margin-bottom: 1rem; background: var(--sp-bg-1); min-height: 360px; }
.pn-label { display: block; font-weight: 700; font-size: 1rem; margin-bottom: 0.5rem; color: var(--sp-text-1); }
.pn-input { width: 100%; padding: 0.65rem 0.8rem; border-radius: 8px; border: 1px solid var(--sp-border-strong); background: var(--sp-bg-1); color: #fff; font-size: 1rem; box-sizing: border-box; outline: none; }
.pn-input:focus { border-color: var(--sp-accent-strong); box-shadow: 0 0 0 2px rgba(108,92,231,0.25); }
.pn-input::placeholder { color: var(--sp-text-3); }
.readme-row { display: flex; align-items: center; gap: 0.5rem; margin: -0.35rem 0 1rem; }
.readme-label { font-size: 0.82rem; color: var(--sp-text-2); }
.readme-switch { position: relative; width: 34px; height: 18px; padding: 0; border-radius: 999px; border: 1px solid var(--sp-border-strong); background: var(--sp-bg-2); cursor: pointer; transition: background 0.15s, border-color 0.15s; }
.readme-switch[aria-checked="true"] { background: var(--sp-accent); border-color: var(--sp-accent); }
.readme-switch-knob { position: absolute; top: 2px; left: 2px; width: 12px; height: 12px; border-radius: 50%; background: var(--sp-text-3); transition: transform 0.15s, background 0.15s; }
.readme-switch[aria-checked="true"] .readme-switch-knob { transform: translateX(16px); background: #fff; }
.readme-lang { min-width: 1.6rem; font-size: 0.72rem; font-weight: 700; letter-spacing: 0.04em; color: var(--sp-text-2); }
.readme-help-wrap { position: relative; display: inline-flex; }
.readme-help { display: inline-flex; align-items: center; justify-content: center; width: 18px; height: 18px; padding: 0; border: none; border-radius: 50%; background: none; color: var(--sp-text-3); cursor: help; }
.readme-help:hover, .readme-help:focus-visible { color: var(--sp-accent-strong); }
.readme-tip { position: absolute; bottom: calc(100% + 6px); left: 50%; transform: translateX(-50%); width: 280px; padding: 0.5rem 0.65rem; background: var(--sp-bg-1); border: 1px solid var(--sp-border-strong); border-radius: 8px; box-shadow: var(--sp-shadow-1); color: var(--sp-text-1); font-size: 0.75rem; line-height: 1.35; z-index: 20; }
.folder-row { display: flex; align-items: center; gap: 0.75rem; margin-top: 0.75rem; flex-wrap: wrap; }
.btn-select-folder { background: var(--sp-accent-soft); color: var(--sp-text-2); padding: 0.5rem 1rem; border-radius: 6px; border: 1px solid var(--sp-border-strong); cursor: pointer; font-size: 0.85rem; white-space: nowrap; }
.btn-select-folder:hover { border-color: var(--sp-accent-strong); color: #fff; }
.folder-path { font-size: 0.8rem; color: var(--sp-text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 400px; }
.path-preview { display: flex; align-items: center; gap: 0.5rem; margin-top: 0.6rem; flex-wrap: wrap; }
.pp-label { font-size: 0.8rem; color: var(--sp-text-3); }
.pp-path { font-size: 0.85rem; color: var(--sp-accent-strong); background: var(--sp-bg-1); padding: 0.2rem 0.5rem; border-radius: 4px; word-break: break-all; }
.pp-checking { font-size: 0.8rem; color: var(--sp-text-3); font-style: italic; }
.pp-exists { font-size: 0.8rem; color: var(--sp-warning); font-weight: 600; }
.conflict-overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.6); display: flex; align-items: center; justify-content: center; z-index: 1000; }
.clear-overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.6); display: flex; align-items: center; justify-content: center; z-index: 1000; }
.clear-dialog { background: var(--sp-bg-1); border: 1px solid var(--sp-danger); border-radius: 12px; padding: 1.5rem; max-width: 460px; width: 90%; }
.clear-dialog h3 { margin: 0 0 0.75rem; color: var(--sp-warning); }
.clear-dialog p { font-size: 0.9rem; color: var(--sp-text-2); margin: 0 0 1.25rem; line-height: 1.4; }
.clear-actions { display: flex; flex-direction: column; gap: 0.6rem; }
.clear-actions button { width: 100%; text-align: center; }
.btn-clear-stack {
  font-size: 0.78rem;
  font-weight: 600;
  color: var(--sp-danger);
  background: rgba(239, 68, 68, 0.1);
  border: 1px solid rgba(239, 68, 68, 0.35);
  border-radius: 8px;
  padding: 0.35rem 0.8rem;
  cursor: pointer;
  margin-bottom: 0.8rem;
  margin-left: 0.5rem;
  transition: background 0.15s;
}
.btn-clear-stack:hover { background: rgba(239, 68, 68, 0.12); border-color: var(--sp-danger); }
.notice-bar { display: inline-block; font-size: 0.78rem; color: var(--sp-warning); background: rgba(245, 158, 11, 0.12); border: 1px solid rgba(245, 158, 11, 0.35); border-radius: 8px; padding: 0.45rem 0.7rem; margin-bottom: 0.8rem; }
.conflict-dialog { background: var(--sp-bg-1); border: 1px solid var(--sp-accent-strong); border-radius: 12px; padding: 1.5rem; max-width: 480px; width: 90%; }
.conflict-dialog h3 { margin: 0 0 0.75rem; color: var(--sp-warning); }
.conflict-dialog p { font-size: 0.9rem; color: var(--sp-text-2); margin: 0 0 1.25rem; line-height: 1.4; }
.conflict-actions { display: flex; flex-direction: column; gap: 0.6rem; }
.conflict-actions button { width: 100%; text-align: center; }

/* ---- Окружение ---- */
.env-warn { color: var(--sp-warning); font-weight: 600; }

/* ---- Примечание о расширении каталога ---- */
.grow-note {
  margin: 1.5rem auto 0;
  padding-top: 1.25rem;
  text-align: center;
  font-size: 0.8rem;
  color: var(--sp-text-3);
  opacity: 0.55;
}


@keyframes skeleton-pulse {
  0%, 100% { opacity: 0.15; }
  50% { opacity: 0.35; }
}
@keyframes skeleton-shimmer {
  0% { background-position: -400px 0; }
  100% { background-position: 400px 0; }
}
.skeleton-wrap { animation: skeleton-pulse 1.6s ease-in-out infinite; }
.skeleton-header { margin-bottom: 1.2rem; }
.skeleton-bar {
  height: 14px;
  border-radius: 6px;
  background: linear-gradient(90deg, var(--sp-bg-3) 25%, var(--sp-bg-2) 50%, var(--sp-bg-3) 75%);
  background-size: 800px 100%;
  animation: skeleton-shimmer 1.8s ease-in-out infinite;
}
.skeleton-bar--title { width: 320px; height: 22px; margin-bottom: 0.6rem; }
.skeleton-bar--subtitle { width: 220px; height: 16px; margin-bottom: 1rem; }
.skeleton-bar--sidebar { width: 100%; margin-bottom: 0.8rem; }
.skeleton-bar--sidebar-short { width: 60%; }
.skeleton-mode-switch { display: flex; gap: 0.5rem; margin-bottom: 1.5rem; }
.skeleton-pill {
  width: 110px;
  height: 34px;
  border-radius: 8px;
  background: var(--sp-bg-3);
}
.skeleton-builder { display: grid; grid-template-columns: 180px 1fr 260px; gap: 1.5rem; }
.skeleton-phases { display: flex; flex-direction: column; gap: 0.6rem; }
.skeleton-phase {
  height: 36px;
  border-radius: 8px;
  background: var(--sp-bg-3);
}
.skeleton-content { min-width: 0; }
.skeleton-grid { display: grid; grid-template-columns: repeat(2, 1fr); gap: 0.8rem; }
.skeleton-card {
  height: 120px;
  border-radius: 12px;
  background: var(--sp-bg-3);
}
.skeleton-sidebar { padding-top: 2rem; }

@media (max-width: 1100px) {
  .type-grid, .fw-grid { grid-template-columns: repeat(3, 1fr); }
}
@media (max-width: 1080px) {
  .tool-menu { grid-template-columns: 1fr; }
}
@media (max-width: 900px) {
  .builder { grid-template-columns: 1fr; }
  .builder-context { position: static; }
  .skeleton-builder { grid-template-columns: 1fr; }
  .skeleton-sidebar { display: none; }
  .type-grid, .fw-grid { grid-template-columns: repeat(2, 1fr); }
}
@media (max-width: 600px) {
  .type-grid, .fw-grid { grid-template-columns: 1fr; }
}
</style>
