<script lang="ts">
  import PageContainer from "$lib/components/ui/PageContainer.svelte";
  import PageHeader from "$lib/components/ui/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import EmptyState from "$lib/components/ui/EmptyState.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import ErrorState from "$lib/components/ui/ErrorState.svelte";
  import { workspaceContext } from "$lib/modules/workspace/context";
  import { listProcesses } from "$lib/modules/workspace/api";
  import { formatDateTime } from "$lib/modules/workspace/status";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let processCount = $state<number | null>(null);
  let dataLoaded = $state(false);
  let error = $state("");

  const project = $derived($workspaceContext.project);
  const wsLoading = $derived($workspaceContext.loading);
  const wsError = $derived($workspaceContext.error);

  $effect(() => {
    if (project && !dataLoaded) {
      dataLoaded = true;
      loadProcesses();
    } else if (!project) {
      dataLoaded = false;
      processCount = null;
    }
  });

  async function loadProcesses() {
    try {
      const procs = await listProcesses();
      processCount = procs.length;
    } catch (e) {
      error = `${i18n.t("ws.load_failed")}: ${String(e)}`;
    }
  }
</script>

<PageContainer width="wide">
  <PageHeader
    title={i18n.t("ws.info_title") as TranslationKey}
    description={i18n.t("ws.info_desc") as TranslationKey}
    icon="info"
  />

  {#if wsLoading}
    <LoadingState label={i18n.t("ws.loading_workspace") as TranslationKey} />
  {:else if wsError}
    <ErrorState title={i18n.t("ws.load_failed") as TranslationKey} message={wsError} />
  {:else if !project}
    <EmptyState
      icon="folder"
      title={i18n.t("ws.no_project_open") as TranslationKey}
      description={i18n.t("ws.info_desc") as TranslationKey}
    />
  {:else if error}
    <ErrorState title={i18n.t("ws.info_title") as TranslationKey} message={error} />
  {:else}
    <Card variant="elevated" padding="none">
      <div class="sp-info-list">
        <div class="sp-info-row">
          <span class="sp-info-key">{i18n.t("ws.profile_name") as TranslationKey}</span>
          <span class="sp-info-val">
            {project.profile_name}
            <Badge tone="violet">{i18n.t("devl.current") as TranslationKey}</Badge>
          </span>
        </div>
        <div class="sp-info-row">
          <span class="sp-info-key">{i18n.t("ws.description") as TranslationKey}</span>
          <span class="sp-info-val">{project.description || "—"}</span>
        </div>
        <div class="sp-info-row">
          <span class="sp-info-key">{i18n.t("ws.project_path") as TranslationKey}</span>
          <span class="sp-info-val sp-info-mono">{project.project_path ?? "—"}</span>
        </div>
        <div class="sp-info-row">
          <span class="sp-info-key">{i18n.t("ws.opened_at") as TranslationKey}</span>
          <span class="sp-info-val">{formatDateTime(project.opened_at)}</span>
        </div>
        <div class="sp-info-row">
          <span class="sp-info-key">{i18n.t("ws.stack") as TranslationKey}</span>
          <span class="sp-info-val">
            {#if project.stack.length > 0}
              <span class="sp-tags">
                {#each project.stack as tech}
                  <Badge tone="blue">{tech}</Badge>
                {/each}
              </span>
            {:else}—{/if}
          </span>
        </div>
        <div class="sp-info-row">
          <span class="sp-info-key">{i18n.t("ws.processes") as TranslationKey}</span>
          <span class="sp-info-val">{processCount ?? "—"}</span>
        </div>
      </div>
    </Card>
  {/if}
</PageContainer>

<style>
  .sp-info-list {
    display: flex;
    flex-direction: column;
  }

  .sp-info-row {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    padding: var(--sp-4) var(--sp-5);
    border-bottom: 1px solid var(--sp-border-faint);
  }

  .sp-info-row:last-child {
    border-bottom: none;
  }

  .sp-info-key {
    flex: 0 0 8rem;
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-medium);
    color: var(--sp-text-3);
  }

  .sp-info-val {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-1);
    word-break: break-all;
  }

  .sp-info-mono {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
  }

  .sp-tags {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-wrap: wrap;
  }
</style>
