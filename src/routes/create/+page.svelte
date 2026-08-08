<script lang="ts">
import { onMount, onDestroy } from "svelte";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  getWizardTree,
  startWizard,
  submitWizardAnswer,
  analyzeProjectTechnologies,
  selectFolder,
  previewProjectRecipe,
  startProjectExecution,
  checkFolderExists,
  getHostPlatform,
  validateProjectStack,
} from "$lib/modules/project_creator/api";
import type {
  WizardTreeData,
  ProjectTypeDef,
  LanguageDef,
  FrameworkDef,
  ToolDef,
  WizardSession,
  AnalysisReport,
  WizardContext,
  ExecutionPlan,
  ExecutionEvent,
  ExecutionEventType,
  StepStatus,
  RecipePreview,
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
} from "$lib/modules/toolchain/api";
import type {
  EnvironmentCheck,
  InstallPlan,
  ToolchainEvent,
  TaskState,
  TaskPhase,
  ProjectRequirements,
  ToolRequirement,
  CheckProgressEvent,
} from "$lib/modules/toolchain/types";
import { statusKind, statusLabel, taskStateKind, taskStateLabel } from "$lib/modules/toolchain/types";

let tree = $state<WizardTreeData | null>(null);
let status = $state<string>("loading");
let session = $state<WizardSession | null>(null);
let hostOs = $state<string>("windows");

let selectedType = $state<ProjectTypeDef | null>(null);
let backendLang = $state<string | null>(null);
let frontendLang = $state<string | null>(null);
let selectedFrameworks = $state<string[]>([]);
let selectedTools = $state<string[]>([]);
let testing = $state(true);
let git = $state(true);
let vscode = $state(true);
let step = $state(0);
const STEP_NAMES = ["Type", "Backend", "Frontend", "Framework", "Tools", "Review", "Environment", "Generate"];

let execPlan = $state<ExecutionPlan | null>(null);
let execStatuses = $state<Map<number, { name: string; status: StepStatus; logs: string[] }>>(new Map());
let execOverallStatus = $state<string>("pending");
let execResult = $state<{ duration: number; status: string } | null>(null);
let execError = $state<string | null>(null);
let execLogs = $state<string[]>([]);
let unlisten: (() => void) | null = null;

// ---- Toolchain Environment (Step 6) ----
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
// Секундомер фазы задачи (чтобы установка не выглядела зависшей)
let envPhaseStart = $state<Map<string, number>>(new Map());
let envTick = $state(0);
let tickTimer: ReturnType<typeof setInterval> | null = null;

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
let newSecrets = $state<Record<string, string> | null>(null);
let secretCopied = $state<string | null>(null);
let unlistenTc: (() => void) | null = null;
let unlistenTcDone: (() => void) | null = null;
let unlistenTcCheck: (() => void) | null = null;
let envCheckProgress = $state<CheckProgressEvent[]>([]);
let envSelectedIds = $state<Set<string>>(new Set());

let analysisMode = $state(false);
let analysisResult = $state<AnalysisReport | null>(null);
let analysisError = $state<string | null>(null);
let analyzing = $state(false);
let analyzedPath = $state<string | null>(null);

// ---- Project name & folder selection (Step 5) ----
let projectName = $state('');
let selectedFolder = $state<string | null>(null);
let folderExists = $state(false);
let folderCheckPending = $state(false);
let showConflictDialog = $state(false);
let conflictResolvedFolder = $state<string | null>(null);
let stackIssues = $state<StackIssue[]>([]);

/** Full path inside which project will be created */
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

