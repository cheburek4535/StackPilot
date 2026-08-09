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
  getStackRecommendations,
} from "$lib/modules/project_creator/api";
import { validateStack, firstError } from "$lib/modules/project_creator/rules";
import type {
  WizardTreeData,
  ProjectTypeDef,
  FrameworkDef,
  ToolDef,
  ProjectPreset,
  AnalysisReport,
  WizardContext,
  ExecutionPlan,
  ExecutionEvent,
  StepStatus,
  StackIssue,
  StackRecommendations,
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
} from "$lib/modules/toolchain/api";
import type {
  EnvironmentCheck,
  InstallPlan,
  ToolchainEvent,
  TaskState,
  CheckProgressEvent,
  ProjectRequirements,
  ToolRequirement,
} from "$lib/modules/toolchain/types";
import { statusKind, statusLabel, taskStateKind, taskStateLabel } from "$lib/modules/toolchain/types";

let tree = $state<WizardTreeData | null>(null);
let status = $state<string>("loading");
let hostOs = $state<string>("windows");

// ---- Mode: Constructor | Templates | Analyze ----
let mode = $state<"constructor" | "presets" | "analyze">("constructor");

// ---- Constructor context ----
const PHASES = ["Project Type", "Stack & Tools", "Review"];
let phase = $state(0); // 0..4 — конструктор; 5 — окружение; 6 — генерация
let selectedType = $state<ProjectTypeDef | null>(null);
let backendLangs = $state<string[]>([]);
let frontendLangs = $state<string[]>([]);
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

// ---- Рекомендации стека (бэкенд recommend.rs) ----
let recommendations = $state<StackRecommendations | null>(null);
let recommendationsError = $state<string | null>(null);
$effect(() => {
  const t = tree;
  const type = selectedType?.id ?? null;
  const backend = backendLangs;
  const frontend = frontendLangs;
  const fws = selectedFrameworks;
  if (!t) return;
  let cancelled = false;
  recommendations = null;
  recommendationsError = null;
  getStackRecommendations(type, backend, frontend, fws)
    .then((r) => {
      if (!cancelled) recommendations = r;
    })
    .catch(() => {
      if (!cancelled) recommendationsError = "Failed to load recommendations";
    });
  return () => {
    cancelled = true;
  };
});

/** Метка фреймворка по id (для рекомендаций) */
function fwLabel(id: string): string {
  return tree?.frameworks.find((f) => f.id === id)?.label ?? id;
}

/** Метка инструмента по id (для рекомендаций) */
function toolLabel(id: string): string {
  return tree?.tools.find((t) => t.id === id)?.label ?? id;
}

/** Сразу добавить рекомендованный инструмент */
function applyRecommendedTool(id: string) {
  toggleTool(id);
}

// ---- Project name & folder ----
let projectName = $state("");
let selectedFolder = $state<string | null>(null);
let folderExists = $state(false);
let folderCheckPending = $state(false);
let showConflictDialog = $state(false);
let conflictResolvedFolder = $state<string | null>(null);

// ---- Analysis ----
let analysisResult = $state<AnalysisReport | null>(null);
let analysisError = $state<string | null>(null);
let analyzing = $state(false);
let analyzedPath = $state<string | null>(null);

// ---- Execution ----
let execPlan = $state<ExecutionPlan | null>(null);
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
let envSelectedIds = $state<Set<string>>(new Set());

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

onMount(async () => {
  try {
    tree = await getWizardTree();
    status = tree.project_types.length > 0 ? "ready" : "empty";
  } catch (e) {
    status = "error";
    console.error(e);
  }
  try {
    hostOs = await getHostPlatform();
  } catch (e) {
    console.error("cannot detect host OS:", e);
  }
});

onDestroy(() => {
  if (unlisten) unlisten();
  if (unlistenTc) unlistenTc();
  if (unlistenTcDone) unlistenTcDone();
  if (unlistenTcCheck) unlistenTcCheck();
  stopTick();
});

function imgSrc(name: string | null): string {
  if (!name) return "";
  return `/images/${name}`;
}

// ----------------------------------------------------------
// Фреймворки
// ----------------------------------------------------------

function availableFrameworks(): FrameworkDef[] {
  if (!tree || !selectedType) return [];
  return tree.frameworks.filter(
    (f) => !f.project_types?.length || f.project_types.includes(selectedType!.id),
  );
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
    icon: "🏗️",
    title: "Full application frameworks",
    note: "Create the whole project scaffold by themselves (Spring Boot, Django, Next.js).",
  },
  {
    id: "inplace",
    icon: "🔧",
    title: "In-place & lightweight",
    note: "Attach into a base project of their language (FastAPI, Express, Gin).",
  },
  {
    id: "side",
    icon: "🧩",
    title: "Side modules & libraries",
    note: "Optional add-ons to the main stack (bots, plugins) — can coexist with anything.",
  },
] as const;

/** Пересчёт языков сторон из выбранных фреймворков. Языки следуют за
 *  фреймворками: на каждой стороне собираются языки её фреймворков. */
function recomputeSideLangs() {
  const t = tree;
  if (!t) return;
  const backend = new Set(backendLangs);
  const frontend = new Set(frontendLangs);
  for (const id of selectedFrameworks) {
    const fw = t.frameworks.find((f) => f.id === id);
    if (!fw) continue;
    const lang = fwLangs[id] ?? fw.recommended_language;
    if (!lang) continue;
    if (fw.side === "frontend") frontend.add(lang);
    else backend.add(lang);
  }
  backendLangs = [...backend];
  frontendLangs = [...frontend];
}

