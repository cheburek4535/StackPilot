<script lang="ts">
  import { createProjectStore, setProjectStore } from "$lib/modules/project_creator/createStore.svelte";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import PageContainer from "$lib/components/ui/PageContainer.svelte";
  import PageHeader from "$lib/components/ui/PageHeader.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import ErrorState from "$lib/components/ui/ErrorState.svelte";
  import EmptyState from "$lib/components/ui/EmptyState.svelte";
  import WizardConstructor from "$lib/components/project-creator/WizardConstructor.svelte";

  const store = createProjectStore();
  setProjectStore(store);
</script>

<PageContainer width="wide">
  <PageHeader
    title={i18n.t("create.title") as TranslationKey}
    description="Мастер создания проектов: подбор архитектуры, фреймворков и инструментов запуска"
    icon="sparkles"
  >
    {#snippet actions()}
      <div class="sp-mode-switch" role="tablist">
        <button
          type="button"
          role="tab"
          aria-selected={store.mode === "constructor"}
          class="sp-mode-btn"
          class:active={store.mode === "constructor"}
          onclick={() => { store.mode = "constructor"; }}
        >
          <Icon name="layers" size={14} />
          <span>{i18n.t("create.mode.constructor") as TranslationKey}</span>
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={store.mode === "presets"}
          class="sp-mode-btn"
          class:active={store.mode === "presets"}
          onclick={() => { store.mode = "presets"; }}
        >
          <Icon name="bookmark" size={14} />
          <span>{i18n.t("create.mode.templates") as TranslationKey}</span>
        </button>
      </div>
    {/snippet}
  </PageHeader>

  {#if store.status === "loading"}
    <LoadingState label={i18n.t("create.init") as TranslationKey} />
  {:else if store.status === "error"}
    <ErrorState
      title={i18n.t("create.init_failed") as TranslationKey}
      message="Не удалось загрузить данные шаблонов и фреймворков"
    />
  {:else if store.status === "empty"}
    <EmptyState
      icon="sparkles"
      title={i18n.t("create.no_types") as TranslationKey}
      description="Типы проектов не найдены в конфигурации"
    />
  {:else}
    {#if store.mode === "analyze"}
      {#if store.AnalyzeMode}
        <store.AnalyzeMode
          analyzing={store.analyzing}
          analyzedPath={store.analyzedPath}
          analysisError={store.analysisError}
          analysisResult={store.analysisResult}
          onrun={store.runAnalysis}
          onapply={store.applyAnalysis}
        />
      {/if}
    {:else if store.mode === "presets"}
      {#if store.PresetsMode}
        <store.PresetsMode tree={store.tree} onapply={store.applyPreset} />
      {/if}
    {:else}
      {#if store.phase === 5 || store.phase === 6}
        <!-- Environment check & install -->
        {#if store.phase === 5}
          {#if store.EnvPanel}
            <store.EnvPanel
              tree={store.tree}
              envCheck={store.envCheck}
              envChecking={store.envChecking}
              envCheckProgress={store.envCheckProgress}
              envSelectedIds={store.envSelectedIds}
              envLocalInfra={store.envLocalInfra}
              envPlan={store.envPlan}
              envInstalling={store.envInstalling}
              envInstallDone={store.envInstallDone}
              envErrors={store.envErrors}
              envLogs={store.envLogs}
              envTaskStates={store.envTaskStates}
              envRestartHint={store.envRestartHint}
              envDownload={store.envDownload}
              envSpeed={store.envSpeed}
              envPhaseStart={store.envPhaseStart}
              envNow={store.envNow}
              envRechecking={store.envRechecking}
              envError={store.envError}
              newSecrets={store.newSecrets}
              secretCopied={store.secretCopied}
              installedTools={store.installedTools}
              envIsolationMode={store.envIsolationMode}
              onchangeIsolationMode={(m: "isolated" | "global") => (store.envIsolationMode = m)}
              ontoggleEnvTool={store.toggleEnvTool}
              onselectAll={store.selectAllEnvTools}
              onoptInLocalInfra={store.optInLocalInfra}
              onrevertLocalInfra={store.revertLocalInfra}
              onstartInstall={store.startInstall}
              oncancelInstall={store.cancelInstall}
              onrecheck={store.recheckEnvironment}
              oncheck={() => store.runEnvironmentCheck()}
              oncontinue={store.doCreateProject}
              onback={store.back}
              onbackReview={() => (store.phase = 2)}
              oncopySecret={store.copySecret}
              ondismissSecrets={() => (store.newSecrets = null)}
            />
          {/if}
        {/if}

        <!-- Execution -->
        {#if store.phase === 6}
          {#if store.ExecPanel}
            <store.ExecPanel
              execPlan={store.execPlan}
              execProjectPath={store.execProjectPath}
              execStatuses={store.execStatuses}
              execOverallStatus={store.execOverallStatus}
              execResult={store.execResult}
              execError={store.execError}
              execLogs={store.execLogs}
              devlProfileExists={store.devlProfileExists}
              oncancel={store.cancelExecution}
              onreset={store.resetAll}
              onopenvscode={store.openInVSCode}
              onreopendevl={store.reopenDevlDialog}
            />
          {/if}
        {/if}
      {:else}
        <WizardConstructor />
      {/if}
    {/if}
  {/if}
</PageContainer>

{#if store.DevlDialogs}
  <store.DevlDialogs
    created={store.devlProfileCreated}
    showReminder={store.devlShowReminder}
    confirmCancel={store.devlConfirmCancel}
    onopen={store.openDevLauncher}
    onclose={store.dismissProfileOk}
    oncancel={store.cancelProfile}
    onconfirmcancel={store.confirmCancelProfile}
    ondismisscancel={store.dismissCancelConfirm}
  />
{/if}

<style>
  .sp-mode-switch {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 3px;
    background: var(--sp-bg-2);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.2);
  }

  .sp-mode-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    padding: 0.35rem 0.85rem;
    border: 1px solid transparent;
    border-radius: var(--sp-radius-md);
    background: transparent;
    color: var(--sp-text-3);
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-medium);
    font-family: inherit;
    cursor: pointer;
    transition: all 0.15s ease;
    white-space: nowrap;
  }

  .sp-mode-btn:hover {
    color: var(--sp-text-1);
    background: rgba(255, 255, 255, 0.04);
  }

  .sp-mode-btn.active {
    background: var(--sp-bg-3);
    border-color: var(--sp-border-strong);
    color: var(--sp-text-1);
    font-weight: var(--sp-fw-semibold);
    box-shadow: var(--sp-shadow-1);
  }
</style>
