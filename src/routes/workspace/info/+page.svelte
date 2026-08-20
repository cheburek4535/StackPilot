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
      error = `Failed to load process count: ${e}`;
    }
  }
</script>

<PageContainer width="wide">
  <PageHeader
    title="Project Information"
    description="Facts about the current workspace project, straight from getCurrentProject()."
    icon="info"
  />

  {#if wsLoading}
    <LoadingState label="Loading workspace…" />
  {:else if wsError}
    <ErrorState title="Failed to load workspace" message={wsError} />
  {:else if !project}
    <EmptyState
      icon="folder"
      title="No project is open"
      description="Open a project to see its details here."
    />
  {:else if error}
    <ErrorState title="Failed to load project info" message={error} />
  {:else}
    <Card variant="elevated" padding="none">
      <div class="sp-info-list">
        <div class="sp-info-row">
          <span class="sp-info-key">Profile name</span>
          <span class="sp-info-val">
            {project.profile_name}
            <Badge tone="violet">current</Badge>
          </span>
        </div>
        <div class="sp-info-row">
          <span class="sp-info-key">Description</span>
          <span class="sp-info-val">{project.description || "—"}</span>
        </div>
        <div class="sp-info-row">
          <span class="sp-info-key">Project path</span>
          <span class="sp-info-val sp-info-mono">{project.project_path ?? "—"}</span>
        </div>
        <div class="sp-info-row">
          <span class="sp-info-key">Opened at</span>
          <span class="sp-info-val">{formatDateTime(project.opened_at)}</span>
        </div>
        <div class="sp-info-row">
          <span class="sp-info-key">Stack</span>
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
          <span class="sp-info-key">Processes</span>
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
