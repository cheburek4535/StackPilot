<script lang="ts">
  import { onMount } from "svelte";
  import type {
    FileEntry,
    ProjectFilePreview,
    RecipePreview,
    WizardContext,
    ProjectTypeDef,
  } from "$lib/modules/project_creator/types";
  import { previewProjectFiles, previewProjectRecipe } from "$lib/modules/project_creator/api";
  import FileTree from "./FileTree.svelte";
  import FilePreviewPane from "./FilePreviewPane.svelte";
  import GenerationSteps from "./GenerationSteps.svelte";
  import SourcesBlock from "./SourcesBlock.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let {
    selectedType,
    backendLangs,
    frontendLangs,
    selectedFrameworks,
    selectedTools,
    envLocalInfra,
    testing,
    git,
    vscode,
    projectName,
    projectFolder,
    removedStepIds = $bindable([]),
  }: {
    selectedType: ProjectTypeDef | null;
    backendLangs: string[];
    frontendLangs: string[];
    selectedFrameworks: string[];
    selectedTools: string[];
    envLocalInfra: Set<string>;
    testing: boolean;
    git: boolean;
    vscode: boolean;
    projectName: string;
    projectFolder: string;
    removedStepIds?: string[];
  } = $props();

  let filePreview = $state<ProjectFilePreview | null>(null);
  let recipePreview = $state<RecipePreview | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let recipeError = $state<string | null>(null);
  let selectedFile = $state<FileEntry | null>(null);
  let activeTab = $state<"tree" | "steps" | "sources">("tree");

  let loadId = 0;

  function buildCtx(): WizardContext {
    return {
      project_path: null,
      project_name: projectName || "preview",
      is_existing: false,
      project_type: selectedType?.id ?? null,
      languages: [...backendLangs, ...frontendLangs],
      backend_languages: backendLangs,
      frontend_languages: frontendLangs,
      frameworks: selectedFrameworks,
      tools: selectedTools,
      local_infra_tools: [...envLocalInfra],
      features: [],
      infrastructure: [],
      docker: false,
      testing,
      ci: false,
      git_init: git,
      vscode_config: vscode,
      answers: {},
    };
  }

  async function loadPreview() {
    const myId = ++loadId;
    loading = true;
    error = null;
    try {
      const ctx = buildCtx();
      const [filesResult, recipeResult] = await Promise.allSettled([
        previewProjectFiles(ctx, projectFolder || ".", removedStepIds),
        // Шаги генерации — отдельный вызов рецепта (без удалений: список
        // шагов полный, удалённые помечаются в UI и их можно вернуть).
        previewProjectRecipe(ctx, projectFolder || "."),
      ]);
      if (myId !== loadId) return;
      if (filesResult.status === "fulfilled") {
        filePreview = filesResult.value;
      } else {
        error = String(filesResult.reason);
      }
      if (recipeResult.status === "fulfilled") {
        recipePreview = recipeResult.value;
        recipeError = null;
      } else {
        recipePreview = null;
        recipeError = String(recipeResult.reason);
      }
    } catch (e) {
      if (myId !== loadId) return;
      error = String(e);
    } finally {
      if (myId === loadId) loading = false;
    }
  }

  onMount(() => {
    loadPreview();
  });

  $effect(() => {
    const _key = [
      selectedType?.id ?? "",
      JSON.stringify(backendLangs),
      JSON.stringify(frontendLangs),
      JSON.stringify(selectedFrameworks),
      JSON.stringify(selectedTools),
      JSON.stringify([...envLocalInfra]),
      testing, git, vscode,
      projectName, projectFolder,
      JSON.stringify(removedStepIds),
    ].join("|");
    void _key;
    loadPreview();
  });

  function handleSelectFile(entry: FileEntry) {
    selectedFile = entry;
  }

  function handleClosePreview() {
    selectedFile = null;
  }

  let steps = $derived(recipePreview?.step_previews ?? []);
  let removableIds = $derived(filePreview?.removable_step_ids ?? []);
  let summary = $derived(filePreview?.summary ?? null);
  let files = $derived(filePreview?.files ?? []);
