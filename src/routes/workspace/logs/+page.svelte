<script lang="ts">
  import { onDestroy } from "svelte";
  import PageContainer from "$lib/components/ui/PageContainer.svelte";
  import PageHeader from "$lib/components/ui/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import EmptyState from "$lib/components/ui/EmptyState.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import ErrorState from "$lib/components/ui/ErrorState.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import { workspaceContext } from "$lib/modules/workspace/context";
  import { listProcesses, getProcessLogs } from "$lib/modules/workspace/api";
  import type { TrackedProcess, ProcessLogs } from "$lib/modules/workspace/types";
  import {
    statusTone,
    statusLabel,
    statusIcon,
    formatDuration,
  } from "$lib/modules/workspace/status";

  let processes = $state<TrackedProcess[]>([]);
  let dataLoaded = $state(false);
  let error = $state("");

  let selectedId = $state<string | null>(null);
  let logs = $state<ProcessLogs | null>(null);
  let logsLoading = $state(false);
  let logsError = $state("");

  let autoRefreshId: ReturnType<typeof setInterval> | null = null;

  const project = $derived($workspaceContext.project);
  const wsLoading = $derived($workspaceContext.loading);
  const wsError = $derived($workspaceContext.error);

  const selectedProc = $derived(
    processes.find((p) => p.id === selectedId) ?? null,
  );

  $effect(() => {
    if (project && !dataLoaded) {
      dataLoaded = true;
      loadProcesses();
      startAutoRefresh();
    } else if (!project) {
      dataLoaded = false;
      processes = [];
      selectedId = null;
      logs = null;
      stopAutoRefresh();
    }
  });

  function startAutoRefresh() {
    if (autoRefreshId) return;
    autoRefreshId = setInterval(() => refreshSelectedLogs(), 3000);
  }

  function stopAutoRefresh() {
    if (autoRefreshId) {
      clearInterval(autoRefreshId);
      autoRefreshId = null;
    }
  }

  onDestroy(() => {
    stopAutoRefresh();
  });

  async function loadProcesses() {
    try {
      processes = await listProcesses();
    } catch (e) {
      error = `Failed to load processes: ${e}`;
    }
  }

  async function refreshSelectedLogs() {
    if (!selectedId || !selectedProc || selectedProc.status !== "Running") return;
    if (logsLoading) return;
    try {
      logs = await getProcessLogs(selectedId);
    } catch {
      // transient refresh failure — keep the last good logs
    }
  }

  async function selectProcess(id: string) {
    selectedId = id;
    logs = null;
    logsError = "";
    logsLoading = true;
    try {
      logs = await getProcessLogs(id);
    } catch (e) {
      logsError = `Failed to load logs: ${e}`;
    }
    logsLoading = false;
  }
</script>