async function resolveFolderConflict(action: 'overwrite' | 'auto-rename' | 'cancel') {
  showConflictDialog = false;
  if (action === 'cancel') return;

  if (action === 'overwrite') {
    conflictResolvedFolder = null; // use original projectName as folder
    await goToEnvironment();
    return;
  }

  if (action === 'auto-rename') {
    let counter = 2;
    let testName = `${projectName}-${counter}`;
    while (await checkFolderExists(`${selectedFolder}/${testName}`)) {
      counter++;
      testName = `${projectName}-${counter}`;
    }
    conflictResolvedFolder = testName;
    folderExists = false;
  }
}

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
  if (!analysisResult) return;
  selectedType = tree?.project_types.find((pt) =>
    analysisResult!.project_type_hints.some((h) => pt.label.toLowerCase().includes(h.toLowerCase()))
  ) ?? null;
  const langIds = analysisResult.detected_technologies
    .filter((t) => tree!.languages.some((l) => l.id === t.name.toLowerCase()))
    .map((t) => t.name.toLowerCase());
  // Try to separate into backend/frontend
  const firstLang = tree?.languages.find((l) => l.id === langIds[0]);
  if (firstLang?.category === "frontend" || firstLang?.category === "static") {
    frontendLang = langIds[0];
  } else {
    backendLang = langIds[0];
    if (langIds.length > 1) frontendLang = langIds[1];
  }
  selectedFrameworks = [];
  selectedTools = analysisResult.detected_technologies
    .filter((t) => tree!.tools.some((tl) => tl.id === t.name.toLowerCase() || tl.label === t.name))
    .map((t) => t.name.toLowerCase());
  if (analysisResult.has_docker && !selectedTools.includes("docker")) {
    selectedTools = [...selectedTools, "docker"];
  }
  testing = analysisResult.has_tests;
  git = analysisResult.has_git;
  analysisMode = false;
  step = 0;
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

function isDockerForced(): boolean {
  return selectedTools.some((tid) => {
    const t = tree?.tools.find((x) => x.id === tid);
    return t?.requires_docker ?? false;
  });
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
});

function imgSrc(name: string | null): string {
  if (!name) return "";
  return `/images/${name}`;
}

// Language categories
function category(langId: string): string | undefined {
  return tree?.languages.find((l) => l.id === langId)?.category ?? undefined;
}
function isBackend(langId: string): boolean {
  const c = category(langId);
  return c === "backend" || c === "both";
}
function isFrontend(langId: string): boolean {
  const c = category(langId);
  return c === "frontend" || c === "both";
}

// ---- Step 0: Project Type ----
function pickType(t: ProjectTypeDef) {
  selectedType = t;
  backendLang = null;
  frontendLang = null;
  selectedFrameworks = [];
  selectedTools = [];
  step = 1;
}

// ---- Step 1: Backend Language ----
function pickBackendLang(id: string | null) {
  // If Tauri was previously selected as framework, clear it when backend changes
  if (backendLang !== id && selectedFrameworks.includes("tauri")) {
    selectedFrameworks = selectedFrameworks.filter((f) => f !== "tauri");
  }
  backendLang = id;
}

function confirmBackendLang() {
  // If user picked Rust + Tauri exists for this project type, auto-skip frontend step
  step = 2;
}

// ---- Step 2: Frontend Language ----
function pickFrontendLang(id: string | null) {
  frontendLang = id;
}

function confirmFrontendLang() {
  step = 3;
}

// ---- Step 3: Framework(s) ----
function frameworkIcon(fwId: string): string {
  const fw = tree?.frameworks.find((f) => f.id === fwId);
  return fw?.icon ? imgSrc(fw.icon) : "";
}

function frameworksForSelection(): FrameworkDef[] {
  if (!tree) return [];
  const ids = new Set<string>();
  const langs: string[] = [];
  if (backendLang) langs.push(backendLang);
  if (frontendLang) langs.push(frontendLang);
  for (const langId of langs) {
    const fwIds = tree.language_framework_map[langId] ?? [];
    for (const id of fwIds) ids.add(id);
  }
  return tree.frameworks.filter((f) => ids.has(f.id));
}

/** Язык недоступен на этой ОС? Возвращает причину блокировки или null */
function languageBlockReason(lang: LanguageDef): string | null {
  if (lang.platforms?.length && !lang.platforms.includes(hostOs)) {
    return `Available only on ${lang.platforms.join(", ")}`;
  }
  return null;
}

