<script lang="ts">
  import PageContainer from "$lib/components/ui/PageContainer.svelte";
  import PageHeader from "$lib/components/ui/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import EmptyState from "$lib/components/ui/EmptyState.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import ErrorState from "$lib/components/ui/ErrorState.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import { workspaceContext } from "$lib/modules/workspace/context";
  import { listProcesses } from "$lib/modules/workspace/api";
  import type { TrackedProcess } from "$lib/modules/workspace/types";
  import { statusLabel, formatStarted } from "$lib/modules/workspace/status";

  let processes = $state<TrackedProcess[]>([]);
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
      processes = [];
    }
  });

  async function loadProcesses() {
    try {
      processes = await listProcesses();
    } catch (e) {
      error = `Failed to load processes: ${e}`;
    }
  }

  // Problems are derived client-side from real process state (the backend
  // get_problems command has no frontend wrapper — contract E).
  const problems = $derived(
    processes.filter((p) => {
      if (p.status === "Crashed") return true;
      if (
        typeof p.status === "object" &&
        "Exited" in p.status &&
        p.status.Exited !== 0
      )
        return true;
      if (p.last_error) return true;
      return false;
    }),
  );

  function problemReason(p: TrackedProcess): string {
    if (p.last_error) return p.last_error;
    if (p.status === "Crashed") return "Process crashed.";
    if (typeof p.status === "object" && "Exited" in p.status) {
      return `Process exited with code ${p.status.Exited}.`;
    }
    return "Process reported a problem.";
  }
</script>

<PageContainer width="wide">
  <PageHeader
    title="Problems"
    description="Issues derived from the real process state under this workspace (backend get_problems has no frontend wrapper, so this is computed from listProcesses())."
    icon="alert"
  />

  {#if wsLoading}
    <LoadingState label="Loading workspace…" />
  {:else if wsError}
    <ErrorState title="Failed to load workspace" message={wsError} />
  {:else if !project}
    <EmptyState
      icon="folder"
      title="No project is open"
      description="Open a project to review its process issues."
    />
  {:else if error}
    <ErrorState title="Failed to load processes" message={error} />
  {:else if problems.length === 0}
    <Card variant="elevated" padding="lg">
      <div class="sp-clear">
        <span class="sp-clear-icon" aria-hidden="true">
          <Icon name="check" size={22} />
        </span>
        <div>
          <h3 class="sp-clear-title">No problems detected</h3>
          <p class="sp-clear-desc">
            None of the {processes.length} tracked process
            {processes.length === 1 ? "" : "es"} is crashed, failed or reported
            an error.
          </p>
        </div>
      </div>
    </Card>
  {:else}
    <div class="sp-problem-list">
      {#each problems as p (p.id)}
        <Card padding="md">
          <div class="sp-problem">
            <div class="sp-problem-head">
              <span class="sp-problem-icon" aria-hidden="true">
                <Icon name="alert" size={15} />
              </span>
              <strong class="sp-problem-label">{p.label}</strong>
              <Badge tone="red">{statusLabel(p.status)}</Badge>
            </div>
            <div class="sp-problem-meta">
              <span>PID <code>{p.pid}</code></span>
              <span class="sp-sep">·</span>
              <span>started {formatStarted(p.started_at)}</span>
              {#if p.restarts > 0}
                <span class="sp-sep">·</span>
                <span>restarts {p.restarts}</span>
              {/if}
            </div>
            <div class="sp-problem-error">{problemReason(p)}</div>
          </div>
        </Card>
      {/each}
    </div>
  {/if}
</PageContainer>

<style>
  .sp-clear {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
  }

  .sp-clear-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 3rem;
    height: 3rem;
    border-radius: var(--sp-radius-full);
    color: var(--sp-success);
    background: rgba(163, 230, 53, 0.12);
    border: 1px solid rgba(163, 230, 53, 0.3);
    flex-shrink: 0;
  }

  .sp-clear-title {
    margin: 0;
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-clear-desc {
    margin: var(--sp-1) 0 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
  }

  .sp-problem-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .sp-problem {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-problem-head {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .sp-problem-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: var(--sp-radius-sm);
    color: var(--sp-danger);
    background: rgba(248, 113, 113, 0.12);
    flex-shrink: 0;
  }

  .sp-problem-label {
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sp-problem-meta {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-wrap: wrap;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
  }

  .sp-problem-meta code {
    font-size: var(--sp-fs-xs);
    background: var(--sp-code-bg);
    padding: 0.05rem 0.35rem;
    border-radius: var(--sp-radius-xs);
    color: var(--sp-text-1);
  }

  .sp-sep {
    color: var(--sp-text-3);
  }

  .sp-problem-error {
    font-size: var(--sp-fs-xs);
    color: var(--sp-danger);
    background: rgba(248, 113, 113, 0.12);
    border: 1px solid rgba(248, 113, 113, 0.25);
    padding: var(--sp-2) var(--sp-3);
    border-radius: var(--sp-radius-sm);
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 6rem;
    overflow-y: auto;
  }
</style>