/** Причина, по которой фреймворк нельзя выбрать (зеркало правил rules.ts) */
function frameworkBlockReason(fwId: string): string | null {
  const fw = tree?.frameworks.find((f) => f.id === fwId);
  if (!fw) return null;
  if (selectedFrameworks.includes(fwId)) return null;
  if (fw.platforms?.length && !fw.platforms.includes(hostOs)) {
    return `Available only on ${fw.platforms.join(", ")}`;
  }
  for (const selId of selectedFrameworks) {
    const sel = tree?.frameworks.find((f) => f.id === selId);
    if (sel?.conflicts?.includes(fwId)) return `Incompatible with ${sel.label}`;
  }
  for (const c of fw.conflicts ?? []) {
    if (selectedFrameworks.includes(c)) {
      const cFw = tree?.frameworks.find((f) => f.id === c);
      return `Incompatible with ${cFw?.label ?? c}`;
    }
  }
  if (fw.side === "backend" || fw.side === "frontend") {
    if (fw.kind === "app") {
      const sameSide = selectedFrameworks.some((id) => {
        const f = tree?.frameworks.find((x) => x.id === id);
        return !!f && f.kind === "app" && f.side === fw.side;
      });
      if (sameSide) return `Only one main ${fw.side} framework`;
    }
  }
  return null;
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
  if (fw.side === "either" && companionOptions(fw).length > 0 && !linkedCompanions[id]) {
    const first = companionOptions(fw)[0];
    addCompanion(fw.id, first.id);
  }
  const dropConflicts = fw.conflicts ?? [];
  selectedFrameworks = [...selectedFrameworks.filter((f) => !dropConflicts.includes(f)), id];
  fwLangs = { ...fwLangs, [id]: fwLangs[id] ?? fw.recommended_language };
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
/** Связки «владелец → подфреймворк» (tauri → svelte): удаление владельца тянет подфреймворк */
let linkedCompanions = $state<Record<string, string>>({});

/** Подфреймворки для side="either" (tauri): совместимые фронтовые приложения */
function companionOptions(fw: FrameworkDef): FrameworkDef[] {
  if (!tree || fw.side !== "either" || fw.kind !== "app") return [];
  const compat = tree.frameworks.filter(
    (c) =>
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

/** Список языков фреймворка одной строкой ("TypeScript, JavaScript") */
function fwLangsLabel(fw: FrameworkDef): string {
  return fw.languages.map((l) => langLabel(l)).join(", ");
}

/** Сводка выбранных языков для чипа на карточке ("TypeScript" / "TypeScript + JavaScript") */
function fwLangSummary(fw: FrameworkDef): string {
  if (!selectedFrameworks.includes(fw.id)) return "";
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
  fwPopup = id;
}

/** Применить черновики попапа (Done) — меню закрывается только явно */
function applyFwPopup() {
  const fw = tree?.frameworks.find((f) => f.id === fwPopup);
  if (!fw) {
    fwPopup = null;
    return;
  }
  if (popupLang && fw.languages.includes(popupLang)) {
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
        fwLangs = { ...fwLangs, [popupCompanion]: popupCompanionLang };
      }
    }
  }
  recomputeSideLangs();
  fwPopup = null;
}

function cancelFwPopup() {
  fwPopup = null;
}

/** Удалить фреймворк вместе со связанным подфреймворком */
function removeFramework(id: string) {
  const prev = linkedCompanions[id];
  const toRemove = new Set([id]);
  if (prev) toRemove.add(prev);
  selectedFrameworks = selectedFrameworks.filter((x) => !toRemove.has(x));
  const nl = { ...linkedCompanions };
  delete nl[id];
  linkedCompanions = nl;
  const nf = { ...fwLangs };
  for (const r of toRemove) delete nf[r];
  fwLangs = nf;
  recomputeSideLangs();
  fwPopup = null;
}

// ----------------------------------------------------------
// Инструменты
// ----------------------------------------------------------

function availableTools(): ToolDef[] {
  if (!tree) return [];
  const ids = new Set<string>();
  const refs = selectedFrameworks.length > 0 ? selectedFrameworks : [""];
  for (const fwId of refs) {
    const toolIds = tree.framework_tool_map[fwId] ?? [];
    for (const id of toolIds) ids.add(id);
  }
  return tree.tools.filter((t) => ids.has(t.id));
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

function toolCategoryIcon(cat: string): string {
  const icons: Record<string, string> = {
    database: "🗄️",
    cache: "⚡",
    container: "📦",
    testing: "🧪",
    tooling: "🔧",
    ci: "🔄",
    monitoring: "📊",
  };
  return icons[cat] ?? "🔹";
}

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
  backendLangs = p.stack.backend_lang ? [p.stack.backend_lang] : [];
  frontendLangs = p.stack.frontend_lang ? [p.stack.frontend_lang] : [];
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
    frontendLangs = langIds.slice(0, 1);
  } else {
    backendLangs = langIds.slice(0, 1);
    if (langIds.length > 1) frontendLangs = [langIds[1]];
  }
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

async function checkProjectFolder() {
  const path = effectiveProjectPath();
  if (!path) return;
  folderCheckPending = true;
  try {
    folderExists = await checkFolderExists(path);
  } catch {
    folderExists = false;
  } finally {
    folderCheckPending = false;
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
    git_init: git,
    vscode_config: vscode,
    docker: dockerEnabled(),
  };
}

async function runEnvironmentCheck(silent = false) {
  envChecking = !silent;
  envError = null;
  envCheckProgress = [];
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
  if (unlistenTcCheck) unlistenTcCheck();
  unlistenTcCheck = await listenCheckProgress(handleCheckProgress);
  await runEnvironmentCheck();
}

function handleCheckProgress(event: CheckProgressEvent) {
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
  } catch (e) {
    envError = String(e);
    envInstalling = false;
    stopTick();
  }
}