<PageContainer width="wide">
  <PageHeader
    title="Logs"
    description="Captured output for the processes under this workspace, from getProcessLogs()."
    icon="terminal"
  />

  {#if wsLoading}
    <LoadingState label="Loading workspace…" />
  {:else if wsError}
    <ErrorState title="Failed to load workspace" message={wsError} />
  {:else if !project}
    <EmptyState
      icon="folder"
      title="No project is open"
      description="Open a project to browse its process logs."
    />
  {:else if error}
    <ErrorState title="Failed to load processes" message={error} />
  {:else if processes.length === 0}
    <EmptyState
      icon="terminal"
      title="No processes"
      description="Spawn a process or run a profile first — its output will appear here."
    />
  {:else}
    <div class="sp-logs-layout">
      <div class="sp-proc-panel">
        <h3 class="sp-panel-title">Processes</h3>
        <div class="sp-proc-select">
          {#each processes as p (p.id)}
            <button
              class="sp-proc-btn"
              class:sp-proc-btn-active={selectedId === p.id}
              onclick={() => selectProcess(p.id)}
            >
              <span class="sp-proc-btn-icon" aria-hidden="true">
                <Icon name={statusIcon(p.status)} size={13} />
              </span>
              <span class="sp-proc-btn-label">{p.label}</span>
              <Badge tone={statusTone(p.status)} dot={p.status === "Running"}>
                {statusLabel(p.status)}
              </Badge>
            </button>
          {/each}
        </div>
      </div>

      <Card padding="none" variant="elevated">
        {#if !selectedId}
          <div class="sp-log-hint">Select a process to view its captured output.</div>
        {:else if logsLoading}
          <div class="sp-log-hint">Loading logs…</div>
        {:else if logsError}
          <div class="sp-log-hint sp-log-hint-err">{logsError}</div>
        {:else if logs}
          <div class="sp-log-head">
            <div class="sp-log-head-main">
              <strong class="sp-log-name">{selectedProc?.label ?? selectedId}</strong>
              <span class="sp-log-meta">
                PID {selectedProc?.pid ?? "—"} · {selectedProc
                  ? formatDuration(selectedProc.duration_secs)
                  : ""}
              </span>
            </div>
            {#if selectedProc?.status === "Running"}
              <Badge tone="lime" dot>live</Badge>
            {/if}
          </div>
          <div class="sp-log-view">
            {#if logs.stdout_lines.length === 0 && logs.stderr_lines.length === 0}
              <div class="sp-log-hint">No output captured for this process.</div>
            {:else}
              {#each logs.stdout_lines as line}
                <pre class="sp-log-line sp-log-out">{line}</pre>
              {/each}
              {#each logs.stderr_lines as line}
                <pre class="sp-log-line sp-log-err">{line}</pre>
              {/each}
            {/if}
          </div>
        {:else}
          <div class="sp-log-hint">No logs available.</div>
        {/if}
      </Card>
    </div>
  {/if}
</PageContainer>

<style>
  .sp-logs-layout {
    display: grid;
    grid-template-columns: minmax(16rem, 22rem) 1fr;
    gap: var(--sp-4);
    align-items: start;
  }

  .sp-proc-panel {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-panel-title {
    margin: 0;
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-semibold);
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--sp-text-3);
  }

  .sp-proc-select {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .sp-proc-btn {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    width: 100%;
    padding: var(--sp-2) var(--sp-3);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    background: var(--sp-bg-1);
    color: var(--sp-text-1);
    font-family: var(--sp-font-sans);
    text-align: left;
    cursor: pointer;
    transition:
      background-color 0.15s ease,
      border-color 0.15s ease;
  }

  .sp-proc-btn:hover {
    background: var(--sp-bg-2);
  }

  .sp-proc-btn-active {
    background: var(--sp-accent-soft);
    border-color: var(--sp-accent-border);
  }

  .sp-proc-btn-icon {
    display: inline-flex;
    color: var(--sp-text-2);
    flex-shrink: 0;
  }

  .sp-proc-btn-label {
    flex: 1;
    min-width: 0;
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-medium);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sp-log-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4);
    border-bottom: 1px solid var(--sp-border-faint);
  }

  .sp-log-head-main {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .sp-log-name {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sp-log-meta {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .sp-log-view {
    background: var(--sp-bg-0);
    padding: var(--sp-3);
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    line-height: var(--sp-lh-normal);
    max-height: 30rem;
    overflow-y: auto;
    border-bottom-left-radius: var(--sp-radius-lg);
    border-bottom-right-radius: var(--sp-radius-lg);
  }

  .sp-log-line {
    margin: 0;
    padding: 0;
    white-space: pre-wrap;
    word-break: break-all;
    color: var(--sp-text-2);
  }

  .sp-log-err {
    color: var(--sp-danger);
  }

  .sp-log-hint {
    padding: var(--sp-10) var(--sp-4);
    text-align: center;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
    font-style: italic;
  }

  .sp-log-hint-err {
    color: var(--sp-danger);
  }

  @media (max-width: 760px) {
    .sp-logs-layout {
      grid-template-columns: 1fr;
    }
  }
</style>