/**
 * Причина, по которой фреймворк нельзя выбрать (или null):
 * платформа или явный конфликт из wizard_tree.json.
 * Взаимоисключений «только 1 backend / 1 frontend» НЕТ: свобода
 * выбора ограничена только тем, что физически не сможет
 * существовать вместе (конфликты файлов/ролей в wizard_tree).
 */
function frameworkBlockReason(fwId: string): string | null {
  const fw = tree?.frameworks.find((f) => f.id === fwId);
  if (!fw) return null;
  if (fw.platforms?.length && !fw.platforms.includes(hostOs)) {
    return `Available only on ${fw.platforms.join(", ")}`;
  }
  if (selectedFrameworks.includes(fwId)) return null;

  // Конфликты — обе стороны
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

  // Правило: не более одного root- и одного subdir-скаффолдера
  const isRoot = fw.scaffold === "root";
  const isSubdir = fw.scaffold === "subdir";
  const rootSelected = selectedFrameworks.some((id) => tree?.frameworks.find((f) => f.id === id)?.scaffold === "root");
  const subdirSelected = selectedFrameworks.some((id) => tree?.frameworks.find((f) => f.id === id)?.scaffold === "subdir");
  if (isRoot && rootSelected) return "Only one framework can create project in root (tauri, spring-boot, django)";
  if (isSubdir && subdirSelected) return "Only one framework can create project in a subfolder (nextjs, flutter, electron...)";

  return null;
}

function toggleFramework(id: string) {
  if (selectedFrameworks.includes(id)) {
    selectedFrameworks = selectedFrameworks.filter((f) => f !== id);
  } else {
    if (frameworkBlockReason(id)) return;
    // When selecting a conflicting framework, deselect current conflicts first
    const fw = tree?.frameworks.find((f) => f.id === id);
    if (fw?.conflicts) {
      selectedFrameworks = selectedFrameworks.filter((f) => !fw.conflicts!.includes(f));
    }
    selectedFrameworks = [...selectedFrameworks, id];
  }
}

function skipFrameworks() {
  step = 4;
}

function confirmFrameworks() {
  step = 4;
}

// Helper to check if framework selection is non-empty
// (user can skip, but if they made selections we enforce conflicts)
function hasFrameworkSelection(): boolean {
  return selectedFrameworks.length > 0;
}

// ---- Step 4: Tools + Features ----
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

function confirmTools() {
  if (isDockerForced() && !selectedTools.includes("docker")) {
    selectedTools = [...selectedTools, "docker"];
  }
  step = 5;
}

// ---- Step 5: Confirm → Step 6: Execute ----
function allSelectedLangs(): string[] {
  const langs: string[] = [];
  if (backendLang) langs.push(backendLang);
  if (frontendLang) langs.push(frontendLang);
  return langs;
}

