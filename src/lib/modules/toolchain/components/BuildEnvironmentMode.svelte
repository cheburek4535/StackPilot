<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  // Режим «Собрать окружение»: выбор стека → канонический профиль бэкенда
  // → группы инструментов → проверка плана установки. Установки по клику
  // на выбор НЕ происходит — только явное одобрение плана.
  import { onMount } from "svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import ErrorState from "$lib/components/ui/ErrorState.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import { getWizardTree } from "$lib/modules/project_creator/api";
  import type { WizardTreeData } from "$lib/modules/project_creator/types";
  import { toolchain } from "../state.svelte";
  import type { ProjectRequirements, ToolScanResult } from "../types";
  import RequirementPicker from "./RequirementPicker.svelte";
  import ProfileResult from "./ProfileResult.svelte";
  import type { CardPlanOp } from "./ToolCard.svelte";

  let {
    liveStateById = {},
    onplan,
    onopenmanage,
  }: {
    /** Живые состояния инструментов из снапшота (для честных статусов). */
    liveStateById?: Record<string, ToolScanResult>;
    onplan: (operation: CardPlanOp, toolIds: string[]) => void;
    onopenmanage: () => void;
  } = $props();

  // ---- wizard tree (варианты выбора приходят с бэкенда) ----
  let tree = $state<WizardTreeData | null>(null);
  let treeLoading = $state(true);
  let treeError = $state<string | null>(null);

  async function loadTree(): Promise<void> {
    treeLoading = true;
    treeError = null;
    try {
      tree = await getWizardTree();
      if (!tree || tree.languages.length === 0) {
        treeError = i18n.t("tc.market.catalog_empty_desc") as TranslationKey;
      }
    } catch (err) {
      treeError = String(err);
    } finally {
      treeLoading = false;
    }
  }

  onMount(() => {
    void loadTree();
  });

  // ---- выбор ----
  let languages = $state<string[]>([]);
  let frameworks = $state<string[]>([]);
  let tools = $state<string[]>([]);
  let localInfra = $state<string[]>([]);
  let git = $state(true);
  let vscode = $state(false);
  let docker = $state(false);

  const selectionEmpty = $derived(
    languages.length === 0 &&
      frameworks.length === 0 &&
      tools.length === 0 &&
      !git &&
      !vscode &&
      !docker,
  );

  function requirements(): ProjectRequirements {
    return {
      languages: [...languages],
      frameworks: [...frameworks],
      tools: [...tools],
      local_infra_tools: [...localInfra],
      git_init: git,
      vscode_config: vscode,
      docker,
    };
  }

  // ---- дебаунс-резолв профиля на бэкенде ----
  let resolveTimer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    // Отслеживаем весь выбор.
    const req = requirements();
    if (selectionEmpty) {
      if (resolveTimer) clearTimeout(resolveTimer);
      toolchain.profile = null;
      return;
    }
    if (resolveTimer) clearTimeout(resolveTimer);
    resolveTimer = setTimeout(() => {
      void toolchain.loadProfile(req);
    }, 400);
    return () => {
      if (resolveTimer) clearTimeout(resolveTimer);
    };
  });

  function reviewPlan(): void {
    const profile = toolchain.profile;
    if (!profile || profile.required.length === 0) return;
    onplan("install", profile.required.map((t) => t.tool_id));
  }
</script>