function handleToolchainEvent(event: ToolchainEvent) {
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
      envErrors = [...envErrors, `${event.tool_id}: ${line.slice("tc:error ".length)}`];
      envLogs = [...envLogs, line];
    } else {
      envLogs = [...envLogs, line];
    }
  }
  if ("TaskCompleted" in t) {
    envTaskStates.set(event.task_id, t.TaskCompleted.state);
    envTaskStates = new Map(envTaskStates);
  }
}

function handleInstallDone(plan: InstallPlan) {
  for (const task of plan.tasks) {
    envTaskStates.set(task.task_id, task.state);
  }
  envTaskStates = new Map(envTaskStates);
  envInstalling = false;
  envInstallDone = true;
  stopTick();
  const installedCount = plan.tasks.filter((t) => taskStateKind(t.state) === "success").length;
  if (installedCount > 0) envRestartHint = true;
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

  try {
    const issues = await validateProjectStack(
      selectedType?.id ?? null,
      backendLangs,
      frontendLangs,
      selectedFrameworks,
    );
    if (issues.some((i) => i.severity === "Error")) return;
  } catch {
    // если валидация недоступна — генерацию заблокирует бэкенд
  }

  folderCheckPending = true;
  try {
    const exists = await checkFolderExists(path);
    if (exists) {
      folderExists = true;
      folderCheckPending = false;
      showConflictDialog = true;
      return;
    }
  } catch {
    // ignore, proceed anyway
  }
  folderCheckPending = false;

  await goToEnvironment();
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
    features: [],
    infrastructure: [],
    docker: dockerEnabled(),
    testing,
    ci: false,
    git_init: git,
    vscode_config: vscode,
    answers: {},
  };

  phase = 6;
  execPlan = null;
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
        entry.logs = [...entry.logs, line];
        execLogs = [...execLogs, line];
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
    }
    if ("Error" in t) {
      const err = (t as Record<string, { message: string }>).Error;
      execError = err?.message ?? String(t);
      execOverallStatus = "error";
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
}

