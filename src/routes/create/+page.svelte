<script lang="ts">
  import { onMount } from "svelte";
  import { getWizardTree, startWizard, submitWizardAnswer } from "$lib/modules/project_creator/api";
  import type {
    WizardTreeData,
    ProjectTypeDef,
    LanguageDef,
    FrameworkDef,
    ToolDef,
    WizardSession,
  } from "$lib/modules/project_creator/types";

  let tree = $state<WizardTreeData | null>(null);
  let status = $state<string>("loading");
  let session = $state<WizardSession | null>(null);

  // Local selections for each step
  let selectedType = $state<ProjectTypeDef | null>(null);
  let selectedLangs = $state<string[]>([]);
  let selectedFrameworks = $state<string[]>([]);
  let selectedTools = $state<string[]>([]);
  let docker = $state(true);
  let testing = $state(true);
  let git = $state(true);
  let vscode = $state(true);

  let step = $state(0);
  const STEP_NAMES = ["Type", "Language", "Framework", "Tools", "Review"];

  onMount(async () => {
    try {
      tree = await getWizardTree();
      status = tree.project_types.length > 0 ? "ready" : "empty";
    } catch (e) {
      status = "error";
      console.error(e);
    }
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
    if (selectedFrameworks.length === 0) return;
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
    if (selectedTools.includes(id)) {
      selectedTools = selectedTools.filter((t) => t !== id);
    } else {
      selectedTools = [...selectedTools, id];
    }
  }

  function confirmTools() {
    step = 4;
  }

  // ---- Step 4: Confirm ----
  async function confirmAll() {
    if (!session) {
      let s = await startWizard();
      s = await submitWizardAnswer(s, "project_type", [selectedType!.id]);
      s = await submitWizardAnswer(s, "languages", selectedLangs);
      s = await submitWizardAnswer(s, "frameworks", selectedFrameworks);
      s = await submitWizardAnswer(s, "tools", [...selectedTools, ...(docker ? ["docker"] : []), ...(testing ? ["testing"] : [])]);
      s = await submitWizardAnswer(s, "confirm", []);
      session = s;
    }
  }

  function resetAll() {
    session = null;
    selectedType = null;
    selectedLangs = [];
    selectedFrameworks = [];
    selectedTools = [];
    docker = true;
    testing = true;
    git = true;
    vscode = true;
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
        <button class="btn-primary" onclick={confirmFrameworks} disabled={selectedFrameworks.length === 0}>
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
            <label class="tool-row" class:checked={selectedTools.includes(tool.id)}>
              <input type="checkbox" checked={selectedTools.includes(tool.id)} onchange={() => toggleTool(tool.id)} />
              <span class="tool-icon">{toolCategoryIcon(tool.category)}</span>
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
        <label class="feature-row" class:checked={docker}>
          <input type="checkbox" bind:checked={docker} />
          <span class="feature-info">
            <span class="feature-name">Docker</span>
            <span class="feature-desc">Containerization with Dockerfile + compose</span>
          </span>
        </label>
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
          <span class="summary-value">{[docker && "Docker", testing && "Testing", git && "Git", vscode && "VS Code"].filter(Boolean).join(", ") || "none"}</span>
          <button class="btn-edit" onclick={() => { step = 3; }}>change</button>
        </div>
      </div>
      <div class="actions">
        <button class="btn-back" onclick={back}>← Back</button>
        <button class="btn-primary" onclick={confirmAll}>Create Project ✓</button>
      </div>
    {/if}

    <!-- Done -->
    {#if session?.is_complete}
      <div class="done">
        <span class="done-icon">✓</span>
        <h2>Project configured!</h2>
        <p class="muted">Context ready for recipe execution (future milestone).</p>
        <pre class="context-dump">{JSON.stringify(session.context, null, 2)}</pre>
        <button class="btn-back" onclick={resetAll}>Create another project</button>
      </div>
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
    .feature-row { border-color: #333; }
    .feature-row.checked { border-color: #7986cb; background: #1a237e44; }
    .summary-row { background: #252525; }
    .context-dump { background: #1a1a1a; }
    .tool-cat { background: #333; color: #aaa; }
  }
</style>
