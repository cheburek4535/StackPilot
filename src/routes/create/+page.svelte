<script lang="ts">
import { onMount, onDestroy } from "svelte";
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
import { statusKind, statusLabel, taskStateKind, taskStateLabel, identityMatches } from "$lib/modules/toolchain/compat";
import TechIcon from "$lib/components/TechIcon.svelte";

let tree = $state<WizardTreeData | null>(null);
let status = $state<string>("loading");
let hostOs = $state<string>("windows");
/** Краткое уведомление при авто-сбросе конфликтующих фреймворков */
let dropNotice = $state<string | null>(null);
/** Показывать ли отдельные карточки заблокированных фреймворков внутри уровня. */
let showUnavailable = $state<Record<string, boolean>>({});

// ---- Mode: Constructor | Templates | Analyze ----
let mode = $state<"constructor" | "presets" | "analyze">("constructor");

// ---- Constructor context ----
const PHASES = ["Project Type", "Stack & Tools", "Review"];
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
    ? validateStack(tree, selectedType?.id ?? null, backendLangs, frontendLangs, selectedFrameworks, hostOs)
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
  tooltipTimer = setTimeout(() => {
    const rect = (e.currentTarget as HTMLElement)?.getBoundingClientRect();
    tooltipData = { x: rect ? rect.left : 0, y: rect ? rect.bottom + 4 : 0, tool };
  }, 600);
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
    projectName,
    selectedFolder,
    conflictResolvedFolder,
    folderExists,
    envLocalInfra: [...envLocalInfra],
    envSelectedIds: [...envSelectedIds],
    envInstalling,
    envInstallDone,
    execOverallStatus,
    execProjectPath: execPlan?.project_path ?? null,
  };
}

function restoreSnapshot(snap: Record<string, unknown>) {
  const s = snap;
  const str = (v: unknown): string => (typeof v === "string" ? v : "");
  const bool = (v: unknown): boolean => v === true;
  const strArr = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x) => typeof x === "string") : []);
  const num = (v: unknown): number => (typeof v === "number" ? v : 0);

  mode = (["constructor", "presets", "analyze"] as const).includes(s.mode as never)
    ? (s.mode as "constructor" | "presets" | "analyze")
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
  projectName = str(s.projectName);
  selectedFolder = str(s.selectedFolder) || null;
  conflictResolvedFolder = str(s.conflictResolvedFolder) || null;
  folderExists = bool(s.folderExists);
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
          execError = "Project creation was interrupted while you were away.";
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
  const saved = loadCreateSession();
  if (saved) {
    try {
      restoreSnapshot(saved);
    } catch (e) {
      console.error("[create] failed to restore session:", e);
    }
  }
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
    (async () => {
      try {
        hostOs = await getHostPlatform();
      } catch (e) {
        console.error("cannot detect host OS:", e);
      }
    })(),
  ]);
  // Не задерживаем первый интерактивный кадр маршрута восстановлением живых
  // сессий и проверкой toolchain: эти вызовы могут обращаться к Tauri долго.
  // После отрисовки конструктора выполняем их в фоне.
  queueMicrotask(async () => {
    await reSyncLiveSessions();
    await refreshInstalledTools();
    persistReady = true;
  });
});

onDestroy(() => {
  persistNow();
  if (unlisten) unlisten();
  if (unlistenTc) unlistenTc();
  if (unlistenTcDone) unlistenTcDone();
  if (unlistenTcCheck) unlistenTcCheck();
  stopTick();
});