</script>

<div class="pp-container">
  {#if loading && !filePreview}
    <div class="pp-loading">
      <div class="pp-spinner"></div>
      <span>{i18n.t("create.preview.loading") as TranslationKey}</span>
    </div>
  {:else if error && !filePreview}
    <div class="pp-error">
      <Icon name="alert" size={16} />
      <span>{error}</span>
      <button class="pp-retry" onclick={loadPreview}>
        {i18n.t("create.preview.retry") as TranslationKey}
      </button>
    </div>
  {:else}
    {#if summary}
      <div class="pp-summary-bar">
        <span class="pp-summary-item pp-summary-certain">
          <Icon name="check" size={12} />
          {summary.certain_count} {i18n.t("create.preview.files_our") as TranslationKey}
        </span>
        <span class="pp-summary-item pp-summary-expected">
          <Icon name="alert" size={12} />
          {summary.expected_count} {i18n.t("create.preview.files_external") as TranslationKey}
        </span>
        <span class="pp-summary-item pp-summary-unknown">
          <Icon name="info" size={12} />
          {summary.unknown_count} {i18n.t("create.preview.files_runtime") as TranslationKey}
        </span>
        {#if summary.dir_count}
          <span class="pp-summary-item pp-summary-dirs">
            <Icon name="folder" size={12} />
            {summary.dir_count}
          </span>
        {/if}
      </div>
    {/if}

    <div class="pp-tabs">
      <button
        class="pp-tab"
        class:pp-tab-active={activeTab === "tree"}
        onclick={() => activeTab = "tree"}
      >
        <Icon name="folder" size={14} />
        {i18n.t("create.preview.tab_files") as TranslationKey}
      </button>
      <button
        class="pp-tab"
        class:pp-tab-active={activeTab === "steps"}
        onclick={() => activeTab = "steps"}
      >
        <Icon name="terminal" size={14} />
        {i18n.t("create.preview.tab_steps") as TranslationKey}
      </button>
      <button
        class="pp-tab"
        class:pp-tab-active={activeTab === "sources"}
        onclick={() => activeTab = "sources"}
      >
        <Icon name="layers" size={14} />
        {i18n.t("create.preview.tab_sources") as TranslationKey}
      </button>
    </div>

    <div class="pp-content">
      {#if activeTab === "tree"}
        {#if files.length === 0}
          <div class="pp-empty-tree">
            <Icon name="folder" size={24} />
            <p>{i18n.t("create.preview.select_stack") as TranslationKey}</p>
            <p class="pp-empty-hint">{i18n.t("create.preview.select_stack_hint") as TranslationKey}</p>
          </div>
        {:else}
          <div class="pp-split">
            <div class="pp-tree-pane" class:pp-tree-pane-wide={selectedFile}>
              <FileTree entries={files} onselect={handleSelectFile} />
            </div>
            {#if selectedFile}
              <div class="pp-preview-pane">
                <FilePreviewPane file={selectedFile} onclose={handleClosePreview} />
              </div>
            {/if}
          </div>
        {/if}
      {:else if activeTab === "steps"}
        {#if recipeError}
          <div class="pp-steps-error">
            <Icon name="alert" size={14} />
            <span>{recipeError}</span>
          </div>
        {:else if steps.length === 0}
          <div class="pp-empty-tree">
            <Icon name="terminal" size={24} />
            <p>{i18n.t("create.preview.steps_empty") as TranslationKey}</p>
            <p class="pp-empty-hint">{i18n.t("create.preview.steps_empty_hint") as TranslationKey}</p>
          </div>
        {:else}
          <div class="pp-tab-scroll">
            <GenerationSteps
              {steps}
              {removableIds}
              bind:removedIds={removedStepIds}
            />
          </div>
        {/if}
      {:else if activeTab === "sources"}
        <div class="pp-tab-scroll">
          <SourcesBlock entries={files} />
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .pp-container {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    min-height: 0;
  }

  .pp-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-3);
    padding: var(--sp-8);
    color: var(--sp-text-3);
    font-size: var(--sp-fs-sm);
  }

  .pp-spinner {
    width: 18px;
    height: 18px;
    border: 2px solid var(--sp-border);
    border-top-color: var(--sp-accent);
    border-radius: 50%;
    animation: pp-spin 0.8s linear infinite;
  }

  @keyframes pp-spin {
    to { transform: rotate(360deg); }
  }

  .pp-error {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-3) var(--sp-4);
    background: rgba(248, 113, 113, 0.08);
    border: 1px solid rgba(248, 113, 113, 0.2);
    border-radius: var(--sp-radius-md);
    color: var(--sp-red);
    font-size: var(--sp-fs-sm);
  }

  .pp-retry {
    margin-left: auto;
    padding: var(--sp-1) var(--sp-3);
    background: none;
    border: 1px solid rgba(248, 113, 113, 0.3);
    border-radius: var(--sp-radius-sm);
    color: var(--sp-red);
    font-size: var(--sp-fs-xs);
    cursor: pointer;
  }

  .pp-retry:hover {
    background: rgba(248, 113, 113, 0.1);
  }

  .pp-summary-bar {
    display: flex;
    gap: var(--sp-4);
    padding: var(--sp-2) var(--sp-3);
    background: var(--sp-bg-2);
    border-radius: var(--sp-radius-sm);
    font-size: var(--sp-fs-xs);
  }

  .pp-summary-item {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
  }

  .pp-summary-certain { color: var(--sp-success); }
  .pp-summary-expected { color: var(--sp-warning); }
  .pp-summary-unknown { color: var(--sp-text-3); }
  .pp-summary-dirs { color: var(--sp-text-3); }

  .pp-tabs {
    display: flex;
    gap: 2px;
    background: var(--sp-bg-2);
    padding: 2px;
    border-radius: var(--sp-radius-sm);
  }

  .pp-tab {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex: 1;
    padding: var(--sp-2) var(--sp-3);
    border: none;
    background: none;
    color: var(--sp-text-3);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-medium);
    cursor: pointer;
    border-radius: var(--sp-radius-xs);
    transition: background 0.1s, color 0.1s;
  }

  .pp-tab:hover {
    color: var(--sp-text-2);
  }

  .pp-tab-active {
    background: var(--sp-bg-1);
    color: var(--sp-text-1);
    box-shadow: var(--sp-shadow-1);
  }

  .pp-content {
    min-height: 0;
    flex: 1;
  }

  .pp-tab-scroll {
    max-height: 320px;
    overflow-y: auto;
    padding-right: 2px;
  }

  .pp-steps-error {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-3) var(--sp-4);
    background: rgba(248, 113, 113, 0.08);
    border: 1px solid rgba(248, 113, 113, 0.2);
    border-radius: var(--sp-radius-sm);
    color: var(--sp-red);
    font-size: var(--sp-fs-sm);
  }

  .pp-split {
    display: flex;
    gap: 2px;
    height: 320px;
    min-height: 0;
  }

  .pp-tree-pane {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    background: var(--sp-bg-2);
    border-radius: var(--sp-radius-sm);
    padding: var(--sp-2);
  }

  .pp-tree-pane-wide {
    flex: 0 0 45%;
  }

  .pp-preview-pane {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    background: var(--sp-bg-2);
    border-radius: var(--sp-radius-sm);
  }

  .pp-empty-tree {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 280px;
    gap: var(--sp-3);
    color: var(--sp-text-3);
    text-align: center;
    padding: var(--sp-8);
  }

  .pp-empty-tree p {
    margin: 0;
    font-size: var(--sp-fs-sm);
  }

  .pp-empty-hint {
    font-size: var(--sp-fs-xs) !important;
    color: var(--sp-text-3);
    max-width: 260px;
  }
</style>