<div class="build-mode">
  <!-- ===== Левая колонка: селектор ===== -->
  <div class="picker-col">
    <div class="col-head">
      <h3 class="col-title">{i18n.t("tc.picker.project_type") as TranslationKey}</h3>
      <p class="col-hint">
        {i18n.t("tc.picker.adds_recommended") as TranslationKey}
      </p>
    </div>

    {#if treeLoading}
      <LoadingState label={i18n.t("tc.ui.loading_env") as TranslationKey} />
    {:else if treeError}
      <ErrorState title={i18n.t("tc.market.catalog_empty") as TranslationKey} message={treeError} retry={() => void loadTree()} />
    {:else if tree}
      <RequirementPicker
        {tree}
        bind:languages
        bind:frameworks
        bind:tools
        bind:localInfra
        bind:git
        bind:vscode
        bind:docker
      />
    {/if}
  </div>

  <!-- ===== Правая колонка: результат резолвера ===== -->
  <div class="result-col" aria-live="polite">
    <div class="col-head">
      <h3 class="col-title">{i18n.t("tc.profile.env_profile") as TranslationKey}</h3>
      <p class="col-hint">
        {i18n.t("tc.profile.select_stack") as TranslationKey}
        <button type="button" class="link-btn" onclick={onopenmanage}>
          {i18n.t("tc.hint_manage") as TranslationKey} <Icon name="chevronRight" size={12} />
        </button>
      </p>
    </div>

    {#if toolchain.profileError && toolchain.profile}
      <p class="profile-warn" role="alert">
        <Icon name="alert" size={14} />
        {i18n.t("tc.ui.load_error") as TranslationKey} {toolchain.profileError}
      </p>
    {/if}

    {#if toolchain.profileLoading}
      <LoadingState label={i18n.t("tc.plan.preparing") as TranslationKey} />
    {:else if toolchain.profileError && !toolchain.profile}
      <ErrorState
        title={i18n.t("tc.plan.not_built") as TranslationKey}
        message={toolchain.profileError}
        retry={() => void toolchain.loadProfile(requirements())}
      />
    {:else if toolchain.profile}
      <ProfileResult
        profile={toolchain.profile}
        {liveStateById}
        resolving={toolchain.profileLoading}
        onlocaltoggle={(id, local) => {
          localInfra = local
            ? [...new Set([...localInfra, id])]
            : localInfra.filter((x) => x !== id);
        }}
        onreviewplan={reviewPlan}
      />
    {:else}
      <div class="empty-result">
        <Icon name="sparkles" size={22} />
        <p>{i18n.t("tc.profile.select_stack") as TranslationKey}</p>
        <p class="muted">{i18n.t("tc.profile.recommendations_note") as TranslationKey}</p>
      </div>
    {/if}

    {#if toolchain.profile?.required.length}
      <div class="review-bar">
        <Button variant="primary" icon="check" onclick={reviewPlan}>
          {i18n.t("tc.profile.review_plan") as TranslationKey} ({toolchain.profile.required.length})
        </Button>
      </div>
    {/if}
  </div>
</div>

<style>
  .build-mode {
    display: grid;
    grid-template-columns: minmax(20rem, 26rem) minmax(0, 1fr);
    gap: var(--sp-5);
    align-items: start;
  }

  @media (max-width: 980px) {
    .build-mode {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .col-head {
    margin-bottom: var(--sp-3);
  }

  .col-title {
    margin: 0 0 var(--sp-1);
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .col-hint {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    line-height: var(--sp-lh-normal);
  }

  .link-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    padding: 0;
    border: none;
    background: transparent;
    color: var(--sp-accent);
    font-size: var(--sp-fs-xs);
    cursor: pointer;
  }

  .link-btn:hover {
    text-decoration: underline;
  }

  .empty-result {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-1);
    padding: var(--sp-10) var(--sp-5);
    border: 1px dashed var(--sp-border-strong);
    border-radius: var(--sp-radius-lg);
    color: var(--sp-accent);
    text-align: center;
  }

  .empty-result p {
    margin: 0;
    color: var(--sp-text-2);
    font-size: var(--sp-fs-sm);
  }

  .empty-result .muted {
    color: var(--sp-text-3);
    font-size: var(--sp-fs-xs);
  }

  .profile-warn {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin: 0;
    padding: var(--sp-2) var(--sp-3);
    font-size: var(--sp-fs-xs);
    color: var(--sp-warning);
    border: 1px solid var(--sp-warning-border);
    background: var(--sp-warning-soft);
    border-radius: var(--sp-radius-md);
  }

  .review-bar {
    position: sticky;
    bottom: var(--sp-4);
    display: flex;
    justify-content: flex-end;
    margin-top: var(--sp-4);
  }
</style>
