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

  // Local selections for each step
  let selectedType = $state<ProjectTypeDef | null>(null);
  let selectedLangs = $state<string[]>([]);
  let selectedFrameworks = $state<string[]>([]);
  let selectedTools = $state<string[]>([]);
  let testing = $state(true);
  let git = $state(true);
  let vscode = $state(true);
  let step = $state(0);
  const STEP_NAMES = ["Type", "Language", "Framework", "Tools", "Review", "Generate"];

  // Execution state
  let execPlan = $state<ExecutionPlan | null>(null);
  let execStatuses = $state<Map<number, { name: string; status: StepStatus; logs: string[] }>>(new Map());
  let execOverallStatus = $state<string>("pending");
  let execResult = $state<{ duration: number; status: string } | null>(null);
  let execError = $state<string | null>(null);
  let execLogs = $state<string[]>([]);
  let unlisten: (() => void) | null = null;

  // Analysis state (existing project)
  let analysisMode = $state(false);
  let analysisResult = $state<AnalysisReport | null>(null);
  let analysisError = $state<string | null>(null);
  let analyzing = $state(false);
  let analyzedPath = $state<string | null>(null);

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
    selectedLangs = analysisResult.detected_technologies
      .filter((t) => tree!.languages.some((l) => l.id === t.name.toLowerCase()))
      .map((t) => t.name.toLowerCase());
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

  // Tooltip state
  let tooltipData = $state<{ x: number; y: number; tool: ToolDef } | null>(null);
  let tooltipTimer: ReturnType<typeof setTimeout> | null = null;

  function showTooltip(tool: ToolDef, e: MouseEvent) {
    if (tooltipTimer) clearTimeout(tooltipTimer);
    tooltipTimer = setTimeout(() => {
      tooltipData = { x: e.clientX, y: e.clientY, tool };
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

  // ---- Step 0: Project Type ----
  function pickType(t: ProjectTypeDef) {
    selectedType = t;
    selectedLangs = [];
    selectedFrameworks = [];
    selectedTools = [];
    step = 1;
  }

  // ---- Step 1: Language(s) ----
  function langIcon(langId: string): string {
    const lang = tree?.languages.find((l) => l.id === langId);
    return lang?.icon ? imgSrc(lang.icon) : "";
  }

  function toggleLang(id: string) {
    if (selectedLangs.includes(id)) {
      selectedLangs = selectedLangs.filter((l) => l !== id);
    } else {
      selectedLangs = [...selectedLangs, id];
    }
  }

  function confirmLangs() {
    if (selectedLangs.length === 0) return;
    step = 2;
  }

  // ---- Step 2: Framework(s) ----
  function frameworkIcon(fwId: string): string {
    const fw = tree?.frameworks.find((f) => f.id === fwId);
    return fw?.icon ? imgSrc(fw.icon) : "";
  }

  function availableFrameworks(): FrameworkDef[] {
    if (!tree) return [];
    const ids = new Set<string>();
    for (const langId of selectedLangs) {
      const fwIds = tree.language_framework_map[langId] ?? [];
      for (const id of fwIds) ids.add(id);
    }
    return tree.frameworks.filter((f) => ids.has(f.id));
  }

  function toggleFramework(id: string) {
    if (selectedFrameworks.includes(id)) {
      selectedFrameworks = selectedFrameworks.filter((f) => f !== id);
    } else {
      selectedFrameworks = [...selectedFrameworks, id];
    }
  }

  function confirmFrameworks() {
    step = 3;
  }

  // ---- Step 3: Tools + Features ----
  function availableTools(): ToolDef[] {
    if (!tree) return [];
    const ids = new Set<string>();
    for (const fwId of selectedFrameworks) {
      const toolIds = tree.framework_tool_map[fwId] ?? [];
      for (const id of toolIds) ids.add(id);
    }
    return tree.tools.filter((t) => ids.has(t.id));
  }

  function toggleTool(id: string) {
    const tool = tree?.tools.find((t) => t.id === id);
    if (!tool) return;

    if (selectedTools.includes(id)) {
      // Remove dependents that require this tool
      const dependents = tree!.tools.filter((t) => t.requires.includes(id));
      const toRemove = new Set([id, ...dependents.map((d) => d.id)]);
      selectedTools = selectedTools.filter((t) => !toRemove.has(t));
    } else {
      // Resolve conflicts — deselect conflicting tools on either side
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
      // Select tool + auto-select its requires
      const toAdd = new Set([id]);
      for (const reqId of tool.requires) {
        if (!selectedTools.includes(reqId)) {
          toAdd.add(reqId);
        }
      }
      // Auto-select Docker if this tool requires it
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
    step = 4;
  }

  // ---- Step 4: Confirm → Step 5: Execute ----
  async function confirmAll() {
    if (!session) {
      let s = await startWizard();
      s = await submitWizardAnswer(s, "project_type", [selectedType!.id]);
      s = await submitWizardAnswer(s, "languages", selectedLangs);
      s = await submitWizardAnswer(s, "frameworks", selectedFrameworks);
      s = await submitWizardAnswer(s, "tools", [...selectedTools, ...(testing ? ["testing"] : [])]);
      s = await submitWizardAnswer(s, "confirm", []);
      session = s;
    }

    // Build WizardContext from selections
    const ctx: WizardContext = {
      project_path: null,
      is_existing: false,
      project_type: selectedType?.id ?? null,
      languages: selectedLangs,
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

    // Pick project folder
    const folder = await selectFolder();
    if (!folder) return;
    const projectPath = `${folder}/${selectedType?.id ?? "project"}`;

    step = 5;
    execPlan = null;
    execStatuses = new Map();
    execLogs = [];
    execOverallStatus = "running";
    execResult = null;
    execError = null;

    // Listen for execution events
    if (unlisten) unlisten();
    unlisten = await listen<ExecutionEvent>("project_creator:step_event", (e) => {
      handleExecEvent(e.payload);
    });

    try {
      execPlan = await startProjectExecution(ctx, projectPath);
    } catch (err) {
      execError = String(err);
      execOverallStatus = "error";
    }
  }

  function stepIsRunning(st: StepStatus | undefined) { return st === "Running"; }
  function stepIsSuccess(st: StepStatus | undefined) { return !!st && (st === "Success" || (typeof st === "object" && "Success" in st)); }
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
    // serde externally-tagged: unit variants are strings, struct variants are objects
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
    // TBD: send cancellation signal
    execOverallStatus = "cancelled";
  }

  function resetAll() {
    session = null;
    selectedType = null;
    selectedLangs = [];
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

    <!-- Analysis mode toggle -->
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

    <!-- Steps indicator -->
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
         Step 1: Language(s)
         ================================================================ -->
    {#if step === 1}
      <p class="prompt">Which language?</p>
      <p class="hint">
        Choose one or more languages for your
        <strong>{selectedType?.label}</strong>
      </p>
      <div class="card-grid lang-grid">
        {#each (tree!.project_language_map[selectedType!.id] ?? []) as langId}
          {@const lang = tree!.languages.find((l) => l.id === langId)}
          {#if lang}
            <button
              class="card"
              class:selected={selectedLangs.includes(lang.id)}
              onclick={() => toggleLang(lang.id)}
            >
              {#if lang.icon}
                <img src={langIcon(lang.id)} alt={lang.label} class="card-img-sm" />
              {:else}
                <span class="card-img-placeholder-sm">◈</span>
              {/if}
              <h3>{lang.label}</h3>
            </button>
          {/if}
        {/each}
      </div>
      <div class="actions">
        <button class="btn-back" onclick={back}>← Back</button>
        <button class="btn-primary" onclick={confirmLangs} disabled={selectedLangs.length === 0}>
          Continue →
        </button>
      </div>
    {/if}

    <!-- ================================================================
         Step 2: Framework(s)
         ================================================================ -->
    {#if step === 2}
      <p class="prompt">Which framework?</p>
      <p class="hint">
        for {#each selectedLangs as lid, i}
          <strong>{tree!.languages.find((l) => l.id === lid)?.label ?? lid}</strong>{i < selectedLangs.length - 1 ? ", " : ""}
        {/each}
      </p>
      <div class="card-grid fw-grid">
        {#each availableFrameworks() as fw}
          <button
            class="card"
            class:selected={selectedFrameworks.includes(fw.id)}
            onclick={() => toggleFramework(fw.id)}
          >
            {#if fw.icon}
              <img src={frameworkIcon(fw.id)} alt={fw.label} class="card-img-sm" />
            {:else}
              <span class="card-img-placeholder-sm">◆</span>
            {/if}
            <h3>{fw.label}</h3>
            <p>{fw.description}</p>
          </button>
        {/each}
      </div>
      <div class="actions">
        <button class="btn-back" onclick={back}>← Back</button>
        <button class="btn-primary" onclick={confirmFrameworks}>
          Continue →
        </button>
      </div>
    {/if}

    <!-- ================================================================
         Step 3: Tools + Features
         ================================================================ -->
    {#if step === 3}
      <p class="prompt">Configure your stack</p>

      <!-- Tools -->
      {#if availableTools().length > 0}
        <p class="section-title">Tools</p>
        <div class="tools-grid">
          {#each availableTools() as tool}
            {@const dockerForced = isDockerForced()}
            {@const isDockerTool = tool.id === "docker"}
            <label
              class="tool-row"
              class:checked={selectedTools.includes(tool.id)}
              class:forced={isDockerTool && dockerForced}
              onmouseenter={(e) => showTooltip(tool, e)}
              onmouseleave={hideTooltip}
            >
              <input
                type="checkbox"
                checked={selectedTools.includes(tool.id)}
                onchange={() => toggleTool(tool.id)}
                disabled={isDockerTool && dockerForced}
              />
              {#if tool.icon}
                <img src={imgSrc(tool.icon)} alt={tool.label} class="tool-icon-img" />
              {:else}
                <span class="tool-icon-placeholder">{toolCategoryIcon(tool.category)}</span>
              {/if}
              <div class="tool-info">
                <span class="tool-name">{tool.label}</span>
                <span class="tool-desc">{tool.description}</span>
              </div>
              <span class="tool-cat">{tool.category}</span>
            </label>
          {/each}
        </div>
      {/if}

      <!-- Features -->
      <p class="section-title">Environment</p>
      <div class="features-list">
        <label class="feature-row" class:checked={testing}>
          <input type="checkbox" bind:checked={testing} />
          <span class="feature-info">
            <span class="feature-name">Testing</span>
            <span class="feature-desc">Test framework configuration</span>
          </span>
        </label>
        <label class="feature-row" class:checked={git}>
          <input type="checkbox" bind:checked={git} />
          <span class="feature-info">
            <span class="feature-name">Git Init</span>
            <span class="feature-desc">Repository + .gitignore</span>
          </span>
        </label>
        <label class="feature-row" class:checked={vscode}>
          <input type="checkbox" bind:checked={vscode} />
          <span class="feature-info">
            <span class="feature-name">VS Code Config</span>
            <span class="feature-desc">Editor settings and extensions</span>
          </span>
        </label>
      </div>

      <div class="actions">
        <button class="btn-back" onclick={back}>← Back</button>
        <button class="btn-primary" onclick={confirmTools}>Review →</button>
      </div>

      <!-- Tooltip overlay -->
      {#if tooltipData}
        <div class="tooltip" style="left: {tooltipData.x + 14}px; top: {tooltipData.y - 10}px;">
          <div class="tooltip-title">{tooltipData.tool.label}</div>
          <div class="tooltip-desc">{tooltipData.tool.description}</div>
          {#if tooltipData.tool.requires_docker}
            <div class="tooltip-line warn">Requires Docker</div>
          {/if}
          {#if tooltipData.tool.requires.length > 0}
            <div class="tooltip-line">Requires: {tooltipData.tool.requires.join(", ")}</div>
          {/if}
          {#if tooltipData.tool.conflicts.length > 0}
            <div class="tooltip-line conflict">Conflicts: {tooltipData.tool.conflicts.join(", ")}</div>
          {/if}
        </div>
      {/if}
    {/if}

    <!-- ================================================================
         Step 4: Summary + Confirm
         ================================================================ -->
    {#if step === 4 && !session}
      <p class="prompt">Review your project</p>
      <div class="summary">
        <div class="summary-row">
          <span class="summary-label">Type</span>
          <span class="summary-value">{selectedType?.label}</span>
          <button class="btn-edit" onclick={() => { step = 0; selectedType = null; }}>change</button>
        </div>
        <div class="summary-row">
          <span class="summary-label">Language(s)</span>
          <span class="summary-value">{selectedLangs.map((l) => tree!.languages.find((x) => x.id === l)?.label ?? l).join(", ")}</span>
          <button class="btn-edit" onclick={() => { step = 1; }}>change</button>
        </div>
        <div class="summary-row">
          <span class="summary-label">Framework(s)</span>
          <span class="summary-value">{selectedFrameworks.map((f) => tree!.frameworks.find((x) => x.id === f)?.label ?? f).join(", ")}</span>
          <button class="btn-edit" onclick={() => { step = 2; }}>change</button>
        </div>
        <div class="summary-row">
          <span class="summary-label">Tools</span>
          <span class="summary-value">{selectedTools.length > 0 ? selectedTools.join(", ") : "none"}</span>
          <button class="btn-edit" onclick={() => { step = 3; }}>change</button>
        </div>
        <div class="summary-row">
          <span class="summary-label">Features</span>
          <span class="summary-value">{[testing && "Testing", git && "Git", vscode && "VS Code"].filter(Boolean).join(", ") || "none"}</span>
          <button class="btn-edit" onclick={() => { step = 3; }}>change</button>
        </div>
      </div>
      <div class="actions">
        <button class="btn-back" onclick={back}>← Back</button>
        <button class="btn-primary" onclick={confirmAll}>Create Project ✓</button>
      </div>
    {/if}

    <!-- ================================================================
         Step 5: Execution
         ================================================================ -->
    {#if step === 5}
      <p class="prompt">Generating project...</p>

      {#if execPlan}
        <p class="hint">Project path: {execPlan.project_path}</p>
      {/if}

      <!-- Step list with statuses -->
      <div class="exec-steps">
        {#each execPlan?.steps ?? [] as s, i}
          {@const entry = execStatuses.get(i)}
          {@const st = entry?.status}
          <div class="exec-step" class:active={st === "Running"}
                                 class:done={stepIsSuccess(st)}
                                 class:failed={stepIsFailed(st)}
                                 class:skipped={stepIsSkipped(st)}>
            <span class="exec-icon">
              {#if st === "Running"}
                ⏳
              {:else if stepIsSuccess(st)}
                ✅
              {:else if stepIsFailed(st)}
                ❌
              {:else if stepIsSkipped(st)}
                ⏭️
              {:else}
                ⏺️
              {/if}
            </span>
            <span class="exec-name">{s.label ?? s.id}</span>
            {#if entry && entry.logs.length > 0}
              <span class="exec-log-preview">{entry.logs[entry.logs.length - 1]}</span>
            {/if}
          </div>
        {/each}
      </div>

      <!-- Live log -->
      {#if execLogs.length > 0}
        <p class="section-title">Log</p>
        <div class="exec-log">
          {#each execLogs as line}
            <span>{line}</span>
          {/each}
        </div>
      {/if}

      <!-- Cancellation / progress indicator -->
      {#if execOverallStatus === "running"}
        <div class="actions">
          <button class="btn-back" onclick={cancelExecution}>Cancel</button>
        </div>
      {:else if execOverallStatus === "done" || execOverallStatus === "error"}
        <div class="done">
          <span class="done-icon">{execOverallStatus === "done" ? "✅" : "❌"}</span>
          <h2>{execOverallStatus === "done" ? "Project created!" : "Execution failed"}</h2>
          {#if execResult}
            <p class="muted">Completed in {(execResult.duration / 1000).toFixed(1)}s</p>
          {/if}
          {#if execError}
            <p class="error">{execError}</p>
          {/if}
          <div class="actions" style="justify-content: center; margin-top: 1rem;">
            {#if execOverallStatus === "done"}
              <button class="btn-primary" onclick={openInVSCode}>Open in VS Code</button>
            {/if}
            <button class="btn-back" onclick={resetAll}>Create another project</button>
          </div>
        </div>
      {/if}
    {/if}

    {/if}
  {/if}
</div>

<style>
  .wizard {
    max-width: 860px;
    margin: 2rem auto;
    padding: 0 1.5rem;
  }
  h1 { margin-bottom: 0.25rem; }
  .prompt {
    font-size: 1.3rem;
    font-weight: 700;
    margin: 1.5rem 0 0.25rem;
  }
  .hint {
    color: #777;
    margin: 0 0 1.25rem;
    font-size: 0.9rem;
  }
  .section-title {
    font-weight: 600;
    font-size: 0.95rem;
    margin: 1.25rem 0 0.5rem;
    color: #555;
  }
  .muted { color: #888; }
  .error { color: #c62828; }

  /* ============================================================
     Mode Switch
     ============================================================ */
  .mode-switch {
    display: flex;
    gap: 0.5rem;
    justify-content: center;
    margin: 0 0 1.5rem;
  }
  .mode-btn {
    padding: 0.5rem 1.25rem;
    border: 2px solid #ccc;
    border-radius: 8px;
    background: transparent;
    cursor: pointer;
    font-size: 0.9rem;
    font-weight: 600;
    color: #666;
    transition: border-color 0.15s, background 0.15s, color 0.15s;
  }
  .mode-btn.active {
    border-color: #283593;
    background: #e8eaf6;
    color: #283593;
  }
  .mode-btn:hover { border-color: #283593; color: #283593; }

  /* ============================================================
     Analysis Panel
     ============================================================ */
  .analysis-panel {
    max-width: 560px;
    margin: 0 auto;
  }
  .analyzed-path {
    font-size: 0.85rem;
    color: #666;
    margin: 0.5rem 0;
  }
  .analysis-result {
    background: #f8f9fa;
    border-radius: 12px;
    padding: 1rem 1.25rem;
    margin: 1rem 0;
  }
  .analysis-summary {
    font-weight: 600;
    margin: 0 0 0.75rem;
  }
  .analysis-section {
    margin: 0.75rem 0;
  }
  .tech-tags, .hint-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }
  .tech-tag, .hint-tag {
    font-size: 0.8rem;
    padding: 0.2rem 0.6rem;
    border-radius: 6px;
    background: #e0e0e0;
    color: #444;
  }
  .tech-tag.certain { background: #c8e6c9; color: #1b5e20; }
  .tech-tag.likely { background: #bbdefb; color: #0d47a1; }
  .tech-tag.possible { background: #fff9c4; color: #f57f17; }
  .hint-tag.docker { background: #e3f2fd; color: #1565c0; }

  /* ============================================================
     Steps Indicator
     ============================================================ */
  .steps {
    display: flex;
    gap: 0;
    margin: 1.5rem 0 2rem;
    justify-content: center;
    background: #f8f9fa;
    border-radius: 12px;
    padding: 0.75rem 1rem;
  }
  .step {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0 1.25rem;
  }
  .step-circle {
    width: 28px; height: 28px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 0.8rem;
    font-weight: 700;
    background: #e0e0e0;
    color: #888;
    transition: background 0.2s, color 0.2s;
  }
  .step.active .step-circle { background: #283593; color: #fff; }
  .step.done .step-circle { background: #2e7d32; color: #fff; }
  .step-label { font-size: 0.85rem; color: #aaa; }
  .step.active .step-label { color: #283593; font-weight: 600; }
  .step.done .step-label { color: #2e7d32; }

  /* ============================================================
     Card Grid (shared)
     ============================================================ */
  .card-grid {
    display: grid;
    gap: 1rem;
  }
  .type-grid {
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  }
  .lang-grid {
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  }
  .fw-grid {
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  }

  .card {
    background: #fff;
    border: 2px solid #e0e0e0;
    border-radius: 12px;
    padding: 1.25rem 1rem;
    cursor: pointer;
    transition: border-color 0.15s, box-shadow 0.15s, transform 0.1s;
    font-family: inherit;
    font-size: inherit;
    color: inherit;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.3rem;
  }
  .card:hover {
    border-color: #283593;
    box-shadow: 0 3px 12px rgba(40, 53, 147, 0.12);
    transform: translateY(-2px);
  }
  .card.selected {
    border-color: #283593;
    background: #e8eaf6;
  }
  .card h3 {
    margin: 0.2rem 0 0.25rem;
    font-size: 0.95rem;
  }
  .card p {
    margin: 0;
    font-size: 0.78rem;
    color: #666;
    line-height: 1.4;
  }

  /* Images — единый размер без искажений */
  .card-img {
    width: 56px;
    height: 56px;
    object-fit: contain;
    pointer-events: none;
  }
  .card-img-sm {
    width: 40px;
    height: 40px;
    object-fit: contain;
    pointer-events: none;
  }
  .card-img-placeholder {
    width: 56px;
    height: 56px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 2rem;
    color: #bbb;
  }
  .card-img-placeholder-sm {
    width: 40px;
    height: 40px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 1.4rem;
    color: #bbb;
  }

  /* ============================================================
     Tools + Features (Step 3)
     ============================================================ */
  .tools-grid {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    max-width: 520px;
  }
  .tool-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.6rem 0.85rem;
    border: 2px solid #e0e0e0;
    border-radius: 10px;
    cursor: pointer;
    transition: border-color 0.15s, background 0.15s;
  }
  .tool-row.checked {
    border-color: #283593;
    background: #e8eaf6;
  }
  .tool-row input[type="checkbox"] {
    width: 16px; height: 16px;
    accent-color: #283593;
  }
  .tool-icon { font-size: 1.2rem; }
  .tool-info { display: flex; flex-direction: column; flex: 1; }
  .tool-name { font-weight: 600; font-size: 0.9rem; }
  .tool-desc { font-size: 0.78rem; color: #888; }
  .tool-cat {
    font-size: 0.7rem;
    background: #eee;
    padding: 0.15rem 0.5rem;
    border-radius: 4px;
    color: #666;
    text-transform: uppercase;
  }

  .tool-icon-img {
    width: 22px; height: 22px;
    object-fit: contain;
  }
  .tool-icon-placeholder {
    font-size: 1.1rem;
    width: 22px; height: 22px;
    display: flex; align-items: center; justify-content: center;
  }
  .tool-row.forced {
    border-color: #f9a825;
    background: #fff8e1;
  }

  /* ============================================================
     Tooltip
     ============================================================ */
  .tooltip {
    position: fixed;
    z-index: 9999;
    background: #1e1e1e;
    color: #e0e0e0;
    padding: 0.6rem 0.85rem;
    border-radius: 8px;
    font-size: 0.8rem;
    max-width: 280px;
    pointer-events: none;
    box-shadow: 0 4px 16px rgba(0,0,0,0.25);
    line-height: 1.5;
  }
  .tooltip-title { font-weight: 700; font-size: 0.9rem; margin-bottom: 0.15rem; color: #fff; }
  .tooltip-desc { color: #bbb; margin-bottom: 0.3rem; }
  .tooltip-line { color: #ccc; font-size: 0.75rem; }
  .tooltip-line.warn { color: #ffb74d; }
  .tooltip-line.conflict { color: #ef9a9a; }

  .features-list {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    max-width: 520px;
  }
  .feature-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.6rem 0.85rem;
    border: 2px solid #e0e0e0;
    border-radius: 10px;
    cursor: pointer;
    transition: border-color 0.15s, background 0.15s;
  }
  .feature-row.checked {
    border-color: #283593;
    background: #e8eaf6;
  }
  .feature-row input[type="checkbox"] {
    width: 16px; height: 16px;
    accent-color: #283593;
  }
  .feature-info { display: flex; flex-direction: column; }
  .feature-name { font-weight: 600; font-size: 0.9rem; }
  .feature-desc { font-size: 0.78rem; color: #888; }

  /* ============================================================
     Summary (Step 4)
     ============================================================ */
  .summary {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    max-width: 560px;
    margin: 1.5rem 0;
  }
  .summary-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.7rem 1rem;
    background: #f8f9fa;
    border-radius: 8px;
  }
  .summary-label { font-weight: 600; min-width: 90px; font-size: 0.85rem; color: #555; }
  .summary-value { flex: 1; font-size: 0.9rem; }
  .btn-edit {
    background: none; border: none;
    color: #283593; text-decoration: underline;
    cursor: pointer; font-size: 0.8rem;
    padding: 0.2rem 0.4rem;
  }
  .btn-edit:hover { color: #1a237e; }

  /* ============================================================
     Buttons
     ============================================================ */
  .actions {
    display: flex;
    gap: 0.75rem;
    margin-top: 1.5rem;
  }
  .btn-primary {
    background: #283593; color: #fff;
    border: none; padding: 0.6rem 1.75rem;
    border-radius: 8px; font-size: 0.95rem;
    font-weight: 600; cursor: pointer;
    transition: background 0.15s;
  }
  .btn-primary:hover { background: #1a237e; }
  .btn-primary:disabled { opacity: 0.4; cursor: default; }

  .btn-back {
    background: none; border: 1px solid #ccc;
    padding: 0.6rem 1.25rem;
    border-radius: 8px; font-size: 0.9rem;
    cursor: pointer; color: #666;
    transition: border-color 0.15s, color 0.15s;
  }
  .btn-back:hover { border-color: #283593; color: #283593; }

  /* ============================================================
     Done
     ============================================================ */
  .done {
    text-align: center;
    padding: 3rem 1rem;
  }
  .done-icon {
    font-size: 3rem;
    background: #2e7d32;
    color: #fff;
    width: 64px; height: 64px;
    border-radius: 50%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    margin-bottom: 1rem;
  }
  .context-dump {
    background: #f0f0f0;
    padding: 1rem;
    border-radius: 8px;
    text-align: left;
    font-size: 0.75rem;
    max-width: 480px;
    margin: 1rem auto;
    overflow-x: auto;
  }

  /* ============================================================
     Execution (Step 5)
     ============================================================ */
  .exec-steps {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    max-width: 560px;
    margin: 1rem 0;
  }
  .exec-step {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.45rem 0.7rem;
    background: #f8f9fa;
    border-radius: 6px;
    font-size: 0.85rem;
    border-left: 3px solid #ccc;
    transition: background 0.2s, border-color 0.2s;
  }
  .exec-step.active {
    border-left-color: #1976d2;
    background: #e3f2fd;
  }
  .exec-step.done {
    border-left-color: #2e7d32;
  }
  .exec-step.failed {
    border-left-color: #c62828;
    background: #ffebee;
  }
  .exec-step.skipped {
    border-left-color: #888;
    opacity: 0.6;
  }
  .exec-icon { font-size: 1rem; width: 1.2rem; text-align: center; }
  .exec-name { font-weight: 600; }
  .exec-log-preview {
    font-size: 0.75rem;
    color: #888;
    margin-left: auto;
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .exec-log {
    background: #1a1a2e;
    color: #a0ffa0;
    font-family: monospace;
    font-size: 0.78rem;
    padding: 0.75rem;
    border-radius: 8px;
    max-height: 240px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    max-width: 560px;
  }
  .exec-log span { line-height: 1.5; }

  /* ============================================================
     Dark mode
     ============================================================ */
  @media (prefers-color-scheme: dark) {
    .card { background: #1e1e1e; border-color: #333; }
    .card:hover { border-color: #7986cb; }
    .card.selected { border-color: #7986cb; background: #1a237e44; }
    .card p { color: #aaa; }
    .steps { background: #252525; }
    .step-circle { background: #444; color: #aaa; }
    .tool-row { border-color: #333; }
    .tool-row.checked { border-color: #7986cb; background: #1a237e44; }
    .tool-row.forced { border-color: #f9a825; background: #3e2b00; }
    .feature-row { border-color: #333; }
    .feature-row.checked { border-color: #7986cb; background: #1a237e44; }
    .summary-row { background: #252525; }
    .context-dump { background: #1a1a1a; }
    .tool-cat { background: #333; color: #aaa; }
    .tooltip { background: #2a2a2a; box-shadow: 0 4px 16px rgba(0,0,0,0.5); }
    .mode-btn { border-color: #444; color: #999; }
    .mode-btn.active { border-color: #7986cb; background: #1a237e44; color: #7986cb; }
    .mode-btn:hover { border-color: #7986cb; color: #7986cb; }
    .analysis-result { background: #252525; }
    .tech-tag { background: #333; color: #ccc; }
    .tech-tag.certain { background: #1b5e20; color: #a5d6a7; }
    .tech-tag.likely { background: #0d47a1; color: #90caf9; }
    .tech-tag.possible { background: #f57f17; color: #fff9c4; }
    .hint-tag { background: #333; color: #ccc; }
    .hint-tag.docker { background: #0d47a1; color: #bbdefb; }
    .exec-step { background: #1e1e1e; }
    .exec-step.active { background: #0d47a144; border-left-color: #42a5f5; }
    .exec-step.failed { background: #b71c1c44; border-left-color: #ef5350; }
    .exec-log { background: #0a0a1a; }
  }
</style>