function resetAll() {
  selectedType = null;
  backendLangs = [];
  frontendLangs = [];
  selectedFrameworks = [];
  fwLangs = {};
  linkedCompanions = {};
  selectedTools = [];
  testing = true;
  git = true;
  vscode = true;
  tooltipData = null;
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
  newSecrets = null;
  secretCopied = null;
  projectName = "";
  selectedFolder = null;
  conflictResolvedFolder = null;
  folderExists = false;
  phase = 0;
  mode = "constructor";
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
            {#if p.icon}
              <img src={imgSrc(p.icon)} alt={p.label} class="card-img" />
            {:else}
              <span class="card-img-placeholder">▣</span>
            {/if}
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
                    <span class="env-select manual-badge" title="Installed manually, no auto-install">⚙️</span>
                  {:else}
                    <label class="env-select">
                      <input
                        type="checkbox"
                        checked={envSelectedIds.has(req.tool_id)}
                        onchange={() => toggleEnvTool(req.tool_id)}
                      />
                    </label>
                  {/if}
                  <span class="env-icon">{toolCategoryIcon(req.category)}</span>
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
              Failed to check environment.
              {envError ? ` (${envError})` : "Try again."}
            </p>
            <div class="btn-row">
              <button class="btn-back" onclick={back}>← Back</button>
              <button class="btn-primary" onclick={() => runEnvironmentCheck()}>Retry</button>
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
              <p class="exec-plan-path">Location: {execPlan?.project_path}</p>
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

          <!-- Phase 0: Project Type -->
          {#if phase === 0}
            <p class="prompt">What are you building?</p>
            <div class="card-grid type-grid">
              {#each tree!.project_types as pt}
                <button class="card" onclick={() => selectType(pt)}>
                  {#if pt.icon}
                    <img src={imgSrc(pt.icon)} alt={pt.label} class="card-img" />
                  {:else}
                    <span class="card-img-placeholder">▣</span>
                  {/if}
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
              {@const summary = fwLangSummary(fw)}
              <div class="fw-card-wrap">
                <button
                  class="card"
                  class:selected={selectedFrameworks.includes(fw.id)}
                  class:blocked={reason !== null}
                  disabled={reason !== null}
                  onclick={() => clickFramework(fw.id)}
                >
                  {#if fw.icon}
                    <img src={imgSrc(fw.icon)} alt={fw.label} class="card-img-sm" />
                  {:else}
                    <span class="card-img-placeholder-sm">▣</span>
                  {/if}
                  <h3>{fw.label}</h3>
                  <p>{fw.description}</p>
                  {#if selectedFrameworks.includes(fw.id) && summary}
                    <span class="fw-lang-chip selected">✓ {summary}</span>
                  {:else}
                    <span class="fw-lang-chip">{fwLangsLabel(fw)}</span>
                  {/if}
                  {#if fw.languages.length > 1}
                    <span class="fw-lang-multi">⚙ choose language</span>
                  {/if}
                  {#if reason}
                    <span class="conflict-badge">{reason}</span>
                  {/if}
                </button>
                {#if fwPopup === fw.id}
                  <div class="fw-popup">
                    <p class="popup-title">{fw.label}</p>
                    {#if companionOptions(fw).length > 0}
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
            <p class="hint">
              Pick frameworks freely — languages are assigned automatically when you select one
              (multi-language frameworks open a language menu). Options marked
              <span class="star">⭐</span> are the recommended defaults and apply automatically.
              One main framework per side — side libraries (bots, plugins) can be added alongside.
            </p>

            {#snippet fwLevel(title: string, icon: string, items: FrameworkDef[], note: string)}
              <details class="fw-level" open>
                <summary>
                  <span class="fw-level-icon">{icon}</span>
                  <span class="fw-level-title">{title}</span>
                  <span class="fw-level-count">{items.length}</span>
                </summary>
                <p class="fw-level-note">{note}</p>
                <div class="card-grid fw-grid">
                  {#each items as fw}
                    {@render fwCard(fw)}
                  {/each}
                </div>
              </details>
            {/snippet}

            {#if backendFws.length > 0}
              <div class="side-section">
                <p class="group-label">⚙️ Backend</p>
                {#each FW_LEVELS as lvl}
                  {@const items = backendFws.filter((f) => fwLevelOf(f) === lvl.id)}
                  {#if items.length > 0}
                    {@render fwLevel(lvl.title, lvl.icon, items, lvl.note)}
                  {/if}
                {/each}
              </div>
            {/if}
            {#if frontendFws.length > 0}
              <div class="side-section">
                <p class="group-label">🖥️ Frontend</p>
                {#each FW_LEVELS as lvl}
                  {@const items = frontendFws.filter((f) => fwLevelOf(f) === lvl.id)}
                  {#if items.length > 0}
                    {@render fwLevel(lvl.title, lvl.icon, items, lvl.note)}
                  {/if}
                {/each}
              </div>
            {/if}
            {#if eitherFws.length > 0}
              <div class="side-section">
                <p class="group-label">💻 Desktop & Full-stack</p>
                <div class="card-grid fw-grid">
                  {#each eitherFws as fw}
                    {@render fwCard(fw)}
                  {/each}
                </div>
              </div>
            {/if}
            {#if fws.length === 0}
              <p class="muted">No frameworks available for this project type.</p>
            {/if}

            {#if recommendations && (recommendations.frameworks.length > 0 || recommendations.side_frameworks.length > 0 || recommendations.tools.length > 0 || recommendations.missing_languages.length > 0)}
              <div class="rec-panel">
                <p class="group-label">Suggestions</p>
                {#if recommendations.missing_languages.length > 0}
                  <p class="rec-note">
                    Add language: {recommendations.missing_languages.join(", ")} — required by your frameworks.
                  </p>
                {/if}
                {#if recommendations.frameworks.length > 0}
                  <div class="rec-row">
                    {#each recommendations.frameworks as sug}
                      <button
                        class="rec-chip"
                        title={sug.note}
                        class:selected={selectedFrameworks.includes(sug.id)}
                        onclick={() => clickFramework(sug.id)}
                      >
                        ⭐ {fwLabel(sug.id)}
                        <span class="rec-chip-note">{sug.note}</span>
                      </button>
                    {/each}
                  </div>
                {/if}
                {#if recommendations.side_frameworks.length > 0}
                  <div class="rec-row">
                    {#each recommendations.side_frameworks as sug}
                      <button
                        class="rec-chip"
                        title={sug.note}
                        class:selected={selectedFrameworks.includes(sug.id)}
                        onclick={() => clickFramework(sug.id)}
                      >
                        ➕ {fwLabel(sug.id)}
                        <span class="rec-chip-note">{sug.note}</span>
                      </button>
                    {/each}
                  </div>
                {/if}
                {#if recommendations.tools.length > 0}
                  <div class="rec-row">
                    {#each recommendations.tools as sug}
                      <button
                        class="rec-chip"
                        title={sug.note}
                        class:selected={selectedTools.includes(sug.id)}
                        onclick={() => applyRecommendedTool(sug.id)}
                      >
                        🛠 {toolLabel(sug.id)}
                        <span class="rec-chip-note">{sug.note}</span>
                      </button>
                    {/each}
                  </div>
                {/if}
              </div>
            {/if}
            {#if recommendationsError}
              <p class="rec-error">{recommendationsError}</p>
            {/if}

            <!-- Языки следуют за фреймворками: отдельного выбора нет.
                 Backend/фронтенд-языки видны на карточках и в сводке. -->

            <!-- Инструменты и фичи -->
            <div class="side-section">
              <p class="group-label">Tools & Features</p>
              {#each ["database", "cache", "messaging", "observability", "testing", "tooling", "container", "orchestration", "etl", "baas", "infra"] as cat}
                {@const catTools = availableTools().filter((t) => t.category === cat)}
                {#if catTools.length > 0}
                  <div class="tool-group">
                    <p class="group-label">{toolCategoryIcon(cat)} {cat}</p>
                    <div class="card-grid tool-grid">
                      {#each catTools as tool}
                        <button
                          class="card card-sm"
                          class:selected={selectedTools.includes(tool.id)}
                          onclick={() => toggleTool(tool.id)}
                          onmouseenter={(e) => showTooltip(tool, e)}
                          onmouseleave={hideTooltip}
                          onfocus={(e) => showTooltip(tool, e)}
                          onblur={hideTooltip}
                        >
                          {#if tool.icon}
                            <img src={imgSrc(tool.icon)} alt={tool.label} class="card-img-xs" />
                          {:else}
                            <span class="card-img-placeholder-xs">▣</span>
                          {/if}
                          <span class="tool-name">{tool.label}</span>
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
                    <p class="tt-docker">🐳 Requires Docker</p>
                  {/if}
                </div>
              {/if}

              <div class="features-panel">
                <p class="group-label">⚙️ Features</p>
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

            <!-- Липкий футер: сводка + переход к финальной сверке -->
            <div class="mega-footer">
              <span class="mega-summary">
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
              <span class="ctx-value">{backendLangs.length > 0 ? backendLangs.join(", ") : "None"}</span>
              <button class="btn-change" onclick={() => goPhase(1)}>change</button>
            </div>
            <div class="ctx-row">
              <span class="ctx-label">Frontend</span>
              <span class="ctx-value">{frontendLangs.length > 0 ? frontendLangs.join(", ") : "None"}</span>
              <button class="btn-change" onclick={() => goPhase(1)}>change</button>
            </div>
            <div class="ctx-row">
              <span class="ctx-label">Frameworks</span>
              <span class="ctx-value">{selectedFrameworks.length > 0 ? selectedFrameworks.join(", ") : "None"}</span>
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

          <button
            class="btn-primary ctx-create"
            disabled={stackError !== null}
            title={stackError ?? undefined}
            onclick={() => goPhase(2)}
          >
            Review & Create →
          </button>
        </aside>
      </div>
      {/if}
    {/if}
  {/if}
</div>

<style>
.wizard { max-width: 1100px; margin: 0 auto; padding: 2rem; }
.muted { color: #888; }
.error { color: #e74c3c; }
.mode-switch { display: flex; gap: 0; margin-bottom: 1.5rem; border-radius: 8px; overflow: hidden; border: 1px solid #444; width: fit-content; }
.mode-btn { padding: 0.5rem 1.25rem; cursor: pointer; border: none; background: #1a1a2e; color: #aaa; font-size: 0.9rem; }
.mode-btn.active { background: #2d2d5e; color: #fff; font-weight: 600; }
.prompt { font-size: 1.4rem; font-weight: 700; margin-bottom: 0.4rem; }
.hint { color: #888; margin-bottom: 1.5rem; font-size: 0.95rem; }

/* ---- Конструктор: две колонки ---- */
.builder { display: grid; grid-template-columns: 1fr 320px; gap: 1.5rem; align-items: start; }
.builder-left { min-width: 0; }
.builder-context { position: sticky; top: 1rem; border: 1px solid #333; border-radius: 12px; background: #15152e; padding: 1rem; }
.ctx-title { font-weight: 700; font-size: 1rem; margin: 0 0 0.75rem; color: #ddd; }
.ctx-group { display: flex; flex-direction: column; }
.ctx-row { display: flex; align-items: center; gap: 0.5rem; padding: 0.5rem 0; border-bottom: 1px solid #222; }
.ctx-row:last-child { border-bottom: none; }
.ctx-label { flex: 0 0 90px; font-weight: 600; color: #888; font-size: 0.8rem; }
.ctx-value { flex: 1; font-size: 0.85rem; color: #ddd; overflow-wrap: anywhere; }
.ctx-create { width: 100%; margin-top: 0.75rem; }
.btn-change { background: none; border: 1px solid #444; color: #888; padding: 0.2rem 0.6rem; border-radius: 6px; cursor: pointer; font-size: 0.75rem; flex: 0 0 auto; }
.btn-change:hover { border-color: #6c5ce7; color: #fff; }

/* ---- Фазы ---- */
.phase-nav { display: flex; gap: 0.75rem; margin-bottom: 1.75rem; flex-wrap: wrap; }
.phase-item { display: flex; align-items: center; gap: 0.4rem; background: none; border: none; cursor: pointer; color: #555; font-size: 0.85rem; padding: 0.25rem 0.5rem; border-radius: 6px; }
.phase-item:hover { color: #aaa; }
.phase-item.active { color: #fff; }
.phase-item.active .phase-circle { background: #6c5ce7; color: #fff; }
.phase-item.done .phase-circle { background: #00b894; color: #fff; }
.phase-circle { width: 26px; height: 26px; border-radius: 50%; display: flex; align-items: center; justify-content: center; background: #2d2d3d; font-weight: 700; font-size: 0.8rem; }
.phase-label { font-weight: 600; }

/* ---- Стороны (Stack) ---- */
.side-section { min-width: 0; }

/* ---- Уровни фреймворков (сворачиваемые колонки) ---- */
.fw-level {
  border: 1px solid #2c2c46;
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
  color: #6c5ce7;
  font-size: 0.8rem;
  transition: transform 0.15s;
}
.fw-level[open] > summary::before { transform: rotate(90deg); }
.fw-level[open] > summary { border-bottom-color: #2c2c46; }
.fw-level > summary:hover { background: rgba(108, 92, 231, 0.08); }
.fw-level-icon { font-size: 1rem; }
.fw-level-title { font-weight: 700; font-size: 0.9rem; color: #eee; }
.fw-level-count {
  margin-left: auto;
  font-size: 0.72rem;
  color: #888;
  background: #22224a;
  border-radius: 999px;
  padding: 0.1rem 0.55rem;
}
.fw-level-note { margin: 0; padding: 0.4rem 0.9rem 0.6rem; font-size: 0.75rem; color: #777; }
.fw-level .fw-grid { margin-bottom: 0; padding: 0.9rem; padding-top: 0.2rem; }

/* ---- Карточки ---- */
.card-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: 0.75rem; margin-bottom: 1.5rem; }
.card { display: flex; flex-direction: column; align-items: center; gap: 0.4rem; padding: 1rem; border: 1px solid #333; border-radius: 10px; background: #1a1a2e; cursor: pointer; transition: all 0.15s; text-align: center; color: #ddd; }
.card:hover { border-color: #6c5ce7; background: #22224a; }
.card.selected { border-color: #6c5ce7; background: #2d2d5e; box-shadow: 0 0 0 2px #6c5ce7; }
.card.blocked { opacity: 0.35; cursor: not-allowed; border-color: #333; background: #15152e; }
.card h3 { margin: 0; font-size: 0.95rem; }
.card p { margin: 0; font-size: 0.78rem; color: #888; }
.card-img { width: 56px; height: 56px; object-fit: contain; }
.card-img-sm { width: 40px; height: 40px; object-fit: contain; }
.card-img-xs { width: 24px; height: 24px; object-fit: contain; }
.card-img-placeholder { font-size: 2rem; }
.card-img-placeholder-sm { font-size: 1.5rem; }
.card-img-placeholder-xs { font-size: 1rem; }
.type-grid { grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); }
.fw-grid { grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); }
.conflict-badge { display: block; font-size: 0.7rem; color: #e74c3c; margin-top: 0.25rem; }
.fw-lang-chip { display: inline-block; font-size: 0.72rem; color: #cdc3f0; background: #2f2460; border: 1px solid #4a3a85; padding: 0.15rem 0.5rem; border-radius: 999px; margin-top: 0.3rem; }
.fw-lang-chip.selected { color: #b8f5d4; background: #14402c; border-color: #1f7a4d; }
.fw-lang-multi { display: block; font-size: 0.68rem; color: #7a6cf0; margin-top: 0.15rem; }

/* ---- Попап настройки фреймворка ---- */
.fw-card-wrap { position: relative; }
.fw-popup {
  position: absolute;
  top: calc(100% + 6px);
  left: 0;
  z-index: 50;
  width: 260px;
  max-width: 90vw;
  background: #1c1c3a;
  border: 1px solid #6c5ce7;
  border-radius: 10px;
  padding: 0.8rem;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
}
.popup-title { margin: 0 0 0.5rem; font-size: 0.85rem; font-weight: 700; color: #fff; }
.popup-label { margin: 0.5rem 0 0.3rem; font-size: 0.72rem; color: #aaa; text-transform: uppercase; letter-spacing: 0.04em; }
.popup-list { display: flex; flex-direction: column; gap: 0.3rem; max-height: 160px; overflow-y: auto; }
.popup-opt {
  display: flex; justify-content: space-between; align-items: center; gap: 0.5rem;
  background: #14142e; border: 1px solid #3a3a6a; color: #ddd;
  padding: 0.45rem 0.6rem; border-radius: 6px; cursor: pointer; font-size: 0.82rem; text-align: left;
}
.popup-opt:hover { border-color: #6c5ce7; }
.popup-opt.selected { border-color: #6c5ce7; background: #2f2460; color: #fff; }
.star { color: #f1c40f; font-size: 0.72rem; white-space: nowrap; }
.popup-actions { display: flex; gap: 0.4rem; margin-top: 0.7rem; align-items: center; flex-wrap: wrap; }
.btn-xs { padding: 0.3rem 0.7rem; font-size: 0.78rem; }
.btn-remove { background: none; border: 1px solid #7a2f3a; color: #e74c3c; padding: 0.3rem 0.7rem; border-radius: 6px; cursor: pointer; font-size: 0.78rem; }
.btn-remove:hover { background: #3a1420; }

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
  border: 1px solid #333;
  border-radius: 10px;
  backdrop-filter: blur(4px);
  z-index: 40;
}
.mega-summary { font-size: 0.85rem; color: #ccc; display: flex; align-items: center; gap: 0.6rem; flex-wrap: wrap; }
.mega-error { color: #e74c3c; font-size: 0.78rem; }
.hint-sm { font-size: 0.75rem; color: #888; margin: 0 0 0.5rem; }
.tauri-note { display: block; margin-top: 0.5rem; font-size: 0.85rem; color: #6c5ce7; }

/* ---- Инструменты ---- */
.tool-group { margin-bottom: 1rem; }
.group-label { font-size: 0.9rem; font-weight: 600; margin-bottom: 0.4rem; color: #aaa; text-transform: capitalize; }
.tool-grid { grid-template-columns: repeat(auto-fill, minmax(100px, 1fr)); }
.card-sm { padding: 0.6rem; }
.tool-name { font-size: 0.78rem; }
.tooltip { position: fixed; background: #1a1a2e; border: 1px solid #6c5ce7; border-radius: 8px; padding: 0.6rem 0.9rem; font-size: 0.8rem; max-width: 240px; z-index: 999; pointer-events: none; color: #ccc; }
.tooltip strong { color: #fff; }
.tt-req, .tt-conf, .tt-docker { margin: 0.2rem 0; font-size: 0.75rem; }
.features-panel { margin-bottom: 1.5rem; display: flex; flex-wrap: wrap; gap: 1rem; align-items: center; }
.feature-toggle { display: flex; align-items: center; gap: 0.4rem; cursor: pointer; font-size: 0.9rem; }
.feature-toggle input { accent-color: #6c5ce7; }

/* ---- Кнопки ---- */
.btn-row { display: flex; gap: 0.75rem; margin-top: 1.5rem; flex-wrap: wrap; }
.btn-back { background: none; border: 1px solid #444; color: #888; padding: 0.4rem 0.9rem; border-radius: 6px; cursor: pointer; font-size: 0.85rem; }
.btn-back:hover { border-color: #6c5ce7; color: #fff; }
.btn-primary { background: #6c5ce7; color: #fff; padding: 0.6rem 1.5rem; border-radius: 8px; border: none; cursor: pointer; font-weight: 600; font-size: 0.95rem; }
.btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
.btn-secondary { background: #2d2d5e; color: #aaa; padding: 0.6rem 1.5rem; border-radius: 8px; border: 1px solid #444; cursor: pointer; font-size: 0.95rem; }
.create-btn { font-size: 1.1rem; padding: 0.75rem 2rem; }

/* ---- Проблемы стека ---- */
.stack-issues { border: 1px solid rgba(231, 76, 60, 0.4); border-radius: 10px; padding: 0.8rem 1rem; margin-bottom: 0.75rem; background: #2a1220; }
.stack-issues p { margin: 0.3rem 0; font-size: 0.8rem; }

/* ---- Рекомендации стека ---- */
.rec-panel { border: 1px solid rgba(108, 92, 231, 0.35); border-radius: 10px; padding: 0.8rem 1rem; margin-bottom: 0.75rem; background: #1a1230; }
.rec-panel .group-label { margin-top: 0; }
.rec-note { font-size: 0.82rem; color: #f39c12; margin: 0.3rem 0 0.5rem; }
.rec-row { display: flex; flex-wrap: wrap; gap: 0.5rem; margin-bottom: 0.5rem; }
.rec-row:last-child { margin-bottom: 0; }
.rec-chip { background: #241a44; border: 1px solid #4a3a85; color: #cdc3f0; padding: 0.35rem 0.7rem; border-radius: 999px; cursor: pointer; font-size: 0.78rem; display: inline-flex; align-items: center; gap: 0.4rem; max-width: 100%; }
.rec-chip:hover { border-color: #6c5ce7; color: #fff; }
.rec-chip.selected { border-color: #6c5ce7; background: #2f2460; color: #fff; }
.rec-chip-note { color: #888; font-size: 0.72rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 260px; }
.rec-error { font-size: 0.8rem; color: #e74c3c; margin: 0.3rem 0; }

/* ---- Summary ---- */
.project-name-section { border: 1px solid #333; border-radius: 10px; padding: 1.25rem; margin-bottom: 1rem; background: #15152e; }
.pn-label { display: block; font-weight: 700; font-size: 1rem; margin-bottom: 0.5rem; color: #ddd; }
.pn-input { width: 100%; padding: 0.65rem 0.8rem; border-radius: 8px; border: 1px solid #444; background: #1a1a2e; color: #fff; font-size: 1rem; box-sizing: border-box; outline: none; }
.pn-input:focus { border-color: #6c5ce7; box-shadow: 0 0 0 2px rgba(108,92,231,0.25); }
.pn-input::placeholder { color: #666; }
.folder-row { display: flex; align-items: center; gap: 0.75rem; margin-top: 0.75rem; flex-wrap: wrap; }
.btn-select-folder { background: #2d2d5e; color: #ccc; padding: 0.5rem 1rem; border-radius: 6px; border: 1px solid #444; cursor: pointer; font-size: 0.85rem; white-space: nowrap; }
.btn-select-folder:hover { border-color: #6c5ce7; color: #fff; }
.folder-path { font-size: 0.8rem; color: #888; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 400px; }
.path-preview { display: flex; align-items: center; gap: 0.5rem; margin-top: 0.6rem; flex-wrap: wrap; }
.pp-label { font-size: 0.8rem; color: #888; }
.pp-path { font-size: 0.85rem; color: #6c5ce7; background: #1a1a2e; padding: 0.2rem 0.5rem; border-radius: 4px; word-break: break-all; }
.pp-checking { font-size: 0.8rem; color: #888; font-style: italic; }
.pp-exists { font-size: 0.8rem; color: #f39c12; font-weight: 600; }
.conflict-overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.6); display: flex; align-items: center; justify-content: center; z-index: 1000; }
.conflict-dialog { background: #1a1a2e; border: 1px solid #6c5ce7; border-radius: 12px; padding: 1.5rem; max-width: 480px; width: 90%; }
.conflict-dialog h3 { margin: 0 0 0.75rem; color: #f39c12; }
.conflict-dialog p { font-size: 0.9rem; color: #aaa; margin: 0 0 1.25rem; line-height: 1.4; }
.conflict-dialog code { color: #6c5ce7; }
.conflict-actions { display: flex; flex-direction: column; gap: 0.6rem; }
.conflict-actions button { width: 100%; text-align: center; }
.secret-row { display: flex; align-items: center; gap: 0.6rem; margin-bottom: 0.6rem; }
.secret-name { flex: 0 0 110px; font-size: 0.85rem; color: #ccc; font-weight: 600; }
.secret-value { flex: 1; font-family: Consolas, monospace; font-size: 0.85rem; background: #11111f; border: 1px solid #333; border-radius: 6px; padding: 0.4rem 0.6rem; color: #6c5ce7; overflow-x: auto; white-space: nowrap; user-select: all; }
.secret-row .btn-secondary { flex: 0 0 auto; }

/* ---- Шаблоны ---- */
.preset-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 0.9rem; }
.preset-card { display: flex; flex-direction: column; align-items: center; gap: 0.5rem; padding: 1.1rem; border: 1px solid #333; border-radius: 12px; background: #1a1a2e; text-align: center; color: #ddd; }
.preset-card:hover { border-color: #6c5ce7; }
.preset-card h3 { margin: 0; font-size: 1rem; }
.preset-desc { font-size: 0.8rem; color: #888; margin: 0; }
.preset-stack { display: flex; flex-wrap: wrap; gap: 0.3rem; justify-content: center; min-height: 1.4rem; }
.preset-chip { font-size: 0.72rem; background: #2d2d5e; color: #ccc; padding: 0.15rem 0.5rem; border-radius: 10px; }
.preset-apply { width: 100%; }

/* ---- Анализ ---- */
.analysis-panel { margin-bottom: 2rem; }
.analysis-summary { font-size: 1rem; font-weight: 600; margin-bottom: 0.5rem; }
.analysis-section { margin: 0.75rem 0; }
.section-title { font-weight: 600; font-size: 0.9rem; color: #aaa; margin-bottom: 0.3rem; }
.tech-tags, .hint-tags { display: flex; flex-wrap: wrap; gap: 0.4rem; }
.tech-tag { padding: 0.2rem 0.5rem; border-radius: 4px; font-size: 0.8rem; background: #2d2d5e; color: #ccc; }
.tech-tag.certaion { background: #1a6b3c; color: #fff; }
.tech-tag.likely { background: #3d3d7e; }
.tech-tag.possible { background: #2d2d3d; }
.hint-tag { padding: 0.2rem 0.5rem; border-radius: 4px; font-size: 0.8rem; background: #2d2d3d; color: #999; }
.hint-tag.docker { background: #1a3d6b; color: #8cf; }
.analyzed-path { font-size: 0.85rem; color: #6c5ce7; margin-top: 0.3rem; }

/* ---- Окружение ---- */
.env-summary { display: flex; flex-wrap: wrap; gap: 0.75rem; align-items: center; padding: 0.75rem 1rem; border: 1px solid #333; border-radius: 10px; background: #15152e; margin-bottom: 1rem; font-size: 0.85rem; color: #ccc; }
.env-warn { color: #f39c12; font-weight: 600; }
.env-list { display: flex; flex-direction: column; gap: 0.4rem; margin-bottom: 1rem; }
.env-row { display: flex; align-items: center; gap: 0.6rem; padding: 0.5rem 0.75rem; border-radius: 6px; background: #1a1a2e; border-left: 3px solid #555; }
.env-row.ok { border-left-color: #00b894; }
.env-row.update { border-left-color: #f39c12; }
.env-row.broken { border-left-color: #e74c3c; }
.env-row.missing { border-left-color: #e74c3c; opacity: 0.8; }
.env-row.manual { border-left-color: #f39c12; }
.env-select { min-width: 22px; display: flex; align-items: center; justify-content: center; cursor: pointer; }
.env-select input { accent-color: #6c5ce7; cursor: pointer; width: 15px; height: 15px; }
.manual-badge { cursor: help; font-size: 0.95rem; }
.env-icon { min-width: 20px; font-size: 0.95rem; }
.env-name { font-weight: 600; font-size: 0.9rem; flex: 0 0 auto; }
.env-source { font-size: 0.75rem; color: #888; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.env-status { font-size: 0.8rem; font-weight: 600; flex: 0 0 auto; }
.env-status.ok { color: #00b894; }
.env-status.update { color: #f39c12; }
.env-status.broken { color: #e74c3c; }
.env-status.missing { color: #e74c3c; }
.env-status.manual { color: #f39c12; }
.env-install { margin-top: 0.5rem; }
.env-progress-list { display: flex; flex-direction: column; gap: 0.35rem; margin-top: 0.75rem; }
.env-progress-row { display: flex; align-items: center; gap: 0.6rem; font-size: 0.85rem; }
.env-progress-row .env-status { margin-left: auto; }
.env-task { display: flex; flex-direction: column; gap: 0.2rem; }
.dl-bar { height: 6px; border-radius: 3px; background: #2a2f3a; overflow: hidden; margin-left: 1.9rem; margin-right: 0.4rem; }
.dl-fill { height: 100%; background: linear-gradient(90deg, #4f8cff, #7c5cff); border-radius: 3px; transition: width 0.3s ease; }
.spin { display: inline-block; width: 0.8rem; height: 0.8rem; border: 2px solid #444; border-top-color: #4f8cff; border-radius: 50%; animation: tc-spin 0.8s linear infinite; vertical-align: -2px; margin-right: 0.3rem; }
@keyframes tc-spin { to { transform: rotate(360deg); } }

/* ---- Исполнение ---- */
.exec-steps { display: flex; flex-direction: column; gap: 0.5rem; margin: 1rem 0; }
.exec-step { display: flex; align-items: flex-start; gap: 0.6rem; padding: 0.5rem; border-radius: 6px; background: #1a1a2e; }
.exec-step.running { border-left: 3px solid #6c5ce7; }
.exec-step.success { border-left: 3px solid #00b894; }
.exec-step.failed { border-left: 3px solid #e74c3c; }
.exec-step.skipped { border-left: 3px solid #888; opacity: 0.6; }
.exec-icon { font-size: 1.1rem; min-width: 24px; }
.exec-detail { flex: 1; min-width: 0; }
.exec-name { font-weight: 600; margin: 0; font-size: 0.9rem; }
.exec-log { font-size: 0.75rem; color: #888; background: #111; padding: 0.3rem; border-radius: 4px; max-height: 80px; overflow-y: auto; margin: 0.3rem 0 0; white-space: pre-wrap; word-break: break-all; }
.exec-full-log { margin: 1rem 0; }
.exec-full-log summary { cursor: pointer; color: #888; font-size: 0.85rem; }
.exec-full-log pre { font-size: 0.75rem; background: #111; padding: 0.5rem; border-radius: 6px; max-height: 200px; overflow-y: auto; white-space: pre-wrap; word-break: break-all; }
.exec-finished { margin: 1rem 0; }
.exec-finished p { margin: 0.3rem 0; }
.exec-finished.error { color: #e74c3c; }
.exec-plan-path { font-size: 0.85rem; color: #888; }

@media (max-width: 900px) {
  .builder { grid-template-columns: 1fr; }
  .builder-context { position: static; }
}
</style>