/** Иконка тула из wizard_tree по tool_id (для requirements/tasks окружения) */
function toolIcon(toolId: string): string | null {
  return tree?.tools.find((t) => t.id === toolId)?.icon ?? null;
}

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
    title: "Full application frameworks",
    note: "Create the whole project scaffold by themselves (Spring Boot, Django, Next.js).",
  },
  {
    id: "inplace",
    title: "In-place & lightweight",
    note: "Attach into a base project of their language (FastAPI, Express, Gin).",
  },
  {
    id: "side",
    title: "Side modules & libraries",
    note: "Optional add-ons to the main stack (bots, plugins) — can coexist with anything.",
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

/** Объяснение конфликта (conflict_notes) в обе стороны */
function conflictNoteOf(fw: FrameworkDef, other: FrameworkDef): string | undefined {
  return fw.conflict_notes?.[other.id] ?? other.conflict_notes?.[fw.id];
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
      message: `Доступен только на ${fw.platforms.join(", ")}`,
      detail: `«${fw.label}» не поддерживает вашу ОС (${hostOs}).`,
      alternatives: frameworkAlternatives(fw),
    };
  }

  // 2. Взаимные конфликты (conflicts + conflict_notes)
  for (const selId of selectedFrameworks) {
    const sel = tree?.frameworks.find((f) => f.id === selId);
    if (!sel) continue;
    if (sel.conflicts?.includes(fwId) || fw.conflicts?.includes(selId)) {
      const note = conflictNoteOf(fw, sel);
      return {
        message: `Несовместим с «${sel.label}»`,
        detail: note
          ? `«${fw.label}» и «${sel.label}»: ${note}`
          : `«${fw.label}» и «${sel.label}» не могут работать вместе — снимите один из них.`,
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
    return {
      message: "Недоступен вместе с бэкенд/фронтенд стеком",
      detail:
        fw.id === "qt"
          ? "«Qt» сам создаёт всё приложение. Если нужен веб-UI — выберите Qt первым, затем в его настройках Qt WebEngine + React/Vue/Svelte."
          : `«${fw.label}» сам создаёт всё приложение и не сочетается с выбранной бэкенд/фронтенд стековой связкой.`,
      alternatives: frameworkAlternatives(fw),
    };
  }
  if (fw.side !== "either" && hasEither) {
    const eitherFw = selectedFrameworks.find((id) => {
      const f = tree?.frameworks.find((x) => x.id === id);
      return !!f && f.side === "either";
    });
    const eitherLabel =
      tree?.frameworks.find((x) => x.id === eitherFw)?.label ?? "desktop";
    return {
      message: `Недоступен вместе с «${eitherLabel}»`,
      detail:
        eitherFw === "qt" && (fw.id === "react" || fw.id === "vue" || fw.id === "svelte")
          ? "«Qt» — самостоятельное приложение, но через Qt WebEngine он умеет встраивать веб-UI. Выберите Qt и в его настройках укажите WebEngine + этот фреймворк."
          : `«${eitherLabel}» — самостоятельное приложение, отдельный UI-слой не нужен.`,
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
        const selLabel =
          tree?.frameworks.find((x) => x.id === sameSide)?.label ?? sameSide;
        return {
          message: `Только один главный ${fw.side === "backend" ? "бэкенд" : "фронтенд"}-фреймворк`,
          detail: `«${selLabel}» и «${fw.label}» — оба главные фреймворки: два каркаса будут перезаписывать файлы друг друга. Снимите один или выберите другой.`,
          alternatives: frameworkAlternatives(fw),
        };
      }
    }
    // 5. Язык
    const langs = fw.side === "backend" ? backendLangs : frontendLangs;
    if (langs.length > 0 && !fw.languages.some((l) => langs.includes(l))) {
      return {
        message: `Нужен язык: ${fw.languages.map((l) => langLabel(l)).join(", ")}`,
        detail: `На стороне «${fw.side === "backend" ? "бэкенд" : "фронтенд"}» нет языка ${fw.languages.map((l) => langLabel(l)).join(" или ")}, необходимого «${fw.label}».`,
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
        return cnote ? `${cfw.label} (${cnote})` : cfw.label;
      })
      .join(", ");
    dropNotice = `Автоматически снято: ${note}.`;
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
  return tree?.languages.find((l) => l.id === id)?.label ?? id;
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
  if (active.length > 0) return `Already ${active.map((l) => langLabel(l)).join(", ")} on this side`;
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
    const parts = mode ? [mode.label] : [];
    if (modeId === "qt-webengine" && qtWebLinked[fw.id]) {
      const wf = tree?.frameworks.find((f) => f.id === qtWebLinked[fw.id]);
      if (wf) parts.push(wf.label);
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

/** Тул «рекомендован» для текущего стека: заявлен в framework_tool_map
 *  выбранных фреймворков или подходит выбранному типу проекта
 *  (etl → airflow/clickhouse/grafana). */
function isToolRecommended(t: ToolDef): boolean {
  if (!tree || !toolFitsStack(t)) return false;
  const tm = tree.framework_tool_map;
  for (const fwId of selectedFrameworks) {
    if ((tm[fwId] ?? []).includes(t.id)) return true;
  }
  if (
    t.for_project_types.length > 0 &&
    selectedType &&
    t.for_project_types.includes(selectedType.id)
  ) {
    return true;
  }
  return false;
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
  { id: "database", label: "Databases" },
  { id: "cache", label: "Caches" },
  { id: "messaging", label: "Messaging & Queues" },
  { id: "observability", label: "Observability" },
  { id: "testing", label: "Testing" },
  { id: "tooling", label: "Tooling" },
  { id: "container", label: "Containers" },
  { id: "orchestration", label: "Orchestration" },
  { id: "etl", label: "ETL & Data" },
  { id: "baas", label: "Backend as a Service" },
  { id: "infra", label: "Infrastructure" },
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
    envError = String(e) || "Неизвестная ошибка при проверке окружения";
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

function selectedMissingTools(): ToolRequirement[] {
  return allMissingTools().filter((r) => envSelectedIds.has(r.tool_id));
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

/** Инструмент установлен локально: заявлен в общем стейте приложения
 *  (state.json) или только что подтверждён текущей проверкой окружения. */
function isLocallyInstalled(toolId: string): boolean {
  if (installedTools.has(toolId)) return true;
  return (
    envCheck?.requirements.some(
      (r) => r.tool_id === toolId && statusKind(r.status) === "ok",
    ) ?? false
  );
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

function formatMb(mb: number): string {
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
  return `${mb} MB`;
}

function downloadStatus(taskId: string): string {
  const dl = envDownload.get(taskId);
  if (!dl || dl.total <= 0) return "";
  const percent = Math.min(100, Math.round((dl.received / dl.total) * 100));
  return `${percent}% (${formatMb(Math.floor(dl.received / 1024 / 1024))} / ${formatMb(Math.floor(dl.total / 1024 / 1024))})`;
}

function dlPercent(taskId: string): number | null {
  const dl = envDownload.get(taskId);
  if (!dl || dl.total <= 0) return null;
  return Math.min(100, Math.round((dl.received / dl.total) * 100));
}

function phaseElapsed(taskId: string): string {
  const started = envPhaseStart.get(taskId);
  if (!started) return "…";
  const secs = Math.max(0, Math.floor((Date.now() - started) / 1000));
  if (secs < 60) return `${secs} с`;
  return `${Math.floor(secs / 60)} мин ${secs % 60} с`;
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

function secretToolName(toolId: string): string {
  const req = envCheck?.requirements.find((r) => r.tool_id === toolId);
  return req?.display ?? toolId;
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
    );
    const blocking = issues.filter((i) => i.severity === "Error");
    if (blocking.length > 0) {
      // Показываем ошибки бэкенд-валидации (нормализация, конфликты,
      // duplicate write paths), которых нет во фронтенд-зеркале rules.ts.
      // Раньше клик молча гасился — кнопка выглядела «сломанной».
      reviewError = blocking[0].message;
      return;
    }
  } catch (e) {
    reviewError = `Validation failed: ${e}`;
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
  };

  phase = 6;
  execPlan = null;
  execProjectPath = path;
  execStatuses = new Map();
  execLogs = [];
  execOverallStatus = "running";
  execResult = null;
  execError = null;

  if (unlisten) unlisten();
  unlisten = await listen<ExecutionEvent>("project_creator:step_event", (e) => {
    handleExecEvent(e.payload);
  });

  try {
    execPlan = await startProjectExecution(ctx, path);
  } catch (err) {
    execError = String(err);
    execOverallStatus = "error";
  }
}

function stepIsRunning(st: StepStatus | undefined) { return st === "Running"; }
function stepIsSuccess(st: StepStatus | undefined) { return !!st && typeof st === "object" && "Success" in st; }
function stepIsFailed(st: StepStatus | undefined) { return !!st && typeof st === "object" && "Failed" in st; }
function stepIsSkipped(st: StepStatus | undefined) { return !!st && typeof st === "object" && "Skipped" in st; }

function handleExecEvent(event: ExecutionEvent) {
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
      if (c) entry.status = c.status;
    }
    if ("AllCompleted" in t) {
      const a = (t as Record<string, { result: { total_duration_ms: number; overall: unknown } }>).AllCompleted;
      execOverallStatus = "done";
      execResult = { duration: a?.result?.total_duration_ms ?? 0, status: JSON.stringify(a?.result?.overall) };
      persistNow();
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

function openInVSCode() {
  if (execPlan) {
    const _ = invoke("open_in_vscode", { path: execPlan.project_path });
  }
}

function cancelExecution() {
  execOverallStatus = "cancelled";
  persistNow();
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
  clearCreateSession();
}
</script>

<div class="wizard">
  <h1>Create Project</h1>

  {#if status === "loading"}
    <p class="muted">Initializing ProjectCreator...</p>
  {:else if status === "error"}
    <p class="error">Failed to load ProjectCreator module. Check console.</p>
  {:else if status === "empty"}
    <p class="muted">No project types configured yet.</p>
  {:else}

    <div class="mode-switch">
      <button class="mode-btn" class:active={mode === "constructor"} onclick={() => { mode = "constructor"; }}>Constructor</button>
      <button class="mode-btn" class:active={mode === "presets"} onclick={() => { mode = "presets"; }}>Templates</button>
      <button class="mode-btn" class:active={mode === "analyze"} onclick={() => { mode = "analyze"; }}>Analyze</button>
    </div>

    {#if mode === "analyze"}
      <div class="analysis-panel">
        <p class="prompt">Analyze an existing project</p>
        <p class="hint">Select a folder to detect technology stack and pre-fill the constructor.</p>
        <button class="btn-primary" onclick={runAnalysis} disabled={analyzing}>
          {analyzing ? "Analyzing..." : "📂 Select Folder"}
        </button>
        {#if analyzedPath}
          <p class="analyzed-path">Selected: {analyzedPath}</p>
        {/if}
        {#if analysisError}
          <p class="error">{analysisError}</p>
        {/if}
        {#if analysisResult}
          <div class="analysis-result">
            <p class="analysis-summary">{analysisResult.summary}</p>
            <div class="analysis-section">
              <p class="section-title">Detected Technologies</p>
              <div class="tech-tags">
                {#each analysisResult.detected_technologies as tech}
                  <span class="tech-tag" class:certaion={tech.confidence === "Certain"}
                                        class:likely={tech.confidence === "Likely"}
                                        class:possible={tech.confidence === "Possible"}>
                    {tech.name}{#if tech.version} ({tech.version}){/if}
                  </span>
                {/each}
              </div>
            </div>
            <div class="analysis-section">
              <p class="section-title">Hints</p>
              <div class="hint-tags">
                {#each analysisResult.project_type_hints as hint}
                  <span class="hint-tag">{hint}</span>
                {/each}
                {#if analysisResult.has_docker}<span class="hint-tag docker">Docker</span>{/if}
                {#if analysisResult.has_git}<span class="hint-tag">Git</span>{/if}
                {#if analysisResult.has_ci}<span class="hint-tag">CI</span>{/if}
                {#if analysisResult.has_tests}<span class="hint-tag">Tests</span>{/if}
              </div>
            </div>
            <button class="btn-primary" onclick={applyAnalysis}>
              Use detected values →
            </button>
          </div>
        {/if}
      </div>

    {:else if mode === "presets"}
      <p class="prompt">Project templates</p>
      <p class="hint">Pick a ready-made stack and adjust it in the constructor.</p>
      <div class="preset-grid">
        {#each tree!.presets as p}
          <div class="preset-card">
            <TechIcon icon={p.icon} alt={p.label} size="xl" />
            <h3>{p.label}</h3>
            <p class="preset-desc">{p.description}</p>
            <div class="preset-stack">
              {#if p.stack.backend_lang}<span class="preset-chip">{p.stack.backend_lang}</span>{/if}
              {#if p.stack.frontend_lang}<span class="preset-chip">{p.stack.frontend_lang}</span>{/if}
              {#each p.stack.frameworks as fw}<span class="preset-chip">{fw}</span>{/each}
              {#if p.stack.tools.length > 0}
                <span class="preset-chip">{p.stack.tools.length} tool{p.stack.tools.length > 1 ? "s" : ""}</span>
              {/if}
            </div>
            <button class="btn-primary preset-apply" onclick={() => applyPreset(p)}>Use template</button>
          </div>
        {/each}
      </div>

    {:else}

      {#if phase === 5 || phase === 6}
        <!-- ================================================================
             Environment check & install
             ================================================================ -->
        {#if phase === 5}
          <p class="prompt">Environment check</p>
          <p class="hint">We check your stack requirements before generating the project.</p>

          {#if envChecking}
            <p class="muted">Checking installed tools…</p>
            {#if envCheckProgress.length > 0}
              <div class="env-progress-list">
                {#each envCheckProgress as ev}
                  <div class="env-progress-row">
                    <span class="env-icon">{statusKind(ev.status) === "ok" ? "✅" : "🔍"}</span>
                    <TechIcon icon={ev.icon ?? toolIcon(ev.tool_id)} alt={ev.display} size="sm" />
                    <span class="env-name">{ev.display}</span>
                    <span class="env-status muted">
                      {statusKind(ev.status) === "ok"
                        ? `✓ ${(ev.status as { Installed: { version: string } }).Installed.version}`
                        : "checking…"}
                    </span>
                  </div>
                {/each}
              </div>
            {/if}
          {:else if envCheck}
            {@const missingAll = allMissingTools()}
            {@const missingSelected = selectedMissingTools()}
            {#if envInstallDone && envPlan}
              <div class="env-summary">
                <span>Installed: {envPlan.tasks.filter((t) => taskStateKind(t.state) === "success").length}/{envPlan.tasks.length}</span>
                <span>Failed: {envPlan.tasks.filter((t) => taskStateKind(t.state) === "failed").length}</span>
                {#if envErrors.length > 0}
                  <span class="env-warn">⚠ {envErrors.length} error{envErrors.length > 1 ? "s" : ""}</span>
                {/if}
              </div>
              <div class="env-install">
                <p class="group-label">Installation finished</p>
                {#each envPlan.tasks as task}
                  {@const st = envTaskStates.get(task.task_id) ?? task.state}
                  <div class="env-row">
                    <span class="env-icon">
                      {#if taskStateKind(st) === "success"}✅
                      {:else if taskStateKind(st) === "failed"}❌
                      {:else if taskStateKind(st) === "skipped"}⏭️
                      {:else}•{/if}
                    </span>
                    <TechIcon icon={task.icon ?? toolIcon(task.tool_id)} alt={task.display} size="sm" />
                    <span class="env-name">{task.display}</span>
                    <span class="env-source">{task.size_mb} MB · {task.source_description}</span>
                    <span
                      class="env-status"
                      class:ok={taskStateKind(st) === "success"}
                      class:broken={taskStateKind(st) === "failed"}
                      class:missing={taskStateKind(st) === "skipped"}
                    >{taskStateLabel(st)}</span>
                  </div>
                {/each}
                {#if envErrors.length > 0}
                  <details class="exec-full-log">
                    <summary>Errors ({envErrors.length})</summary>
                    <pre>{envErrors.join("\n")}</pre>
                  </details>
                {/if}
                {#if envLogs.length > 0}
                  <details class="exec-full-log">
                    <summary>Log ({envLogs.length} lines)</summary>
                    <pre>{envLogs.join("\n")}</pre>
                  </details>
                {/if}
              </div>
              <div class="btn-row">
                <button class="btn-back" onclick={() => (phase = 2)}>← Back</button>
                <button class="btn-secondary" onclick={recheckEnvironment}>Re-check environment</button>
                <button class="btn-primary" onclick={doCreateProject}>Continue</button>
              </div>
              {#if envRestartHint}
                <p class="env-warn">💡 New tools were installed. Restart your open terminals and editors to pick up the updated PATH.</p>
              {/if}
            {:else}
            <div class="env-summary">
              <span>Ready: {envCheck.requirements.filter((r) => statusKind(r.status) === "ok" || statusKind(r.status) === "manual").length}/{envCheck.requirements.length}</span>
              {#if missingAll.length > 0}
                <span>To install: {missingSelected.length}/{missingAll.length}</span>
              {/if}
              <span>Download: {missingSelected.reduce((sum, r) => sum + r.size_mb, 0)} MB</span>
              <span>Free space: {envCheck.free_space_mb} MB</span>
              {#if !envCheck.enough_space}
                <span class="env-warn">⚠ Not enough disk space</span>
              {/if}
              {#if envCheck.needs_admin_any}
                <span class="env-warn">⚠ Admin rights may be required</span>
              {/if}
            </div>

            <div class="env-list">
              {#each envCheck.requirements as req}
                {@const kind = statusKind(req.status)}
                <div
                  class="env-row"
                  class:ok={kind === "ok"}
                  class:update={kind === "update"}
                  class:broken={kind === "broken"}
                  class:missing={kind === "missing"}
                  class:manual={kind === "manual"}
                >
                  {#if kind === "ok"}
                    <span class="env-select">✅</span>
                  {:else if kind === "manual"}
                    <span class="env-select manual-badge" title="Installed manually, no auto-install"><TechIcon alt="" size="sm" /></span>
                  {:else}
                    <label class="env-select">
                      <input
                        type="checkbox"
                        checked={envSelectedIds.has(req.tool_id)}
                        onchange={() => toggleEnvTool(req.tool_id)}
                      />
                    </label>
                  {/if}
                  <span class="env-icon"><TechIcon icon={req.icon ?? toolIcon(req.tool_id)} alt={req.display} size="sm" /></span>
                  <span class="env-name">{req.display}</span>
                  <span class="env-source">{req.source_description}</span>
                  <span
                    class="env-status"
                    class:ok={kind === "ok"}
                    class:update={kind === "update"}
                    class:broken={kind === "broken"}
                    class:missing={kind === "missing"}
                    class:manual={kind === "manual"}
                  >{statusLabel(req.status)}</span>
                </div>
              {/each}
            </div>

            {#if (envCheck.optional_requirements ?? []).length > 0 || envLocalInfra.size > 0}
              <div class="env-optional" class:env-optional-local={envLocalInfra.size > 0}>
                <p class="group-label">Optional — run in Docker</p>
                <p class="hint">
                  These tools are deployed as Docker containers with the project by default.
                  Switch a tool to <strong>Use Host Machine</strong> to install and run it on
                  this machine instead — it is checked and installed like the requirements
                  above, excluded from docker-compose, and its local setup is documented
                  in <code>LOCAL_INFRA.md</code>.
                </p>
                {#snippet infraToggle(toolId: string, onHost: boolean)}
                  <span class="infra-toggle" role="group" aria-label="Run in Docker or on the host machine">
                    <button
                      class="infra-toggle-opt"
                      class:active={!onHost}
                      title="Deploy as a Docker container with the project (docker-compose.yaml)"
                      onclick={() => {
                        if (onHost) revertLocalInfra(toolId);
                      }}
                    >
                      Run in Docker
                    </button>
                    <button
                      class="infra-toggle-opt"
                      class:active={onHost}
                      class:host={onHost}
                      title="Install and run on this machine — excluded from docker-compose, see LOCAL_INFRA.md"
                      onclick={() => {
                        if (!onHost) optInLocalInfra(toolId);
                      }}
                    >
                      Use Host Machine
                    </button>
                  </span>
                {/snippet}
                {#each envCheck.optional_requirements ?? [] as req}
                  {@const installedHere = isLocallyInstalled(req.tool_id)}
                  <div class="env-row" class:ok={installedHere}>
                    <span class="env-select"><TechIcon icon="docker.svg" alt="Docker" size="sm" /></span>
                    <span class="env-icon"><TechIcon icon={req.icon ?? toolIcon(req.tool_id)} alt={req.display} size="sm" /></span>
                    <span class="env-name">{req.display}</span>
                    <span class="env-source">
                      {installedHere ? "Running locally on this machine" : "Docker (docker-compose.yaml)"}
                    </span>
                    {#if installedHere}
                      <span class="env-status ok">✓ Installed Locally (Host)</span>
                    {/if}
                    {@render infraToggle(req.tool_id, false)}
                  </div>
                {/each}
                {#each [...envLocalInfra] as toolId}
                  {@const req = envCheck.requirements.find((r) => r.tool_id === toolId)}
                  {@const installedHere = isLocallyInstalled(toolId)}
                  <div class="env-row ok">
                    <span class="env-select"><TechIcon icon="docker.svg" alt="Docker" size="sm" /></span>
                    <span class="env-icon"><TechIcon icon={req?.icon ?? toolIcon(toolId)} alt={req?.display ?? toolId} size="sm" /></span>
                    <span class="env-name">{req?.display ?? toolId}</span>
                    <span class="env-source">
                      {installedHere ? "Running locally on this machine" : "Local install pending"}
                    </span>
                    {#if installedHere}
                      <span class="env-status ok">✓ Installed Locally (Host)</span>
                    {/if}
                    {@render infraToggle(toolId, true)}
                  </div>
                {/each}
              </div>
            {/if}

            {#if envPlan && envInstalling}
              <div class="env-install">
                <p class="group-label">Installing…</p>
                {#each envPlan.tasks as task}
                  {@const st = envTaskStates.get(task.task_id) ?? task.state}
                  {@const kind = taskStateKind(st)}
                  {@const running = kind === "running" && typeof st === "object" && "Running" in st}
                  {@const downloading = running && st.Running.phase === "Downloading"}
                  {@const pct = downloading ? dlPercent(task.task_id) : null}
                  {@const dl = envDownload.get(task.task_id)}
                  <div class="env-task">
                    <div class="env-row">
                      <span class="env-icon">
                        {#if kind === "running"}⏳
                        {:else if kind === "success"}✅
                        {:else if kind === "failed"}❌
                        {:else if kind === "skipped"}⏭️
                        {:else}•{/if}
                      </span>
                      <TechIcon icon={task.icon ?? toolIcon(task.tool_id)} alt={task.display} size="sm" />
                      <span class="env-name">{task.display}</span>
                      <span class="env-source">{task.size_mb} MB · {task.source_description}</span>
                      <span class="env-status">
                        {#if running}
                          {#if downloading && pct !== null && dl}
                            Downloading… {pct}% ({formatMb(Math.floor(dl.received / 1024 / 1024))} / {formatMb(Math.floor(dl.total / 1024 / 1024))})
                          {:else}
                            <span class="spin" aria-hidden="true"></span> {taskStateLabel(st)} · {phaseElapsed(task.task_id)}
                          {/if}
                        {:else}
                          {taskStateLabel(st)}
                        {/if}
                      </span>
                    </div>
                    {#if downloading && pct !== null}
                      <div class="dl-bar" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
                        <div class="dl-fill" style="width: {pct}%"></div>
                      </div>
                    {/if}
                  </div>
                {/each}
                {#if envLogs.length > 0}
                  <details class="exec-full-log">
                    <summary>Log ({envLogs.length} lines)</summary>
                    <pre>{envLogs.join("\n")}</pre>
                  </details>
                {/if}
              </div>
            {/if}

            {#if envError}
              <p class="error">{envError}</p>
            {/if}

            <div class="btn-row">
              <button class="btn-back" onclick={() => (phase = 2)} disabled={envInstalling}>← Back</button>
              {#if envInstalling}
                <button class="btn-secondary" onclick={cancelInstall}>Abort</button>
              {:else if missingAll.length > 0}
                <button class="btn-primary" onclick={startInstall} disabled={missingSelected.length === 0}>
                  Install selected ({missingSelected.length})
                </button>
                {#if missingSelected.length < missingAll.length}
                  <button class="btn-secondary" onclick={selectAllEnvTools}>Select all ({missingAll.length})</button>
                {/if}
                <button class="btn-secondary" onclick={doCreateProject}>Continue anyway</button>
              {:else}
                <button class="btn-primary" onclick={doCreateProject}>🚀 Create Project</button>
              {/if}
            </div>

            {#if envRestartHint}
              <p class="env-warn">
                💡 New tools were installed. Restart your open terminals and editors to pick up the updated PATH.
              </p>
            {/if}
            {/if}
          {:else}
            <p class="error">
              {envError
                ? `Failed to check environment: ${envError}`
                : "Environment not checked yet — run a check to see what your stack needs."}
            </p>
            <div class="btn-row">
              <button class="btn-back" onclick={back}>← Back</button>
              <button class="btn-primary" onclick={() => runEnvironmentCheck()}>
                {envError ? "Retry" : "Check environment"}
              </button>
            </div>
          {/if}

          {#if newSecrets}
            <div class="conflict-overlay" onclick={() => { newSecrets = null; }}>
              <div class="conflict-dialog" onclick={(e) => e.stopPropagation()}>
                <h3>🔑 Generated passwords</h3>
                <p class="hint">Save these now — they will not be shown again.</p>
                {#each Object.entries(newSecrets) as [toolId, value]}
                  <div class="secret-row">
                    <span class="secret-name">{secretToolName(toolId)}</span>
                    <code class="secret-value">{value}</code>
                    <button class="btn-secondary" onclick={() => copySecret(toolId, value)}>
                      {secretCopied === toolId ? "Copied ✓" : "Copy"}
                    </button>
                  </div>
                {/each}
                <div class="btn-row">
                  <button class="btn-primary" onclick={() => { newSecrets = null; }}>Got it</button>
                </div>
              </div>
            </div>
          {/if}
        {/if}

        <!-- ================================================================
             Execution
             ================================================================ -->
        {#if phase === 6}
          <p class="prompt">Generating your project...</p>

          <div class="exec-steps">
            {#each [...execStatuses.entries()] as [idx, entry]}
              <div class="exec-step"
                   class:running={stepIsRunning(entry.status)}
                   class:success={stepIsSuccess(entry.status)}
                   class:failed={stepIsFailed(entry.status)}
                   class:skipped={stepIsSkipped(entry.status)}>
                <div class="exec-icon">
                  {#if stepIsRunning(entry.status)}
                    ⏳
                  {:else if stepIsSuccess(entry.status)}
                    ✅
                  {:else if stepIsFailed(entry.status)}
                    ❌
                  {:else if stepIsSkipped(entry.status)}
                    ⏭️
                  {:else}
                    ⏳
                  {/if}
                </div>
                <div class="exec-detail">
                  <p class="exec-name">{entry.name}</p>
                  {#if entry.logs.length > 0}
                    <pre class="exec-log">{entry.logs.join("\n")}</pre>
                  {/if}
                </div>
              </div>
            {/each}
          </div>

          {#if execLogs.length > 0}
            <details class="exec-full-log">
              <summary>Full log ({execLogs.length} lines)</summary>
              <pre>{execLogs.join("\n")}</pre>
            </details>
          {/if}

          {#if execOverallStatus === "running"}
            <div class="btn-row">
              <button class="btn-secondary" onclick={cancelExecution}>Cancel</button>
            </div>
          {:else if execOverallStatus === "done"}
            <div class="exec-finished">
              <p>✅ Project generated in {execResult?.duration ?? 0}ms</p>
              <p class="exec-plan-path">Location: {execPlan?.project_path ?? execProjectPath}</p>
            </div>
            <div class="btn-row">
              <button class="btn-secondary" onclick={openInVSCode}>Open in VS Code</button>
              <button class="btn-primary" onclick={resetAll}>Create Another</button>
            </div>
          {:else if execOverallStatus === "error" || execOverallStatus === "cancelled"}
            <div class="exec-finished error">
              <p>{execOverallStatus === "cancelled" ? "Cancelled" : `Error: ${execError}`}</p>
            </div>
            <div class="btn-row">
              <button class="btn-primary" onclick={resetAll}>Start Over</button>
            </div>
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
                      ? "Integrated App Mode"
                      : "Decoupled Architecture Mode"}
                  </p>
                  <p class="arch-text">
                    {archMode === "integrated"
                      ? "Single integrated project structure."
                      : "Generates two independent projects in ./backend and ./frontend connected via REST/GraphQL API."}
                  </p>
                  <p class="arch-examples">
                    {archMode === "integrated"
                      ? "e.g., Tauri + Svelte, Qt + C++, Go + Cobra"
                      : "e.g., NestJS + Next.js, Django + Vue, Expo + FastAPI"}
                  </p>
                </div>
              </div>
            {/if}
          {/snippet}

          <!-- Phase 0: Project Type -->
          {#if phase === 0}
            <p class="prompt">What are you building?</p>
            <div class="card-grid type-grid">
              {#each tree!.project_types as pt}
                <button class="card" onclick={() => selectType(pt)}>
                  <TechIcon icon={pt.icon} alt={pt.label} size="xl" />
                  <h3>{pt.label}</h3>
                  <p>{pt.description}</p>
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
                  <TechIcon icon={fw.icon} alt={fw.label} size="lg" />
                  <h3>{fw.label}</h3>
                  <p>{fw.description}</p>
                  {#if selectedFrameworks.includes(fw.id) && summary}
                    <span class="fw-lang-chip selected">✓ {summary}</span>
                  {:else}
                    <span class="fw-lang-chip">{fwLangsLabel(fw)}</span>
                  {/if}
                  {#if fw.languages.length > 1}
                    <span class="fw-lang-multi">choose language</span>
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
                        <span class="conflict-alts-label">Вместо этого:</span>
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
                              {altFw.label}
                            </span>
                          {/if}
                        {/each}
                      </span>
                    {/if}
                  {/if}
                </button>
                {#if fwPopup === fw.id}
                  <div class="fw-popup">
                    <p class="popup-title">{fw.label}</p>
                    {#if fw.qt_ui_options?.length}
                      {@const selectedMode = fw.qt_ui_options.find((m) => m.id === popupQtUi) ?? fw.qt_ui_options[0]}
                      {@const webDefs = (selectedMode.web_framework_options ?? [])
                        .map((wid) => tree?.frameworks.find((f) => f.id === wid))
                        .filter((d): d is FrameworkDef => !!d)}
                      <p class="popup-label">UI technology</p>
                      <div class="popup-list">
                        {#each fw.qt_ui_options as mode}
                          <button
                            class="popup-opt stack"
                            class:selected={popupQtUi === mode.id}
                            onclick={() => (popupQtUi = mode.id)}
                          >
                            <span class="popup-opt-label">
                              {mode.label}
                              {#if (mode.web_framework_options ?? []).length > 0}
                                <span class="popup-opt-tag">web UI</span>
                              {/if}
                            </span>
                            <span class="popup-opt-desc">{mode.description}</span>
                          </button>
                        {/each}
                      </div>
                      {#if webDefs.length > 0}
                        <p class="popup-label">Web frontend</p>
                        <div class="popup-list">
                          {#each webDefs as wf}
                            <button
                              class="popup-opt stack"
                              class:selected={popupWebFw === wf.id}
                              onclick={() => (popupWebFw = wf.id)}
                            >
                              <span class="popup-opt-label">{wf.label}</span>
                              <span class="popup-opt-desc">{wf.description}</span>
                            </button>
                          {/each}
                        </div>
                      {/if}
                    {:else if companionOptions(fw).length > 0}
                      <p class="popup-label">Frontend framework</p>
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
                            <span>{c.label}</span>
                            {#if ci === 0}
                              <span class="star">⭐ recommended</span>
                            {/if}
                          </button>
                        {/each}
                      </div>
                      {#if popupCompanion}
                        {@const cfw = tree?.frameworks.find((f) => f.id === popupCompanion)}
                        {#if cfw}
                          <p class="popup-label">Frontend language</p>
                          <div class="popup-list">
                            {#each cfw.languages as l}
                              <button
                                class="popup-opt"
                                class:selected={popupCompanionLang === l}
                                onclick={() => (popupCompanionLang = l)}
                              >
                                <span>{langLabel(l)}</span>
                                {#if l === cfw.recommended_language}
                                  <span class="star">⭐ recommended</span>
                                {/if}
                              </button>
                            {/each}
                          </div>
                        {/if}
                      {/if}
                    {:else}
                      <p class="popup-label">Language</p>
                      <div class="popup-list">
                        {#each fw.languages as l}
                          <button
                            class="popup-opt"
                            class:selected={popupLang === l}
                            onclick={() => (popupLang = l)}
                          >
                            <span>{langLabel(l)}</span>
                            {#if l === fw.recommended_language}
                              <span class="star">⭐ recommended</span>
                            {/if}
                          </button>
                        {/each}
                      </div>
                    {/if}
                    <div class="popup-actions">
                      <button class="btn-primary btn-xs" onclick={applyFwPopup}>✓ Done</button>
                      <button class="btn-secondary btn-xs" onclick={cancelFwPopup}>Cancel</button>
                      {#if selectedFrameworks.includes(fw.id)}
                        <button class="btn-remove" onclick={() => removeFramework(fw.id)}>✕ Remove</button>
                      {/if}
                    </div>
                  </div>
                {/if}
              </div>
            {/snippet}

            <p class="prompt">Stack & Tools</p>
            {@render archBanner()}
            {#if selectedFrameworks.length > 0 || selectedTools.length > 0}
              <button
                class="btn-clear-stack"
                title="Сбросить все выбранные фреймворки и инструменты"
                onclick={() => (confirmClearStack = true)}
              >
                ✕ Clear stack
              </button>
            {/if}
            {#if dropNotice}
              <p class="notice-bar" role="status">{dropNotice}</p>
            {/if}
            <p class="hint">
              Pick frameworks freely — languages are assigned automatically when you select one
              (multi-language frameworks open a language menu). Options marked
              <span class="star">⭐</span> are the recommended defaults and apply automatically.
              One main framework per side — side libraries (bots, plugins) can be added alongside.
              Conflicting picks are removed automatically; on blocked cards you'll find the reason
              and compatible alternatives.
            </p>

            {#if confirmClearStack}
              <div class="clear-overlay" onclick={() => (confirmClearStack = false)}>
                <div class="clear-dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
                  <h3>✕ Clear the selected stack?</h3>
                  <p>
                    This will remove all selected <strong>frameworks and tools</strong>
                    ({selectedFrameworks.length} framework{selectedFrameworks.length === 1 ? "" : "s"},{" "}
                    {selectedTools.length} tool{selectedTools.length === 1 ? "" : "s"}).
                    Project type and languages will stay untouched.
                  </p>
                  <div class="clear-actions">
                    <button class="btn-primary" onclick={() => clearStack()}>Yes, clear everything</button>
                    <button class="btn-back" onclick={() => (confirmClearStack = false)}>Cancel</button>
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
                      {showUnavailable[levelKey] ? "Hide unavailable" : "Show unavailable"}
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
                      <strong>{unavailable.length} unavailable framework{unavailable.length === 1 ? "" : "s"}</strong>
                      <span>Blocked by the current stack. Show them dimmed.</span>
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
                "Backend territory",
                "Server-side: APIs, services, bots — goes into backend/",
                backendFws,
                backendLangs,
              )}
            {/if}
            {#if frontendFws.length > 0}
              {@render territory(
                "frontend",
                "Frontend territory",
                "Client-side: interfaces for the browser or apps — goes into frontend/",
                frontendFws,
                frontendLangs,
              )}
            {/if}
            {#if eitherFws.length > 0}
              {@render territory(
                "either",
                "Desktop & standalone territory",
                "Whole-project apps that own everything (Tauri, Qt) — conflicts with backend/frontend stacks",
                eitherFws,
                [],
              )}
            {/if}
            {#if !hasBackend}
              <p class="hint backendless-note">
                This project type has no backend — the «Backend Language» and «Backend Framework»
                steps are skipped. Only client-side technologies apply.
              </p>
            {/if}
            {#if fws.length === 0}
              <p class="muted">No frameworks available for this project type.</p>
            {/if}

            <!-- Языки без фреймворков (необязательно): чистый стек или поддержка -->
            <section class="territory territory-langs">
              <header class="territory-head">
                <TechIcon alt="" size="md" />
                <div class="territory-title-wrap">
                  <h3 class="territory-title">Plain languages</h3>
                  <p class="territory-desc">
                    Optional — pick languages without frameworks (pure backend, plain JS frontend).
                    Frameworks adapt to them automatically.
                  </p>
                </div>
              </header>
              <div class="territory-body">
                <div class="lang-sides">
                  {#if hasBackend && backendCandidates().length > 0}
                    <div class="lang-side">
                      <p class="lang-side-title">Backend language</p>
                      <p class="hint-sm">One per side — picking another replaces it. ✓ = active.</p>
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
                            <TechIcon icon={lang.icon} alt={lang.label} size="lg" />
                            <h3>{lang.label}</h3>
                            {#if backendLangs.includes(lang.id)}
                              <span class="fw-lang-chip selected">✓ active</span>
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
                      <p class="lang-side-title">Frontend language</p>
                      <p class="hint-sm">One per side — picking another replaces it. ✓ = active.</p>
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
                            <TechIcon icon={lang.icon} alt={lang.label} size="lg" />
                            <h3>{lang.label}</h3>
                            {#if lang.category === "static"}
                              <p>Plain HTML, CSS & JS</p>
                            {/if}
                            {#if frontendLangs.includes(lang.id)}
                              <span class="fw-lang-chip selected">✓ active</span>
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
                      Backend language selection is skipped for this project type — it has no server side.
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
                  <h3 class="territory-title">Tools & Features</h3>
                  <p class="territory-desc">Databases, caches, testing, containers — pick what your stack needs.</p>
                </div>
                <span class="territory-count">{selectedTools.length} selected</span>
              </header>
              <div class="territory-body">
                {#each TOOL_CATEGORIES as cat}
                  {@const catTools = availableTools().filter((t) => t.category === cat.id)}
                  {@const recTools = catTools.filter((t) => isToolRecommended(t))}
                  {#if catTools.length > 0}
                    <div class="tool-group">
                      <p class="tool-cat-title">
                        <TechIcon alt="" size="xs" />
                        {cat.label}
                        <span class="tool-cat-count">{catTools.length}</span>
                        {#if recTools.length > 0}
                          <span class="tool-cat-rec">⭐ {recTools.length} for your stack</span>
                        {/if}
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
                            <TechIcon icon={tool.icon} alt={tool.label} size="md" />
                            <span class="tool-item-text">
                              <span class="tool-item-name">{tool.label}</span>
                              <span class="tool-item-desc">{tool.description}</span>
                            </span>
                            <span class="tool-item-badges">
                              {#if isToolRecommended(tool)}
                                <span class="tool-item-badge rec">⭐ recommended</span>
                              {/if}
                              {#if tool.requires_docker}
                                <span class="tool-item-badge docker"><TechIcon icon="docker.svg" alt="" size="xs" /> Docker</span>
                              {/if}
                              {#if tool.conflicts.length > 0}
                                <span class="tool-item-badge conflict">
                                  ⚠ conflicts {tool.conflicts.length > 1 ? `(${tool.conflicts.length})` : ""}
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
                  <div class="tooltip" style="left: {tooltipData.x + 12}px; top: {tooltipData.y - 10}px;">
                    <strong>{tooltipData.tool.label}</strong>
                    <p>{tooltipData.tool.description}</p>
                    {#if tooltipData.tool.requires.length > 0}
                      <p class="tt-req">Requires: {tooltipData.tool.requires.join(", ")}</p>
                    {/if}
                    {#if tooltipData.tool.conflicts.length > 0}
                      <p class="tt-conf">Conflicts with: {tooltipData.tool.conflicts.join(", ")}</p>
                    {/if}
                    {#if tooltipData.tool.requires_docker}
                      <p class="tt-docker"><TechIcon icon="docker.svg" alt="" size="xs" /> Requires Docker</p>
                    {/if}
                  </div>
                {/if}

                <div class="features-panel">
                  <p class="group-label">Features</p>
                  <label class="feature-toggle">
                    <input type="checkbox" bind:checked={testing} />
                    <span>Testing</span>
                  </label>
                  <label class="feature-toggle">
                    <input type="checkbox" bind:checked={git} />
                    <span>Git Init</span>
                  </label>
                  <label class="feature-toggle">
                    <input type="checkbox" bind:checked={vscode} />
                    <span>VS Code Config</span>
                  </label>
                  <label class="feature-toggle">
                    <input type="checkbox" checked={dockerEnabled()} disabled />
                    <span>Docker {isDockerForced() ? "(required by tools)" : ""}</span>
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
                {selectedFrameworks.length} framework{selectedFrameworks.length === 1 ? "" : "s"} · {selectedTools.length} tool{selectedTools.length === 1 ? "" : "s"}
                {#if stackError}
                  <span class="mega-error">⚠ {stackError}</span>
                {/if}
              </span>
              <button class="btn-primary" onclick={() => (phase = 2)} disabled={!!stackError}>
                Review & Create →
              </button>
            </div>
          {/if}

          <!-- Phase 2: Review -->
          {#if phase === 2}
            <p class="prompt">Review & Create</p>
            {@render archBanner()}

            <div class="project-name-section">
              <label class="pn-label" for="project-name">Project Name</label>
              <input
                id="project-name"
                class="pn-input"
                type="text"
                placeholder="my-awesome-app"
                bind:value={projectName}
                oninput={onProjectNameInput}
              />
              <div class="folder-row">
                <button class="btn-select-folder" onclick={pickProjectFolder}>
                  📁 {selectedFolder ? "Change folder" : "Select destination folder"}
                </button>
                {#if selectedFolder}
                  <span class="folder-path" title={selectedFolder}>{selectedFolder}</span>
                {/if}
              </div>
              {#if selectedFolder && projectName}
                <div class="path-preview">
                  <span class="pp-label">Full path:</span>
                  <code class="pp-path">{effectiveProjectPath()}</code>
                  {#if folderCheckPending}
                    <span class="pp-checking">checking…</span>
                  {:else if folderExists}
                    <span class="pp-exists">⚠️ Folder already exists</span>
                  {/if}
                </div>
              {/if}
            </div>

            {#if showConflictDialog}
              <div class="conflict-overlay" onclick={() => { showConflictDialog = false; }}>
                <div class="conflict-dialog" onclick={(e) => e.stopPropagation()}>
                  <h3>⚠️ Folder already exists</h3>
                  <p>
                    The folder <strong>{effectiveProjectPath()}</strong> already exists.
                    Creating a project here may overwrite existing files.
                  </p>
                  <div class="conflict-actions">
                    <button class="btn-primary" onclick={() => resolveFolderConflict('overwrite')}>
                      Overwrite
                    </button>
                    <button class="btn-secondary" onclick={() => resolveFolderConflict('auto-rename')}>
                      Auto-rename folder to <code>{projectName}-2</code>
                    </button>
                    <button class="btn-back" onclick={() => resolveFolderConflict('cancel')}>
                      Use different name
                    </button>
                  </div>
                </div>
              </div>
            {/if}

            {#if reviewError}
              <p class="review-error" role="alert">⛔ {reviewError}</p>
            {/if}
            {#if stackError && (!projectName || !selectedFolder)}
              <p class="review-hint">⚠ {stackError}</p>
            {/if}

            <div class="btn-row">
              <button class="btn-back" onclick={back}>← Back</button>
              <button
                class="btn-primary create-btn"
                disabled={!projectName || !selectedFolder || stackError !== null}
                title={stackError ?? undefined}
                onclick={confirmAll}
              >
                🚀 Create Project
              </button>
            </div>
            {#if !projectName || !selectedFolder}
              <p class="review-hint">
                {!projectName ? "Enter a project name" : "Select a destination folder"}
                {stackError ? ` · ${stackError}` : ""}
              </p>
            {/if}
          {/if}
        </div>

        <!-- ============ Панель контекста (правая колонка) ============ -->
        <aside class="builder-context">
          <p class="ctx-title">Your stack</p>

          {#if stackIssues.length > 0}
            <div class="stack-issues">
              {#each stackIssues as issue}
                <p class={issue.severity === "Error" ? "error" : "env-warn"}>
                  {issue.severity === "Error" ? "⛔" : "⚠️"} {issue.message}
                </p>
              {/each}
            </div>
          {/if}

          <div class="ctx-group">
            <div class="ctx-row">
              <span class="ctx-label">Project Type</span>
              <span class="ctx-value">{selectedType?.label ?? "—"}</span>
              <button class="btn-change" onclick={() => goPhase(0)}>change</button>
            </div>
            <div class="ctx-row">
              <span class="ctx-label">Backend</span>
              <span class="ctx-value">
                {hasBackend
                  ? (backendLangs.length > 0 ? backendLangs.join(", ") : "None")
                  : "N/A (no backend)"}
              </span>
              {#if hasBackend}
                <button class="btn-change" onclick={() => goPhase(1)}>change</button>
              {/if}
            </div>
            <div class="ctx-row">
              <span class="ctx-label">Frontend</span>
              <span class="ctx-value">{frontendLangs.length > 0 ? frontendLangs.join(", ") : "None"}</span>
              <button class="btn-change" onclick={() => goPhase(1)}>change</button>
            </div>
            <div class="ctx-row">
              <span class="ctx-label">Frameworks</span>
              <span class="ctx-value">
                {selectedFrameworks.length > 0
                  ? selectedFrameworks
                      .map((id) => tree?.frameworks.find((f) => f.id === id)?.label ?? id)
                      .join(", ")
                  : "None"}
              </span>
              <button class="btn-change" onclick={() => goPhase(1)}>change</button>
            </div>
            <div class="ctx-row">
              <span class="ctx-label">Tools</span>
              <span class="ctx-value">
                {selectedTools.length > 0
                  ? `${selectedTools.length} tool${selectedTools.length > 1 ? "s" : ""}`
                  : "None"}
              </span>
              <button class="btn-change" onclick={() => goPhase(1)}>change</button>
            </div>
            <div class="ctx-row">
              <span class="ctx-label">Features</span>
              <span class="ctx-value">
                {[
                  git && "Git Init",
                  testing && "Testing",
                  vscode && "VS Code",
                  dockerEnabled() && "Docker",
                ].filter(Boolean).join(", ") || "None"}
              </span>
              <button class="btn-change" onclick={() => goPhase(1)}>change</button>
            </div>
          </div>

        </aside>
      </div>
      {/if}
    {/if}
  {/if}
</div>

<style>
.wizard { max-width: 1100px; margin: 0 auto; padding: 2rem; }
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
.builder-context { position: sticky; top: 1rem; border: 1px solid var(--sp-border-strong); border-radius: 12px; background: var(--sp-bg-1); padding: 1rem; }
.ctx-title { font-weight: 700; font-size: 1rem; margin: 0 0 0.75rem; color: var(--sp-text-1); }
.ctx-group { display: flex; flex-direction: column; }
.ctx-row { display: flex; align-items: center; gap: 0.5rem; padding: 0.5rem 0; border-bottom: 1px solid var(--sp-border-faint); }
.ctx-row:last-child { border-bottom: none; }
.ctx-label { flex: 0 0 90px; font-weight: 600; color: var(--sp-text-3); font-size: 0.8rem; }
.ctx-value { flex: 1; font-size: 0.85rem; color: var(--sp-text-1); overflow-wrap: anywhere; }
.btn-change { background: none; border: 1px solid var(--sp-border-strong); color: var(--sp-text-3); padding: 0.2rem 0.6rem; border-radius: 6px; cursor: pointer; font-size: 0.75rem; flex: 0 0 auto; }
.btn-change:hover { border-color: var(--sp-accent-strong); color: #fff; }

/* ---- Фазы ---- */
.phase-nav { display: flex; gap: 0.75rem; margin-bottom: 1.75rem; flex-wrap: wrap; }
.phase-item { display: flex; align-items: center; gap: 0.4rem; background: none; border: none; cursor: pointer; color: var(--sp-text-3); font-size: 0.85rem; padding: 0.25rem 0.5rem; border-radius: 6px; }
.phase-item:hover { color: var(--sp-text-2); }
.phase-item.active { color: #fff; }
.phase-item.active .phase-circle { background: var(--sp-accent-strong); color: #fff; }
.phase-item.done .phase-circle { background: var(--sp-success); color: #fff; }
.phase-circle { width: 26px; height: 26px; border-radius: 50%; display: flex; align-items: center; justify-content: center; background: var(--sp-bg-2); font-weight: 700; font-size: 0.8rem; }
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
  background: rgba(24, 24, 44, 0.6);
}
.arch-integrated { border-color: rgba(0, 184, 148, 0.45); background: rgba(0, 184, 148, 0.07); }
.arch-decoupled { border-color: rgba(108, 92, 231, 0.5); background: rgba(108, 92, 231, 0.08); }
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
  background: rgba(26, 26, 46, 0.55);
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
.fw-level > summary:hover { background: rgba(108, 92, 231, 0.08); }
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

/* ---- Карточки ---- */
.card-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: 0.75rem; margin-bottom: 1.5rem; }
.card { display: flex; flex-direction: column; align-items: center; gap: 0.4rem; padding: 1rem; border: 1px solid var(--sp-border-strong); border-radius: 10px; background: var(--sp-bg-1); cursor: pointer; transition: all 0.15s; text-align: center; color: var(--sp-text-1); }
.card:hover { border-color: var(--sp-accent-strong); background: var(--sp-bg-2); }
.card.selected { border-color: var(--sp-accent-strong); background: var(--sp-accent-soft); box-shadow: 0 0 0 2px var(--sp-accent-strong); }
.card.blocked { opacity: 0.55; cursor: not-allowed; border-color: var(--sp-border-strong); background: var(--sp-bg-1); }
.card.blocked:hover { border-color: var(--sp-border-strong); background: var(--sp-bg-1); }

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
.type-grid { grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); }
.fw-grid { grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); grid-auto-rows: 1fr; align-items: stretch; }
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
.warn-badge { display: block; font-size: 0.68rem; color: var(--sp-warning); background: rgba(251, 191, 36, 0.12); border: 1px solid rgba(251, 191, 36, 0.35); border-radius: 6px; padding: 0.1rem 0.45rem; margin-top: 0.25rem; }
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
.notice-bar { display: block; font-size: 0.78rem; color: var(--sp-warning); background: rgba(251, 191, 36, 0.12); border: 1px solid rgba(251, 191, 36, 0.35); border-radius: 8px; padding: 0.45rem 0.7rem; margin-bottom: 0.8rem; }
.fw-lang-chip { display: inline-block; font-size: 0.72rem; color: var(--sp-accent); background: var(--sp-accent-soft); border: 1px solid var(--sp-accent-border); padding: 0.15rem 0.5rem; border-radius: 999px; margin-top: 0.3rem; }
.fw-lang-chip.selected { color: var(--sp-success); background: rgba(163, 230, 53, 0.12); border-color: rgba(163, 230, 53, 0.4); }
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
  background: rgba(34, 211, 238, 0.12);
  border: 1px solid rgba(34, 211, 238, 0.35);
  border-radius: 999px;
  padding: 0.05rem 0.45rem;
  font-weight: 600;
}
.star { color: var(--sp-warning); font-size: 0.72rem; white-space: nowrap; }
.popup-actions { display: flex; gap: 0.4rem; margin-top: 0.7rem; align-items: center; flex-wrap: wrap; }
.btn-xs { padding: 0.3rem 0.7rem; font-size: 0.78rem; }
.btn-remove { background: none; border: 1px solid rgba(248, 113, 113, 0.35); color: var(--sp-danger); padding: 0.3rem 0.7rem; border-radius: 6px; cursor: pointer; font-size: 0.78rem; }
.btn-remove:hover { background: rgba(248, 113, 113, 0.12); }

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
  background: rgba(21, 21, 46, 0.95);
  border: 1px solid var(--sp-border-strong);
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
  background: rgba(24, 24, 44, 0.6);
  margin-bottom: 1rem;
}
.territory-head {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--sp-border);
  border-radius: 11px 11px 0 0;
}
.territory-backend .territory-head { background: rgba(108, 92, 231, 0.12); border-bottom-color: rgba(108, 92, 231, 0.35); }
.territory-frontend .territory-head { background: rgba(46, 196, 182, 0.1); border-bottom-color: rgba(46, 196, 182, 0.3); }
.territory-either .territory-head { background: rgba(241, 196, 15, 0.08); border-bottom-color: rgba(241, 196, 15, 0.3); }
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
.lang-grid { grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); margin-bottom: 0; }

/* ---- Инструменты ---- */
.tool-group { margin-bottom: 1.25rem; }
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
.tool-cat-rec {
  font-size: 0.68rem;
  color: var(--sp-warning);
  background: rgba(251, 191, 36, 0.12);
  border: 1px solid rgba(251, 191, 36, 0.35);
  border-radius: 999px;
  padding: 0.1rem 0.5rem;
  font-weight: 600;
  margin-left: 0.1rem;
}
.tool-menu { display: flex; flex-direction: column; gap: 0.45rem; }
.tool-item {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  width: 100%;
  background: var(--sp-bg-2);
  border: 1px solid var(--sp-border-strong);
  border-radius: 9px;
  padding: 0.55rem 0.8rem;
  cursor: pointer;
  text-align: left;
  color: var(--sp-text-1);
  transition: border-color 0.15s, background 0.15s;
}
.tool-item:hover { border-color: var(--sp-accent-strong); background: var(--sp-bg-2); }
.tool-item.selected { border-color: var(--sp-accent-strong); background: var(--sp-accent-soft); box-shadow: inset 0 0 0 1px var(--sp-accent-strong); }
.tool-item-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 0.1rem; }
.tool-item-name { font-size: 0.88rem; font-weight: 600; }
.tool-item-desc { font-size: 0.74rem; color: var(--sp-text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.tool-item-badges { display: flex; align-items: center; gap: 0.35rem; flex: 0 0 auto; }
.tool-item-badge { font-size: 0.68rem; padding: 0.12rem 0.45rem; border-radius: 999px; white-space: nowrap; }
.tool-item-badge.docker { color: var(--sp-info); background: rgba(34, 211, 238, 0.12); border: 1px solid rgba(34, 211, 238, 0.35); }
.tool-item-badge.conflict { color: var(--sp-danger); background: rgba(248, 113, 113, 0.12); border: 1px solid rgba(248, 113, 113, 0.35); }
.tool-item-badge.rec { color: var(--sp-warning); background: rgba(251, 191, 36, 0.12); border: 1px solid rgba(251, 191, 36, 0.35); }
.tool-item-check { color: var(--sp-accent-strong); font-weight: 700; font-size: 0.95rem; }
.group-label { font-size: 0.9rem; font-weight: 600; margin-bottom: 0.4rem; color: var(--sp-text-2); text-transform: capitalize; }
.tooltip { position: fixed; background: var(--sp-bg-1); border: 1px solid var(--sp-accent-strong); border-radius: 8px; padding: 0.6rem 0.9rem; font-size: 0.8rem; max-width: 240px; z-index: 999; pointer-events: none; color: var(--sp-text-2); }
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
.stack-issues { border: 1px solid rgba(231, 76, 60, 0.4); border-radius: 10px; padding: 0.8rem 1rem; margin-bottom: 0.75rem; background: rgba(248, 113, 113, 0.1); }
.stack-issues p { margin: 0.3rem 0; font-size: 0.8rem; }

/* ---- Summary ---- */
.project-name-section { border: 1px solid var(--sp-border-strong); border-radius: 10px; padding: 1.25rem; margin-bottom: 1rem; background: var(--sp-bg-1); }
.pn-label { display: block; font-weight: 700; font-size: 1rem; margin-bottom: 0.5rem; color: var(--sp-text-1); }
.pn-input { width: 100%; padding: 0.65rem 0.8rem; border-radius: 8px; border: 1px solid var(--sp-border-strong); background: var(--sp-bg-1); color: #fff; font-size: 1rem; box-sizing: border-box; outline: none; }
.pn-input:focus { border-color: var(--sp-accent-strong); box-shadow: 0 0 0 2px rgba(108,92,231,0.25); }
.pn-input::placeholder { color: var(--sp-text-3); }
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
  background: rgba(248, 113, 113, 0.1);
  border: 1px solid rgba(248, 113, 113, 0.35);
  border-radius: 8px;
  padding: 0.35rem 0.8rem;
  cursor: pointer;
  margin-bottom: 0.8rem;
  margin-left: 0.5rem;
  transition: background 0.15s;
}
.btn-clear-stack:hover { background: rgba(248, 113, 113, 0.12); border-color: var(--sp-danger); }
.notice-bar { display: inline-block; font-size: 0.78rem; color: var(--sp-warning); background: rgba(251, 191, 36, 0.12); border: 1px solid rgba(251, 191, 36, 0.35); border-radius: 8px; padding: 0.45rem 0.7rem; margin-bottom: 0.8rem; }
.conflict-dialog { background: var(--sp-bg-1); border: 1px solid var(--sp-accent-strong); border-radius: 12px; padding: 1.5rem; max-width: 480px; width: 90%; }
.conflict-dialog h3 { margin: 0 0 0.75rem; color: var(--sp-warning); }
.conflict-dialog p { font-size: 0.9rem; color: var(--sp-text-2); margin: 0 0 1.25rem; line-height: 1.4; }
.conflict-dialog code { color: var(--sp-accent-strong); }
.conflict-actions { display: flex; flex-direction: column; gap: 0.6rem; }
.conflict-actions button { width: 100%; text-align: center; }
.secret-row { display: flex; align-items: center; gap: 0.6rem; margin-bottom: 0.6rem; }
.secret-name { flex: 0 0 110px; font-size: 0.85rem; color: var(--sp-text-2); font-weight: 600; }
.secret-value { flex: 1; font-family: Consolas, monospace; font-size: 0.85rem; background: var(--sp-bg-2); border: 1px solid var(--sp-border-strong); border-radius: 6px; padding: 0.4rem 0.6rem; color: var(--sp-accent-strong); overflow-x: auto; white-space: nowrap; user-select: all; }
.secret-row .btn-secondary { flex: 0 0 auto; }

/* ---- Шаблоны ---- */
.preset-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 0.9rem; }
.preset-card { display: flex; flex-direction: column; align-items: center; gap: 0.5rem; padding: 1.1rem; border: 1px solid var(--sp-border-strong); border-radius: 12px; background: var(--sp-bg-1); text-align: center; color: var(--sp-text-1); }
.preset-card:hover { border-color: var(--sp-accent-strong); }
.preset-card h3 { margin: 0; font-size: 1rem; }
.preset-desc { font-size: 0.8rem; color: var(--sp-text-3); margin: 0; }
.preset-stack { display: flex; flex-wrap: wrap; gap: 0.3rem; justify-content: center; min-height: 1.4rem; }
.preset-chip { font-size: 0.72rem; background: var(--sp-accent-soft); color: var(--sp-text-2); padding: 0.15rem 0.5rem; border-radius: 10px; }
.preset-apply { width: 100%; }

/* ---- Анализ ---- */
.analysis-panel { margin-bottom: 2rem; }
.analysis-summary { font-size: 1rem; font-weight: 600; margin-bottom: 0.5rem; }
.analysis-section { margin: 0.75rem 0; }
.section-title { font-weight: 600; font-size: 0.9rem; color: var(--sp-text-2); margin-bottom: 0.3rem; }
.tech-tags, .hint-tags { display: flex; flex-wrap: wrap; gap: 0.4rem; }
.tech-tag { padding: 0.2rem 0.5rem; border-radius: 4px; font-size: 0.8rem; background: var(--sp-accent-soft); color: var(--sp-text-2); }
.tech-tag.certaion { background: rgba(163, 230, 53, 0.2); color: #fff; }
.tech-tag.likely { background: var(--sp-accent-soft); }
.tech-tag.possible { background: var(--sp-bg-2); }
.hint-tag { padding: 0.2rem 0.5rem; border-radius: 4px; font-size: 0.8rem; background: var(--sp-bg-2); color: var(--sp-text-3); }
.hint-tag.docker { background: rgba(34, 211, 238, 0.15); color: var(--sp-info); }
.analyzed-path { font-size: 0.85rem; color: var(--sp-accent-strong); margin-top: 0.3rem; }

/* ---- Окружение ---- */
.env-summary { display: flex; flex-wrap: wrap; gap: 0.75rem; align-items: center; padding: 0.75rem 1rem; border: 1px solid var(--sp-border-strong); border-radius: 10px; background: var(--sp-bg-1); margin-bottom: 1rem; font-size: 0.85rem; color: var(--sp-text-2); }
.env-warn { color: var(--sp-warning); font-weight: 600; }
.env-list { display: flex; flex-direction: column; gap: 0.4rem; margin-bottom: 1rem; }
.env-row { display: flex; align-items: center; gap: 0.6rem; padding: 0.5rem 0.75rem; border-radius: 6px; background: var(--sp-bg-1); border-left: 3px solid var(--sp-border-strong); }
.env-row.ok { border-left-color: var(--sp-success); }
.env-row.update { border-left-color: var(--sp-warning); }
.env-row.broken { border-left-color: var(--sp-danger); }
.env-row.missing { border-left-color: var(--sp-danger); opacity: 0.8; }
.env-row.manual { border-left-color: var(--sp-warning); }
.env-optional { margin-bottom: 1rem; padding: 0.75rem 1rem; border: 1px dashed var(--sp-danger); border-radius: 10px; background: rgba(231, 76, 60, 0.06); }
.env-optional .group-label { color: var(--sp-danger); margin: 0 0 0.35rem; }
.env-optional .env-row { background: rgba(248, 113, 113, 0.08); }
.env-optional .env-row.ok { border-left-color: var(--sp-success); background: rgba(163, 230, 53, 0.08); }
.env-optional-local { border-color: var(--sp-success); background: rgba(0, 184, 148, 0.06); }
.env-optional-local .group-label { color: var(--sp-success); }
.infra-toggle {
  display: inline-flex;
  align-items: center;
  gap: 0;
  flex: 0 0 auto;
  border: 1px solid var(--sp-border-strong);
  border-radius: 999px;
  overflow: hidden;
  background: var(--sp-bg-1);
}
.infra-toggle-opt {
  border: none;
  background: transparent;
  color: var(--sp-text-3);
  font-size: 0.72rem;
  font-weight: 600;
  padding: 0.3rem 0.75rem;
  cursor: pointer;
  white-space: nowrap;
  transition: background 0.15s, color 0.15s;
}
.infra-toggle-opt:hover { color: var(--sp-text-1); background: rgba(108, 92, 231, 0.12); }
.infra-toggle-opt.active { background: var(--sp-accent-strong); color: #fff; }
.infra-toggle-opt.active.host { background: var(--sp-success); }
.env-select { min-width: 22px; display: flex; align-items: center; justify-content: center; cursor: pointer; }
.env-select input { accent-color: var(--sp-accent-strong); cursor: pointer; width: 15px; height: 15px; }
.manual-badge { cursor: help; font-size: 0.95rem; }
.env-icon { min-width: 20px; font-size: 0.95rem; }
.env-name { font-weight: 600; font-size: 0.9rem; flex: 0 0 auto; }
.env-source { font-size: 0.75rem; color: var(--sp-text-3); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.env-status { font-size: 0.8rem; font-weight: 600; flex: 0 0 auto; }
.env-status.ok { color: var(--sp-success); }
.env-status.update { color: var(--sp-warning); }
.env-status.broken { color: var(--sp-danger); }
.env-status.missing { color: var(--sp-danger); }
.env-status.manual { color: var(--sp-warning); }
.env-install { margin-top: 0.5rem; }
.env-progress-list { display: flex; flex-direction: column; gap: 0.35rem; margin-top: 0.75rem; }
.env-progress-row { display: flex; align-items: center; gap: 0.6rem; font-size: 0.85rem; }
.env-progress-row .env-status { margin-left: auto; }
.env-task { display: flex; flex-direction: column; gap: 0.2rem; }
.dl-bar { height: 6px; border-radius: 3px; background: var(--sp-bg-3); overflow: hidden; margin-left: 1.9rem; margin-right: 0.4rem; }
.dl-fill { height: 100%; background: linear-gradient(90deg, var(--sp-blue), var(--sp-accent)); border-radius: 3px; transition: width 0.3s ease; }
.spin { display: inline-block; width: 0.8rem; height: 0.8rem; border: 2px solid var(--sp-border-strong); border-top-color: var(--sp-blue); border-radius: 50%; animation: tc-spin 0.8s linear infinite; vertical-align: -2px; margin-right: 0.3rem; }
@keyframes tc-spin { to { transform: rotate(360deg); } }

/* ---- Исполнение ---- */
.exec-steps { display: flex; flex-direction: column; gap: 0.5rem; margin: 1rem 0; }
.exec-step { display: flex; align-items: flex-start; gap: 0.6rem; padding: 0.5rem; border-radius: 6px; background: var(--sp-bg-1); }
.exec-step.running { border-left: 3px solid var(--sp-accent-strong); }
.exec-step.success { border-left: 3px solid var(--sp-success); }
.exec-step.failed { border-left: 3px solid var(--sp-danger); }
.exec-step.skipped { border-left: 3px solid var(--sp-text-3); opacity: 0.6; }
.exec-icon { font-size: 1.1rem; min-width: 24px; }
.exec-detail { flex: 1; min-width: 0; }
.exec-name { font-weight: 600; margin: 0; font-size: 0.9rem; }
.exec-log { font-size: 0.75rem; color: var(--sp-text-3); background: var(--sp-bg-2); padding: 0.3rem; border-radius: 4px; max-height: 80px; overflow-y: auto; margin: 0.3rem 0 0; white-space: pre-wrap; word-break: break-all; }
.exec-full-log { margin: 1rem 0; }
.exec-full-log summary { cursor: pointer; color: var(--sp-text-3); font-size: 0.85rem; }
.exec-full-log pre { font-size: 0.75rem; background: var(--sp-bg-2); padding: 0.5rem; border-radius: 6px; max-height: 200px; overflow-y: auto; white-space: pre-wrap; word-break: break-all; }
.exec-finished { margin: 1rem 0; }
.exec-finished p { margin: 0.3rem 0; }
.exec-finished.error { color: var(--sp-danger); }
.exec-plan-path { font-size: 0.85rem; color: var(--sp-text-3); }

@media (max-width: 900px) {
  .builder { grid-template-columns: 1fr; }
  .builder-context { position: static; }
}
</style>