async function confirmAll() {
  const path = effectiveProjectPath();
  if (!path) return;

  // Валидация стека на бэкенде (лимиты, платформы, конфликты, языки)
  try {
    const issues = await validateProjectStack(allSelectedLangs(), selectedFrameworks);
    stackIssues = issues;
    if (issues.some((i) => i.severity === "Error")) return;
  } catch {
    // если валидация недоступна — генерацию заблокирует бэкенд
  }

  // Check if folder exists and show conflict dialog if needed
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

// ---- Step 6: Environment check & install ----
function buildRequirements(): ProjectRequirements {
  return {
    languages: allSelectedLangs(),
    frameworks: selectedFrameworks,
    tools: selectedTools,
    git_init: git,
    vscode_config: vscode,
    docker: isDockerForced() || selectedTools.includes("docker"),
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
    console.log("[env] проверка завершена:", fresh?.requirements?.length, "требований");
  } catch (e) {
    console.error("[env] ОШИБКА проверки:", e);
    envError = String(e) || "Неизвестная ошибка при проверке окружения";
  } finally {
    envChecking = false;
  }
}

async function goToEnvironment() {
  step = 6;
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
  // Manual-тулы (движки, SDK) не «не хватает» — они ставятся вручную,
  // в список установки не попадают.
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

function phaseElapsed(taskId: string, tick: number): string {
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

async function doCreateProject(useProjectName: string) {
  const path = effectiveProjectPath();
  if (!path || !selectedFolder) return;

  if (!session) {
    let s = await startWizard();
    s = await submitWizardAnswer(s, "project_type", [selectedType!.id]);
    s = await submitWizardAnswer(s, "languages", allSelectedLangs());
    s = await submitWizardAnswer(s, "frameworks", selectedFrameworks);
    s = await submitWizardAnswer(s, "tools", [...selectedTools, ...(testing ? ["testing"] : [])]);
    s = await submitWizardAnswer(s, "confirm", []);
    session = s;
  }

  const ctx: WizardContext = {
    project_path: null,
    project_name: useProjectName,
    is_existing: false,
    project_type: selectedType?.id ?? null,
    languages: allSelectedLangs(),
    frameworks: selectedFrameworks,
    tools: selectedTools,
    features: [],
    infrastructure: [],
    docker: isDockerForced() || selectedTools.includes("docker"),
    testing,
    ci: false,
    git_init: git,
    vscode_config: vscode,
    answers: {},
  };

  step = 7;
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
  session = null;
  selectedType = null;
  backendLang = null;
  frontendLang = null;
  selectedFrameworks = [];
  selectedTools = [];
  stackIssues = [];
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
  step = 0;
}

function back() {
  if (step > 0) step--;
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

// ---- Compute available backend/frontend languages for current project type ----
function backendLangs(): LanguageDef[] {
  if (!tree || !selectedType) return [];
  return (tree.project_language_map[selectedType.id] ?? [])
    .map((id) => tree!.languages.find((l) => l.id === id)!)
    .filter((l) => l && (l.category === "backend" || l.category === "both"));
}

function frontendLangs(): LanguageDef[] {
  if (!tree || !selectedType) return [];
  return (tree.project_language_map[selectedType.id] ?? [])
    .map((id) => tree!.languages.find((l) => l.id === id)!)
    .filter((l) => l && (l.category === "frontend" || l.category === "both" || l.category === "static"));
}

/** Tauri implies a frontend — if Tauri is selected we can hide frontend lang step */
function hasTauriFramework(): boolean {
  return selectedFrameworks.includes("tauri");
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
      <button class="mode-btn" class:active={!analysisMode} onclick={() => { analysisMode = false; analysisResult = null; }}>New Project</button>
      <button class="mode-btn" class:active={analysisMode} onclick={() => { analysisMode = true; }}>Analyze Existing</button>
    </div>

    {#if analysisMode}
      <div class="analysis-panel">
        <p class="prompt">Analyze an existing project</p>
        <p class="hint">Select a folder to detect technology stack and pre-fill the wizard.</p>
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
              Use detected values in wizard →
            </button>
          </div>
        {/if}
        <button class="btn-back" onclick={() => { analysisMode = false; }}>← Back</button>
      </div>
    {:else}

    <div class="steps">
      {#each STEP_NAMES as name, i}
        <div class="step" class:active={i === step} class:done={i < step}>
          <div class="step-circle">{i < step ? "✓" : i + 1}</div>
          <span class="step-label">{name}</span>
        </div>
      {/each}
    </div>

    <!-- ================================================================
         Step 0: Project Type
         ================================================================ -->
    {#if step === 0}
      <p class="prompt">What are you building?</p>
      <div class="card-grid type-grid">
        {#each tree!.project_types as pt}
          <button class="card" onclick={() => pickType(pt)}>
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

    <!-- ================================================================
         Step 1: Backend Language
         ================================================================ -->
    {#if step === 1}
      <p class="prompt">Pick a backend language</p>
      <p class="hint">
        Choose the primary backend language for your <strong>{selectedType?.label}</strong>.
        You can skip this step for static sites or frontend-only apps.
      </p>
      <div class="card-grid lang-grid">
        {#each backendLangs() as lang}
          {@const blockedReason = languageBlockReason(lang)}
          <button
            class="card"
            class:selected={backendLang === lang.id}
            class:blocked={blockedReason !== null}
            disabled={blockedReason !== null}
            onclick={() => pickBackendLang(lang.id)}
          >
            {#if lang.icon}
              <img src={imgSrc(lang.icon)} alt={lang.label} class="card-img-sm" />
            {:else}
              <span class="card-img-placeholder-sm">▣</span>
            {/if}
            <h3>{lang.label}</h3>
            {#if blockedReason}
              <span class="conflict-badge">{blockedReason}</span>
            {/if}
          </button>
        {/each}
        <!-- Skip option -->
        <button class="card card-skip" onclick={() => pickBackendLang(null)}>
          <span class="card-img-placeholder-sm">—</span>
          <h3>No backend</h3>
          <p>Static site or frontend-only</p>
        </button>
      </div>
      <div class="btn-row">
        <button class="btn-back" onclick={back}>← Back</button>
        <button class="btn-primary" onclick={confirmBackendLang}>
          Next →
        </button>
      </div>
    {/if}

    <!-- ================================================================
         Step 2: Frontend Language
         ================================================================ -->
    {#if step === 2}
      <p class="prompt">Pick a frontend language</p>
      <p class="hint">
        Choose the frontend language for your <strong>{selectedType?.label}</strong>.
        Skip if this is a pure backend project.
      </p>
      <div class="card-grid lang-grid">
        {#each frontendLangs() as lang}
          {@const blockedReason = languageBlockReason(lang)}
          <button
            class="card"
            class:selected={frontendLang === lang.id}
            class:blocked={blockedReason !== null}
            disabled={blockedReason !== null}
            onclick={() => pickFrontendLang(lang.id)}
          >
            {#if lang.icon}
              <img src={imgSrc(lang.icon)} alt={lang.label} class="card-img-sm" />
            {:else}
              <span class="card-img-placeholder-sm">▣</span>
            {/if}
            <h3>{lang.label}</h3>
            {#if lang.category === "static"}
              <p>Plain HTML, CSS & JS — no framework</p>
            {/if}
            {#if blockedReason}
              <span class="conflict-badge">{blockedReason}</span>
            {/if}
          </button>
        {/each}
        <button class="card card-skip" onclick={() => pickFrontendLang(null)}>
          <span class="card-img-placeholder-sm">—</span>
          <h3>No frontend</h3>
          <p>Backend-only or API project</p>
        </button>
      </div>
      <div class="btn-row">
        <button class="btn-back" onclick={back}>← Back</button>
        <button class="btn-primary" onclick={confirmFrontendLang}>
          Next →
        </button>
      </div>
    {/if}

    <!-- ================================================================
         Step 3: Framework(s) — with mutual exclusion + skip
         ================================================================ -->
    {#if step === 3}
      {@const availFrameworks = frameworksForSelection()}
      {@const fwReasons = Object.fromEntries(availFrameworks.map((f) => [f.id, frameworkBlockReason(f.id) ?? ""]))}
      <p class="prompt">Select frameworks</p>
      <p class="hint">
        {#if backendLang && frontendLang}
          Backend: <strong>{backendLang}</strong> &nbsp;|&nbsp; Frontend: <strong>{frontendLang}</strong>
        {:else if backendLang}
          Backend: <strong>{backendLang}</strong>
        {:else if frontendLang}
          Frontend: <strong>{frontendLang}</strong>
        {/if}
        {#if hasTauriFramework()}
          <span class="tauri-note">⚡ Tauri selected — frontend is managed by Tauri CLI</span>
        {/if}
      </p>
      <div class="card-grid fw-grid">
        {#each availFrameworks as fw}
          {@const blocked = fwReasons[fw.id] !== ""}
          <button
            class="card"
            class:selected={selectedFrameworks.includes(fw.id)}
            class:blocked={blocked}
            disabled={blocked}
            onclick={() => toggleFramework(fw.id)}
          >
            {#if fw.icon}
              <img src={frameworkIcon(fw.id)} alt={fw.label} class="card-img-sm" />
            {:else}
              <span class="card-img-placeholder-sm">▣</span>
            {/if}
            <h3>{fw.label}</h3>
            <p>{fw.description}</p>
            {#if blocked}
              <span class="conflict-badge">{fwReasons[fw.id]}</span>
            {/if}
          </button>
        {/each}
      </div>
      {#if availFrameworks.length === 0}
        <p class="muted">No frameworks available for the selected languages.</p>
      {/if}
      <div class="btn-row">
        <button class="btn-back" onclick={back}>← Back</button>
        <button class="btn-secondary" onclick={skipFrameworks}>Skip frameworks</button>
        <button class="btn-primary" onclick={confirmFrameworks}>
          {hasFrameworkSelection() ? "Next →" : "Skip →"}
        </button>
      </div>
    {/if}

    <!-- ================================================================
         Step 4: Tools + Features
         ================================================================ -->
    {#if step === 4}
      <p class="prompt">Tools & Services</p>
      <p class="hint">Add tools, databases, and extra features.</p>

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
          <input type="checkbox" checked={isDockerForced() || selectedTools.includes("docker")} disabled />
          <span>Docker {isDockerForced() ? "(required by tools)" : ""}</span>
        </label>
      </div>

      <div class="btn-row">
        <button class="btn-back" onclick={back}>← Back</button>
        <button class="btn-primary" onclick={confirmTools}>Next →</button>
      </div>
    {/if}

    <!-- ================================================================
         Step 5: Review / Summary
         ================================================================ -->
    {#if step === 5}
      <p class="prompt">Review your project</p>

      <!-- ========== Project Name & Folder ========== -->
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

      <!-- ========== Conflict Dialog ========== -->
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

      <!-- ========== Tech Summary ========== -->
      <div class="summary">
        <div class="summary-row">
          <span class="summary-label">Project Type</span>
          <span>{selectedType?.label}</span>
          <button class="btn-change" onclick={() => { step = 0; }}>change</button>
        </div>
        <div class="summary-row">
          <span class="summary-label">Backend</span>
          <span>{backendLang ?? "None"}</span>
          <button class="btn-change" onclick={() => { step = 1; }}>change</button>
        </div>
        <div class="summary-row">
          <span class="summary-label">Frontend</span>
          <span>{frontendLang ?? "None"}</span>
          <button class="btn-change" onclick={() => { step = 2; }}>change</button>
        </div>
        <div class="summary-row">
          <span class="summary-label">Frameworks</span>
          <span>{selectedFrameworks.length > 0 ? selectedFrameworks.join(", ") : "None"}</span>
          <button class="btn-change" onclick={() => { step = 3; }}>change</button>
        </div>
        <div class="summary-row">
          <span class="summary-label">Tools</span>
          <span>{selectedTools.length > 0 ? selectedTools.join(", ") : "None"}</span>
          <button class="btn-change" onclick={() => { step = 4; }}>change</button>
        </div>
        <div class="summary-row">
          <span class="summary-label">Features</span>
          <span>{[git && "Git Init", testing && "Testing", vscode && "VS Code"].filter(Boolean).join(", ") || "None"}</span>
          <button class="btn-change" onclick={() => { step = 4; }}>change</button>
        </div>
        <div class="summary-row">
          <span class="summary-label">Docker</span>
          <span>{isDockerForced() || selectedTools.includes("docker") ? "Yes" : "No"}</span>
          <button class="btn-change" onclick={() => { step = 4; }}>change</button>
        </div>
      </div>

      {#if stackIssues.length > 0}
        <div class="stack-issues">
          {#each stackIssues as issue}
            <p class={issue.severity === "Error" ? "error" : "env-warn"}>
              {issue.severity === "Error" ? "⛔" : "⚠️"} {issue.message}
            </p>
          {/each}
        </div>
      {/if}

      <div class="btn-row">
        <button class="btn-back" onclick={back}>← Back</button>
        <button
          class="btn-primary create-btn"
          disabled={!projectName || !selectedFolder}
          onclick={confirmAll}
        >
          🚀 Create Project
        </button>
      </div>
    {/if}

    <!-- ================================================================
         Step 6: Environment check & install
         ================================================================ -->
    {#if step === 6}
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
            <span>Установлено: {envPlan.tasks.filter((t) => taskStateKind(t.state) === "success").length}/{envPlan.tasks.length}</span>
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
            <button class="btn-back" onclick={back}>← Back</button>
            <button class="btn-secondary" onclick={recheckEnvironment}>Перепроверить окружение</button>
            <button class="btn-primary" onclick={() => doCreateProject(projectName)}>Продолжить</button>
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
                <span class="env-select manual-badge" title="Устанавливается вручную, автоматической установки нет">⚙️</span>
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
                        Скачивание… {pct}% ({formatMb(Math.floor(dl.received / 1024 / 1024))} / {formatMb(Math.floor(dl.total / 1024 / 1024))})
                      {:else}
                        <span class="spin" aria-hidden="true"></span> {taskStateLabel(st)} · {phaseElapsed(task.task_id, envTick)}
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
          <button class="btn-back" onclick={back} disabled={envInstalling}>← Back</button>
          {#if envInstalling}
            <button class="btn-secondary" onclick={cancelInstall}>Abort</button>
          {:else if missingAll.length > 0}
            <button class="btn-primary" onclick={startInstall} disabled={missingSelected.length === 0}>
              Install selected ({missingSelected.length})
            </button>
            {#if missingSelected.length < missingAll.length}
              <button class="btn-secondary" onclick={selectAllEnvTools}>Select all ({missingAll.length})</button>
            {/if}
            <button class="btn-secondary" onclick={() => doCreateProject(projectName)}>Continue anyway</button>
          {:else}
            <button class="btn-primary" onclick={() => doCreateProject(projectName)}>🚀 Create Project</button>
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
          Не удалось выполнить проверку окружения.
          {envError ? ` (${envError})` : "Попробуйте ещё раз."}
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
         Step 7: Execution
         ================================================================ -->
    {#if step === 7}
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

    {/if}
  {/if}
</div>

<style>
.wizard { max-width: 960px; margin: 0 auto; padding: 2rem; }
.muted { color: #888; }
.error { color: #e74c3c; }
.mode-switch { display: flex; gap: 0; margin-bottom: 1.5rem; border-radius: 8px; overflow: hidden; border: 1px solid #444; width: fit-content; }
.mode-btn { padding: 0.5rem 1.25rem; cursor: pointer; border: none; background: #1a1a2e; color: #aaa; font-size: 0.9rem; }
.mode-btn.active { background: #2d2d5e; color: #fff; font-weight: 600; }
.steps { display: flex; gap: 1.5rem; justify-content: center; margin-bottom: 2rem; flex-wrap: wrap; }
.step { display: flex; align-items: center; gap: 0.4rem; color: #555; font-size: 0.85rem; }
.step.active .step-circle { background: #6c5ce7; color: #fff; }
.step.done .step-circle { background: #00b894; color: #fff; }
.step-circle { width: 28px; height: 28px; border-radius: 50%; display: flex; align-items: center; justify-content: center; background: #2d2d3d; font-weight: 700; font-size: 0.8rem; }
.prompt { font-size: 1.4rem; font-weight: 700; margin-bottom: 0.4rem; }
.hint { color: #888; margin-bottom: 1.5rem; font-size: 0.95rem; }
.card-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: 0.75rem; margin-bottom: 1.5rem; }
.card { display: flex; flex-direction: column; align-items: center; gap: 0.4rem; padding: 1rem; border: 1px solid #333; border-radius: 10px; background: #1a1a2e; cursor: pointer; transition: all 0.15s; text-align: center; color: #ddd; }
.card:hover { border-color: #6c5ce7; background: #22224a; }
.card.selected { border-color: #6c5ce7; background: #2d2d5e; box-shadow: 0 0 0 2px #6c5ce7; }
.card.blocked { opacity: 0.35; cursor: not-allowed; border-color: #333; background: #15152e; }
.stack-issues { border: 1px solid rgba(231, 76, 60, 0.4); border-radius: 10px; padding: 0.8rem 1rem; margin-bottom: 1rem; background: #2a1220; }
.stack-issues p { margin: 0.3rem 0; font-size: 0.85rem; }
.card.card-skip { border-style: dashed; border-color: #555; }
.card-img { width: 56px; height: 56px; object-fit: contain; }
.card-img-sm { width: 40px; height: 40px; object-fit: contain; }
.card-img-xs { width: 24px; height: 24px; object-fit: contain; }
.card-img-placeholder { font-size: 2rem; }
.card-img-placeholder-sm { font-size: 1.5rem; }
.card-img-placeholder-xs { font-size: 1rem; }
.card h3 { margin: 0; font-size: 0.95rem; }
.card p { margin: 0; font-size: 0.78rem; color: #888; }
.type-grid { grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); }
.lang-grid { grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)); }
.fw-grid { grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); }
.conflict-badge { display: block; font-size: 0.7rem; color: #e74c3c; margin-top: 0.25rem; }
.tauri-note { display: block; margin-top: 0.5rem; font-size: 0.85rem; color: #6c5ce7; }
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
.btn-row { display: flex; gap: 0.75rem; margin-top: 1.5rem; flex-wrap: wrap; }
.btn-back, .btn-change { background: none; border: 1px solid #444; color: #888; padding: 0.4rem 0.9rem; border-radius: 6px; cursor: pointer; font-size: 0.85rem; }
.btn-back:hover, .btn-change:hover { border-color: #6c5ce7; color: #fff; }
.btn-primary { background: #6c5ce7; color: #fff; padding: 0.6rem 1.5rem; border-radius: 8px; border: none; cursor: pointer; font-weight: 600; font-size: 0.95rem; }
.btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
.btn-secondary { background: #2d2d5e; color: #aaa; padding: 0.6rem 1.5rem; border-radius: 8px; border: 1px solid #444; cursor: pointer; font-size: 0.95rem; }
.summary { border: 1px solid #333; border-radius: 10px; padding: 1rem; margin-bottom: 1rem; }
.summary-row { display: flex; align-items: center; gap: 0.75rem; padding: 0.5rem 0; border-bottom: 1px solid #222; }
.summary-row:last-child { border-bottom: none; }
.summary-label { min-width: 100px; font-weight: 600; color: #888; font-size: 0.85rem; }
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
.create-btn { font-size: 1.1rem; padding: 0.75rem 2rem; }
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
.env-progress-list { display: flex; flex-direction: column; gap: 0.35rem; margin-top: 0.75rem; }
.env-progress-row { display: flex; align-items: center; gap: 0.6rem; font-size: 0.85rem; }
.env-progress-row .env-status { margin-left: auto; }
.env-task { display: flex; flex-direction: column; gap: 0.2rem; }
.dl-bar { height: 6px; border-radius: 3px; background: #2a2f3a; overflow: hidden; margin-left: 1.9rem; margin-right: 0.4rem; }
.dl-fill { height: 100%; background: linear-gradient(90deg, #4f8cff, #7c5cff); border-radius: 3px; transition: width 0.3s ease; }
.spin { display: inline-block; width: 0.8rem; height: 0.8rem; border: 2px solid #444; border-top-color: #4f8cff; border-radius: 50%; animation: tc-spin 0.8s linear infinite; vertical-align: -2px; margin-right: 0.3rem; }
@keyframes tc-spin { to { transform: rotate(360deg); } }
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
</style>