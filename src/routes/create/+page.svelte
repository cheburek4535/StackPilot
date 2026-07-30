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
} from "$lib/modules/project_creator/types";

let tree = $state<WizardTreeData | null>(null);
let status = $state<string>("loading");
let session = $state<WizardSession | null>(null);

let selectedType = $state<ProjectTypeDef | null>(null);
let backendLang = $state<string | null>(null);
let frontendLang = $state<string | null>(null);
let selectedFrameworks = $state<string[]>([]);
let selectedTools = $state<string[]>([]);
let testing = $state(true);
let git = $state(true);
let vscode = $state(true);
let step = $state(0);
const STEP_NAMES = ["Type", "Backend", "Frontend", "Framework", "Tools", "Review", "Generate"];

let execPlan = $state<ExecutionPlan | null>(null);
let execStatuses = $state<Map<number, { name: string; status: StepStatus; logs: string[] }>>(new Map());
let execOverallStatus = $state<string>("pending");
let execResult = $state<{ duration: number; status: string } | null>(null);
let execError = $state<string | null>(null);
let execLogs = $state<string[]>([]);
let unlisten: (() => void) | null = null;

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
    await doCreateProject(projectName);
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
});

onDestroy(() => {
  if (unlisten) unlisten();
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

/** Frameworks that are blocked because a conflicting framework is already selected */
function conflictedFrameworkIds(): Set<string> {
  const conflicted = new Set<string>();
  for (const fwId of selectedFrameworks) {
    const fw = tree?.frameworks.find((f) => f.id === fwId);
    if (fw?.conflicts) {
      for (const c of fw.conflicts) conflicted.add(c);
    }
  }
  // Also check reverse: if a not-selected framework conflicts with a selected one
  for (const fw of tree?.frameworks ?? []) {
    if (selectedFrameworks.includes(fw.id)) continue;
    for (const c of (fw.conflicts ?? [])) {
      if (selectedFrameworks.includes(c)) {
        conflicted.add(fw.id);
      }
    }
  }
  return conflicted;
}

function toggleFramework(id: string) {
  if (selectedFrameworks.includes(id)) {
    selectedFrameworks = selectedFrameworks.filter((f) => f !== id);
  } else {
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

  await doCreateProject(projectName);
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

  step = 6;
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
  testing = true;
  git = true;
  vscode = true;
  tooltipData = null;
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
          <button
            class="card"
            class:selected={backendLang === lang.id}
            onclick={() => pickBackendLang(lang.id)}
          >
            {#if lang.icon}
              <img src={imgSrc(lang.icon)} alt={lang.label} class="card-img-sm" />
            {:else}
              <span class="card-img-placeholder-sm">▣</span>
            {/if}
            <h3>{lang.label}</h3>
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
          <button
            class="card"
            class:selected={frontendLang === lang.id}
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
      {@const conflicted = conflictedFrameworkIds()}
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
          {@const blocked = conflicted.has(fw.id)}
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
              <span class="conflict-badge">Incompatible with current selection</span>
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
         Step 6: Execution
         ================================================================ -->
    {#if step === 6}
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